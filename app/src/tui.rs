//! The headless `warp-tui` front-end: a real (headless) Warp app whose root
//! window is a [`RootTuiView`] rendered through the `tui`-gated WarpUI backend.
//!
//! `RootTuiView` composes two child views — a [`TuiTranscriptView`] filling the
//! space above a bottom-anchored single-row [`TuiInputView`] — and routes
//! submissions into the shared [`TuiTerminalSession`]. A leading `!` runs the
//! rest as a command through the persistent `TerminalModel`; plain text is
//! appended to the local transcript. Keystrokes are forwarded to the PTY when a
//! command is running or the alt-screen is active. [`init`] is called from
//! `run_internal` once the headless app is up (see [`crate::run_tui`]). Ctrl-C
//! quit is handled by the runtime's input loop.

mod command_output;
mod input_view;
mod session;
mod transcript_view;

use std::time::Duration;

use input_view::{InputEvent, TuiInputView};
use session::{encode_keydown, TuiTerminalSession};
use transcript_view::TuiTranscriptView;
use warpui_core::elements::tui::{
    TuiBuffer, TuiChildView, TuiColumn, TuiConstrainedBox, TuiConstraint, TuiElement,
    TuiEventContext, TuiPresentationContext, TuiRect, TuiSize,
};
use warpui_core::platform::{TerminationMode, WindowStyle};
use warpui_core::runtime::{spawn_tui_driver, TuiDriverHandle};
use warpui_core::{
    AddWindowOptions, AppContext, Entity, Event, SingletonEntity, TuiView, TypedActionView,
    ViewContext, ViewHandle,
};

use crate::terminal::model::terminal_model::TerminalInputState;

/// The bottom input frame's height: one text row inside a single-cell rounded
/// border (top + bottom), i.e. three rows total.
const INPUT_ROWS: u16 = 3;

/// The interrupt byte (Ctrl-C) sent to the PTY on Esc/Cancel.
const INTERRUPT_BYTE: u8 = 0x03;

/// How often the background task checks for terminal size changes.
const RESIZE_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// The root TUI view: a transcript that grows upward above a fixed,
/// bottom-anchored input. It owns both child views and forwards the input's
/// submissions into the shared terminal session.
struct RootTuiView {
    transcript: ViewHandle<TuiTranscriptView>,
    input: ViewHandle<TuiInputView>,
}

impl RootTuiView {
    fn new(ctx: &mut ViewContext<Self>) -> Self {
        let transcript = ctx.add_tui_view(|_| TuiTranscriptView::default());
        let input = ctx.add_typed_action_tui_view(|_| TuiInputView::default());

        ctx.subscribe_to_view(&input, |root, _input, event, ctx| match event {
            InputEvent::Submitted(text) => {
                if let Some(command) = text.strip_prefix('!') {
                    let command = command.to_string();
                    TuiTerminalSession::handle(ctx)
                        .update(ctx, |session, ctx| session.run_command(&command, ctx));
                } else {
                    let text = text.clone();
                    root.transcript
                        .update(ctx, |transcript, ctx| transcript.append(text, ctx));
                }
            }
            InputEvent::Cancel => {
                TuiTerminalSession::handle(ctx).update(ctx, |session, ctx| {
                    session.write_input_bytes(vec![INTERRUPT_BYTE], ctx);
                });
            }
        });

        ctx.focus(&input);

        // Periodically check the terminal size and resize the model + PTY when
        // it changes. The TUI runtime invalidates on resize but doesn't call
        // back into the session, so we poll from a background timer.
        let (resize_tx, resize_rx) = async_channel::unbounded::<(usize, usize)>();
        ctx.background_executor()
            .spawn(async move {
                let mut last = current_terminal_cells();
                loop {
                    warpui::r#async::Timer::after(RESIZE_POLL_INTERVAL).await;
                    let now = current_terminal_cells();
                    if now != last {
                        last = now;
                        if let Some((cols, rows)) = now {
                            let _ = resize_tx.send((rows as usize, cols as usize)).await;
                        }
                    }
                }
            })
            .detach();

        ctx.spawn_stream_local(
            resize_rx,
            |_view, (rows, cols), ctx| {
                TuiTerminalSession::handle(ctx)
                    .update(ctx, |session, ctx| session.resize(rows, cols, ctx));
            },
            |_, _| {},
        );

        Self { transcript, input }
    }
}

impl Entity for RootTuiView {
    type Event = ();
}

impl TuiView for RootTuiView {
    fn ui_name() -> &'static str {
        "RootTuiView"
    }

    fn render(&self, ctx: &AppContext) -> Box<dyn TuiElement> {
        let transcript = TuiChildView::new(&self.transcript, ctx);
        let input = TuiChildView::new(&self.input, ctx);

        let column = TuiColumn::new()
            .flex_child(transcript)
            .child(TuiConstrainedBox::new(input).with_max_rows(INPUT_ROWS));

        // Wrap the column in a key interceptor that forwards keystrokes to the
        // PTY when a command is running or the alt-screen is active.
        Box::new(TuiKeyInterceptor::new(Box::new(column)))
    }
}

impl TypedActionView for RootTuiView {
    type Action = ();
}

/// A wrapper element that intercepts `KeyDown` events before they reach the
/// child. When the terminal is in `LongRunningCommand` or `AltScreen` state,
/// keystrokes are encoded and forwarded to the PTY (the TUI behaves like a real
/// terminal). In `InputEditor` or `NotBootstrapped` state, events pass through
/// to the child unchanged.
struct TuiKeyInterceptor {
    child: Box<dyn TuiElement>,
}

impl TuiKeyInterceptor {
    fn new(child: Box<dyn TuiElement>) -> Self {
        Self { child }
    }
}

impl TuiElement for TuiKeyInterceptor {
    fn layout(&mut self, constraint: TuiConstraint) -> TuiSize {
        self.child.layout(constraint)
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer) {
        self.child.render(area, buffer);
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.child.desired_height(width)
    }

    fn cursor_position(&self, area: TuiRect) -> Option<(u16, u16)> {
        self.child.cursor_position(area)
    }

    fn present(&mut self, ctx: &mut TuiPresentationContext<'_>) {
        self.child.present(ctx);
    }

    fn dispatch_event(
        &mut self,
        event: &Event,
        area: TuiRect,
        ctx: &mut TuiEventContext,
        app: &AppContext,
    ) -> bool {
        if let Event::KeyDown {
            keystroke,
            chars,
            details,
            ..
        } = event
        {
            let session = TuiTerminalSession::as_ref(app);
            let model = session.model();
            let input_state = model.lock().terminal_input_state();

            if matches!(
                input_state,
                TerminalInputState::LongRunningCommand | TerminalInputState::AltScreen
            ) {
                let key_without_modifiers = details.key_without_modifiers.as_deref();
                let bytes = encode_keydown(keystroke, key_without_modifiers, chars, &model)
                    .or_else(|| {
                        if chars.is_empty() {
                            None
                        } else {
                            Some(chars.as_bytes().to_vec())
                        }
                    });

                if let Some(bytes) = bytes {
                    if !bytes.is_empty() {
                        ctx.dispatch_app_update(move |ctx| {
                            TuiTerminalSession::handle(ctx).update(ctx, |session, ctx| {
                                session.write_input_bytes(bytes, ctx);
                            });
                        });
                    }
                }
                return true;
            }
        }

        self.child.dispatch_event(event, area, ctx, app)
    }
}

/// Holds the live TUI session for the app's lifetime; dropping it on app
/// teardown restores the terminal.
struct TuiSession {
    _handle: TuiDriverHandle,
}

impl Entity for TuiSession {
    type Event = ();
}

impl SingletonEntity for TuiSession {}

/// Creates the TUI root window and starts the headless draw + input driver.
/// The [`TuiTerminalSession`] singleton is registered first so the session core
/// exists before any view renders or key events dispatch.
pub fn init(ctx: &mut AppContext) {
    TuiTerminalSession::register(ctx);

    let (window_id, root) = ctx.add_tui_window(
        AddWindowOptions {
            window_style: WindowStyle::NotStealFocus,
            ..Default::default()
        },
        RootTuiView::new,
    );

    match spawn_tui_driver(ctx, window_id, root) {
        Ok(handle) => {
            ctx.add_singleton_model(|_| TuiSession { _handle: handle });
        }
        Err(error) => {
            log::error!("failed to start the TUI driver: {error}");
            // Not in the alternate screen yet (entering it is what failed), so
            // print to stderr too — otherwise the process just exits instantly
            // with the reason buried in the log file.
            eprintln!(
                "warp-tui: could not start the terminal UI: {error}\n\
                 Run it directly in an interactive terminal (a real TTY), not piped or backgrounded."
            );
            ctx.terminate_app(TerminationMode::ForceTerminate, None);
        }
    }
}

/// Reads the current terminal size in cells from crossterm.
fn current_terminal_cells() -> Option<(u16, u16)> {
    crossterm::terminal::size().ok()
}

#[cfg(test)]
#[path = "tui_tests.rs"]
mod tests;
