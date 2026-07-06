use ui_components::{button, Component as _, Options as _};
use warp_core::features::FeatureFlag;
use warp_core::ui::appearance::Appearance;
use warp_core::ui::theme::color::internal_colors;
use warp_core::ui::theme::Fill;
use warp_core::ui::Icon;
use warpui_core::elements::{
    Border, ClippedScrollStateHandle, ConstrainedBox, Container, CornerRadius, CrossAxisAlignment,
    Flex, FormattedTextElement, MainAxisAlignment, MainAxisSize, ParentElement, Radius,
};
use warpui_core::fonts::Weight;
use warpui_core::keymap::Keystroke;
use warpui_core::prelude::Align;
use warpui_core::text_layout::TextAlignment;
use warpui_core::ui_components::components::{UiComponent as _, UiComponentStyles};
use warpui_core::{
    AppContext, Element, Entity, ModelHandle, SingletonEntity as _, TypedActionView, View,
    ViewContext,
};

use super::OnboardingSlide;
use crate::model::OnboardingStateModel;
use crate::slides::brand::bang_logo_mark;
use crate::slides::{bottom_nav, layout, slide_content};
use crate::visuals::intention_visual;
use crate::AI_FEATURES;

#[derive(Debug, Clone)]
pub enum IntentionSlideAction {
    BackClicked,
    NextClicked,
}

pub struct IntentionSlide {
    onboarding_state: ModelHandle<OnboardingStateModel>,
    back_button: button::Button,
    next_button: button::Button,
    scroll_state: ClippedScrollStateHandle,
}

impl IntentionSlide {
    pub(crate) fn new(onboarding_state: ModelHandle<OnboardingStateModel>) -> Self {
        Self {
            onboarding_state,
            back_button: button::Button::default(),
            next_button: button::Button::default(),
            scroll_state: ClippedScrollStateHandle::new(),
        }
    }

    fn render_content(&self, appearance: &Appearance) -> Box<dyn Element> {
        let bottom_nav = Align::new(self.render_bottom_nav(appearance)).finish();

        slide_content::onboarding_slide_content(
            vec![
                Align::new(self.render_header(appearance)).left().finish(),
                Align::new(self.render_options(appearance)).finish(),
            ],
            bottom_nav,
            self.scroll_state.clone(),
            appearance,
        )
    }

    fn render_header(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();

        let logo = bang_logo_mark(64.);

        let title = appearance
            .ui_builder()
            .paragraph("Welcome to Bang")
            .with_style(UiComponentStyles {
                font_size: Some(36.),
                font_weight: Some(Weight::Medium),
                ..Default::default()
            })
            .build()
            .finish();

        let subtitle = FormattedTextElement::from_str(
            "Here's what you get with Bang.",
            appearance.ui_font_family(),
            16.,
        )
        .with_color(internal_colors::text_sub(
            theme,
            theme.background().into_solid(),
        ))
        .with_weight(Weight::Normal)
        .with_alignment(TextAlignment::Left)
        .with_line_height_ratio(1.0)
        .finish();

        Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            // Offset icon built in padding to left align icon with title.
            .with_child(Container::new(logo).with_margin_left(-7.).finish())
            .with_child(Container::new(title).with_margin_top(11.).finish())
            .with_child(Container::new(subtitle).with_margin_top(16.).finish())
            .finish()
    }

    fn render_options(&self, appearance: &Appearance) -> Box<dyn Element> {
        let agent_card = self.render_agent_card(appearance);

        Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(agent_card)
                .finish(),
        )
        .with_margin_top(38.)
        .finish()
    }

    /// The single, active "Build faster with agents" panel. Rendered as a static,
    /// non-interactive informational card using the accent (selected) chrome since
    /// it is now the only path forward from this slide.
    fn render_agent_card(&self, appearance: &Appearance) -> Box<dyn Element> {
        const RADIUS: f32 = 8.;

        let theme = appearance.theme();
        let bg_solid = theme.background().into_solid();
        let label_color = internal_colors::text_main(theme, bg_solid);
        let description_color = internal_colors::text_sub(theme, bg_solid);
        let checklist_color = label_color;
        let icon_fill = Fill::Solid(label_color);

        let header_row = {
            let label = appearance
                .ui_builder()
                .paragraph("Build faster with agents")
                .with_style(UiComponentStyles {
                    font_size: Some(16.),
                    font_weight: Some(Weight::Semibold),
                    font_color: Some(label_color),
                    ..Default::default()
                })
                .build()
                .finish();

            let mut icon_row = Flex::row()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Center);
            for (i, icon) in [Icon::Oz, Icon::ClaudeLogo, Icon::OpenAILogo]
                .iter()
                .enumerate()
            {
                let el = ConstrainedBox::new(icon.to_warpui_icon(icon_fill).finish())
                    .with_width(16.)
                    .with_height(16.)
                    .finish();
                icon_row = if i == 0 {
                    icon_row.with_child(el)
                } else {
                    icon_row.with_child(Container::new(el).with_margin_left(8.).finish())
                };
            }

            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(label)
                .with_child(icon_row.finish())
                .finish()
        };

        let description = FormattedTextElement::from_str(
            "Get AI features to accelerate terminal and agent-driven workflows:",
            appearance.ui_font_family(),
            14.,
        )
        .with_color(description_color)
        .with_weight(Weight::Normal)
        .with_alignment(TextAlignment::Left)
        .with_line_height_ratio(1.2)
        .finish();

        let checklist = {
            let items = AI_FEATURES;
            // Use the theme's green to match the "Blended ANSI/green_fg" token in
            // the design for the active agent card.
            let check_fill = Fill::Solid(theme.ansi_fg_green());
            let mut col = Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Start);
            for &item in items {
                let icon_el = ConstrainedBox::new(Icon::Check.to_warpui_icon(check_fill).finish())
                    .with_width(16.)
                    .with_height(16.)
                    .finish();
                let text_el = appearance
                    .ui_builder()
                    .paragraph(item.to_string())
                    .with_style(UiComponentStyles {
                        font_size: Some(14.),
                        font_weight: Some(Weight::Normal),
                        font_color: Some(checklist_color),
                        ..Default::default()
                    })
                    .build()
                    .finish();
                let row = Flex::row()
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(icon_el)
                    .with_child(Container::new(text_el).with_margin_left(8.).finish())
                    .finish();
                col = col.with_child(
                    Container::new(row)
                        .with_padding_top(4.)
                        .with_padding_bottom(4.)
                        .finish(),
                );
            }
            col.finish()
        };

        let content = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(header_row)
            .with_child(Container::new(description).with_margin_top(12.).finish())
            .with_child(Container::new(checklist).with_margin_top(12.).finish())
            .finish();

        Container::new(content)
            .with_uniform_padding(24.)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(RADIUS)))
            .with_border(Border::all(1.).with_border_fill(theme.accent()))
            .with_background(internal_colors::accent_overlay_1(theme))
            .finish()
    }

    fn render_bottom_nav(&self, appearance: &Appearance) -> Box<dyn Element> {
        let back_button = self.back_button.render(
            appearance,
            button::Params {
                content: button::Content::Label("Back".into()),
                theme: &button::themes::Naked,
                options: button::Options {
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(IntentionSlideAction::BackClicked);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );

        let enter = Keystroke::parse("enter").unwrap_or_default();
        let next_button = self.next_button.render(
            appearance,
            button::Params {
                content: button::Content::Label("Next".into()),
                theme: &button::themes::Primary,
                options: button::Options {
                    keystroke: Some(enter),
                    on_click: Some(Box::new(|ctx, _app, _pos| {
                        ctx.dispatch_typed_action(IntentionSlideAction::NextClicked);
                    })),
                    ..button::Options::default(appearance)
                },
            },
        );

        let (step_index, step_count) = if FeatureFlag::OpenWarpNewSettingsModes.is_enabled() {
            (0, 5)
        } else {
            (1, 4)
        };
        bottom_nav::onboarding_bottom_nav(
            appearance,
            step_index,
            step_count,
            Some(back_button),
            Some(next_button),
        )
    }

    /// All onboarding image paths used by the intention slide visual.
    pub(crate) const VISUAL_IMAGE_PATHS: &'static [&'static str] = &[
        "async/png/onboarding/welcome_agent.png",
        "async/png/onboarding/welcome_terminal.png",
    ];

    fn render_visual(&self, appearance: &Appearance) -> Box<dyn Element> {
        let theme = appearance.theme();

        if FeatureFlag::OpenWarpNewSettingsModes.is_enabled() {
            layout::onboarding_right_panel_with_bg(
                Self::VISUAL_IMAGE_PATHS[0],
                layout::FOREGROUND_LAYOUT_DEFAULT,
            )
        } else {
            let panel_background = internal_colors::neutral_2(theme);
            let neutral = internal_colors::neutral_4(theme);
            let blue = theme.ansi_fg_blue();
            let green = theme.ansi_fg_green();
            let yellow = theme.ansi_fg_yellow();
            let visual = intention_visual(panel_background, neutral, blue, green, yellow);

            Container::new(visual)
                .with_background_color(internal_colors::neutral_1(theme))
                .finish()
        }
    }
}

impl Entity for IntentionSlide {
    type Event = ();
}

impl View for IntentionSlide {
    fn ui_name() -> &'static str {
        "IntentionSlide"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let appearance = Appearance::as_ref(app);

        // Background is rendered by the parent onboarding view (including background images).
        layout::static_left(
            || self.render_content(appearance),
            || self.render_visual(appearance),
        )
    }
}

impl IntentionSlide {
    fn next(&mut self, ctx: &mut ViewContext<Self>) {
        self.onboarding_state.update(ctx, |model, ctx| {
            model.next(ctx);
        });
    }
}

impl OnboardingSlide for IntentionSlide {
    fn on_enter(&mut self, ctx: &mut ViewContext<Self>) {
        self.next(ctx);
    }
}

impl TypedActionView for IntentionSlide {
    type Action = IntentionSlideAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            IntentionSlideAction::BackClicked => {
                let onboarding_state = self.onboarding_state.clone();
                onboarding_state.update(ctx, |model, ctx| {
                    model.back(ctx);
                });
            }
            IntentionSlideAction::NextClicked => {
                self.next(ctx);
            }
        }
    }
}
