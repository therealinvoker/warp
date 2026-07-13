use std::any::Any;
use std::sync::{Arc, Mutex};

use warpui::elements::Point;
use warpui::event::DispatchedEvent;
use warpui::geometry::rect::RectF;
use warpui::geometry::vector::Vector2F;
use warpui::{
    AfterLayoutContext, AppContext, Element, Event, EventContext, LayoutContext, PaintContext,
    SizeConstraint,
};

use crate::pane_group::PaneId;
use crate::workspace::action::WorkspaceAction;
use crate::workspace::PaneViewLocator;

/// Shared, panel-lived handle tracking which pane row (if any) is currently the
/// target of an in-progress OS file drag. Written by [`VerticalTabFileDropElement`]
/// on drag events and read by the row renderer to paint a drop highlight. One
/// instance is shared across every row so at most one row highlights at a time.
pub(super) type FileDropTargetHandle = Arc<Mutex<Option<PaneId>>>;

/// Wraps a vertical-tab row so dragging image files from the OS (e.g. Finder)
/// over the row highlights it, and dropping loads the images into that
/// tab/pane's chat composer.
///
/// OS file drags arrive as window-level `Event::DragFiles` (hover),
/// `Event::DragFileExit` (drag left the window), and `Event::DragAndDropFiles`
/// (drop) rather than through the in-app `Draggable`/`DropTarget` machinery
/// (which is for tab reordering). This element mirrors `TerminalSizeElement`: it
/// claims those events when the cursor is within its bounds. Hover updates the
/// shared [`FileDropTargetHandle`] (so the renderer can highlight this row) and
/// a drop routes the image-filtered paths to the pane via
/// `WorkspaceAction::AttachImagesToPane`, which focuses the tab and attaches the
/// images to its composer.
pub(super) struct VerticalTabFileDropElement {
    child: Box<dyn Element>,
    locator: PaneViewLocator,
    pane_id: PaneId,
    drop_target: FileDropTargetHandle,
}

impl VerticalTabFileDropElement {
    pub(super) fn new(
        locator: PaneViewLocator,
        pane_id: PaneId,
        drop_target: FileDropTargetHandle,
        child: Box<dyn Element>,
    ) -> Self {
        Self {
            child,
            locator,
            pane_id,
            drop_target,
        }
    }

    fn mouse_position_is_in_bounds(&self, position: Vector2F) -> bool {
        self.bounds()
            .is_some_and(|bounds| bounds.contains_point(position))
    }

    /// Marks (or clears) this row as the current drop target. Returns `true`
    /// when the shared state actually changed, so the caller can request a
    /// repaint only when the highlight needs to move.
    fn set_is_target(&self, is_target: bool) -> bool {
        let Ok(mut target) = self.drop_target.lock() else {
            return false;
        };
        let currently_targeted = *target == Some(self.pane_id);
        if is_target && !currently_targeted {
            *target = Some(self.pane_id);
            true
        } else if !is_target && currently_targeted {
            *target = None;
            true
        } else {
            false
        }
    }
}

impl Element for VerticalTabFileDropElement {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        self.child.layout(constraint, ctx, app)
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.child.paint(origin, ctx, app)
    }

    fn size(&self) -> Option<Vector2F> {
        self.child.size()
    }

    fn origin(&self) -> Option<Point> {
        self.child.origin()
    }

    fn bounds(&self) -> Option<RectF> {
        self.child.bounds()
    }

    fn parent_data(&self) -> Option<&dyn Any> {
        self.child.parent_data()
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        let handled_by_child = self.child.dispatch_event(event, ctx, app);
        let Some(z_index) = self.z_index() else {
            return handled_by_child;
        };

        if !handled_by_child {
            if let Some(event_at_z_index) = event.at_z_index(z_index, ctx) {
                match event_at_z_index {
                    Event::DragFiles { location } => {
                        if self.set_is_target(self.mouse_position_is_in_bounds(*location)) {
                            ctx.notify();
                        }
                        // Don't claim the hover: other rows must also see it so
                        // they can clear their own highlight when the cursor
                        // moves off them.
                    }
                    Event::DragFileExit => {
                        if self.set_is_target(false) {
                            ctx.notify();
                        }
                    }
                    Event::DragAndDropFiles { paths, location } => {
                        if self.set_is_target(false) {
                            ctx.notify();
                        }
                        let image_paths: Vec<String> =
                            warpui::clipboard_utils::get_image_filepaths_from_paths(paths);
                        if self.mouse_position_is_in_bounds(*location) && !image_paths.is_empty() {
                            ctx.dispatch_typed_action(WorkspaceAction::AttachImagesToPane {
                                locator: self.locator,
                                paths: image_paths,
                            });
                            return true;
                        }
                    }
                    _ => {}
                }
            }
        }
        handled_by_child
    }
}
