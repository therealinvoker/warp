pub(crate) mod agent_input_footer;
mod agent_message_bar;
pub(crate) mod agent_progress_modal;
mod agent_view_block;
mod controller;
mod conversation_selection;
mod ephemeral_message_model;
mod inline_agent_view_header;
// TODO: Move orchestration_conversation_links module import elsewhere.
pub(crate) mod orchestration_avatar;
pub(crate) mod orchestration_conversation_links;
pub mod orchestration_pill_bar;
pub mod orchestration_pill_bar_model;
pub mod shortcuts;
pub(crate) mod working_agents_indicator;
mod zero_state_block;

use std::sync::LazyLock;

pub use agent_input_footer::*;
pub use agent_message_bar::*;
pub use agent_view_block::*;
pub use controller::*;
pub(crate) use conversation_selection::AgentViewConversationSelection;
pub use ephemeral_message_model::*;
pub use inline_agent_view_header::*;
pub use orchestration_pill_bar::{render_orchestration_breadcrumbs, OrchestrationPillBar};
use pathfinder_color::ColorU;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::color::blend::Blend;
use warp_core::ui::theme::Fill;
use warpui::keymap::Keystroke;
use warpui::{AppContext, SingletonEntity};
pub use zero_state_block::*;

use crate::terminal::model::TerminalModel;

pub static ENTER_AGENT_VIEW_NEW_CONVERSATION_KEYSTROKE: LazyLock<Keystroke> = LazyLock::new(|| {
    cfg_if::cfg_if! {
        if #[cfg(target_os = "macos")] {
            Keystroke {
                cmd: true,
                key: "enter".to_owned(),
                ..Default::default()
            }
        } else {
            Keystroke {
                ctrl: true,
                shift: true,
                key: "enter".to_owned(),
                ..Default::default()
            }
        }
    }
});

pub static ENTER_CLOUD_AGENT_VIEW_NEW_CONVERSATION_KEYSTROKE: LazyLock<Keystroke> =
    LazyLock::new(|| {
        cfg_if::cfg_if! {
            if #[cfg(target_os = "macos")] {
                Keystroke {
                    cmd: true,
                    alt: true,
                    key: "enter".to_owned(),
                    ..Default::default()
                }
            } else {
                Keystroke {
                    ctrl: true,
                    alt: true,
                    key: "enter".to_owned(),
                    ..Default::default()
                }
            }
        }
    });

/// Returns `true` when the current pane is in a cloud or remote context.
pub fn is_in_cloud_context(
    agent_view_state: &AgentViewState,
    terminal_model: &TerminalModel,
) -> bool {
    let origin_is_cloud = matches!(
        agent_view_state,
        AgentViewState::Active { origin, .. }
            if matches!(
                origin,
                AgentViewEntryOrigin::CloudAgent | AgentViewEntryOrigin::ThirdPartyCloudAgent
            )
    );
    origin_is_cloud
        || terminal_model.is_conversation_transcript_viewer()
        || terminal_model.is_dummy_cloud_mode_session()
}

pub fn agent_view_bg_fill(app: &AppContext) -> Fill {
    let appearance = Appearance::as_ref(app);
    appearance.theme().surface_overlay_1()
}

pub fn agent_view_bg_color(app: &AppContext) -> ColorU {
    agent_view_bg_fill(app)
        .blend(&Appearance::as_ref(app).theme().background())
        .into_solid()
}
