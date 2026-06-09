//! [`TuiColumn`]: a vertical stack that lays its children out top-to-bottom.
//!
//! # Construction
//! Start from [`TuiColumn::new`] (empty) and append children with
//! [`child`](TuiColumn::child), or build from an iterator with
//! [`with_children`](TuiColumn::with_children).
//!
//! # Layout policy
//! The column fills the width it is offered and gives every child that same
//! width. Each child is allocated exactly its
//! [`desired_height`](TuiElement::desired_height) at that width; children are
//! stacked without gaps from the top, and the column's own height is the sum of
//! those heights clamped to the constraint. Children that fall past the
//! available height are clipped.

use crate::elements::TuiElement;
use crate::{TuiBuffer, TuiConstraint, TuiRect, TuiSize};

#[derive(Default)]
pub struct TuiColumn {
    children: Vec<Box<dyn TuiElement>>,
}

impl TuiColumn {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn child(mut self, child: impl TuiElement + 'static) -> Self {
        self.children.push(Box::new(child));
        self
    }

    pub fn with_children(children: impl IntoIterator<Item = Box<dyn TuiElement>>) -> Self {
        Self {
            children: children.into_iter().collect(),
        }
    }
}

impl TuiElement for TuiColumn {
    fn layout(&mut self, constraint: TuiConstraint) -> TuiSize {
        let width = constraint.constrain_width(constraint.max.width);
        let mut total_height: u16 = 0;
        for child in &mut self.children {
            let child_height = child.desired_height(width);
            let child_constraint =
                TuiConstraint::new(TuiSize::new(width, 0), TuiSize::new(width, child_height));
            let size = child.layout(child_constraint);
            total_height = total_height.saturating_add(size.height);
        }
        TuiSize::new(width, constraint.constrain_height(total_height))
    }

    fn render(&self, area: TuiRect, buffer: &mut TuiBuffer) {
        let mut remaining = area;
        for child in &self.children {
            if remaining.is_empty() {
                break;
            }
            let child_height = child.desired_height(remaining.width);
            let (slot, rest) = remaining.split_top(child_height);
            child.render(slot, buffer);
            remaining = rest;
        }
    }

    fn desired_height(&self, width: u16) -> u16 {
        self.children.iter().fold(0, |total, child| {
            total.saturating_add(child.desired_height(width))
        })
    }
}

#[cfg(test)]
#[path = "column_tests.rs"]
mod tests;
