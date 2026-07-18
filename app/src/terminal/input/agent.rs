use std::sync::atomic::Ordering;

use warp_cli::agent::Harness;
use warp_core::settings::Setting;
use warp_core::ui::theme::Fill;
use warpui::elements::{
    AnchorPair, Border, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
    DispatchEventResult, DropShadow, DropTarget, Element, Empty, EventHandler, Expanded, Flex,
    Hoverable, MainAxisSize, OffsetPositioning, OffsetType, ParentElement,
    PositionedElementOffsetBounds, PositioningAxis, Radius, SavePosition, Stack, XAxisAnchor,
    YAxisAnchor,
};
use warpui::presenter::ChildView;
use warpui::{AppContext, SingletonEntity as _};

use super::common::{
    add_command_xray_overlay, add_input_suggestions_overlays, add_voltron_overlay,
    add_workflow_info_overlay, floating_input_box, maybe_add_buy_credits_banner,
    wrap_input_with_terminal_padding_and_focus_handler, FLOATING_INPUT_MARGIN,
};
use super::{Input, InputAction, InputDropTargetData};
use crate::ai::blocklist::agent_view::shortcuts::{
    render_agent_shortcuts_view, AgentShortcutsViewContext,
};
use crate::ai::blocklist::agent_view::AgentViewState;
use crate::ai::blocklist::InputType;
use crate::ai::harness_availability::HarnessAvailabilityModel;
use crate::appearance::Appearance;
use crate::context_chips::spacing::{self};
use crate::editor::position_id_for_cursor;
use crate::features::FeatureFlag;
use crate::settings::InputModeSettings;
use crate::terminal::input::inline_menu::styles as inline_styles;
use crate::terminal::settings::TerminalSettings;
use crate::terminal::view::{TerminalAction, PADDING_LEFT};
use crate::BlocklistAIHistoryModel;

pub(super) const CLOUD_MODE_V2_MAX_WIDTH: f32 = 720.;

const CLOUD_MODE_V2_TOP_ROW_GAP: f32 = 10.;

const CLOUD_MODE_V2_TOP_ROW_INNER_GAP: f32 = 4.;

// Top padding above the attachment chips row inside the V2 input container.
const CLOUD_MODE_V2_CHIPS_ROW_TOP_PADDING: f32 = 4.;

// Gap between the top of the input box and the bottom of the inline suggestion
// menu overlay (slash commands, model/profile selector, etc.).
const INLINE_MENU_OVERLAY_GAP: f32 = 4.;
// Corner radius for the inline suggestion menu overlay frame (matches
// cloud-mode-v2's `MENU_CORNER_RADIUS`).
const INLINE_MENU_OVERLAY_CORNER_RADIUS: f32 = 6.;

impl Input {
    pub fn is_cloud_mode_input_v2_composing(&self, app: &AppContext) -> bool {
        FeatureFlag::CloudModeInputV2.is_enabled()
            && FeatureFlag::CloudMode.is_enabled()
            && self.ambient_agent_view_model().is_some_and(|model| {
                let view_model = model.as_ref(app);
                view_model.is_configuring_ambient_agent()
                    // The handoff pane intentionally stays on the existing input UI even
                    // when V2 is on — V2 is for fresh cloud-mode runs only, and handoff has
                    // its own pre-spawn flow (submit interception).
                    && !view_model.is_local_to_cloud_handoff()
            })
    }

    /// Renders the input when there is an active `AgentView`.
    ///
    /// Only used when `FeatureFlag::AgentView` is enabled.
    pub(super) fn render_agent_input(&self, app: &AppContext) -> Box<dyn Element> {
        if self.is_cloud_mode_input_v2_composing(app) {
            return self.render_cloud_mode_v2_composing_input(app);
        }

        let appearance = Appearance::as_ref(app);
        let menu_positioning = self.menu_positioning(app);

        // In the standard agent layout the footer toolbar is split across the
        // grey input box: mic/lightning/attachment stay inside, the rest render
        // on a row below the box. Cloud-mode-v2 and active CLI-agent footers keep
        // the combined single row. Computed once; both inputs are lock-free.
        let use_split_footer = self
            .agent_input_footer
            .as_ref(app)
            .uses_standard_split_layout(app);

        // We should likely rework this stack to not need to use `with_constrain_absolute_children`,
        // by reworking the positioning of the children to not depend on this.
        let mut stack = Stack::new().with_constrain_absolute_children();

        let input_mode = *InputModeSettings::as_ref(app).input_mode.value();

        let mut column = Flex::column();

        if let Some(banner) =
            self.render_input_banner(appearance, app, input_mode, /*is_compact_mode=*/ false)
        {
            column.add_child(
                Container::new(banner)
                    .with_margin_top(spacing::UDI_CHIP_MARGIN)
                    .finish(),
            );
        }

        let ai_input_model = self.ai_input_model.as_ref(app);

        if FeatureFlag::ImageAsContext.is_enabled()
            && matches!(ai_input_model.input_type(), InputType::AI)
        {
            if let Some(images) = self.render_attachment_chips(appearance) {
                column.add_child(
                    Container::new(images)
                        .with_margin_top(spacing::UDI_CHIP_MARGIN)
                        .finish(),
                );
            }
        }

        let show_harness_row = FeatureFlag::CloudMode.is_enabled()
            && HarnessAvailabilityModel::as_ref(app).should_show_harness_selector()
            && self
                .ambient_agent_view_model()
                .is_some_and(|ambient_agent_model| {
                    ambient_agent_model
                        .as_ref(app)
                        .is_configuring_ambient_agent()
                });
        if show_harness_row {
            if let Some(harness_selector) = self.harness_selector() {
                // Temporarily render the harness selector in the cloud mode UDI until we fully
                // implement the new designs.
                let harness_row = Flex::row()
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_child(ChildView::new(harness_selector).finish())
                    .finish();
                column.add_child(
                    Container::new(harness_row)
                        .with_padding_top(spacing::UDI_CHIP_MARGIN)
                        .with_padding_bottom(4.)
                        .finish(),
                );
            }
        }

        let terminal_spacing = TerminalSettings::as_ref(app)
            .terminal_input_spacing(appearance.line_height_ratio(), app);
        let editor_top_margin =
            terminal_spacing.prompt_to_editor_padding * spacing::UDI_PROMPT_BOTTOM_PADDING_FACTOR;
        // Small, balanced vertical padding for the single-row composer: the editor
        // is rendered with this bottom padding (instead of the larger
        // footer-below padding) and mirrored on top below so the text sits centered
        // with tight, equal space above and below.
        const SPLIT_EDITOR_VPAD: f32 = 4.;
        let split_bottom_padding = if use_split_footer {
            Some(SPLIT_EDITOR_VPAD)
        } else {
            None
        };
        let editor_box = self.render_input_box(
            /*show_vim_status=*/ false,
            split_bottom_padding,
            appearance,
            app,
        );
        // The footer view is always drawn as a `ChildView` (never by building its
        // buttons inline here) so its controls stay parented to `AgentInputFooter`
        // and their `AgentInputFooterAction` clicks route to the footer's handler —
        // otherwise the mic/lightning buttons render but are silently unclickable.
        let footer_controls = SavePosition::new(
            ChildView::new(&self.agent_input_footer).finish(),
            &self.prompt_save_position_id(),
        )
        .finish();

        if use_split_footer {
            // Single composer row: `[+] [editor] [mic/lightning]` laid out as a
            // real flex row (no overlay). The "+" attach button is owned by
            // `Input` (it dispatches `InputAction::SelectImage`) so it can sit as
            // a first-class flex child to the *left* of the editor while still
            // routing correctly; the footer `ChildView` draws only the right-hand
            // mic/lightning cluster, keeping those parented to `AgentInputFooter`.
            // A plain flex row (rather than the previous Stack overlay) keeps the
            // editor a normal hit target so it stays clickable. The row's cross-axis
            // alignment (chosen below) keeps the `+`/mic/lightning lined up with the
            // editor's first line of text whether it is one line or many.
            let attach_button =
                Container::new(ChildView::new(&self.composer_attach_button).finish())
                    .with_margin_right(spacing::UDI_CHIP_MARGIN)
                    .finish();
            // The editor is rendered with a small bottom padding (`SPLIT_EDITOR_VPAD`)
            // for this inline-footer layout; mirror it on top so the single text line
            // is vertically centered with tight, equal space above and below, and
            // `CrossAxisAlignment::Center` lines the `+`/mic/lightning up with it.
            let editor_box = Container::new(editor_box)
                .with_padding_top(SPLIT_EDITOR_VPAD)
                .finish();

            // Decide the composer row's vertical alignment — single-line centered
            // vs. multi-line top-aligned (see the row built below) — from a
            // *width-stable* signal rather than the editor's live soft-wrap count.
            // The editor is always an `Expanded` child of the row, so its measured
            // width doesn't change when the text wraps; but a soft-wrap count read
            // from the current layout would still flip-flop for borderline-length
            // text on any re-render (notably mouseover).
            //
            // Instead, compare the text's rendered width against the editor's
            // measured width (cached across frames). That reference is stable, so
            // the alignment only changes when the text itself does: it stays a
            // centered single line until the text no longer fits on one line.
            let is_multiline = {
                let editor = self.editor.as_ref(app);
                let text = editor.buffer_text(app);
                if text.contains('\n') {
                    // An explicit newline is unambiguously multiline, width aside.
                    true
                } else {
                    let was_multiline = self.composer_controls_below.load(Ordering::Relaxed);
                    // The editor's measured width *is* the authoritative "fits on one
                    // line" width — cache it while the row is centered (single-line) so
                    // the alignment decision stays put across frames.
                    if !was_multiline {
                        let window_id = self.editor.window_id(app);
                        if let Some(rect) = app.element_position_by_id_at_last_frame(
                            window_id,
                            self.editor_save_position_id(),
                        ) {
                            self.composer_single_row_editor_width_bits
                                .store(rect.width().to_bits(), Ordering::Relaxed);
                        }
                    }
                    let single_row_width = f32::from_bits(
                        self.composer_single_row_editor_width_bits
                            .load(Ordering::Relaxed),
                    );
                    let em_width = editor.em_width(app.font_cache(), appearance);
                    let text_width = text.chars().count() as f32 * em_width;
                    // Switch a couple of characters early (the cached width includes the
                    // box's inner padding) so the row top-aligns *before* the text would
                    // visibly wrap inside it.
                    single_row_width > 1. && text_width > single_row_width - em_width * 2.
                }
            };
            self.composer_controls_below
                .store(is_multiline, Ordering::Relaxed);

            // Single line vs. multi-line composer layout. A single line keeps the
            // compact one-row form `[+] [editor] [model/mic/lightning]`; once the
            // text wraps to 2+ lines the control row drops *below* a full-width
            // editor (Cursor-style) so long content isn't squeezed beside the
            // inline controls. The decision is driven by the width-stable
            // `is_multiline` signal above, so the layout doesn't flip-flop.
            if is_multiline {
                // Multi-line: the editor spans the full width on its own row and
                // the control row (`[+] … [model/mic/lightning]`) sits below it, with
                // the `+` and the footer cluster pushed to opposite ends by an
                // expanding spacer.
                let controls_row = Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(attach_button)
                    .with_child(Expanded::new(1., Empty::new().finish()).finish())
                    .with_child(footer_controls)
                    .finish();
                // The editor already carries `SPLIT_EDITOR_VPAD` bottom padding, so
                // the control row needs no extra top margin — an additional margin
                // here stacks on that padding and nudges the `+`/controls a few
                // pixels lower than the single-row layout.
                column.add_child(Container::new(editor_box).finish());
                column.add_child(Container::new(controls_row).finish());
            } else {
                // Single line: `[+] [editor] [model/mic/lightning]` on one
                // vertically-centered flex row, with the editor as an `Expanded`
                // middle child.
                let row = Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(attach_button)
                    .with_child(Expanded::new(1., editor_box).finish())
                    .with_child(footer_controls)
                    .finish();
                // No top margin here: the balanced `SPLIT_EDITOR_VPAD` on the editor
                // is the only vertical padding, keeping the box tight and symmetric.
                column.add_child(Container::new(row).finish());
            }
        } else {
            // CLI-agent / non-split layout: the footer renders its own toolbar row
            // directly below the editor.
            column.add_child(
                Container::new(editor_box)
                    .with_margin_top(editor_top_margin)
                    .finish(),
            );
            column.add_child(footer_controls);
        }

        stack.add_child(wrap_input_with_terminal_padding_and_focus_handler(
            self.is_active_session(app),
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

        if self.is_voltron_open && self.is_pane_focused(app) {
            add_voltron_overlay(&mut stack, &self.voltron_view, menu_positioning);
        }

        if self.is_pane_focused(app) {
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

        let border_color = if self.handoff_compose_state.as_ref(app).is_active() {
            appearance.theme().ansi_fg_magenta()
        } else if !self.ai_input_model.as_ref(app).is_ai_input_enabled()
            && !self.suggestions_mode_model.as_ref(app).is_slash_commands()
            && !self.slash_command_model.as_ref(app).state().is_detected_command()
            // If NLD, don't color the border if the input is empty, because the current
            // classification is necessarily stale (intentionally inherited from the last
            // classification prior to clearing the input)
            && (!self.editor.as_ref(app).is_empty(app)
                || self.ai_input_model.as_ref(app).is_input_type_locked())
        {
            appearance.theme().ansi_fg_blue()
        } else {
            styles::default_border_color(appearance.theme())
        };

        let input = floating_input_box(
            Hoverable::new(self.hoverable_handle.clone(), |_| drop_target)
                .on_hover(|is_hovered, ctx, _app, _position| {
                    ctx.dispatch_typed_action(InputAction::SetUDIHovered(is_hovered));
                })
                .on_middle_click(|ctx, _app, _position| {
                    ctx.dispatch_typed_action(TerminalAction::MiddleClickOnInput)
                })
                .finish(),
            border_color,
            appearance,
        )
        // Paint the same stable opaque fill the Terminal input uses
        // (`neutral_2` = 10% fg over the background) so the Agent, Terminal, and
        // Cloud Agent input boxes read identically regardless of what surface is
        // composited behind them. Kept a step darker than the border
        // (`default_border_color` = `neutral_3`) so the border reads as a subtle
        // lighter outline around the box.
        .with_background_color(crate::ui_components::blended_colors::neutral_2(
            appearance.theme(),
        ))
        // Give the mic/lightning cluster breathing room from the box's right edge
        // (the left side already gets `PADDING_LEFT` via the terminal-padding
        // wrapper), and keep the box tight vertically.
        .with_padding_right(*PADDING_LEFT)
        .with_padding_bottom(0.)
        .finish();

        let mut column = Flex::column();

        // The inline suggestion menus (model/profile selector, slash commands,
        // prompts, etc.) used to be in-flow siblings *above* the input in this
        // column, so opening one reflowed the layout and pushed the input box
        // down. They now render as an overlay anchored above the input box (see
        // `outer_stack.add_positioned_overlay_child` below), so the input stays
        // pinned in place while the menu expands upward.
        let inline_menu: Option<Box<dyn Element>> = if self
            .suggestions_mode_model
            .as_ref(app)
            .is_inline_model_selector()
        {
            Some(ChildView::new(&self.inline_model_selector_view).finish())
        } else if FeatureFlag::InlineProfileSelector.is_enabled()
            && self
                .suggestions_mode_model
                .as_ref(app)
                .is_profile_selector()
        {
            Some(ChildView::new(&self.inline_profile_selector_view).finish())
        } else if self.suggestions_mode_model.as_ref(app).is_slash_commands()
            && !self.is_cloud_mode_input_v2_composing(app)
        {
            Some(ChildView::new(&self.inline_slash_commands_view).finish())
        } else if self.suggestions_mode_model.as_ref(app).is_prompts_menu() {
            Some(ChildView::new(&self.inline_prompts_menu_view).finish())
        } else if self
            .suggestions_mode_model
            .as_ref(app)
            .is_conversation_menu()
        {
            Some(ChildView::new(&self.inline_conversation_menu_view).finish())
        } else if FeatureFlag::ListSkills.is_enabled()
            && self.suggestions_mode_model.as_ref(app).is_skill_menu()
        {
            Some(ChildView::new(&self.inline_skill_selector_view).finish())
        } else if self.suggestions_mode_model.as_ref(app).is_user_query_menu() {
            Some(ChildView::new(&self.user_query_menu_view).finish())
        } else if self.suggestions_mode_model.as_ref(app).is_rewind_menu() {
            Some(ChildView::new(&self.rewind_menu_view).finish())
        } else if self
            .suggestions_mode_model
            .as_ref(app)
            .is_inline_history_menu()
        {
            Some(ChildView::new(&self.inline_history_menu_view).finish())
        } else if self.suggestions_mode_model.as_ref(app).is_repos_menu() {
            Some(ChildView::new(&self.inline_repos_menu_view).finish())
        } else if self.suggestions_mode_model.as_ref(app).is_plan_menu() {
            Some(ChildView::new(&self.inline_plan_menu_view).finish())
        } else {
            None
        };

        if self
            .agent_shortcut_view_model
            .as_ref(app)
            .is_shortcut_view_open()
        {
            let agent_view_controller = self.agent_view_controller.as_ref(app);
            let (is_cloud_agent, has_submitted_first_prompt) =
                match agent_view_controller.agent_view_state() {
                    AgentViewState::Active {
                        conversation_id,
                        origin,
                        ..
                    } => {
                        let is_cloud_agent = origin.is_cloud_agent();
                        let has_submitted_first_prompt = if is_cloud_agent {
                            BlocklistAIHistoryModel::as_ref(app)
                                .conversation(conversation_id)
                                .is_some_and(|c| c.initial_user_query().is_some())
                        } else {
                            true
                        };
                        (is_cloud_agent, has_submitted_first_prompt)
                    }
                    // When inactive, show all shortcuts (treat as not-cloud and not in the zero-state).
                    AgentViewState::Inactive => (false, true),
                };

            column.add_child(render_agent_shortcuts_view(
                AgentShortcutsViewContext {
                    is_cloud_agent,
                    has_submitted_first_prompt,
                },
                app,
            ));
        }

        // Composer-anchored "N Working" orchestration indicator, sitting above
        // the status/hints row (the `? for help  / for commands …` line).
        // Renders empty unless there are orchestration workers and
        // `AgentProgressUI` is enabled. Left-inset to line up with the input
        // box's content, matching the outside-controls footer row.
        column.add_child(
            Container::new(ChildView::new(&self.working_agents_indicator).finish())
                .with_margin_left(FLOATING_INPUT_MARGIN + *PADDING_LEFT)
                .with_margin_bottom(4.)
                .finish(),
        );

        // The agent status/message bar renders the `? for help  / for commands …`
        // hint row at rest, but collapses (produces a shorter/empty message) while
        // an inline suggestion menu is open. In the vertically-centered zero-state
        // composer that height change reflows the column and shifts the otherwise
        // -pinned input box. Reserve the bar's last-frame height (as a min-height)
        // while a menu is open so the input never moves.
        let status_hints_id = self.agent_status_hints_save_position_id();
        let status_bar = ChildView::new(&self.agent_status_view).finish();
        let status_bar = if inline_menu.is_some() {
            match app.element_position_by_id_at_last_frame(
                self.editor.window_id(app),
                status_hints_id.clone(),
            ) {
                Some(rect) if rect.height() > 0. => ConstrainedBox::new(status_bar)
                    .with_min_height(rect.height())
                    .finish(),
                _ => status_bar,
            }
        } else {
            status_bar
        };
        column.add_child(SavePosition::new(status_bar, &status_hints_id).finish());
        if let Some(panel) = self.queued_prompts_panel.as_ref() {
            if panel.as_ref(app).should_render(app) {
                column.add_child(ChildView::new(panel).finish());
            }
        }

        // The inline suggestion menus (model/profile selector, slash commands,
        // prompts, etc.) render as a floating overlay anchored to the *top* of
        // the input box, expanding upward. Wrapping the input in a plain
        // (non-`with_constrain_absolute_children`) `Stack` is what keeps the
        // input box pinned: positioned overlay children of such a stack don't
        // contribute to its measured size, so the box occupies the same layout
        // slot whether the menu is open or closed. (Adding it to the outer
        // `with_constrain_absolute_children` stack grew that stack to contain the
        // upward menu, which re-centered and shifted the whole composer.) This
        // mirrors the cloud-mode-v2 overlay pattern in
        // `render_cloud_mode_v2_composing_input`.
        if let Some(menu) = inline_menu {
            let input_anchor = self.status_free_input_save_position_id();
            let theme = appearance.theme();
            // Restore the opaque menu frame (surface fill + border + rounded
            // corners + drop shadow) the in-flow path got from the pane behind
            // it; as an overlay the menu floats over conversation content, so it
            // needs to paint its own background. Matches cloud-mode-v2's
            // `render_menu_panel` frame.
            let framed_menu = Container::new(menu)
                .with_background(Fill::Solid(inline_styles::menu_background_color(app)))
                .with_border(
                    Border::all(1.).with_border_fill(Fill::Solid(theme.outline().into_solid())),
                )
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                    INLINE_MENU_OVERLAY_CORNER_RADIUS,
                )))
                .with_drop_shadow(DropShadow::default())
                .finish();

            // Clamp the overlay to the input box's width so it lines up with the
            // box instead of stretching to the window edge. The non-constraining
            // stack (needed to keep the input pinned) doesn't bound the menu's
            // width the way the previous `with_constrain_absolute_children` stack
            // did, so we constrain it explicitly from the box's last-frame width.
            let framed_menu = match app.element_position_by_id_at_last_frame(
                self.editor.window_id(app),
                self.status_free_input_save_position_id(),
            ) {
                Some(rect) if rect.width() > 0. => ConstrainedBox::new(framed_menu)
                    .with_max_width(rect.width())
                    .finish(),
                _ => framed_menu,
            };

            let mut input_stack = Stack::new();
            input_stack.add_child(input);
            input_stack.add_positioned_overlay_child(
                framed_menu,
                OffsetPositioning::from_axes(
                    PositioningAxis::relative_to_stack_child(
                        &input_anchor,
                        PositionedElementOffsetBounds::WindowByPosition,
                        OffsetType::Pixel(0.),
                        AnchorPair::new(XAxisAnchor::Left, XAxisAnchor::Left),
                    ),
                    PositioningAxis::relative_to_stack_child(
                        &input_anchor,
                        PositionedElementOffsetBounds::Unbounded,
                        OffsetType::Pixel(-INLINE_MENU_OVERLAY_GAP),
                        AnchorPair::new(YAxisAnchor::Top, YAxisAnchor::Bottom),
                    ),
                ),
            );
            column.add_child(input_stack.finish());
        } else {
            column.add_child(input);
        }

        if use_split_footer {
            // The remaining toolbar controls (dir chip, aA, model selector,
            // remote-control, etc.) render on their own row below the grey input
            // box, on the surrounding pane background. Left-inset to line up with
            // the box's content (box margin + terminal padding).
            let outside = self
                .agent_input_footer
                .as_ref(app)
                .render_outside_controls(app);
            column.add_child(
                Container::new(outside)
                    .with_margin_left(FLOATING_INPUT_MARGIN + *PADDING_LEFT)
                    .with_margin_right(FLOATING_INPUT_MARGIN)
                    .finish(),
            );
        }

        let mut outer_stack = Stack::new().with_constrain_absolute_children();
        outer_stack.add_child(column.finish());

        // Re-acquire the model lock only for the banner check; kept out of the
        // span above so the footer split methods can lock the model safely.
        let model = self.model.lock();
        let is_input_at_top = self.is_input_at_top(&model, app);
        drop(model);
        maybe_add_buy_credits_banner(
            &mut outer_stack,
            &self.buy_credits_banner,
            self.is_pane_focused(app),
            self.terminal_view_id,
            is_input_at_top,
            app,
        );

        SavePosition::new(outer_stack.finish(), &self.save_position_id()).finish()
    }

    fn render_cloud_mode_v2_composing_input(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);
        let menu_positioning = self.menu_positioning(app);
        let model = self.model.lock();

        let mut stack = Stack::new();

        // The compose UI is a fresh, pre-run state, so it renders at natural height and
        // the view centers it in the main area (see `render_centered_first_run_input`),
        // matching a new Agent / Terminal tab rather than docking to the bottom. No
        // horizontal gutter / max-width is applied here; the left inset (terminal
        // `PADDING_LEFT`) is applied inside the content and the centering wrapper
        // constrains the width.
        let input_content = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_child(self.render_cloud_mode_v2_content(appearance, app))
            .finish();

        let input_content = if self.is_active_session(app) {
            EventHandler::new(input_content)
                .on_left_mouse_down(|ctx, _, _| {
                    ctx.dispatch_typed_action(TerminalAction::ClearSelectionsWhenShellMode);
                    ctx.dispatch_typed_action(InputAction::FocusInputBox);
                    ctx.dispatch_typed_action(InputAction::DismissCloudModeV2SlashCommandsMenu);
                    DispatchEventResult::StopPropagation
                })
                .finish()
        } else {
            input_content
        };

        stack.add_child(input_content);

        if let Some(history_menu) = self.render_cloud_mode_v2_history_menu(app) {
            let prompt_position = self.prompt_save_position_id();
            stack.add_positioned_overlay_child(
                ConstrainedBox::new(history_menu)
                    .with_max_width(CLOUD_MODE_V2_MAX_WIDTH)
                    .finish(),
                OffsetPositioning::from_axes(
                    PositioningAxis::relative_to_stack_child(
                        &prompt_position,
                        PositionedElementOffsetBounds::WindowByPosition,
                        OffsetType::Pixel(0.),
                        AnchorPair::new(XAxisAnchor::Left, XAxisAnchor::Left),
                    ),
                    PositioningAxis::relative_to_stack_child(
                        &prompt_position,
                        PositionedElementOffsetBounds::Unbounded,
                        OffsetType::Pixel(-CLOUD_MODE_V2_TOP_ROW_GAP),
                        AnchorPair::new(YAxisAnchor::Top, YAxisAnchor::Bottom),
                    ),
                ),
            );
        }

        if self.suggestions_mode_model.as_ref(app).is_slash_commands() {
            if let Some(view) = self.cloud_mode_v2_slash_commands_view.as_ref() {
                let cursor_position = position_id_for_cursor(self.editor.id());
                stack.add_positioned_overlay_child(
                    ChildView::new(view).finish(),
                    OffsetPositioning::from_axes(
                        PositioningAxis::relative_to_stack_child(
                            &cursor_position,
                            PositionedElementOffsetBounds::WindowByPosition,
                            OffsetType::Pixel(0.),
                            AnchorPair::new(XAxisAnchor::Left, XAxisAnchor::Left),
                        ),
                        PositioningAxis::relative_to_stack_child(
                            &cursor_position,
                            PositionedElementOffsetBounds::Unbounded,
                            OffsetType::Pixel(4.),
                            AnchorPair::new(YAxisAnchor::Bottom, YAxisAnchor::Top),
                        ),
                    ),
                );
            }
        }

        if let Some(selected_workflow_state) = self.workflows_state.selected_workflow_state.as_ref()
        {
            if selected_workflow_state.should_show_more_info_view {
                let prompt_position = self.prompt_save_position_id();
                let workflows_info_view = Container::new(
                    ChildView::new(&selected_workflow_state.more_info_view).finish(),
                )
                .finish();
                stack.add_positioned_overlay_child(
                    ConstrainedBox::new(workflows_info_view)
                        .with_max_width(CLOUD_MODE_V2_MAX_WIDTH)
                        .with_max_height(self.size_info(app).pane_height_px().as_f32() * 0.35)
                        .finish(),
                    OffsetPositioning::from_axes(
                        PositioningAxis::relative_to_stack_child(
                            &prompt_position,
                            PositionedElementOffsetBounds::WindowByPosition,
                            OffsetType::Pixel(0.),
                            AnchorPair::new(XAxisAnchor::Left, XAxisAnchor::Left),
                        ),
                        PositioningAxis::relative_to_stack_child(
                            &prompt_position,
                            PositionedElementOffsetBounds::Unbounded,
                            OffsetType::Pixel(0.),
                            AnchorPair::new(YAxisAnchor::Top, YAxisAnchor::Bottom),
                        ),
                    ),
                );
            }
        }
        if self.is_voltron_open && self.is_pane_focused(app) {
            add_voltron_overlay(&mut stack, &self.voltron_view, menu_positioning);
        }
        if self.is_pane_focused(app) {
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

        let input = Hoverable::new(self.hoverable_handle.clone(), |_| drop_target)
            .on_hover(|is_hovered, ctx, _app, _position| {
                ctx.dispatch_typed_action(InputAction::SetUDIHovered(is_hovered));
            })
            .on_middle_click(|ctx, _app, _position| {
                ctx.dispatch_typed_action(TerminalAction::MiddleClickOnInput)
            })
            .finish();

        let mut outer_stack = Stack::new().with_constrain_absolute_children();
        outer_stack.add_child(input);
        maybe_add_buy_credits_banner(
            &mut outer_stack,
            &self.buy_credits_banner,
            self.is_pane_focused(app),
            self.terminal_view_id,
            self.is_input_at_top(&model, app),
            app,
        );

        SavePosition::new(outer_stack.finish(), &self.save_position_id()).finish()
    }

    pub(super) fn should_show_auth_secret_ftux(&self, app: &AppContext) -> bool {
        let Some(view_model) = self.ambient_agent_view_model() else {
            return false;
        };
        let vm = view_model.as_ref(app);
        let harness = vm.selected_harness();
        if harness == Harness::Oz {
            return false;
        }
        // Skip FTUX for harnesses that have no auth secret types defined.
        if crate::ai::auth_secret_types::auth_secret_types_for_harness(harness).is_empty() {
            return false;
        }
        if let Some(ftux_view) = self.auth_secret_ftux_view() {
            if ftux_view.as_ref(app).has_creation_state() {
                return true;
            }
        }
        if crate::ai::cloud_agent_settings::CloudAgentSettings::as_ref(app)
            .is_harness_auth_ftux_completed(harness)
        {
            return false;
        }
        vm.selected_harness_auth_secret_name().is_none()
    }

    fn render_cloud_mode_v2_content(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let mut column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_spacing(CLOUD_MODE_V2_TOP_ROW_GAP);

        // Left-inset the top row so it lines up with the editor and the agent
        // input's left inset (terminal `PADDING_LEFT`). Skip it entirely when it
        // has no content so the column's inter-child spacing collapses.
        if let Some(top_row) = self.render_cloud_mode_v2_top_row(app) {
            column.add_child(
                Container::new(top_row)
                    .with_padding_left(*PADDING_LEFT)
                    .finish(),
            );
        }

        if let Some(panel) = self.queued_prompts_panel.as_ref() {
            if panel.as_ref(app).should_render(app) {
                column.add_child(
                    Container::new(ChildView::new(panel).finish())
                        .with_padding_left(*PADDING_LEFT)
                        .finish(),
                );
            }
        }

        if self.should_show_auth_secret_ftux(app) {
            column.add_child(self.render_auth_secret_ftux_content());
        } else {
            column.add_child(self.render_cloud_mode_v2_input_container(appearance, app));
        }

        column.finish()
    }

    fn render_auth_secret_ftux_content(&self) -> Box<dyn Element> {
        match self.auth_secret_ftux_view() {
            Some(view) => ChildView::new(view).finish(),
            None => Empty::new().finish(),
        }
    }

    fn render_cloud_mode_v2_history_menu(&self, app: &AppContext) -> Option<Box<dyn Element>> {
        if !self
            .suggestions_mode_model
            .as_ref(app)
            .is_inline_history_menu()
        {
            return None;
        }
        let view = self.cloud_mode_v2_history_menu_view.as_ref()?;
        Some(ChildView::new(view).finish())
    }

    fn render_cloud_mode_v2_top_row(&self, app: &AppContext) -> Option<Box<dyn Element>> {
        let mut row = Flex::row()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(CLOUD_MODE_V2_TOP_ROW_INNER_GAP);

        let mut has_content = false;

        // Only show the host selector when a default host is configured.
        if let Some(host) = self.host_selector() {
            if host.as_ref(app).has_default_host() {
                row.add_child(ChildView::new(host).finish());
                has_content = true;
            }
        }

        if let Some(auth_secret_selector) = self.auth_secret_selector() {
            let harness = self
                .ambient_agent_view_model()
                .map(|m| m.as_ref(app).selected_harness())
                .unwrap_or(warp_cli::agent::Harness::Oz);
            if harness != warp_cli::agent::Harness::Oz && !self.should_show_auth_secret_ftux(app) {
                row.add_child(ChildView::new(auth_secret_selector).finish());
                has_content = true;
            }
        }

        // The harness selector is now rendered in the footer's left cluster (see
        // `render_cloud_mode_v2_footer`), so it is intentionally not added here.
        // When neither the host nor auth-secret selectors are present the top row
        // has no content; return `None` so the parent column drops the child (and
        // its inter-child spacing), matching the local agent input's vertical
        // padding.
        has_content.then(|| row.finish())
    }

    fn render_cloud_mode_v2_input_container(
        &self,
        appearance: &Appearance,
        app: &AppContext,
    ) -> Box<dyn Element> {
        let terminal_spacing = TerminalSettings::as_ref(app)
            .terminal_input_spacing(appearance.line_height_ratio(), app);

        let mut editor_column = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min);

        let ai_input_model = self.ai_input_model.as_ref(app);
        let show_chips = FeatureFlag::ImageAsContext.is_enabled()
            && matches!(ai_input_model.input_type(), InputType::AI);
        if show_chips {
            if let Some(chips) = self.render_attachment_chips(appearance) {
                editor_column.add_child(
                    Container::new(chips)
                        .with_padding_top(CLOUD_MODE_V2_CHIPS_ROW_TOP_PADDING)
                        .with_padding_left(*PADDING_LEFT)
                        .finish(),
                );
            }
        }

        // Natural (content-driven) height editor with the same top spacing as the
        // agent input, left-inset by terminal `PADDING_LEFT`.
        editor_column.add_child(
            Container::new(self.render_input_box(
                /*show_vim_status=*/ false, /*bottom_padding_override=*/ None,
                appearance, app,
            ))
            .with_margin_top(
                terminal_spacing.prompt_to_editor_padding
                    * spacing::UDI_PROMPT_BOTTOM_PADDING_FACTOR,
            )
            .with_padding_left(*PADDING_LEFT)
            .finish(),
        );

        let editor = editor_column.finish();

        let footer = Container::new(ChildView::new(&self.agent_input_footer).finish())
            .with_padding_left(*PADDING_LEFT)
            .finish();

        let stacked = Flex::column()
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_main_axis_size(MainAxisSize::Min)
            .with_child(editor)
            .with_child(footer)
            .finish();

        // Mirror the local agent input's chrome via the shared floating-box
        // helper: a slightly lighter-gray fill, rounded corners, and an
        // all-around border (see `floating_input_box`).
        let border_color = if self.handoff_compose_state.as_ref(app).is_active() {
            appearance.theme().ansi_fg_magenta()
        } else if !self.ai_input_model.as_ref(app).is_ai_input_enabled()
            && !self.suggestions_mode_model.as_ref(app).is_slash_commands()
            && !self.slash_command_model.as_ref(app).state().is_detected_command()
            // If NLD, don't color the border if the input is empty, because the current
            // classification is necessarily stale (intentionally inherited from the last
            // classification prior to clearing the input)
            && (!self.editor.as_ref(app).is_empty(app)
                || self.ai_input_model.as_ref(app).is_input_type_locked())
        {
            appearance.theme().ansi_fg_blue()
        } else {
            styles::default_border_color(appearance.theme())
        };

        let input = floating_input_box(
            SavePosition::new(stacked, &self.prompt_save_position_id()).finish(),
            border_color,
            appearance,
        )
        // Match the Agent and Terminal input boxes' stable opaque fill so the
        // Cloud Agent box reads identically (see `render_agent_input`).
        .with_background_color(crate::ui_components::blended_colors::neutral_2(
            appearance.theme(),
        ))
        .with_padding_bottom(4.);

        input.finish()
    }

    pub(super) fn render_ambient_agent_status_footer(&self, app: &AppContext) -> Box<dyn Element> {
        let Some(ambient_agent_model) = self.ambient_agent_view_model() else {
            return Empty::new().finish();
        };
        let ambient_agent_model = ambient_agent_model.as_ref(app);
        let mut stack = Stack::new().with_constrain_absolute_children();

        // Don't render status bar when agent has failed or is waiting for session
        let show_status_bar = ambient_agent_model.error_message().is_none()
            && !ambient_agent_model.is_waiting_for_session();

        let model = self.model.lock();
        maybe_add_buy_credits_banner(
            &mut stack,
            &self.buy_credits_banner,
            self.focus_handle.as_ref().is_none_or(|h| h.is_focused(app)),
            self.terminal_view_id,
            self.is_input_at_top(&model, app),
            app,
        );

        let save_position =
            SavePosition::new(stack.finish(), &self.status_free_input_save_position_id()).finish();

        let input = Hoverable::new(self.hoverable_handle.clone(), |_| save_position)
            .on_hover(|is_hovered, ctx, _app, _position| {
                ctx.dispatch_typed_action(InputAction::SetUDIHovered(is_hovered));
            })
            .on_middle_click(|ctx, _app, _position| {
                ctx.dispatch_typed_action(TerminalAction::MiddleClickOnInput);
            })
            .finish();

        let mut column = Flex::column();
        if show_status_bar {
            column.add_child(ChildView::new(&self.agent_status_view).finish());
        }
        column.add_child(input);

        SavePosition::new(column.finish(), &self.save_position_id()).finish()
    }
}

pub mod styles {
    use pathfinder_color::ColorU;
    use warp_core::ui::theme::WarpTheme;

    use crate::ui_components::blended_colors;

    pub fn default_border_color(theme: &WarpTheme) -> ColorU {
        // One step lighter than the box fill (`neutral_2`) so the border reads as
        // a subtle lighter outline around the input box.
        blended_colors::neutral_3(theme)
    }
}
