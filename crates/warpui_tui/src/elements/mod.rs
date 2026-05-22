mod column;
mod container;
mod text;

pub use column::TuiColumn;
pub use container::TuiContainer;
pub use text::TuiText;

use warpui_core::Event;

use crate::{TuiBuffer, TuiConstraint, TuiRect, TuiSize};

pub trait TuiElement {
    fn layout(&mut self, constraint: TuiConstraint) -> TuiSize;

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer);

    fn desired_height(&self, width: u16) -> u16;

    fn cursor_position(&self, _area: TuiRect) -> Option<(u16, u16)> {
        None
    }

    fn dispatch_event(&mut self, _event: &Event) -> bool {
        false
    }
}

impl TuiElement for () {
    fn layout(&mut self, _: TuiConstraint) -> TuiSize {
        TuiSize::default()
    }

    fn render(&self, _: TuiRect, _: &mut TuiBuffer) {}

    fn desired_height(&self, _: u16) -> u16 {
        0
    }
}
