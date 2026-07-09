use warp_core::settings::Setting;
use warpui::elements::{
    Clipped, ConstrainedBox, Container, CrossAxisAlignment, DropTarget, Element, Flex, Hoverable,
    MainAxisSize, ParentElement, SavePosition, Stack,
};
use warpui::presenter::ChildView;
use warpui::{AppContext, SingletonEntity};

use super::common::{
    add_command_xray_overlay, add_input_suggestions_overlays, add_voltron_overlay,
    add_workflow_info_overlay, floating_input_box, should_show_terminal_input_message_bar,
    wrap_input_with_terminal_padding_and_focus_handler,
};
use super::{
    render_session_mode_segmented_control, Input, InputAction, InputDropTargetData,
    SessionModeSegment,
};
use crate::appearance::Appearance;
use crate::context_chips::spacing;
use crate::features::FeatureFlag;
use crate::settings::{AISettings, AppEditorSettings, InputModeSettings};
use crate::terminal::block_list_viewport::InputMode;
use crate::terminal::settings::TerminalSettings;
use crate::terminal::view::TerminalAction;
use crate::view_components::action_button::ButtonSize;

impl Input {
    /// Renders the terminal mode input when `FeatureFlag::AgentView` is enabled and there is no
    /// active agent view.
    pub(super) fn render_terminal_input(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let menu_positioning = self.menu_positioning(app);

        let model = self.model.lock();

        // We should likely rework this stack to not need to use `with_constrain_absolute_children`,
        // by reworking the positioning of the children to not depend on this.
        let mut stack = Stack::new().with_constrain_absolute_children();

        let vim_state = self.editor.as_ref(app).vim_state(app);
        let app_editor_settings = AppEditorSettings::as_ref(app);
        let show_vim_status = vim_state.is_some() && *app_editor_settings.vim_status_bar.value();
        let input_mode = *InputModeSettings::as_ref(app).input_mode.value();

        let mut column = Flex::column();

        if matches!(input_mode, InputMode::PinnedToBottom | InputMode::Waterfall) {
            if let Some(banner) = self.render_input_banner(appearance, app, input_mode, false) {
                column.add_child(
                    Container::new(banner)
                        .with_margin_top(spacing::UDI_CHIP_MARGIN)
                        .finish(),
                );
            }
        }

        // Persistent session-mode segmented control ("Agent | Cloud Agent | Terminal").
        // Only shown when agent modes are actually usable so the plain terminal input is
        // left untouched when AI is disabled.
        let show_session_mode_control =
            FeatureFlag::AgentView.is_enabled() && AISettings::as_ref(app).is_any_ai_enabled(app);
        let show_message_bar = should_show_terminal_input_message_bar(&model, app);

        // Once bootstrapped, the prompt row is just context chips (working directory, git
        // branch, ...). When the session-mode footer is shown, move those chips into the
        // footer below the editor so their placement matches the Agent footer; the
        // pre-bootstrap "Starting…" status stays above the editor.
        let move_chips_to_footer =
            show_session_mode_control && model.block_list().is_bootstrapped();

        // In the footer the chips are baseline-aligned with the segmented control, so
        // skip the above-editor top padding that would otherwise offset their text.
        let prompt_elements = self
            .prompt_render_helper
            .render_universal_developer_input_prompt(
                &model,
                appearance,
                !move_chips_to_footer,
                app,
            );

        let mut footer_prompt_chips = None;
        if move_chips_to_footer {
            footer_prompt_chips = Some(prompt_elements);
        } else {
            column.add_child(prompt_elements);
        }

        let terminal_spacing = TerminalSettings::as_ref(app)
            .terminal_input_spacing(appearance.line_height_ratio(), app);
        column.add_child(
            Container::new(self.render_input_box(show_vim_status, appearance, app))
                .with_margin_top(
                    terminal_spacing.prompt_to_editor_padding
                        * spacing::UDI_PROMPT_BOTTOM_PADDING_FACTOR,
                )
                .finish(),
        );

        if show_session_mode_control {
            let include_cloud_agent = FeatureFlag::CloudMode.is_enabled();
            let mut mode_row = Flex::row()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(render_session_mode_segmented_control(
                    SessionModeSegment::Terminal,
                    include_cloud_agent,
                    &self.session_mode_mouse_states,
                    appearance,
                ));

            // Context chips (working directory, git branch, ...) moved down from the prompt
            // row, placed to the right of the segmented control to mirror the Agent footer.
            if let Some(chips) = footer_prompt_chips.take() {
                mode_row = mode_row.with_child(Container::new(chips).with_margin_left(8.).finish());
            }

            // Match the Agent footer's vertical spacing exactly: no top padding (the gap from the
            // editor comes from the editor's own `margin_top`, shared with the agent render path)
            // and an 8px bottom padding identical to the agent footer container
            // (`Container::new(content).with_padding_bottom(8.0)` in agent_input_footer/mod.rs).
            //
            // The agent footer's row height is driven by its `AgentInputButton`-sized chips
            // (model selector, send, etc.), which vertically center the segmented control. The
            // terminal footer has no such chips, so reserve the same row height and center the
            // control here — otherwise it collapses to the control's own height and sits lower
            // (closer to the pane bottom) than in agent mode.
            let footer_row_height = ButtonSize::AgentInputButton.button_height(appearance, app);
            column.add_child(
                Container::new(
                    ConstrainedBox::new(mode_row.finish())
                        .with_min_height(footer_row_height)
                        .finish(),
                )
                .with_padding_bottom(8.0)
                .finish(),
            );
        } else if !(matches!(input_mode, InputMode::PinnedToTop)
            && self
                .suggestions_mode_model
                .as_ref(app)
                .is_inline_menu_open())
        {
            column.add_child(
                Container::new(Flex::row().finish())
                    .with_margin_bottom(8.)
                    .finish(),
            );
        }

        if matches!(input_mode, InputMode::PinnedToTop) {
            if let Some(banner) = self.render_input_banner(appearance, app, input_mode, false) {
                column.add_child(
                    Container::new(banner)
                        .with_margin_bottom(spacing::UDI_CHIP_MARGIN)
                        .finish(),
                );
            }
        }

        stack.add_child(wrap_input_with_terminal_padding_and_focus_handler(
            self.focus_handle
                .as_ref()
                .is_some_and(|h| h.is_active_session(app)),
            column.finish(),
            false,
        ));

        if let Some(selected_workflow_state) = self.workflows_state.selected_workflow_state.as_ref()
        {
            if selected_workflow_state.should_show_more_info_view {
                add_workflow_info_overlay(
                    &mut stack,
                    selected_workflow_state,
                    self.size_info(app).pane_height_px().as_f32(),
                    menu_positioning,
                );
            }
        }

        let is_focused = self.focus_handle.as_ref().is_none_or(|h| h.is_focused(app));
        if self.is_voltron_open && is_focused {
            add_voltron_overlay(&mut stack, &self.voltron_view, menu_positioning);
        }

        if is_focused {
            add_input_suggestions_overlays(self, &mut stack, appearance, menu_positioning, app);
        }

        if let Some(token_description) = &self.command_x_ray_description {
            add_command_xray_overlay(
                self,
                &mut stack,
                token_description,
                appearance,
                menu_positioning,
                app,
            );
        }

        let drop_target = DropTarget::new(
            SavePosition::new(stack.finish(), &self.status_free_input_save_position_id()).finish(),
            InputDropTargetData::new(self.weak_view_handle.clone()),
        )
        .finish();

        let hoverable_input = Hoverable::new(self.hoverable_handle.clone(), |_| drop_target)
            .on_hover(|is_hovered, ctx, _app, _position| {
                ctx.dispatch_typed_action(InputAction::SetUDIHovered(is_hovered));
            })
            .on_middle_click(|ctx, _app, _position| {
                ctx.dispatch_typed_action(TerminalAction::MiddleClickOnInput)
            })
            .finish();

        let input = floating_input_box(
            hoverable_input,
            styles::default_border_color(appearance.theme()),
            appearance,
        )
        // The shared floating box fill (`surface_overlay_2`) is a translucent 10%-foreground
        // overlay, so over the dark terminal content it reads noticeably darker than the Agent
        // input. The Agent input effectively composites the agent-view surface
        // (`surface_overlay_1`, 5% fg) under its own box fill (`surface_overlay_2`, 10% fg), for
        // ~15% foreground. Paint the opaque equivalent (`neutral_3` = 15% fg over the background)
        // so the terminal box shows a stable fill that matches the Agent input regardless of
        // what's behind it.
        .with_background_color(crate::ui_components::blended_colors::neutral_3(
            appearance.theme(),
        ))
        .with_padding_bottom(4.)
        .finish();

        // Contextual hints (e.g. "⌘↑ attach output as agent context") render above the input
        // box here, matching where the Agent input renders its message bar.
        let message_bar_above = show_message_bar.then(|| {
            Clipped::new(ChildView::new(&self.terminal_input_message_bar).finish()).finish()
        });

        let mut column = Flex::column();
        let is_slash_commands = self.suggestions_mode_model.as_ref(app).is_slash_commands();
        let is_conversation_menu = self
            .suggestions_mode_model
            .as_ref(app)
            .is_conversation_menu();
        let is_prompts_menu = self.suggestions_mode_model.as_ref(app).is_prompts_menu();
        let is_skill_menu = self.suggestions_mode_model.as_ref(app).is_skill_menu();
        let is_inline_history_menu = self
            .suggestions_mode_model
            .as_ref(app)
            .is_inline_history_menu();
        let is_repos_menu = self.suggestions_mode_model.as_ref(app).is_repos_menu();
        let hide_menu = self
            .inline_terminal_menu_positioner
            .as_ref(app)
            .should_hide_inline_menu_for_pane_size(app);
        match input_mode {
            InputMode::PinnedToBottom => {
                column.add_children(
                    [
                        if hide_menu {
                            None
                        } else if is_slash_commands {
                            Some(ChildView::new(&self.inline_slash_commands_view).finish())
                        } else if is_prompts_menu {
                            Some(ChildView::new(&self.inline_prompts_menu_view).finish())
                        } else if is_conversation_menu {
                            Some(ChildView::new(&self.inline_conversation_menu_view).finish())
                        } else if FeatureFlag::ListSkills.is_enabled() && is_skill_menu {
                            Some(ChildView::new(&self.inline_skill_selector_view).finish())
                        } else if is_inline_history_menu {
                            Some(ChildView::new(&self.inline_history_menu_view).finish())
                        } else if is_repos_menu {
                            Some(ChildView::new(&self.inline_repos_menu_view).finish())
                        } else {
                            None
                        },
                        Some(ChildView::new(&self.agent_status_view).finish()),
                        message_bar_above,
                        Some(input),
                    ]
                    .into_iter()
                    .flatten(),
                );
            }
            InputMode::PinnedToTop => {
                column.add_children(
                    [
                        message_bar_above,
                        Some(input),
                        Some(ChildView::new(&self.agent_status_view).finish()),
                        if hide_menu {
                            None
                        } else if is_slash_commands {
                            Some(ChildView::new(&self.inline_slash_commands_view).finish())
                        } else if is_prompts_menu {
                            Some(ChildView::new(&self.inline_prompts_menu_view).finish())
                        } else if is_conversation_menu {
                            Some(ChildView::new(&self.inline_conversation_menu_view).finish())
                        } else if FeatureFlag::ListSkills.is_enabled() && is_skill_menu {
                            Some(ChildView::new(&self.inline_skill_selector_view).finish())
                        } else if is_inline_history_menu {
                            Some(ChildView::new(&self.inline_history_menu_view).finish())
                        } else if is_repos_menu {
                            Some(ChildView::new(&self.inline_repos_menu_view).finish())
                        } else {
                            None
                        },
                    ]
                    .into_iter()
                    .flatten(),
                );
            }
            InputMode::Waterfall => {
                let should_render_below = self
                    .inline_terminal_menu_positioner
                    .as_ref(app)
                    .should_render_inline_menu_below_input();

                if !hide_menu {
                    if is_slash_commands && !should_render_below {
                        column.add_child(ChildView::new(&self.inline_slash_commands_view).finish());
                    } else if is_prompts_menu && !should_render_below {
                        column.add_child(ChildView::new(&self.inline_prompts_menu_view).finish());
                    } else if is_conversation_menu && !should_render_below {
                        column.add_child(
                            ChildView::new(&self.inline_conversation_menu_view).finish(),
                        );
                    } else if FeatureFlag::ListSkills.is_enabled()
                        && is_skill_menu
                        && !should_render_below
                    {
                        column.add_child(ChildView::new(&self.inline_skill_selector_view).finish());
                    } else if is_inline_history_menu && !should_render_below {
                        column.add_child(ChildView::new(&self.inline_history_menu_view).finish());
                    } else if is_repos_menu && !should_render_below {
                        column.add_child(ChildView::new(&self.inline_repos_menu_view).finish());
                    }
                }

                column.add_children(
                    [
                        Some(ChildView::new(&self.agent_status_view).finish()),
                        message_bar_above,
                        Some(input),
                    ]
                    .into_iter()
                    .flatten(),
                );

                if !hide_menu {
                    if is_slash_commands && should_render_below {
                        column.add_child(ChildView::new(&self.inline_slash_commands_view).finish());
                    } else if is_prompts_menu && should_render_below {
                        column.add_child(ChildView::new(&self.inline_prompts_menu_view).finish());
                    } else if is_conversation_menu && should_render_below {
                        column.add_child(
                            ChildView::new(&self.inline_conversation_menu_view).finish(),
                        );
                    } else if FeatureFlag::ListSkills.is_enabled()
                        && is_skill_menu
                        && should_render_below
                    {
                        column.add_child(ChildView::new(&self.inline_skill_selector_view).finish());
                    } else if is_inline_history_menu && should_render_below {
                        column.add_child(ChildView::new(&self.inline_history_menu_view).finish());
                    } else if is_repos_menu && should_render_below {
                        column.add_child(ChildView::new(&self.inline_repos_menu_view).finish());
                    }
                }
            }
        }

        SavePosition::new(column.finish(), &self.save_position_id()).finish()
    }
}

pub mod styles {
    use pathfinder_color::ColorU;
    use warp_core::ui::theme::WarpTheme;

    use crate::ui_components::blended_colors;

    pub fn default_border_color(theme: &WarpTheme) -> ColorU {
        // Match the Agent input box border (`agent::styles::default_border_color`).
        blended_colors::neutral_2(theme)
    }
}
