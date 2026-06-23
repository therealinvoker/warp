//! The headless `warp-tui` front-end: a real (headless) Warp app whose root
//! window is a [`RootTuiView`] rendered through the `tui`-gated WarpUI backend.
//!
//! `RootTuiView` renders the shared [`TerminalModel`]'s terminal history above a
//! bottom-anchored [`TuiInputView`] and routes submissions into the shared
//! [`TuiTerminalSession`]. A leading `!` runs the rest as a command through the
//! persistent `TerminalModel`; plain text is reserved for the future agent
//! prompt and ignored for now. Keystrokes are forwarded to the PTY when a
//! command is running or the alt-screen is active. [`init`] is called from
//! `run_internal` once the headless app is up (see [`crate::run_tui`]). Ctrl-C
//! quit is handled by the runtime's input loop.

mod grid_render;
mod input_view;
mod session;
mod terminal_history_source;

use std::sync::Arc;
use std::time::Duration;

use input_view::{InputEvent, TuiInputView};
use parking_lot::FairMutex;
use session::{encode_keydown, TuiTerminalSession};
use terminal_history_source::{TerminalHistoryItemId, TerminalHistorySource};
use warpui_core::elements::tui::{
    TuiBuffer, TuiChildView, TuiColumn, TuiConstrainedBox, TuiConstraint, TuiElement,
    TuiEventContext, TuiPresentationContext, TuiRect, TuiSize, TuiVirtualList,
    TuiVirtualListHandle,
};
use warpui_core::platform::{TerminationMode, WindowStyle};
use warpui_core::runtime::{spawn_tui_driver, TuiDriverHandle};
use warpui_core::{
    AddWindowOptions, AppContext, Entity, Event, SingletonEntity, TuiView, TypedActionView,
    ViewContext, ViewHandle,
};

use crate::terminal::color;
use crate::terminal::model::terminal_model::{TerminalInputState, TerminalModel};

/// The bottom input frame's height: one text row inside a single-cell rounded
/// border (top + bottom), i.e. three rows total.
const INPUT_ROWS: u16 = 3;

/// The interrupt byte (Ctrl-C) sent to the PTY on Esc/Cancel.
const INTERRUPT_BYTE: u8 = 0x03;

/// How often the background task checks for terminal size changes.
const RESIZE_POLL_INTERVAL: Duration = Duration::from_millis(200);

/// The root TUI view: the shared model's virtualized history above a fixed,
/// bottom-anchored input. It owns the input view and forwards its submissions
/// into the shared terminal session.
struct RootTuiView {
    input: ViewHandle<TuiInputView>,
    history_scroll: TuiVirtualListHandle<TerminalHistoryItemId>,
}

impl RootTuiView {
    fn new(ctx: &mut ViewContext<Self>) -> Self {
        let input = ctx.add_typed_action_tui_view(|_| TuiInputView::default());

        ctx.subscribe_to_view(&input, |_root, _input, event, ctx| match event {
            InputEvent::Submitted(text) => {
                // Only `!`-prefixed input runs as a shell command today; plain
                // text is reserved for the future agent prompt and ignored.
                if let Some(command) = text.strip_prefix('!') {
                    let command = command.to_string();
                    TuiTerminalSession::handle(ctx)
                        .update(ctx, |session, ctx| session.run_command(&command, ctx));
                }
            }
            InputEvent::Cancel => {
                TuiTerminalSession::handle(ctx).update(ctx, |session, ctx| {
                    session.write_input_bytes(vec![INTERRUPT_BYTE], ctx);
                });
            }
        });

        ctx.focus(&input);

        // Repaint when the model changes (PTY output, block updates, etc.).
        // Gracefully skip if the session singleton isn't registered (tests).
        if let Some(session) = TuiTerminalSession::handle(ctx).downgrade().upgrade(ctx) {
            let model_events = session.read(ctx, |s, _| s.model_events().clone());
            ctx.subscribe_to_model(&model_events, |_, _, _event, ctx| {
                ctx.notify();
            });

            // Drain the PTY wakeup channel to repaint on terminal output. The
            // wakeup channel is the terminal's redraw signal (fired on every
            // PTY read); without draining it the receiver is dropped, the
            // sender logs "Failed to send Wakeup event: Closed", and streamed
            // command output never triggers a redraw.
            if let Some(wakeups_rx) = session.update(ctx, |s, _| s.take_wakeups_rx()) {
                ctx.spawn_stream_local(
                    wakeups_rx,
                    |_view, _wakeup, ctx| ctx.notify(),
                    |_view, _ctx| {},
                );
            }
        }

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

        Self {
            input,
            history_scroll: TuiVirtualListHandle::new(),
        }
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
        let input = TuiChildView::new(&self.input, ctx);

        // If the session singleton isn't registered (tests), render just the input.
        let Some(session) = TuiTerminalSession::handle(ctx).downgrade().upgrade(ctx) else {
            return Box::new(TuiKeyInterceptor::new(Box::new(
                TuiColumn::new().child(TuiConstrainedBox::new(input).with_max_rows(INPUT_ROWS)),
            )));
        };

        let model = session.read(ctx, |s, _| s.model());
        let colors = model.lock().colors();

        // When the alt-screen is active, render it full-pane (no input view).
        if model.lock().is_alt_screen_active() {
            return Box::new(TuiKeyInterceptor::new(Box::new(TuiAltScreenElement::new(
                model, colors,
            ))));
        }

        // Otherwise: virtualized terminal history + input.
        let history = TuiVirtualList::new(
            self.history_scroll.clone(),
            TerminalHistorySource::new(model, colors),
        );

        let column = TuiColumn::new()
            .flex_child(history)
            .child(TuiConstrainedBox::new(input).with_max_rows(INPUT_ROWS));

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

/// Renders the alt-screen grid full-pane.
struct TuiAltScreenElement {
    model: Arc<FairMutex<TerminalModel>>,
    colors: color::List,
}

impl TuiAltScreenElement {
    fn new(model: Arc<FairMutex<TerminalModel>>, colors: color::List) -> Self {
        Self { model, colors }
    }
}

impl TuiElement for TuiAltScreenElement {
    fn layout(&mut self, constraint: TuiConstraint) -> TuiSize {
        constraint.clamp(constraint.max)
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer) {
        let model = self.model.lock();
        let grid = model.alt_screen().grid_handler();
        grid_render::render_grid(grid, area, buffer, &self.colors);
    }

    fn desired_height(&self, _width: u16) -> u16 {
        use crate::terminal::model::grid::Dimensions as _;
        let model = self.model.lock();
        let grid = model.alt_screen().grid_handler();
        grid.len_displayed().unwrap_or_else(|| grid.visible_rows()) as u16
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
