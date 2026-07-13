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

use futures::SinkExt;
use serde::Deserialize;
use voice_input::{RealtimeVoiceStream, VoiceInput};
use warp_core::channel::ChannelState;
use warpui::{Entity, ModelContext, SingletonEntity};
use websocket::{Message, WebSocket, WebsocketMessage};

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
}

impl RealtimeVoice {
    pub fn new_singleton(_ctx: &mut ModelContext<Self>) -> Self {
        Self { active: false }
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    /// Begin streaming mic audio to the harness and relaying transcription/turn
    /// events. No-op if already active. Emits `Failed` if the mic stream or the
    /// connection can't be started.
    pub fn start(&mut self, ctx: &mut ModelContext<Self>) {
        if self.active {
            return;
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

        let auth_client = ServerApiProvider::as_ref(ctx).get_auth_client();
        let url = realtime_ws_url();
        ctx.spawn(
            async move {
                let token = auth_client
                    .get_or_refresh_access_token()
                    .await
                    .ok()
                    .and_then(|token| token.bearer_token());
                let headers: Vec<(&str, String)> = token
                    .map(|token| ("authorization", format!("Bearer {token}")))
                    .into_iter()
                    .collect();
                let socket = WebSocket::connect_with_headers(&url, None::<&str>, headers).await?;
                anyhow::Ok(socket.split().await)
            },
            move |me, connection, ctx| match connection {
                Ok((sink, stream)) => me.on_connected(audio, sink, stream, ctx),
                Err(e) => {
                    log::warn!("realtime voice: connect failed: {e:?}");
                    me.active = false;
                    VoiceInput::handle(ctx).update(ctx, |voice, _| voice.stop_streaming());
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
        VoiceInput::handle(ctx).update(ctx, |voice, _| voice.stop_streaming());
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
                VoiceInput::handle(ctx).update(ctx, |voice, _| voice.stop_streaming());
                ctx.emit(RealtimeVoiceEvent::Closed);
            },
        );

        // Outbound: drain PCM frames -> binary WS frames. Ends when streaming
        // stops (channel closes), then says goodbye + closes the socket.
        ctx.spawn(
            async move {
                let mut sink = sink;
                while let Some(frame) = audio.next_frame().await {
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
                VoiceInput::handle(ctx).update(ctx, |voice, _| voice.stop_streaming());
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

// Derive the harness Realtime WS URL from the configured server root
// (http(s)://host -> ws(s)://host/ai/realtime).
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
