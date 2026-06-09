//! Tests for the context-window usage circle icon mapping and the
//! long-context warning state.
//!
//! Regression guard for the color semantics of the context-window circle:
//! the solid (white) marks represent the context *remaining*, not the amount
//! used. An empty conversation (0% used → 100% remaining) shows a full white
//! circle and it counts down to an all-grey circle as the window fills up
//! (100% used → 0% remaining).

use warp_core::ui::Icon;

use super::{icon_for_context_window_usage, LongContextWarningState};
use crate::ai::llms::{LLMId, LLMProvider};

#[test]
fn new_initializes_visibility_from_long_context_used() {
    let visible = LongContextWarningState::new(LLMId::from("gpt-5"), LLMProvider::OpenAI, true);
    assert!(visible.is_visible());

    let hidden = LongContextWarningState::new(LLMId::from("gpt-5"), LLMProvider::OpenAI, false);
    assert!(!hidden.is_visible());
}

#[test]
fn sync_from_server_overwrites_visibility() {
    let mut state = LongContextWarningState::new(LLMId::from("gpt-5"), LLMProvider::OpenAI, false);
    state.sync_from_server(true);
    assert!(state.is_visible());

    // A later short request clears the warning.
    state.sync_from_server(false);
    assert!(!state.is_visible());
}

#[test]
fn changing_effective_model_resets_warning() {
    let mut state = LongContextWarningState::new(LLMId::from("gpt-5"), LLMProvider::OpenAI, true);
    assert!(state.is_visible());

    // Selecting a different base model hides the prior model's warning. Both models are
    // OpenAI here so this isolates the model-change reset from the provider gate.
    state.update_effective_model(LLMId::from("gpt-5.1"), LLMProvider::OpenAI);
    assert!(!state.is_visible());
}

#[test]
fn reselecting_same_effective_model_does_not_reset_warning() {
    let mut state = LongContextWarningState::new(LLMId::from("gpt-5"), LLMProvider::OpenAI, true);
    assert!(state.is_visible());

    // Re-selecting the same effective model must not reset the warning.
    state.update_effective_model(LLMId::from("gpt-5"), LLMProvider::OpenAI);
    assert!(state.is_visible());
}

#[test]
fn server_value_remains_authoritative_after_model_change() {
    let mut state = LongContextWarningState::new(LLMId::from("gpt-5"), LLMProvider::OpenAI, true);
    state.update_effective_model(LLMId::from("gpt-5.1"), LLMProvider::OpenAI);
    assert!(!state.is_visible());

    // The next streamed/restored server value can show the warning again.
    state.sync_from_server(true);
    assert!(state.is_visible());
}

#[test]
fn warning_hidden_for_non_openai_provider_even_when_long_context_used() {
    // The server may report long-context usage for non-OpenAI models (e.g. Gemini), but the
    // OpenAI-specific pricing warning must not surface for them.
    let anthropic =
        LongContextWarningState::new(LLMId::from("claude-sonnet"), LLMProvider::Anthropic, true);
    assert!(!anthropic.is_visible());

    let google =
        LongContextWarningState::new(LLMId::from("gemini-3-pro"), LLMProvider::Google, true);
    assert!(!google.is_visible());
}

#[test]
fn sync_from_server_does_not_show_for_non_openai_provider() {
    let mut state =
        LongContextWarningState::new(LLMId::from("claude-sonnet"), LLMProvider::Anthropic, false);
    state.sync_from_server(true);
    assert!(!state.is_visible());
}

#[test]
fn switching_to_non_openai_model_hides_warning_even_with_server_true() {
    let mut state = LongContextWarningState::new(LLMId::from("gpt-5"), LLMProvider::OpenAI, true);
    assert!(state.is_visible());

    // Switching to a non-OpenAI model hides the warning, and a later server "true" must not
    // resurface it while a non-OpenAI model is the effective model.
    state.update_effective_model(LLMId::from("claude-sonnet"), LLMProvider::Anthropic);
    assert!(!state.is_visible());
    state.sync_from_server(true);
    assert!(!state.is_visible());
}

#[test]
fn long_context_warning_forces_full_icon() {
    // With the warning active, the icon shows the context-full state regardless of usage.
    assert_eq!(
        icon_for_context_window_usage(0.0, true),
        Icon::ContextRemaining0
    );
    assert_eq!(
        icon_for_context_window_usage(0.5, true),
        Icon::ContextRemaining0
    );
}

#[test]
fn empty_conversation_shows_full_white_circle() {
    // 0% used == 100% remaining -> all-white circle.
    assert_eq!(
        icon_for_context_window_usage(0.0, false),
        Icon::ContextRemaining100
    );
}

#[test]
fn full_context_window_shows_all_grey_circle() {
    // 100% used == 0% remaining -> all-grey circle.
    assert_eq!(
        icon_for_context_window_usage(1.0, false),
        Icon::ContextRemaining0
    );
}

#[test]
fn icon_brightness_tracks_remaining_not_used() {
    // Lightly-used conversation: lots of context remaining -> mostly white.
    assert_eq!(
        icon_for_context_window_usage(0.1, false),
        Icon::ContextRemaining90
    );
    // Half used -> half white.
    assert_eq!(
        icon_for_context_window_usage(0.5, false),
        Icon::ContextRemaining50
    );
    // Heavily used (the original report's 88%): little remaining -> mostly grey.
    assert_eq!(
        icon_for_context_window_usage(0.88, false),
        Icon::ContextRemaining10
    );
}

#[test]
fn mapping_is_monotonic_more_usage_never_brightens_the_circle() {
    // As usage increases, the number of bright (remaining) marks must be
    // non-increasing — the circle only ever empties as context fills.
    let icon_rank = |usage: f32| match icon_for_context_window_usage(usage, false) {
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
            current <= previous,
            "icon brightness increased as usage rose to {usage}: {previous} -> {current}"
        );
        previous = current;
        usage += 0.05;
    }
}
