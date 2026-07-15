//! Platform abstraction for the voice + annotation overlay.
//!
//! All net-new native macOS work — the always-on-top puck window, screen
//! capture, global pointer capture for freeform drawing, the annotation
//! compositor, and the Screen Recording / Accessibility / Microphone permission
//! flows — sits behind these traits. This keeps the overlay's `OverlayController`
//! logic portable and unit-testable, and quarantines the native code in the
//! `warpui` platform layer (added in later phases).
//!
//! Phase 0 ships no-op stubs so the controller compiles and the architecture is
//! in place; the real macOS implementations land in Phases 1-5.

/// Status of a macOS TCC permission the overlay depends on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionState {
    Granted,
    Denied,
    NotDetermined,
    /// The permission is not applicable on this platform/build.
    Unsupported,
}

/// The small always-on-top "puck" window that hosts the Bang logo and reflects
/// the overlay's listening/annotating state.
pub trait OverlayWindow: Send {
    fn show(&mut self);
    fn hide(&mut self);
    fn is_visible(&self) -> bool;
    /// Reflect listening state on the puck (e.g. a highlight ring). No-op by
    /// default so non-macOS stubs don't need to implement it.
    fn set_listening(&mut self, _listening: bool) {}
    /// Reflect paused state on the puck (dim it). No-op by default.
    fn set_paused(&mut self, _paused: bool) {}
    /// Feed the live mic level (0.0..~1.0) to modulate the puck. No-op by default.
    fn set_level(&mut self, _level: f32) {}
    /// Show/hide the "thinking" spotlight on the submit puck. No-op by default.
    fn set_thinking(&mut self, _thinking: bool) {}
    /// Show the streaming result box. No-op by default.
    fn show_result_box(&mut self) {}
    /// Hide the streaming result box. No-op by default.
    fn hide_result_box(&mut self) {}
    /// Set the read-only conversation history (markdown) shown above the input
    /// line. Empty string clears + hides it. No-op by default.
    fn set_history_text(&mut self, _text: &str) {}
    /// Set the always-editable input line (mirrors the composer / live transcript).
    /// No-op by default.
    fn set_input_text(&mut self, _text: &str) {}
    /// Set the result box background color (RGBA, 0.0..1.0) to match the main UI.
    /// No-op by default.
    fn set_box_background(&mut self, _r: f32, _g: f32, _b: f32, _a: f32) {}
    /// Reflect the auto-submit setting on the box's in-overlay toggle. No-op by
    /// default.
    fn set_box_auto_submit(&mut self, _on: bool) {}
    /// Speak the given text aloud (native TTS). No-op by default.
    fn speak(&mut self, _text: &str) {}
    /// Stop any in-progress speech. No-op by default.
    fn stop_speaking(&mut self) {}
    /// Reflect the voice-output setting on the box's settings menu. No-op by
    /// default.
    fn set_voice_enabled(&mut self, _on: bool) {}
    /// Set the shared puck accent color to a preset index. No-op by default.
    fn set_puck_color(&mut self, _index: i32) {}
    /// Reflect the transcription language (ISO code, "" = auto) on the settings
    /// popover. No-op by default.
    fn set_language(&mut self, _code: &str) {}
    /// Reflect the response-verbosity level (0-10) on the settings popover.
    /// No-op by default.
    fn set_verbosity(&mut self, _level: u8) {}
    /// Enter annotation mode: hide the pucks/result box and show the full-screen
    /// drawing canvas. No-op by default.
    fn begin_annotation(&mut self) {}
    /// Exit annotation mode: hide the canvas and restore the pucks/result box.
    /// No-op by default.
    fn end_annotation(&mut self) {}
}

/// Captures the screen to PNG bytes. Backed on macOS by the `screencapture`
/// wrapper in the `computer_use` crate (Phase 3).
pub trait ScreenCapturer: Send {
    /// Capture the full main display as PNG bytes.
    fn capture_full_screen_png(&self) -> Result<Vec<u8>, String>;
    /// Interactive region select (drag a box); `Ok(None)` if the user cancels.
    fn capture_interactive_region_png(&self) -> Result<Option<Vec<u8>>, String>;
}

/// Captures pointer input for freeform drawing while the annotation canvas is
/// active (Phase 4).
pub trait GlobalPointer: Send {
    fn begin_capture(&mut self);
    fn end_capture(&mut self);
}

/// Freeform annotation compositor: draws strokes over a captured screenshot and
/// returns the composited PNG (Phase 4).
pub trait Annotator: Send {
    /// Begin an annotation session over the given background PNG.
    fn begin(&mut self, background_png: Vec<u8>);
    /// Finish the session and return the composited (background + strokes) PNG,
    /// or `None` if the session was cancelled or nothing was drawn.
    fn finish(&mut self) -> Option<Vec<u8>>;
    fn is_active(&self) -> bool;
}

/// macOS permission facade for the overlay (Phase 5).
pub trait OverlayPermissions: Send {
    fn microphone(&self) -> PermissionState;
    fn screen_recording(&self) -> PermissionState;
    fn accessibility(&self) -> PermissionState;
    fn request_screen_recording(&self);
    fn request_accessibility(&self);
}

// ----------------------------- Phase 0 no-op stubs -----------------------------

#[derive(Default)]
pub struct NoopOverlayWindow {
    visible: bool,
}

impl OverlayWindow for NoopOverlayWindow {
    fn show(&mut self) {
        self.visible = true;
        log::debug!("[overlay] (stub) window show");
    }
    fn hide(&mut self) {
        self.visible = false;
        log::debug!("[overlay] (stub) window hide");
    }
    fn is_visible(&self) -> bool {
        self.visible
    }
}

#[derive(Default)]
pub struct NoopScreenCapturer;

impl ScreenCapturer for NoopScreenCapturer {
    fn capture_full_screen_png(&self) -> Result<Vec<u8>, String> {
        Err("screen capture not implemented on this platform".into())
    }
    fn capture_interactive_region_png(&self) -> Result<Option<Vec<u8>>, String> {
        Err("screen capture not implemented on this platform".into())
    }
}

#[derive(Default)]
pub struct NoopGlobalPointer;

impl GlobalPointer for NoopGlobalPointer {
    fn begin_capture(&mut self) {}
    fn end_capture(&mut self) {}
}

#[derive(Default)]
pub struct NoopAnnotator {
    active: bool,
}

impl Annotator for NoopAnnotator {
    fn begin(&mut self, _background_png: Vec<u8>) {
        self.active = true;
    }
    fn finish(&mut self) -> Option<Vec<u8>> {
        self.active = false;
        None
    }
    fn is_active(&self) -> bool {
        self.active
    }
}

#[derive(Default)]
pub struct NoopPermissions;

impl OverlayPermissions for NoopPermissions {
    fn microphone(&self) -> PermissionState {
        PermissionState::Unsupported
    }
    fn screen_recording(&self) -> PermissionState {
        PermissionState::Unsupported
    }
    fn accessibility(&self) -> PermissionState {
        PermissionState::Unsupported
    }
    fn request_screen_recording(&self) {}
    fn request_accessibility(&self) {}
}
