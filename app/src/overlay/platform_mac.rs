//! macOS implementations of the overlay platform traits.
//!
//! The native puck (`native/overlay_puck.m`) is driven through the FFI below.
//! Puck clicks come back into the app via `bang_overlay_puck_clicked`, which
//! bridges from the AppKit main thread into an `AppContext` using a `WeakApp`
//! captured at startup (see `install_overlay_app_bridge`).

use std::cell::RefCell;
use std::ffi::CString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

#[cfg(feature = "voice_input")]
use voice_input::RealtimeVoiceStream;
use voice_tts::{Aec, EspeakPhonemizer, PiperEngine, PiperTts, TtsEngine, WebRtcAec};
use warpui::{SingletonEntity, WeakApp};

use super::platform::OverlayWindow;

// Implemented in app/src/overlay/native/overlay_puck.m (compiled by build.rs).
// All calls must happen on the main thread.
extern "C" {
    fn bang_overlay_puck_show();
    fn bang_overlay_puck_hide();
    fn bang_overlay_puck_set_listening(listening: bool);
    fn bang_overlay_puck_set_paused(paused: bool);
    fn bang_overlay_puck_set_level(level: f64);
    fn bang_overlay_puck_set_thinking(thinking: bool);
    fn bang_overlay_box_show();
    fn bang_overlay_box_hide();
    fn bang_overlay_box_set_history(utf8: *const std::os::raw::c_char);
    fn bang_overlay_box_set_input(utf8: *const std::os::raw::c_char);
    fn bang_overlay_box_set_bg(r: f64, g: f64, b: f64, a: f64);
    fn bang_overlay_box_set_auto_submit(on: bool);
    fn bang_overlay_set_voice_enabled(on: bool);
    fn bang_overlay_set_puck_color(index: i32);
    fn bang_overlay_set_language(utf8: *const std::os::raw::c_char);
    fn bang_overlay_set_verbosity(level: i32);
    fn bang_tts_play_wav(bytes: *const u8, len: usize);
    fn bang_tts_stop();
    fn bang_overlay_canvas_show();
    fn bang_overlay_canvas_hide();
}

// Implemented in app/src/overlay/native/aec_capture.m (only compiled when the
// `voice_input` feature is enabled — see build.rs). Echo-cancelled mic capture
// for hands-free barge-in; `start` returns false when unavailable.
#[cfg(feature = "voice_input")]
extern "C" {
    // Returns a BANG_AEC_* status: 0 = ok, non-zero = reason it couldn't start
    // (kept in sync with the enum in aec_capture.m).
    fn bang_aec_start() -> i32;
    fn bang_aec_stop();
    fn bang_aec_last_error() -> *const std::os::raw::c_char;
}

/// The last native AEC failure detail (NSError text), or empty. Surfaced because
/// this process's `NSLog` isn't captured by unified logging.
#[cfg(feature = "voice_input")]
fn aec_last_error() -> String {
    // SAFETY: returns a pointer to a static NUL-terminated buffer owned by the
    // ObjC side; valid for the duration of this call.
    let ptr = unsafe { bang_aec_last_error() };
    if ptr.is_null() {
        return String::new();
    }
    unsafe { std::ffi::CStr::from_ptr(ptr) }
        .to_string_lossy()
        .into_owned()
}

/// Human-readable reason for a non-zero `bang_aec_start` status (mirrors the
/// `BANG_AEC_*` enum in `aec_capture.m`), for diagnostics in the Rust log.
#[cfg(feature = "voice_input")]
fn aec_start_reason(code: i32) -> &'static str {
    match code {
        1 => "unsupported macOS (< 10.15)",
        2 => "voice processing unavailable",
        3 => "invalid input format",
        4 => "audio converter init failed",
        5 => "audio engine failed to start",
        _ => "unknown error",
    }
}

/// Sender for echo-cancelled PCM16 frames produced by the native AEC tap. Held
/// here (rather than a thread-local) because `bang_aec_frame` is invoked on a
/// realtime audio thread, not the AppKit main thread. Dropping the sender (on
/// [`stop_aec_capture`]) closes the channel and ends the Realtime stream.
#[cfg(feature = "voice_input")]
static AEC_TX: Mutex<Option<async_channel::Sender<Vec<u8>>>> = Mutex::new(None);

/// Set once the native tap delivers its first frame. Lets the Realtime pipeline
/// confirm the AEC capture is actually producing audio before committing to it
/// (some devices start the engine but never fire the tap), falling back to cpal
/// otherwise. Reset when a capture starts.
#[cfg(feature = "voice_input")]
static AEC_GOT_FRAME: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Max absolute PCM16 amplitude seen since the last [`start_aec_capture`]. A
/// broken voice-processing capture emits exact digital zero, while a working mic
/// always has a non-zero noise floor — so the Realtime watchdog treats "frames
/// arrived but peak stayed 0" as silent capture and falls back to cpal.
#[cfg(feature = "voice_input")]
static AEC_MAX_PEAK: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// Whether the native AEC tap has delivered at least one frame since the last
/// [`start_aec_capture`]. Used by the Realtime watchdog fallback.
#[cfg(feature = "voice_input")]
pub fn aec_has_frames() -> bool {
    AEC_GOT_FRAME.load(std::sync::atomic::Ordering::Relaxed)
}

/// Max absolute PCM16 amplitude observed since capture started (0 = pure
/// digital silence, i.e. the capture isn't really working).
#[cfg(feature = "voice_input")]
pub fn aec_peak() -> u32 {
    AEC_MAX_PEAK.load(std::sync::atomic::Ordering::Relaxed)
}

/// Diagnostic: periodically log the peak amplitude of the echo-cancelled PCM16
/// frames so we can tell (from `warp-oss.log`) whether the AEC capture is
/// actually carrying audio vs. silence. Rate-limited to ~every 50 frames.
#[cfg(feature = "voice_input")]
fn aec_log_signal(frame: &[u8]) {
    use std::sync::atomic::{AtomicU32, Ordering};
    static FRAMES: AtomicU32 = AtomicU32::new(0);
    static PEAK: AtomicU32 = AtomicU32::new(0);

    let mut peak: u32 = 0;
    for pair in frame.chunks_exact(2) {
        let sample = i16::from_le_bytes([pair[0], pair[1]]);
        peak = peak.max(sample.unsigned_abs() as u32);
    }
    PEAK.fetch_max(peak, Ordering::Relaxed);
    // Cumulative peak (not reset per window) so the watchdog can tell real
    // capture from digital silence.
    AEC_MAX_PEAK.fetch_max(peak, Ordering::Relaxed);
    let count = FRAMES.fetch_add(1, Ordering::Relaxed) + 1;
    if count.is_multiple_of(50) {
        let window_peak = PEAK.swap(0, Ordering::Relaxed);
        log::info!(
            "overlay AEC: {count} frames captured, recent peak amplitude {window_peak}/32767"
        );
    }
}

/// Called from `aec_capture.m` on a realtime audio thread with one 24kHz mono
/// PCM16 LE frame. Must stay cheap + non-blocking: it copies the bytes and
/// non-blockingly pushes them into the Realtime stream channel.
///
/// # Safety
/// `bytes` points to `len` valid bytes for the duration of the call.
#[cfg(feature = "voice_input")]
#[no_mangle]
pub extern "C-unwind" fn bang_aec_frame(bytes: *const u8, len: usize) {
    if bytes.is_null() || len == 0 {
        return;
    }
    // SAFETY: caller guarantees `len` readable bytes at `bytes` for this call.
    let frame = unsafe { std::slice::from_raw_parts(bytes, len) }.to_vec();
    AEC_GOT_FRAME.store(true, std::sync::atomic::Ordering::Relaxed);
    aec_log_signal(&frame);
    if let Ok(guard) = AEC_TX.lock() {
        if let Some(tx) = guard.as_ref() {
            // Drop the frame if the consumer is behind rather than block audio.
            let _ = tx.try_send(frame);
        }
    }
}

/// Start echo-cancelled mic capture and return a stream of 24kHz mono PCM16 LE
/// frames for the Realtime pipeline, or `None` if voice processing / the audio
/// engine can't be set up (caller then falls back to plain cpal streaming).
#[cfg(feature = "voice_input")]
pub fn start_aec_capture() -> Option<RealtimeVoiceStream> {
    let (tx, rx) = async_channel::unbounded::<Vec<u8>>();
    AEC_GOT_FRAME.store(false, std::sync::atomic::Ordering::Relaxed);
    AEC_MAX_PEAK.store(0, std::sync::atomic::Ordering::Relaxed);
    {
        let mut guard = AEC_TX.lock().ok()?;
        *guard = Some(tx);
    }
    // SAFETY: FFI to the AEC helper; called on the main thread.
    let status = unsafe { bang_aec_start() };
    if status != 0 {
        log::warn!(
            "overlay AEC: capture unavailable ({}: {}); using cpal — no hands-free barge-in",
            aec_start_reason(status),
            aec_last_error()
        );
        if let Ok(mut guard) = AEC_TX.lock() {
            *guard = None;
        }
        return None;
    }
    log::info!("overlay AEC: capture engine started");
    Some(RealtimeVoiceStream::from_receiver(rx))
}

/// Stop echo-cancelled mic capture and close the frame channel.
#[cfg(feature = "voice_input")]
pub fn stop_aec_capture() {
    // SAFETY: FFI to the AEC helper; called on the main thread.
    unsafe { bang_aec_stop() }
    if let Ok(mut guard) = AEC_TX.lock() {
        *guard = None;
    }
}

thread_local! {
    /// Main-thread handle back into the app, used by the puck click callback.
    /// `WeakApp` is `Rc`-based, hence thread-local rather than a `static`.
    static OVERLAY_APP: RefCell<Option<WeakApp>> = const { RefCell::new(None) };
}

/// Lazily-loaded local neural TTS voice (Piper libritts_r + eSpeak phonemizer).
/// Loaded once on first spoken answer; `None` if the model/phonemizer can't be
/// located (we then simply don't speak). Immutable after init — synthesis uses
/// interior locking — so a `OnceLock` is sufficient.
static TTS_ENGINE: OnceLock<Option<PiperEngine<EspeakPhonemizer>>> = OnceLock::new();

/// Preload the TTS voice model off the main thread (e.g. when the overlay
/// opens) so the first spoken answer doesn't pause while the ~75MB model loads.
/// Safe to call repeatedly; the `OnceLock` init runs at most once.
pub fn preload_tts() {
    std::thread::spawn(|| {
        let _ = tts_engine();
    });
}

fn tts_engine() -> Option<&'static PiperEngine<EspeakPhonemizer>> {
    TTS_ENGINE
        .get_or_init(|| match load_tts_engine() {
            Ok(engine) => Some(engine),
            Err(e) => {
                log::warn!("overlay TTS: voice unavailable ({e}); answers won't be read aloud");
                None
            }
        })
        .as_ref()
}

fn load_tts_engine() -> anyhow::Result<PiperEngine<EspeakPhonemizer>> {
    let dir = resolve_voice_dir()
        .ok_or_else(|| anyhow::anyhow!("could not locate voice model dir (voice.onnx)"))?;
    let started = instant::Instant::now();
    let mut tts = PiperTts::from_paths(dir.join("voice.onnx"), dir.join("voice.onnx.json"))?;
    let speaker = std::env::var("BANG_VOICE_SPEAKER")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(0);
    tts.set_speaker(speaker);
    let voice = tts.espeak_voice().to_string();
    let espeak = resolve_espeak();
    log::info!(
        "overlay TTS: loaded voice from {dir:?} (speaker {speaker}, espeak {espeak:?}) in {:?}",
        started.elapsed()
    );
    Ok(PiperEngine::new(tts, EspeakPhonemizer::new(espeak, voice)))
}

/// Locate the directory holding `voice.onnx` (+ `.json`): explicit override,
/// then the app bundle's `Resources/voices`, then the dev source tree.
fn resolve_voice_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("BANG_VOICE_DIR") {
        let dir = PathBuf::from(dir);
        if dir.join("voice.onnx").exists() {
            return Some(dir);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            // Bundled: Bang.app/Contents/MacOS/bang -> ../Resources/voices
            let bundled = parent.join("../Resources/voices");
            if bundled.join("voice.onnx").exists() {
                return Some(bundled);
            }
        }
    }
    // Dev: app crate source tree.
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/voices");
    if dev.join("voice.onnx").exists() {
        return Some(dev);
    }
    None
}

/// Locate the `espeak-ng` phonemizer binary: explicit override, common dev
/// install locations, the app bundle helper, else rely on `PATH`.
fn resolve_espeak() -> PathBuf {
    if let Ok(bin) = std::env::var("BANG_ESPEAK_BIN") {
        return PathBuf::from(bin);
    }
    for candidate in ["/opt/homebrew/bin/espeak-ng", "/usr/local/bin/espeak-ng"] {
        if Path::new(candidate).exists() {
            return PathBuf::from(candidate);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let helper = parent.join("../Resources/espeak-ng");
            if helper.exists() {
                return helper;
            }
        }
    }
    PathBuf::from("espeak-ng")
}

// ===== Hands-free barge-in: software acoustic echo cancellation =====
//
// While the agent's answer plays, we feed the played PCM as the AEC far-end
// reference and echo-cancel the mic before it reaches the Realtime transcriber,
// so the transcriber's VAD fires only on the user — enabling barge-in on
// speakers, unlike the OS VoiceProcessing path. The mic capture stays 24kHz
// (for the transcriber); the AEC resamples internally to 48kHz.

/// The shared echo canceller (lazily created). Accessed from the mic outbound
/// loop and the reference-feed thread, hence a `Mutex`.
static AEC: Mutex<Option<WebRtcAec>> = Mutex::new(None);
/// Whether an answer is currently being spoken (mic should be echo-cancelled).
static AEC_ENGAGED: AtomicBool = AtomicBool::new(false);
/// Bumped on each new spoken answer / stop so stale reference-feed threads exit.
static AEC_EPOCH: AtomicU32 = AtomicU32::new(0);
/// Processed-frame counter used to rate-limit AEC stats logging. Only read from
/// the (voice_input-gated) `aec_process_capture`.
#[cfg(feature = "voice_input")]
static AEC_FRAME_COUNT: AtomicU32 = AtomicU32::new(0);

const AEC_MIC_RATE: u32 = 24_000;
const AEC_REFERENCE_RATE: u32 = 22_050; // libritts_r model rate

fn with_aec<R>(f: impl FnOnce(&mut WebRtcAec) -> R) -> Option<R> {
    let mut guard = AEC.lock().ok()?;
    if guard.is_none() {
        match WebRtcAec::new(AEC_MIC_RATE, AEC_REFERENCE_RATE) {
            Ok(aec) => *guard = Some(aec),
            Err(e) => {
                log::warn!("overlay AEC: init failed ({e}); no hands-free barge-in");
                return None;
            }
        }
    }
    guard.as_mut().map(f)
}

/// Echo-cancel one 24kHz PCM16 mic frame against the currently-playing answer.
/// Passthrough when no answer is being spoken (AEC not engaged), so normal
/// dictation is never altered. Called from the Realtime outbound loop.
#[cfg(feature = "voice_input")]
pub fn aec_process_capture(frame: &[u8]) -> Vec<u8> {
    if !AEC_ENGAGED.load(Ordering::Relaxed) {
        return frame.to_vec();
    }
    let mic: Vec<f32> = frame
        .chunks_exact(2)
        .map(|b| i16::from_le_bytes([b[0], b[1]]) as f32 / 32768.0)
        .collect();
    let result = with_aec(|aec| {
        let cleaned = aec.process_capture(&mic);
        (cleaned, aec.stats())
    });
    match result {
        Some((cleaned, stats)) => {
            // Periodically log echo-cancellation effectiveness so we can tell
            // on-device whether the AEC is removing the agent's voice (high ERLE
            // = good; residual_echo near 1 = leaking → would self-interrupt).
            let n = AEC_FRAME_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
            if n.is_multiple_of(50) {
                log::info!(
                    "overlay AEC: ERLE={:?}dB ERL={:?}dB delay={:?}ms residual_echo={:?}",
                    stats.erle.map(|v| (v * 10.0).round() / 10.0),
                    stats.erl.map(|v| (v * 10.0).round() / 10.0),
                    stats.delay_ms,
                    stats.residual_echo.map(|v| (v * 100.0).round() / 100.0),
                );
            }
            let mut out = Vec::with_capacity(cleaned.len() * 2);
            for s in cleaned {
                let v = (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
                out.extend_from_slice(&v.to_le_bytes());
            }
            out
        }
        None => frame.to_vec(),
    }
}

/// Begin feeding a spoken answer's PCM as the AEC far-end reference, paced to
/// real time so it stays roughly aligned with playback (AEC3's delay estimator
/// absorbs the residual). Engages echo cancellation for the answer's duration
/// plus a short acoustic tail.
fn engage_aec_reference(samples: Vec<f32>, reference_rate: u32) {
    let epoch = AEC_EPOCH.fetch_add(1, Ordering::SeqCst) + 1;
    with_aec(|aec| aec.reset());
    AEC_ENGAGED.store(true, Ordering::Relaxed);
    std::thread::spawn(move || {
        // Feed the reference at real-time *rate* by pacing to a monotonic clock
        // rather than fixed `sleep(10ms)` chunks: `sleep` overshoots, so fixed
        // chunks fall progressively behind actual playback and the AEC loses
        // lock (which caused the self-interruption). The fixed start offset
        // (audio HAL / playback startup latency) is absorbed by AEC3's delay
        // estimator; only the rate needs to match.
        let start = instant::Instant::now();
        let rate = reference_rate as f64;
        let total = samples.len();
        let mut fed = 0usize;
        while fed < total {
            if AEC_EPOCH.load(Ordering::SeqCst) != epoch {
                return; // superseded by a newer answer or a stop
            }
            let elapsed = start.elapsed().as_secs_f64();
            let target = ((elapsed * rate) as usize).min(total);
            if target > fed {
                let frame: Vec<f32> = samples[fed..target].to_vec();
                with_aec(|aec| aec.push_reference(&frame));
                fed = target;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        // Let the acoustic tail flush, then disengage if still current.
        std::thread::sleep(std::time::Duration::from_millis(250));
        if AEC_EPOCH.load(Ordering::SeqCst) == epoch {
            AEC_ENGAGED.store(false, Ordering::Relaxed);
        }
    });
}

/// Stop echo cancellation immediately (answer interrupted or finished).
fn disengage_aec() {
    AEC_EPOCH.fetch_add(1, Ordering::SeqCst);
    AEC_ENGAGED.store(false, Ordering::Relaxed);
}

/// Capture a `WeakApp` so native puck clicks can re-enter app context. Call once
/// at startup on the main thread.
pub fn install_overlay_app_bridge(weak_app: WeakApp) {
    OVERLAY_APP.with(|slot| *slot.borrow_mut() = Some(weak_app));
}

/// Called from `overlay_puck.m` when a puck is clicked (not dragged). `kind` is
/// 0 for the mic puck (pause/resume) and 1 for the submit puck.
///
/// # Safety
/// Invoked by AppKit on the main thread. Must not be called re-entrantly while
/// an `AppContext` borrow is held (AppKit dispatches these between frames, as
/// with the global-hotkey/menu callbacks).
#[no_mangle]
pub extern "C-unwind" fn bang_overlay_puck_clicked(kind: i32) {
    let weak = OVERLAY_APP.with(|slot| slot.borrow().clone());
    let Some(weak) = weak else {
        return;
    };
    let Some(mut app) = weak.upgrade() else {
        return;
    };
    app.update(|ctx| {
        let input = super::OverlayController::handle(ctx)
            .as_ref(ctx)
            .active_input();
        let Some(input) = input.and_then(|weak| weak.upgrade(ctx)) else {
            return;
        };
        input.update(ctx, |input, ctx| match kind {
            1 => input.overlay_submit(ctx),
            2 => input.overlay_start_annotation(ctx),
            _ => input.overlay_toggle_pause(ctx),
        });
    });
}

/// Called from `overlay_puck.m` when the user presses Done in the annotation
/// canvas. `(x, y, w, h)` is the picked window's screen rect (CoreGraphics
/// global, top-left origin, points) to hand to `screencapture`.
///
/// # Safety
/// Invoked by AppKit on the main thread (see `bang_overlay_puck_clicked`).
#[no_mangle]
pub extern "C-unwind" fn bang_overlay_canvas_done(x: f64, y: f64, w: f64, h: f64) {
    let weak = OVERLAY_APP.with(|slot| slot.borrow().clone());
    let Some(weak) = weak else {
        return;
    };
    let Some(mut app) = weak.upgrade() else {
        return;
    };
    app.update(|ctx| {
        let input = super::OverlayController::handle(ctx)
            .as_ref(ctx)
            .active_input();
        let Some(input) = input.and_then(|weak| weak.upgrade(ctx)) else {
            return;
        };
        input.update(ctx, |input, ctx| {
            input.overlay_finish_annotation(x, y, w, h, ctx)
        });
    });
}

/// Called from `overlay_puck.m` when the user cancels the annotation canvas
/// (Cancel button, or Done with nothing drawn).
///
/// # Safety
/// Invoked by AppKit on the main thread (see `bang_overlay_puck_clicked`).
#[no_mangle]
pub extern "C-unwind" fn bang_overlay_canvas_cancel() {
    let weak = OVERLAY_APP.with(|slot| slot.borrow().clone());
    let Some(weak) = weak else {
        return;
    };
    let Some(mut app) = weak.upgrade() else {
        return;
    };
    app.update(|ctx| {
        let input = super::OverlayController::handle(ctx)
            .as_ref(ctx)
            .active_input();
        let Some(input) = input.and_then(|weak| weak.upgrade(ctx)) else {
            return;
        };
        input.update(ctx, |input, ctx| input.overlay_cancel_annotation(ctx));
    });
}

/// Called from `overlay_puck.m` when the user edits the result box (dictation
/// mode). Routes the new text back into the composer.
///
/// # Safety
/// `utf8` is a valid NUL-terminated C string owned by AppKit for the duration of
/// the call; invoked on the main thread.
#[no_mangle]
pub extern "C-unwind" fn bang_overlay_box_edited(utf8: *const std::os::raw::c_char) {
    if utf8.is_null() {
        return;
    }
    // SAFETY: caller guarantees a valid NUL-terminated string for this call.
    let text = match unsafe { std::ffi::CStr::from_ptr(utf8) }.to_str() {
        Ok(text) => text.to_owned(),
        Err(_) => return,
    };
    let weak = OVERLAY_APP.with(|slot| slot.borrow().clone());
    let Some(weak) = weak else {
        return;
    };
    let Some(mut app) = weak.upgrade() else {
        return;
    };
    app.update(|ctx| {
        let input = super::OverlayController::handle(ctx)
            .as_ref(ctx)
            .active_input();
        let Some(input) = input.and_then(|weak| weak.upgrade(ctx)) else {
            return;
        };
        input.update(ctx, |input, ctx| input.overlay_box_edited(&text, ctx));
    });
}

/// Always-on-top circular puck windows (see `overlay_puck.m`).
#[derive(Default)]
pub struct MacOverlayWindow {
    visible: bool,
}

impl OverlayWindow for MacOverlayWindow {
    fn show(&mut self) {
        self.visible = true;
        // SAFETY: FFI to the puck helper; safe on the main thread, which is where
        // the overlay controller runs.
        unsafe { bang_overlay_puck_show() }
    }

    fn hide(&mut self) {
        self.visible = false;
        // SAFETY: see `show`.
        unsafe { bang_overlay_puck_hide() }
    }

    fn is_visible(&self) -> bool {
        self.visible
    }

    fn set_listening(&mut self, listening: bool) {
        // SAFETY: see `show`.
        unsafe { bang_overlay_puck_set_listening(listening) }
    }

    fn set_paused(&mut self, paused: bool) {
        // SAFETY: see `show`.
        unsafe { bang_overlay_puck_set_paused(paused) }
    }

    fn set_level(&mut self, level: f32) {
        // SAFETY: see `show`.
        unsafe { bang_overlay_puck_set_level(level as f64) }
    }

    fn set_thinking(&mut self, thinking: bool) {
        // SAFETY: see `show`.
        unsafe { bang_overlay_puck_set_thinking(thinking) }
    }

    fn show_result_box(&mut self) {
        // SAFETY: see `show`.
        unsafe { bang_overlay_box_show() }
    }

    fn hide_result_box(&mut self) {
        // SAFETY: see `show`.
        unsafe { bang_overlay_box_hide() }
    }

    fn set_history_text(&mut self, text: &str) {
        // `NSString stringWithUTF8String` needs a NUL-terminated C string; strip
        // any interior NULs so `CString::new` can't fail.
        let sanitized = text.replace('\0', "");
        if let Ok(c) = CString::new(sanitized) {
            // SAFETY: `c` outlives the call; ObjC copies the string.
            unsafe { bang_overlay_box_set_history(c.as_ptr()) }
        }
    }

    fn set_input_text(&mut self, text: &str) {
        let sanitized = text.replace('\0', "");
        if let Ok(c) = CString::new(sanitized) {
            // SAFETY: `c` outlives the call; ObjC copies the string.
            unsafe { bang_overlay_box_set_input(c.as_ptr()) }
        }
    }

    fn set_box_background(&mut self, r: f32, g: f32, b: f32, a: f32) {
        // SAFETY: see `show`.
        unsafe { bang_overlay_box_set_bg(r as f64, g as f64, b as f64, a as f64) }
    }

    fn set_box_auto_submit(&mut self, on: bool) {
        // SAFETY: see `show`.
        unsafe { bang_overlay_box_set_auto_submit(on) }
    }

    fn speak(&mut self, text: &str) {
        let Some(engine) = tts_engine() else {
            return;
        };
        let text = text.replace('\0', "");
        if text.trim().is_empty() {
            return;
        }
        match engine.synthesize(&text) {
            Ok(pcm) => {
                // Feed this answer's PCM as the AEC far-end reference so the mic
                // is echo-cancelled while it plays (hands-free barge-in).
                engage_aec_reference(pcm.samples.clone(), pcm.sample_rate);
                let wav = pcm.to_wav_bytes();
                // SAFETY: `bang_tts_play_wav` copies the bytes into an NSData
                // before returning, so `wav` can drop after the call.
                unsafe { bang_tts_play_wav(wav.as_ptr(), wav.len()) }
            }
            Err(e) => log::warn!("overlay TTS: synthesis failed: {e}"),
        }
    }

    fn stop_speaking(&mut self) {
        disengage_aec();
        // SAFETY: see `show`.
        unsafe { bang_tts_stop() }
    }

    fn set_voice_enabled(&mut self, on: bool) {
        // SAFETY: see `show`.
        unsafe { bang_overlay_set_voice_enabled(on) }
    }

    fn set_puck_color(&mut self, index: i32) {
        // SAFETY: see `show`.
        unsafe { bang_overlay_set_puck_color(index) }
    }

    fn set_language(&mut self, code: &str) {
        let sanitized = code.replace('\0', "");
        if let Ok(c) = CString::new(sanitized) {
            // SAFETY: `c` outlives the call; ObjC copies the string.
            unsafe { bang_overlay_set_language(c.as_ptr()) }
        }
    }

    fn set_verbosity(&mut self, level: u8) {
        // SAFETY: plain scalar FFI call; clamped native-side too.
        unsafe { bang_overlay_set_verbosity(level.min(10) as i32) }
    }

    fn begin_annotation(&mut self) {
        // Hide the pucks + result box so they aren't captured, then show the
        // full-screen drawing canvas above everything.
        // SAFETY: see `show`.
        unsafe {
            bang_overlay_puck_hide();
            bang_overlay_box_hide();
            bang_overlay_canvas_show();
        }
    }

    fn end_annotation(&mut self) {
        // SAFETY: see `show`.
        unsafe {
            bang_overlay_canvas_hide();
            bang_overlay_puck_show();
            bang_overlay_box_show();
        }
    }
}

/// Called from `overlay_puck.m` when the box's auto-submit toggle is clicked.
/// Flips the persisted setting and pushes the new state back to the toggle.
///
/// # Safety
/// Invoked by AppKit on the main thread (see `bang_overlay_puck_clicked`).
#[no_mangle]
pub extern "C-unwind" fn bang_overlay_auto_submit_clicked() {
    use settings::ToggleableSetting;
    let weak = OVERLAY_APP.with(|slot| slot.borrow().clone());
    let Some(weak) = weak else {
        return;
    };
    let Some(mut app) = weak.upgrade() else {
        return;
    };
    app.update(|ctx| {
        let on = match crate::settings::AISettings::handle(ctx).update(ctx, |settings, ctx| {
            settings
                .voice_overlay_auto_submit
                .toggle_and_save_value(ctx)
        }) {
            Ok(value) => value,
            Err(_) => *crate::settings::AISettings::as_ref(ctx).voice_overlay_auto_submit,
        };
        super::OverlayController::handle(ctx)
            .update(ctx, |controller, _| controller.set_box_auto_submit(on));
    });
}

/// Called from `overlay_puck.m` when the settings popover's "Read answers aloud"
/// switch is toggled. Flips the persisted setting and pushes the new state back
/// to the switch. When turned off, any in-progress speech is stopped and, if we
/// were holding off on listening to read the answer, listening resumes.
///
/// # Safety
/// Invoked by AppKit on the main thread (see `bang_overlay_puck_clicked`).
#[no_mangle]
pub extern "C-unwind" fn bang_overlay_voice_toggled() {
    use settings::ToggleableSetting;
    let weak = OVERLAY_APP.with(|slot| slot.borrow().clone());
    let Some(weak) = weak else {
        return;
    };
    let Some(mut app) = weak.upgrade() else {
        return;
    };
    app.update(|ctx| {
        let on = match crate::settings::AISettings::handle(ctx).update(ctx, |settings, ctx| {
            settings
                .voice_overlay_tts_enabled
                .toggle_and_save_value(ctx)
        }) {
            Ok(value) => value,
            Err(_) => *crate::settings::AISettings::as_ref(ctx).voice_overlay_tts_enabled,
        };
        super::OverlayController::handle(ctx).update(ctx, |controller, _| {
            controller.set_voice_enabled(on);
            if !on {
                controller.stop_speaking();
            }
        });
        if !on {
            // Stopping speech fires the native didCancel (not didFinish), so the
            // resume callback won't run; resume listening here if we were paused
            // to read the answer. `overlay_tts_finished` is a no-op otherwise.
            let input = super::OverlayController::handle(ctx)
                .as_ref(ctx)
                .active_input();
            if let Some(input) = input.and_then(|weak| weak.upgrade(ctx)) {
                input.update(ctx, |input, ctx| input.overlay_tts_finished(ctx));
            }
        }
    });
}

/// Called from `overlay_puck.m` when a color swatch in the settings popover is
/// chosen. Persists the preset index and pushes it back to the native pucks.
///
/// # Safety
/// Invoked by AppKit on the main thread (see `bang_overlay_puck_clicked`).
#[no_mangle]
pub extern "C-unwind" fn bang_overlay_puck_color_clicked(index: i32) {
    let weak = OVERLAY_APP.with(|slot| slot.borrow().clone());
    let Some(weak) = weak else {
        return;
    };
    let Some(mut app) = weak.upgrade() else {
        return;
    };
    use settings::Setting;
    let idx = index.max(0) as usize;
    app.update(|ctx| {
        let _ = crate::settings::AISettings::handle(ctx).update(ctx, |settings, ctx| {
            settings.voice_overlay_puck_color.set_value(idx, ctx)
        });
        super::OverlayController::handle(ctx)
            .update(ctx, |controller, _| controller.set_puck_color(index));
    });
}

/// Called from `overlay_puck.m` when a language is chosen in the settings
/// popover. Persists the ISO code (empty = auto-detect) and reconnects the
/// transcription session so the new language takes effect immediately.
///
/// # Safety
/// `utf8` is a valid NUL-terminated C string for the call; invoked by AppKit on
/// the main thread (see `bang_overlay_puck_clicked`).
#[no_mangle]
pub extern "C-unwind" fn bang_overlay_language_selected(utf8: *const std::os::raw::c_char) {
    let code = if utf8.is_null() {
        String::new()
    } else {
        // SAFETY: caller guarantees a valid NUL-terminated string for this call.
        match unsafe { std::ffi::CStr::from_ptr(utf8) }.to_str() {
            Ok(s) => s.to_owned(),
            Err(_) => return,
        }
    };
    let weak = OVERLAY_APP.with(|slot| slot.borrow().clone());
    let Some(weak) = weak else {
        return;
    };
    let Some(mut app) = weak.upgrade() else {
        return;
    };
    use settings::Setting;
    app.update(|ctx| {
        let _ = crate::settings::AISettings::handle(ctx).update(ctx, |settings, ctx| {
            settings.voice_overlay_language.set_value(code.clone(), ctx)
        });
        // Reconnect the live transcription session (if any) to apply the new
        // language; a paused/idle session will pick it up on the next start.
        #[cfg(feature = "voice_input")]
        {
            let input = super::OverlayController::handle(ctx)
                .as_ref(ctx)
                .active_input();
            if let Some(input) = input.and_then(|weak| weak.upgrade(ctx)) {
                input.update(ctx, |input, ctx| input.overlay_restart_listening(ctx));
            }
        }
    });
}

/// Called from `overlay_puck.m` when the verbosity slider (0-10) changes in the
/// settings popover. Persists it to the shared `agent_verbosity` setting so it
/// takes effect on the next agent request (same setting as Settings > AI).
///
/// # Safety
/// Invoked by AppKit on the main thread (see `bang_overlay_puck_clicked`).
#[no_mangle]
pub extern "C-unwind" fn bang_overlay_verbosity_selected(level: i32) {
    let level = level.clamp(0, 10) as usize;
    let weak = OVERLAY_APP.with(|slot| slot.borrow().clone());
    let Some(weak) = weak else {
        return;
    };
    let Some(mut app) = weak.upgrade() else {
        return;
    };
    use settings::Setting;
    app.update(|ctx| {
        let _ = crate::settings::AISettings::handle(ctx).update(ctx, |settings, ctx| {
            settings.agent_verbosity.set_value(level, ctx)
        });
    });
}

/// Called from `tts.m` when an utterance finishes on its own. Resumes overlay
/// listening (we suppress it while speaking so the mic doesn't hear the TTS).
///
/// # Safety
/// Invoked by AppKit on the main thread (see `bang_overlay_puck_clicked`).
#[no_mangle]
pub extern "C-unwind" fn bang_tts_did_finish() {
    let weak = OVERLAY_APP.with(|slot| slot.borrow().clone());
    let Some(weak) = weak else {
        return;
    };
    let Some(mut app) = weak.upgrade() else {
        return;
    };
    app.update(|ctx| {
        let input = super::OverlayController::handle(ctx)
            .as_ref(ctx)
            .active_input();
        let Some(input) = input.and_then(|weak| weak.upgrade(ctx)) else {
            return;
        };
        input.update(ctx, |input, ctx| input.overlay_tts_finished(ctx));
    });
}
