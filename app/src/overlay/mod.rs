//! Floating voice + annotation overlay for Bang (macOS).
//!
//! A system-wide, always-on-top "puck" launched from a lightning button (and a
//! global hotkey). While open it continuously transcribes speech into the chat;
//! a hotkey drops into a full-screen freeform-pencil mode where the user circles
//! something on any app, speaks, and an annotated screenshot plus the transcript
//! is sent to the agent.
//!
//! # Architecture
//!
//! Everything routes through the [`AgentSink`] seam so the overlay never touches
//! IDE internals directly, and all native macOS work sits behind the platform
//! traits in [`platform`]. The [`OverlayController`] owns the session state
//! machine and is the only piece that knows the whole flow.
//!
//! This module is built in-place under `app/src/overlay/` (hybrid packaging);
//! once the shape stabilizes it is intended to be extracted into a dedicated
//! `bang_overlay` crate, leaving only the `ExistingChatAdapter` in the app.
//!
//! Gated at runtime by [`warp_features::FeatureFlag::VoiceOverlay`].

use warpui::{Entity, ModelContext, SingletonEntity, WeakViewHandle};

use crate::terminal::input::Input;

pub mod platform;
pub mod sink;

#[cfg(feature = "voice_input")]
pub mod realtime;

#[cfg(target_os = "macos")]
mod platform_mac;
#[cfg(target_os = "macos")]
pub use platform_mac::{install_overlay_app_bridge, preload_tts};

pub use platform::{
    Annotator, GlobalPointer, NoopAnnotator, NoopGlobalPointer, NoopOverlayWindow, NoopPermissions,
    NoopScreenCapturer, OverlayPermissions, OverlayWindow, PermissionState, ScreenCapturer,
};
pub use sink::{AgentSink, LoggingAgentSink};

/// Session state for the overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayState {
    /// The puck is closed.
    Closed,
    /// The puck is open and idle (not listening).
    Idle,
    /// Continuously transcribing speech into the chat.
    Listening,
    /// The draw-to-annotate canvas is active.
    Annotating,
    /// Composing + submitting the captured message.
    Submitting,
}

/// Orchestrates the overlay session: owns the state machine and wires the
/// voice, annotation, capture, window, and permission pieces to the
/// [`AgentSink`]. All collaborators are injected as trait objects so the
/// controller carries no native or IDE dependencies.
pub struct OverlayController {
    state: OverlayState,
    // `sink` (Phase 2: voice -> chat) and `capturer` (Phase 3: screenshot) are
    // stored now and consumed as those phases land.
    #[allow(dead_code)]
    sink: Box<dyn AgentSink>,
    window: Box<dyn OverlayWindow>,
    #[allow(dead_code)]
    capturer: Box<dyn ScreenCapturer>,
    annotator: Box<dyn Annotator>,
    pointer: Box<dyn GlobalPointer>,
    permissions: Box<dyn OverlayPermissions>,
    /// Whether a voice chunk is currently capturing audio. Driven by the focused
    /// composer (which owns the voice pipeline) via `set_recording`, and read
    /// back to decide when to end/restart chunks. See `terminal::input`.
    recording: bool,
    /// Whether the user has paused listening (clicked the mic puck). Paused dims
    /// the puck and suppresses chunk restarts until resumed.
    paused: bool,
    /// The composer that opened the overlay and drives its voice pipeline. Puck
    /// clicks (pause/submit) route here. Temporary coupling to `Input`; when the
    /// overlay is extracted into its own crate this moves behind the AgentSink.
    active_input: Option<WeakViewHandle<Input>>,
    /// Peak mic level during the current recording window; used to skip
    /// transcribing silent windows.
    chunk_peak: f32,
    /// Suppresses the continuous-voice auto-restart while a submit is in flight:
    /// we hold off on listening again until the agent has finished responding.
    /// Cleared when the response completes (or a fallback timeout).
    restart_suppressed: bool,
    /// Whether the agent's response has actually begun (conversation went
    /// in-progress) since the last submit. Guards against resuming on a stale
    /// "done" status observed before the new request starts.
    response_started: bool,
    /// Set once the Realtime pipeline reports it can't be used this session, so
    /// listening falls back to the chunked Whisper path.
    use_chunked_fallback: bool,
    /// Whether the result box is shown (dictation transcript or agent result).
    result_box_visible: bool,
    /// Finalized conversation history rendered for the read-only history region:
    /// completed exchanges as `> prompt\n\nanswer`, joined by blank lines. The
    /// in-flight exchange (`last_prompt` + streaming answer) is appended on top of
    /// this when building the display, then folded in on completion.
    history: String,
    /// The last prompt submitted from the overlay. Shown above the agent's reply
    /// in the result box so the overlay reads as a chat stream (user turn +
    /// answer), not just the answer. Cleared into `history` when the reply ends.
    last_prompt: Option<String>,
    /// Whether the agent's answer is currently being read aloud (TTS). Enables
    /// click-to-interrupt: tapping the mic puck while speaking silences the agent
    /// and starts listening.
    speaking: bool,
    /// Whether incoming transcript deltas should be accepted into the composer
    /// (the current prompt). With hands-free barge-in the Realtime session stays
    /// live across submit + response + TTS, so we gate on this flag instead of
    /// stopping/restarting the mic each turn: `true` while dictating a prompt,
    /// `false` while the agent is thinking/speaking (until the user barges in or
    /// the turn completes).
    accepting_dictation: bool,
    /// Whether echo-cancelled capture is active this session, i.e. hands-free
    /// barge-in is available (the mic can stay live while the agent speaks).
    /// When false we keep the suppress-during-TTS + tap-to-interrupt behavior.
    aec_active: bool,
}

impl OverlayController {
    pub fn new(
        sink: Box<dyn AgentSink>,
        window: Box<dyn OverlayWindow>,
        capturer: Box<dyn ScreenCapturer>,
        annotator: Box<dyn Annotator>,
        pointer: Box<dyn GlobalPointer>,
        permissions: Box<dyn OverlayPermissions>,
    ) -> Self {
        Self {
            state: OverlayState::Closed,
            sink,
            window,
            capturer,
            annotator,
            pointer,
            permissions,
            recording: false,
            paused: false,
            active_input: None,
            chunk_peak: 0.0,
            restart_suppressed: false,
            response_started: false,
            use_chunked_fallback: false,
            result_box_visible: false,
            history: String::new(),
            last_prompt: None,
            speaking: false,
            accepting_dictation: false,
            aec_active: false,
        }
    }

    /// Singleton constructor for `add_singleton_model`. Selects the native puck
    /// window on macOS; other collaborators are still stubs until later phases.
    pub fn new_singleton(_ctx: &mut ModelContext<Self>) -> Self {
        #[cfg(target_os = "macos")]
        let window: Box<dyn OverlayWindow> = Box::new(platform_mac::MacOverlayWindow::default());
        #[cfg(not(target_os = "macos"))]
        let window: Box<dyn OverlayWindow> = Box::<NoopOverlayWindow>::default();

        Self::new(
            Box::<LoggingAgentSink>::default(),
            window,
            Box::<NoopScreenCapturer>::default(),
            Box::<NoopAnnotator>::default(),
            Box::<NoopGlobalPointer>::default(),
            Box::<NoopPermissions>::default(),
        )
    }

    /// Builds a controller wired to Phase 0 stubs (no native, logging sink).
    /// Real collaborators replace these in later phases.
    pub fn with_stubs() -> Self {
        Self::new(
            Box::<LoggingAgentSink>::default(),
            Box::<NoopOverlayWindow>::default(),
            Box::<NoopScreenCapturer>::default(),
            Box::<NoopAnnotator>::default(),
            Box::<NoopGlobalPointer>::default(),
            Box::<NoopPermissions>::default(),
        )
    }

    pub fn state(&self) -> OverlayState {
        self.state
    }

    pub fn is_open(&self) -> bool {
        self.state != OverlayState::Closed
    }

    /// Toggle the puck open/closed. When opening, shows the window and moves to
    /// `Idle`; when closing, stops any active listening/annotation and hides.
    pub fn toggle(&mut self) {
        if self.is_open() {
            self.close();
        } else {
            self.open();
        }
    }

    /// Open the puck and enter continuous listening. The composer drives the
    /// actual voice capture; the puck ring reflects that we're live.
    /// Open the puck and enter continuous listening. The composer drives the
    /// actual voice capture; the puck ring reflects that we're live.
    pub fn open(&mut self) {
        if self.is_open() {
            return;
        }
        self.paused = false;
        self.restart_suppressed = false;
        self.response_started = false;
        self.use_chunked_fallback = false;
        self.speaking = false;
        self.accepting_dictation = true;
        self.aec_active = false;
        self.window.set_paused(false);
        self.window.show();
        self.window.set_listening(true);
        // Fresh session: empty history + empty editable input line.
        self.result_box_visible = true;
        self.history.clear();
        self.last_prompt = None;
        self.window.show_result_box();
        self.window.set_history_text("");
        self.window.set_input_text("");
        self.state = OverlayState::Listening;
    }

    pub fn close(&mut self) {
        if !self.is_open() {
            return;
        }
        if self.annotator.is_active() {
            let _ = self.annotator.finish();
        }
        self.pointer.end_capture();
        self.window.stop_speaking();
        self.window.set_listening(false);
        self.window.set_level(0.0);
        self.window.set_thinking(false);
        self.window.hide_result_box();
        self.window.hide();
        self.recording = false;
        self.paused = false;
        self.restart_suppressed = false;
        self.active_input = None;
        self.chunk_peak = 0.0;
        self.restart_suppressed = false;
        self.response_started = false;
        self.result_box_visible = false;
        self.history.clear();
        self.last_prompt = None;
        self.speaking = false;
        self.accepting_dictation = false;
        self.aec_active = false;
        self.state = OverlayState::Closed;
    }

    /// Whether a voice chunk is currently capturing audio.
    pub fn is_recording(&self) -> bool {
        self.recording
    }

    /// Record whether a voice chunk is currently capturing. Called by the
    /// composer as its voice state changes.
    pub fn set_recording(&mut self, recording: bool) {
        self.recording = recording;
    }

    pub fn is_paused(&self) -> bool {
        self.paused
    }

    /// Set paused (clicked mic puck): dims the puck and suppresses restarts.
    pub fn set_paused(&mut self, paused: bool) {
        self.paused = paused;
        self.window.set_paused(paused);
        if paused {
            self.window.set_level(0.0);
        }
        self.state = if paused {
            OverlayState::Idle
        } else {
            OverlayState::Listening
        };
    }

    /// Feed the live mic level (0.0..~1.0) to modulate the puck. Also tracks the
    /// peak level for the current recording window so silent windows can be
    /// discarded (Whisper hallucinates phrases like "thanks for watching" on
    /// silence).
    pub fn set_level(&mut self, level: f32) {
        if level > self.chunk_peak {
            self.chunk_peak = level;
        }
        self.window.set_level(level);
    }

    /// Show/hide the "thinking" spotlight on the submit puck.
    pub fn set_thinking(&mut self, thinking: bool) {
        self.window.set_thinking(thinking);
    }

    /// Show the streaming result box.
    pub fn show_result_box(&mut self) {
        self.result_box_visible = true;
        self.window.show_result_box();
    }

    /// Whether the result box is currently shown.
    pub fn is_result_box_visible(&self) -> bool {
        self.result_box_visible
    }

    /// Update the read-only history region (markdown; empty clears it).
    pub fn set_history_text(&mut self, text: &str) {
        self.window.set_history_text(text);
    }

    /// Update the editable input line (composer mirror / live transcript).
    pub fn set_input_text(&mut self, text: &str) {
        self.window.set_input_text(text);
    }

    /// Set the box background color (RGBA, 0.0..1.0) to match the main composer.
    pub fn set_box_background(&mut self, r: f32, g: f32, b: f32, a: f32) {
        self.window.set_box_background(r, g, b, a);
    }

    /// Reflect the auto-submit setting on the in-overlay toggle.
    pub fn set_box_auto_submit(&mut self, on: bool) {
        self.window.set_box_auto_submit(on);
    }

    /// Speak text aloud via native TTS (reads the agent's answer in voice mode).
    pub fn speak(&mut self, text: &str) {
        self.speaking = true;
        self.window.speak(text);
    }

    /// Stop any in-progress speech.
    pub fn stop_speaking(&mut self) {
        self.speaking = false;
        self.window.stop_speaking();
    }

    /// Whether the agent's answer is currently being read aloud.
    pub fn is_speaking(&self) -> bool {
        self.speaking
    }

    /// Whether transcript deltas should be accepted into the composer right now
    /// (dictating a prompt). False while the agent is thinking/speaking.
    pub fn is_accepting_dictation(&self) -> bool {
        self.accepting_dictation
    }

    /// Set whether transcript deltas are accepted into the composer.
    pub fn set_accepting_dictation(&mut self, accepting: bool) {
        self.accepting_dictation = accepting;
    }

    /// Whether echo-cancelled capture is active (hands-free barge-in available).
    pub fn is_aec_active(&self) -> bool {
        self.aec_active
    }

    /// Record whether the current session uses echo-cancelled capture. Set from
    /// the Realtime pipeline once it negotiates its audio source.
    pub fn set_aec_active(&mut self, active: bool) {
        self.aec_active = active;
    }

    /// Reflect the voice-output setting on the overlay's settings menu.
    pub fn set_voice_enabled(&mut self, on: bool) {
        self.window.set_voice_enabled(on);
    }

    /// Set the shared puck accent color to a preset palette index.
    pub fn set_puck_color(&mut self, index: i32) {
        self.window.set_puck_color(index);
    }

    /// Reflect the transcription language (ISO code, "" = auto) on the settings
    /// popover.
    pub fn set_language(&mut self, code: &str) {
        self.window.set_language(code);
    }

    /// Reflect the response-verbosity level (0-10) on the settings popover.
    pub fn set_verbosity(&mut self, level: u8) {
        self.window.set_verbosity(level);
    }

    /// Enter freeform annotation mode: hides the pucks/box and shows the
    /// full-screen drawing canvas. The composer drives capture on Done.
    pub fn begin_annotation(&mut self) {
        self.window.stop_speaking();
        self.window.begin_annotation();
        self.state = OverlayState::Annotating;
    }

    /// Exit annotation mode (Done or Cancel): hides the canvas and restores the
    /// pucks/box, returning to the listening state.
    pub fn end_annotation(&mut self) {
        self.window.end_annotation();
        self.state = OverlayState::Listening;
    }

    /// Whether the annotation canvas is active.
    pub fn is_annotating(&self) -> bool {
        self.state == OverlayState::Annotating
    }

    /// The last prompt submitted from the overlay (shown above the reply).
    pub fn last_prompt(&self) -> Option<&str> {
        self.last_prompt.as_deref()
    }

    /// Record the last submitted prompt (see `last_prompt`).
    pub fn set_last_prompt(&mut self, prompt: Option<String>) {
        self.last_prompt = prompt;
    }

    /// Fold a completed exchange into the finalized history.
    pub fn push_history_exchange(&mut self, prompt: &str, answer: &str) {
        if !self.history.is_empty() {
            self.history.push_str("\n\n");
        }
        self.history.push_str("> ");
        self.history.push_str(prompt);
        self.history.push_str("\n\n");
        self.history.push_str(answer);
    }

    /// Build the read-only history text: finalized exchanges plus the in-flight
    /// exchange (`last_prompt` + `streaming_answer`) if one is active.
    pub fn history_display(&self, streaming_answer: &str) -> String {
        let mut out = self.history.clone();
        if let Some(prompt) = &self.last_prompt {
            if !prompt.is_empty() {
                if !out.is_empty() {
                    out.push_str("\n\n");
                }
                out.push_str("> ");
                out.push_str(prompt);
                out.push_str("\n\n");
                out.push_str(streaming_answer);
            }
        }
        out
    }

    /// Peak mic level observed during the current recording window.
    pub fn chunk_peak(&self) -> f32 {
        self.chunk_peak
    }

    /// Reset the per-window peak (call when a new recording window starts).
    pub fn reset_chunk_peak(&mut self) {
        self.chunk_peak = 0.0;
    }

    /// Whether the continuous-voice auto-restart is currently suppressed.
    pub fn is_restart_suppressed(&self) -> bool {
        self.restart_suppressed
    }

    /// Suppress/allow the continuous-voice auto-restart (used across a submit).
    pub fn set_restart_suppressed(&mut self, suppressed: bool) {
        self.restart_suppressed = suppressed;
    }

    /// Whether the agent response has begun since the last submit.
    pub fn response_started(&self) -> bool {
        self.response_started
    }

    pub fn set_response_started(&mut self, started: bool) {
        self.response_started = started;
    }

    /// Whether listening should use the chunked Whisper fallback (Realtime
    /// unavailable this session).
    pub fn use_chunked_fallback(&self) -> bool {
        self.use_chunked_fallback
    }

    pub fn set_use_chunked_fallback(&mut self, fallback: bool) {
        self.use_chunked_fallback = fallback;
    }

    /// The composer currently driving the overlay's voice pipeline.
    pub fn active_input(&self) -> Option<WeakViewHandle<Input>> {
        self.active_input.clone()
    }

    /// Register the composer that opened the overlay (see `active_input`).
    pub fn set_active_input(&mut self, input: WeakViewHandle<Input>) {
        self.active_input = Some(input);
    }

    /// Access the injected permission facade (used by later phases to gate
    /// capture/annotation on Screen Recording / Accessibility).
    pub fn permissions(&self) -> &dyn OverlayPermissions {
        self.permissions.as_ref()
    }

    // Voice (Phase 2), capture (Phase 3), and annotation (Phase 4) drive
    // transitions among Listening/Annotating/Submitting and feed `self.sink`.
    // Those methods are added as each phase lands.
}

impl Entity for OverlayController {
    type Event = ();
}

impl SingletonEntity for OverlayController {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toggle_moves_between_closed_and_listening() {
        let mut controller = OverlayController::with_stubs();
        assert_eq!(controller.state(), OverlayState::Closed);
        controller.toggle();
        assert_eq!(controller.state(), OverlayState::Listening);
        assert!(controller.is_open());
        controller.toggle();
        assert_eq!(controller.state(), OverlayState::Closed);
        assert!(!controller.is_open());
    }
}
