use crossterm::style::Color;

use super::TuiContainer;
use crate::elements::{TuiElement, TuiText};
use crate::{TuiBuffer, TuiConstraint, TuiRect, TuiSize};

fn render_to_lines(element: &dyn TuiElement, size: TuiSize) -> Vec<String> {
    let mut buffer = TuiBuffer::new(size);
    element.render(TuiRect::from_size(size), &mut buffer);
    buffer.to_lines()
}

#[test]
fn padding_offsets_the_child() {
    let container = TuiContainer::new(TuiText::new("X")).with_padding(1);
    assert_eq!(container.desired_height(3), 3);
    assert_eq!(
        render_to_lines(&container, TuiSize::new(3, 3)),
        vec!["   ", " X ", "   "],
    );
}

#[test]
fn border_frames_the_child() {
    let container = TuiContainer::new(TuiText::new("X")).with_border();
    assert_eq!(
        render_to_lines(&container, TuiSize::new(3, 3)),
        vec!["┌─┐", "│X│", "└─┘"],
    );
}

#[test]
fn border_and_padding_compose() {
    let mut container = TuiContainer::new(TuiText::new("X"))
        .with_border()
        .with_padding(1);

    // Child inset by 2 (border + padding) on each side: 1x1 child -> 5x5 total.
    let size = container.layout(TuiConstraint::loose(TuiSize::new(20, 20)));
    assert_eq!(size, TuiSize::new(5, 5));

    assert_eq!(
        render_to_lines(&container, TuiSize::new(5, 5)),
        vec!["┌───┐", "│   │", "│ X │", "│   │", "└───┘"],
    );
}

#[test]
fn background_fills_the_padding_area() {
    let container = TuiContainer::new(TuiText::new("X"))
        .with_padding(1)
        .with_background(Color::Blue);

    let mut buffer = TuiBuffer::new(TuiSize::new(3, 3));
    container.render(TuiRect::new(0, 0, 3, 3), &mut buffer);

    // A padding cell carries the background fill...
    assert_eq!(
        buffer.get(0, 0).expect("cell in bounds").style().background,
        Some(Color::Blue),
    );
    // ...and the child glyph lands in the center.
    assert_eq!(buffer.get(1, 1).expect("cell in bounds").symbol(), "X");
}
