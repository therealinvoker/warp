//! `AgentSink` — the single decoupling seam between the overlay and whatever
//! consumes its output.
//!
//! The overlay never touches `Input`/`EditorView`/terminal internals directly.
//! It only ever calls this trait. The v1 implementation (`ExistingChatAdapter`,
//! added in a later phase) lives in the app and drives the existing agent chat;
//! a future standalone conversation surface can be swapped in without changing
//! the overlay at all. This is the seam that lets the overlay become Bang's core
//! surface rather than an IDE bolt-on.

/// Receives the overlay's voice + annotation output and turns it into an agent
/// conversation.
///
/// Implementations are responsible for scheduling work onto whatever context
/// they need (e.g. an app/view context); the overlay calls these methods with
/// plain data and holds no knowledge of the target surface.
pub trait AgentSink: Send {
    /// Stream a chunk of transcribed text into the active conversation input.
    fn insert_text(&mut self, text: &str);

    /// Attach a PNG image (e.g. an annotated screenshot) to the pending message.
    fn attach_image_png(&mut self, png: Vec<u8>, file_name: &str);

    /// Submit the pending message (accumulated text + attachments).
    fn submit(&mut self);
}

/// Phase 0 stub sink that logs what it would do. Replaced by `ExistingChatAdapter`
/// once the overlay is wired to the live chat pipeline.
#[derive(Default)]
pub struct LoggingAgentSink;

impl AgentSink for LoggingAgentSink {
    fn insert_text(&mut self, text: &str) {
        log::info!("[overlay] (stub sink) insert_text: {text:?}");
    }
    fn attach_image_png(&mut self, png: Vec<u8>, file_name: &str) {
        log::info!(
            "[overlay] (stub sink) attach_image_png: {file_name} ({} bytes)",
            png.len()
        );
    }
    fn submit(&mut self) {
        log::info!("[overlay] (stub sink) submit");
    }
}
