use super::TuiColumn;
use crate::elements::{TuiElement, TuiText};
use crate::{TuiBuffer, TuiConstraint, TuiRect, TuiSize};

fn render_to_lines(element: &dyn TuiElement, size: TuiSize) -> Vec<String> {
    let mut buffer = TuiBuffer::new(size);
    element.render(TuiRect::from_size(size), &mut buffer);
    buffer.to_lines()
}

#[test]
fn stacks_two_children_top_to_bottom() {
    let mut column = TuiColumn::new()
        .child(TuiText::new("AA"))
        .child(TuiText::new("BB"));

    assert_eq!(column.desired_height(2), 2);
    let size = column.layout(TuiConstraint::loose(TuiSize::new(2, 10)));
    assert_eq!(size, TuiSize::new(2, 2));

    assert_eq!(
        render_to_lines(&column, TuiSize::new(2, 2)),
        vec!["AA", "BB"]
    );
}

#[test]
fn sums_multi_row_children_at_the_correct_offsets() {
    // The middle child spans two rows, so the trailing child must land on row 3.
    let column = TuiColumn::new()
        .child(TuiText::new("A"))
        .child(TuiText::new("BB\nCC").truncate())
        .child(TuiText::new("D"));

    assert_eq!(column.desired_height(2), 4);
    assert_eq!(
        render_to_lines(&column, TuiSize::new(2, 4)),
        vec!["A ", "BB", "CC", "D "],
    );
}

#[test]
fn clamps_total_height_to_the_constraint_and_clips_overflow() {
    let mut column = TuiColumn::new()
        .child(TuiText::new("A"))
        .child(TuiText::new("BB\nCC").truncate())
        .child(TuiText::new("D"));

    let size = column.layout(TuiConstraint::new(TuiSize::ZERO, TuiSize::new(2, 3)));
    assert_eq!(size, TuiSize::new(2, 3));

    // Only the first three rows fit; the final child is clipped away.
    assert_eq!(
        render_to_lines(&column, TuiSize::new(2, 3)),
        vec!["A ", "BB", "CC"],
    );
}
