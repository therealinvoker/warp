use ::local_control::ActionKind;
use uuid::Uuid;
use warpui::elements::{ChildView, Container, Dismiss, Empty};
use warpui::ui_components::components::UiComponent;
use warpui::{
    AppContext, Element, Entity, SingletonEntity as _, TypedActionView, View, ViewContext,
    ViewHandle,
};

use crate::appearance::Appearance;
use crate::local_control::LocalControlBridge;
use crate::ui_components::dialog::{dialog_styles, Dialog};
use crate::view_components::action_button::{ActionButton, DangerPrimaryTheme, NakedTheme};

const DIALOG_WIDTH: f32 = 450.;

#[derive(Debug, Clone)]
pub(crate) struct LocalControlConfirmationPrompt {
    pub confirmation_id: Uuid,
    pub action: ActionKind,
    pub target_summary: String,
}

#[derive(Debug)]
pub(crate) enum LocalControlConfirmationAction {
    Deny,
    Approve,
}

pub(crate) struct LocalControlConfirmationDialog {
    prompt: Option<LocalControlConfirmationPrompt>,
    deny_button: ViewHandle<ActionButton>,
    approve_button: ViewHandle<ActionButton>,
}

impl LocalControlConfirmationDialog {
    pub fn new(ctx: &mut ViewContext<Self>) -> Self {
        let deny_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Deny", NakedTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(LocalControlConfirmationAction::Deny);
            })
        });
        let approve_button = ctx.add_typed_action_view(|_| {
            ActionButton::new("Allow close", DangerPrimaryTheme).on_click(|ctx| {
                ctx.dispatch_typed_action(LocalControlConfirmationAction::Approve);
            })
        });
        Self {
            prompt: None,
            deny_button,
            approve_button,
        }
    }

    pub fn show(
        &mut self,
        prompt: LocalControlConfirmationPrompt,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        if self.prompt.is_some() {
            return false;
        }
        self.prompt = Some(prompt);
        ctx.notify();
        true
    }

    pub fn is_visible(&self) -> bool {
        self.prompt.is_some()
    }

    pub fn dismiss(&mut self, confirmation_id: Uuid, ctx: &mut ViewContext<Self>) {
        if self
            .prompt
            .as_ref()
            .is_some_and(|prompt| prompt.confirmation_id == confirmation_id)
        {
            self.prompt = None;
            ctx.notify();
        }
    }

    fn decide(&mut self, approved: bool, ctx: &mut ViewContext<Self>) {
        let Some(prompt) = self.prompt.take() else {
            return;
        };
        LocalControlBridge::handle(ctx).update(ctx, |bridge, _| {
            bridge.resolve_confirmation(prompt.confirmation_id, approved);
        });
        ctx.notify();
    }
}

impl Entity for LocalControlConfirmationDialog {
    type Event = ();
}

impl View for LocalControlConfirmationDialog {
    fn ui_name() -> &'static str {
        "LocalControlConfirmationDialog"
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let Some(prompt) = &self.prompt else {
            return Empty::new().finish();
        };
        let appearance = Appearance::as_ref(app);
        let description = format!(
            "A verified Warp terminal requested {}.\n\n{}",
            prompt.action.as_str(),
            prompt.target_summary
        );
        let dialog = Dialog::new(
            "Allow Warp control close request?".to_owned(),
            Some(description),
            dialog_styles(appearance),
        )
        .with_bottom_row_child(ChildView::new(&self.deny_button).finish())
        .with_bottom_row_child(
            Container::new(ChildView::new(&self.approve_button).finish())
                .with_margin_left(12.)
                .finish(),
        )
        .with_width(DIALOG_WIDTH)
        .build()
        .finish();
        Dismiss::new(dialog)
            .prevent_interaction_with_other_elements()
            .on_dismiss(|ctx, _| ctx.dispatch_typed_action(LocalControlConfirmationAction::Deny))
            .finish()
    }
}

impl TypedActionView for LocalControlConfirmationDialog {
    type Action = LocalControlConfirmationAction;

    fn handle_action(&mut self, action: &Self::Action, ctx: &mut ViewContext<Self>) {
        match action {
            LocalControlConfirmationAction::Deny => self.decide(false, ctx),
            LocalControlConfirmationAction::Approve => self.decide(true, ctx),
        }
    }
}
