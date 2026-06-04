use warp_core::ui::theme::{Fill, WarpTheme};
use warp_core::ui::Icon;
use warpui::Element;

use crate::ai::execution_profiles::has_configurable_context_window;
use crate::ai::llms::{LLMInfo, LLMProvider};
use crate::persistence::model::ModelTokenUsage;

pub mod conversation_usage_view;
pub mod rollup;

pub fn has_long_context_usage(model_usage: &[ModelTokenUsage], llm: &LLMInfo) -> bool {
    model_usage.iter().any(|usage| {
        let is_active_model =
            usage.model_id == llm.display_name || usage.model_id == llm.id.as_str();
        is_active_model
            && usage.long_context_used
            && (usage.warp_tokens > 0 || usage.byok_tokens > 0)
    })
}

pub fn should_show_long_context_usage_warning(
    model_usage: &[ModelTokenUsage],
    llm: &LLMInfo,
) -> bool {
    llm.provider == LLMProvider::OpenAI
        && has_configurable_context_window(llm)
        && has_long_context_usage(model_usage, llm)
}

pub fn icon_for_context_window_usage(
    context_window_usage: f32,
    should_show_long_context_warning: bool,
) -> Icon {
    if should_show_long_context_warning {
        return Icon::ConversationContext100;
    }
    // Match the context window usage to the nearest 10% icon.
    if context_window_usage >= 0.95 {
        Icon::ConversationContext100
    } else if context_window_usage >= 0.85 {
        Icon::ConversationContext90
    } else if context_window_usage >= 0.75 {
        Icon::ConversationContext80
    } else if context_window_usage >= 0.65 {
        Icon::ConversationContext70
    } else if context_window_usage >= 0.55 {
        Icon::ConversationContext60
    } else if context_window_usage >= 0.45 {
        Icon::ConversationContext50
    } else if context_window_usage >= 0.35 {
        Icon::ConversationContext40
    } else if context_window_usage >= 0.25 {
        Icon::ConversationContext30
    } else if context_window_usage >= 0.15 {
        Icon::ConversationContext20
    } else if context_window_usage >= 0.05 {
        Icon::ConversationContext10
    } else {
        Icon::ConversationContext0
    }
}

pub fn render_context_window_usage_icon(
    context_window_usage: f32,
    theme: &WarpTheme,
    color_override: Option<Fill>,
) -> Box<dyn Element> {
    let icon = icon_for_context_window_usage(context_window_usage, false);

    let fill = if context_window_usage >= 0.8 {
        Fill::Solid(theme.ansi_fg_red())
    } else {
        color_override.unwrap_or_else(|| theme.main_text_color(theme.background()))
    };

    icon.to_warpui_icon(fill).finish()
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
