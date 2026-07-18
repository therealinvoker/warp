//! Tests for the context-window usage ring icon mapping.
//!
//! Regression guard for the semantics of the context-window ring: the solid
//! (white) arc represents the context *used*, not the amount remaining. An
//! empty conversation (0% used) shows just the dim track and the bright arc
//! sweeps to a full ring as the window fills (100% used). The white sweep
//! therefore equals the used fraction (`1 - remaining`).

use warp_core::ui::Icon;

use super::icon_for_context_window_usage;

#[test]
fn empty_conversation_shows_all_grey_circle() {
    // 0% used -> empty (all-grey) circle.
    assert_eq!(icon_for_context_window_usage(0.0), Icon::ContextRemaining0);
}

#[test]
fn full_context_window_shows_full_white_circle() {
    // 100% used -> full white circle.
    assert_eq!(
        icon_for_context_window_usage(1.0),
        Icon::ContextRemaining100
    );
}

#[test]
fn icon_brightness_tracks_used_not_remaining() {
    // Lightly-used conversation: little context used -> mostly grey.
    assert_eq!(icon_for_context_window_usage(0.1), Icon::ContextRemaining10);
    // Half used -> half white.
    assert_eq!(icon_for_context_window_usage(0.5), Icon::ContextRemaining50);
    // Heavily used (the original report's 88%): lots used -> mostly white.
    assert_eq!(
        icon_for_context_window_usage(0.88),
        Icon::ContextRemaining90
    );
}

#[test]
fn mapping_is_monotonic_more_usage_never_dims_the_circle() {
    // As usage increases, the number of bright (used) marks must be
    // non-decreasing — the circle only ever fills as context fills.
    let icon_rank = |usage: f32| match icon_for_context_window_usage(usage) {
        Icon::ContextRemaining0 => 0,
        Icon::ContextRemaining10 => 10,
        Icon::ContextRemaining20 => 20,
        Icon::ContextRemaining30 => 30,
        Icon::ContextRemaining40 => 40,
        Icon::ContextRemaining50 => 50,
        Icon::ContextRemaining60 => 60,
        Icon::ContextRemaining70 => 70,
        Icon::ContextRemaining80 => 80,
        Icon::ContextRemaining90 => 90,
        Icon::ContextRemaining100 => 100,
        other => panic!("unexpected icon: {other:?}"),
    };

    let mut usage = 0.0;
    let mut previous = icon_rank(usage);
    while usage <= 1.0 {
        let current = icon_rank(usage);
        assert!(
            current >= previous,
            "icon brightness decreased as usage rose to {usage}: {previous} -> {current}"
        );
        previous = current;
        usage += 0.05;
    }
}
