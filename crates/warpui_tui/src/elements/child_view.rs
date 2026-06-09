//! [`TuiChildView`]: embeds another [`TuiView`]'s rendered element tree as a
//! node in this view's tree.
//!
//! # Construction
//! Build with [`TuiChildView::new`], passing a [`TuiViewHandle`] and the current
//! [`AppContext`]; the child view is rendered immediately and stored as the
//! wrapped element.
//!
//! # Tree participation
//! `TuiChildView` is otherwise transparent — layout, render, height, and cursor
//! all delegate to the embedded element. It additionally hooks the two passes
//! that are view-aware:
//! - [`present`](TuiElement::present) enters the child's view id on the
//!   presentation context (recording the parent/child relationship) before
//!   descending, then exits it.
//! - [`dispatch_event`](TuiElement::dispatch_event) marks the child's view id as
//!   the action origin for the duration of the subtree's dispatch.

use warpui_core::{AppContext, EntityId, Event, TuiView, TuiViewHandle};

use crate::elements::{TuiElement, TuiPresentationContext};
use crate::{TuiBuffer, TuiConstraint, TuiEventContext, TuiRect, TuiSize};

pub struct TuiChildView {
    view_id: EntityId,
    child: Box<dyn TuiElement>,
}

impl TuiChildView {
    /// Renders `handle`'s view now and embeds the resulting element tree.
    pub fn new<V>(handle: &TuiViewHandle<V>, app: &AppContext) -> Self
    where
        V: TuiView<RenderOutput = Box<dyn TuiElement>>,
    {
        Self {
            view_id: handle.id(),
            child: handle.read(app, |view, ctx| view.render_tui(ctx)),
        }
    }

    /// Constructs a child view directly from an already-rendered element,
    /// bypassing the live `App`. Used by headless tests to exercise the
    /// embedding/recursion contract without standing up a real view.
    #[cfg(test)]
    pub(crate) fn from_rendered(view_id: EntityId, child: Box<dyn TuiElement>) -> Self {
        Self { view_id, child }
    }
}

impl TuiElement for TuiChildView {
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
        ctx.enter_child(self.view_id);
        self.child.present(ctx);
        ctx.exit_child();
    }

    fn dispatch_event(
        &mut self,
        event: &Event,
        area: TuiRect,
        ctx: &mut TuiEventContext,
        app: &AppContext,
    ) -> bool {
        let previous_origin = ctx.set_origin_view(Some(self.view_id));
        let handled = self.child.dispatch_event(event, area, ctx, app);
        ctx.set_origin_view(previous_origin);
        handled
    }
}

#[cfg(test)]
#[path = "child_view_tests.rs"]
mod tests;
