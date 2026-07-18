//! Client side of the hands-free Realtime voice pipeline.
//!
//! Streams mic PCM16 to the harness `/ai/realtime` WebSocket proxy (which fronts
//! an OpenAI Realtime transcription session) and surfaces the harness's
//! simplified events as [`RealtimeVoiceEvent`]s. The terminal `Input` subscribes
//! to these to drive the composer, the puck, and (on turn end) auto-submit.
//!
//! The OpenAI key never touches the client; the harness holds it. If the harness
//! can't establish the session (not configured / connect error), we emit
//! [`RealtimeVoiceEvent::Failed`] so the overlay can fall back to the chunked
//! Whisper path.

use std::sync::Arc;

use futures::SinkExt;
use serde::Deserialize;
use settings::Setting;
use voice_input::{RealtimeVoiceStream, VoiceInput};
use warp_core::channel::ChannelState;
use warpui::{Entity, ModelContext, SingletonEntity};
use websocket::{Message, WebSocket, WebsocketMessage};

use crate::server::server_api::auth::AuthClient;
use crate::server::server_api::ServerApiProvider;

/// Events surfaced from the harness Realtime proxy.
#[derive(Debug, Clone)]
pub enum RealtimeVoiceEvent {
    /// The upstream session is configured and ready; audio is flowing.
    Ready,
    /// VAD detected the user started speaking.
    SpeechStarted,
    /// VAD detected the user stopped speaking.
    SpeechStopped,
    /// Incremental transcript text for the current segment.
    TranscriptDelta(String),
    /// The current segment's final transcript.
    TranscriptDone(String),
    /// A segment/turn completed (auto-submit cue).
    TurnEnd,
    /// The session could not be established; caller should fall back.
    Failed,
    /// The session closed.
    Closed,
}

#[derive(Deserialize)]
struct ServerEvent {
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    text: Option<String>,
}

/// Singleton driving the client Realtime voice session.
pub struct RealtimeVoice {
    active: bool,
    /// Whether the current session is sourced from the macOS echo-cancelled
    /// (AEC) capture rather than plain cpal streaming. When true, hands-free
    /// barge-in is available: the mic can stay live while the agent's answer is
    /// read aloud. When false, the overlay keeps the suppress-during-TTS +
    /// tap-to-interrupt behavior.
    aec: bool,
}

impl RealtimeVoice {
    pub fn new_singleton(_ctx: &mut ModelContext<Self>) -> Self {
        Self {
            active: false,
            aec: false,
        }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Whether the active session uses echo-cancelled capture (hands-free
    /// barge-in available). Always false off macOS or when AEC couldn't start.
    pub fn is_aec(&self) -> bool {
        self.aec
    }

    /// Begin streaming mic audio to the harness and relaying transcription/turn
    /// events. No-op if already active. Emits `Failed` if the mic stream or the
    /// connection can't be started.
    pub fn start(&mut self, ctx: &mut ModelContext<Self>) {
        if self.active {
            return;
        }

        // The OS VoiceProcessing capture (below) produces silence on some audio
        // devices, so it's opt-in via `BANG_OVERLAY_AEC=1`. Hands-free barge-in
        // instead uses software AEC on the cpal mic (see `platform_mac`), which
        // is device-independent — so the default path is plain cpal streaming.
        #[cfg(target_os = "macos")]
        if std::env::var("BANG_OVERLAY_AEC").as_deref() == Ok("1") {
            if let Some(stream) = super::platform_mac::start_aec_capture() {
                self.active = true;
                self.aec = false;
                self.connect_with_aec(stream, ctx);
                return;
            }
            log::warn!("realtime voice: OS AEC unavailable; using cpal");
        }

        let audio =
            match VoiceInput::handle(ctx).update(ctx, |voice, ctx| voice.start_streaming(ctx)) {
                Ok(audio) => audio,
                Err(e) => {
                    log::warn!("realtime voice: failed to start mic streaming: {e:?}");
                    ctx.emit(RealtimeVoiceEvent::Failed);
                    return;
                }
            };
        self.active = true;
        self.aec = false;
        self.connect(audio, ctx);
    }

    /// Connect the harness WebSocket for an already-started audio source and,
    /// on success, begin relaying transcription/turn events.
    fn connect(&mut self, audio: RealtimeVoiceStream, ctx: &mut ModelContext<Self>) {
        let auth_client = ServerApiProvider::as_ref(ctx).get_auth_client();
        let language = crate::settings::AISettings::as_ref(ctx)
            .voice_overlay_language
            .value()
            .clone();
        let url = realtime_ws_url_with_language(&language);
        ctx.spawn(
            async move {
                let socket = connect_ws(auth_client, &url).await?;
                anyhow::Ok(socket.split().await)
            },
            move |me, connection, ctx| match connection {
                Ok((sink, stream)) => {
                    if !me.active {
                        return;
                    }
                    log::info!("realtime voice: session connected (aec=false)");
                    me.on_connected(audio, sink, stream, ctx);
                }
                Err(e) => {
                    log::warn!("realtime voice: connect failed: {e:?}");
                    me.active = false;
                    me.stop_audio_source(ctx);
                    ctx.emit(RealtimeVoiceEvent::Failed);
                }
            },
        );
    }

    /// Connect for the opt-in echo-cancelled capture: confirm the native tap is
    /// actually delivering audio before committing, otherwise fall back to cpal
    /// streaming (started in the completion handler where we have `ctx`).
    #[cfg(target_os = "macos")]
    fn connect_with_aec(&mut self, aec_stream: RealtimeVoiceStream, ctx: &mut ModelContext<Self>) {
        let auth_client = ServerApiProvider::as_ref(ctx).get_auth_client();
        let language = crate::settings::AISettings::as_ref(ctx)
            .voice_overlay_language
            .value()
            .clone();
        let url = realtime_ws_url_with_language(&language);
        ctx.spawn(
            async move {
                let aec_stream = confirm_aec_stream(Some(aec_stream)).await;
                let socket = match connect_ws(auth_client, &url).await {
                    Ok(socket) => socket,
                    Err(e) => {
                        if aec_stream.is_some() {
                            super::platform_mac::stop_aec_capture();
                        }
                        return Err(e);
                    }
                };
                let (sink, stream) = socket.split().await;
                anyhow::Ok((aec_stream, sink, stream))
            },
            move |me, connection, ctx| match connection {
                Ok((aec_stream, sink, stream)) => {
                    if !me.active {
                        if aec_stream.is_some() {
                            super::platform_mac::stop_aec_capture();
                        }
                        return;
                    }
                    let audio = if let Some(stream) = aec_stream {
                        me.aec = true;
                        stream
                    } else {
                        me.aec = false;
                        match VoiceInput::handle(ctx)
                            .update(ctx, |voice, ctx| voice.start_streaming(ctx))
                        {
                            Ok(audio) => audio,
                            Err(e) => {
                                log::warn!("realtime voice: failed to start mic streaming: {e:?}");
                                me.active = false;
                                ctx.emit(RealtimeVoiceEvent::Failed);
                                return;
                            }
                        }
                    };
                    log::info!("realtime voice: session connected (aec={})", me.aec);
                    me.on_connected(audio, sink, stream, ctx);
                }
                Err(e) => {
                    log::warn!("realtime voice: connect failed: {e:?}");
                    me.active = false;
                    me.stop_audio_source(ctx);
                    ctx.emit(RealtimeVoiceEvent::Failed);
                }
            },
        );
    }

    /// Stop the session: stop mic streaming (which closes the PCM channel and, in
    /// turn, the outbound loop + WebSocket).
    pub fn stop(&mut self, ctx: &mut ModelContext<Self>) {
        if !self.active {
            return;
        }
        self.active = false;
        self.stop_audio_source(ctx);
    }

    /// Stop whichever audio source is feeding the session. Always tears down the
    /// macOS AEC engine (idempotent — covers the case where a capture was
    /// started but not yet committed via `self.aec`), and stops cpal streaming
    /// when that's the source. Clears the `aec` flag so the next session
    /// re-negotiates the source.
    fn stop_audio_source(&mut self, ctx: &mut ModelContext<Self>) {
        #[cfg(target_os = "macos")]
        super::platform_mac::stop_aec_capture();
        if !self.aec {
            VoiceInput::handle(ctx).update(ctx, |voice, _| voice.stop_streaming());
        }
        self.aec = false;
    }

    fn on_connected(
        &mut self,
        audio: RealtimeVoiceStream,
        sink: impl websocket::Sink,
        stream: impl websocket::Stream,
        ctx: &mut ModelContext<Self>,
    ) {
        // Inbound: harness JSON events -> RealtimeVoiceEvent (main thread).
        ctx.spawn_stream_local(
            stream,
            |me, message, ctx| {
                if let Ok(message) = message {
                    me.on_ws_message(&message, ctx);
                }
            },
            |me, ctx| {
                me.active = false;
                me.stop_audio_source(ctx);
                ctx.emit(RealtimeVoiceEvent::Closed);
            },
        );

        // Outbound: drain PCM frames -> binary WS frames. Ends when streaming
        // stops (channel closes), then says goodbye + closes the socket.
        ctx.spawn(
            async move {
                let mut sink = sink;
                while let Some(frame) = audio.next_frame().await {
                    // Echo-cancel the mic against the currently-playing answer
                    // (no-op unless an answer is being spoken), so the
                    // transcriber's VAD only hears the user → hands-free barge-in.
                    #[cfg(target_os = "macos")]
                    let frame = super::platform_mac::aec_process_capture(&frame);
                    if sink.send(Message::new_binary(frame)).await.is_err() {
                        break;
                    }
                }
                let _ = sink
                    .send(Message::new(r#"{"type":"bye"}"#.to_string()))
                    .await;
                let _ = sink.close().await;
            },
            |_, _, _| {},
        );
    }

    fn on_ws_message(&mut self, message: &Message, ctx: &mut ModelContext<Self>) {
        let Some(text) = message.text() else {
            return;
        };
        let Ok(event) = serde_json::from_str::<ServerEvent>(text) else {
            return;
        };
        match event.kind.as_str() {
            "ready" => ctx.emit(RealtimeVoiceEvent::Ready),
            "speech.started" => ctx.emit(RealtimeVoiceEvent::SpeechStarted),
            "speech.stopped" => ctx.emit(RealtimeVoiceEvent::SpeechStopped),
            "transcript.delta" => ctx.emit(RealtimeVoiceEvent::TranscriptDelta(
                event.text.unwrap_or_default(),
            )),
            "transcript.done" => ctx.emit(RealtimeVoiceEvent::TranscriptDone(
                event.text.unwrap_or_default(),
            )),
            "turn.end" => ctx.emit(RealtimeVoiceEvent::TurnEnd),
            "error" => {
                log::warn!("realtime voice: upstream error");
                self.active = false;
                self.stop_audio_source(ctx);
                ctx.emit(RealtimeVoiceEvent::Failed);
            }
            _ => {}
        }
    }
}

impl Entity for RealtimeVoice {
    type Event = RealtimeVoiceEvent;
}

impl SingletonEntity for RealtimeVoice {}

/// Confirm the macOS AEC capture is actually delivering audio before we rely on
/// it. Some devices/OS versions start the engine but never fire the input tap;
/// in that case we tear it down and return `None` so the caller falls back to
/// plain cpal streaming (hands-free barge-in is then unavailable, but dictation
/// works). Waits up to ~1.2s for the first frame.
#[cfg(target_os = "macos")]
async fn confirm_aec_stream(stream: Option<RealtimeVoiceStream>) -> Option<RealtimeVoiceStream> {
    let stream = stream?;
    // Require frames that carry actual signal: a working mic always has a
    // non-zero noise floor, whereas a broken voice-processing capture emits
    // exact digital zero. Waiting for signal (not just frame arrival) means we
    // never commit to a silent capture. Wait up to ~1.5s.
    for _ in 0..30 {
        if super::platform_mac::aec_has_frames() && super::platform_mac::aec_peak() > 0 {
            return Some(stream);
        }
        warpui::r#async::Timer::after(std::time::Duration::from_millis(50)).await;
    }
    log::warn!(
        "realtime voice: AEC produced no signal in ~1.5s (peak {}); falling back to cpal",
        super::platform_mac::aec_peak()
    );
    super::platform_mac::stop_aec_capture();
    None
}

// Open the harness Realtime WebSocket with a Bearer token (the OpenAI key stays
// server-side; we only forward our access token).
async fn connect_ws(auth_client: Arc<dyn AuthClient>, url: &str) -> anyhow::Result<WebSocket> {
    let token = auth_client
        .get_or_refresh_access_token()
        .await
        .ok()
        .and_then(|token| token.bearer_token());
    let headers: Vec<(&str, String)> = token
        .map(|token| ("authorization", format!("Bearer {token}")))
        .into_iter()
        .collect();
    WebSocket::connect_with_headers(url, None::<&str>, headers).await
}

// Derive the harness Realtime WS URL from the configured server root
// (http(s)://host -> ws(s)://host/ai/realtime), optionally pinning the
// transcription language via a `?language=` query param (empty = auto-detect).
fn realtime_ws_url_with_language(language: &str) -> String {
    let base = realtime_ws_url();
    let language = language.trim();
    if language.is_empty() {
        base
    } else {
        format!("{base}?language={language}")
    }
}

fn realtime_ws_url() -> String {
    let root = ChannelState::server_root_url();
    let root = root.trim_end_matches('/');
    let ws = if let Some(rest) = root.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = root.strip_prefix("http://") {
        format!("ws://{rest}")
    } else {
        root.to_string()
    };
    format!("{ws}/ai/realtime")
}
