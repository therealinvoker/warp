use warpui_core::AppContext;

use crate::elements::TuiElement;
use crate::{TuiBuffer, TuiConstraint, TuiRect, TuiSize, TuiView};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TuiFrame {
    pub buffer: TuiBuffer,
    pub cursor_position: Option<(u16, u16)>,
}

pub struct TuiPresenter {
    frame_count: usize,
}

impl TuiPresenter {
    pub fn new() -> Self {
        Self { frame_count: 0 }
    }

    pub fn frame_count(&self) -> usize {
        self.frame_count
    }

    pub fn render_view(
        &mut self,
        view: &impl TuiView<RenderOutput = Box<dyn TuiElement>>,
        app: &AppContext,
        size: TuiSize,
    ) -> TuiFrame {
        let mut root = view.render_tui(app);
        root.layout(TuiConstraint::tight(size));

        let mut buffer = TuiBuffer::new(size);
        let area = TuiRect::new(0, 0, size.width, size.height);
        root.render(area, &mut buffer);
        let cursor_position = root.cursor_position(area);
        self.frame_count += 1;
        TuiFrame {
            buffer,
            cursor_position,
        }
    }
}

impl Default for TuiPresenter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use warpui_core::{App, Entity, ModelHandle};

    use super::*;
    use crate::elements::{TuiContainer, TuiText};

    struct GreetingModel {
        greeting: String,
    }

    impl Entity for GreetingModel {
        type Event = ();
    }

    struct GreetingView {
        model: ModelHandle<GreetingModel>,
    }

    impl Entity for GreetingView {
        type Event = ();
    }

    impl TuiView for GreetingView {
        type RenderOutput = Box<dyn crate::elements::TuiElement>;
        fn ui_name() -> &'static str {
            "GreetingView"
        }

        fn render_tui(&self, app: &AppContext) -> Box<dyn crate::elements::TuiElement> {
            let greeting = self.model.read(app, |model, _| model.greeting.clone());
            Box::new(TuiContainer::new(TuiText::new(greeting)).with_border())
        }
    }

    #[test]
    fn renders_view_from_shared_model_state() {
        App::test((), |mut app| async move {
            let model = app.add_model(|_| GreetingModel {
                greeting: "hello tui".to_string(),
            });
            let (_, view) = app.add_tui_window(|_| GreetingView { model });

            let mut presenter = TuiPresenter::new();
            let frame = app.read(|ctx| {
                view.read(ctx, |view, ctx| {
                    presenter.render_view(view, ctx, TuiSize::new(12, 3))
                })
            });

            assert_eq!(
                frame.buffer.lines(),
                vec![
                    "┌──────────┐".to_string(),
                    "│hello tui │".to_string(),
                    "└──────────┘".to_string(),
                ]
            );
            assert_eq!(presenter.frame_count(), 1);
        });
    }
}
