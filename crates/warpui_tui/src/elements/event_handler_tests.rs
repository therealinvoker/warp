use std::cell::Cell;
use std::rc::Rc;

use warpui_core::event::KeyEventDetails;
use warpui_core::keymap::Keystroke;
use warpui_core::{App, Event};

use super::TuiEventHandler;
use crate::elements::TuiElement;
use crate::{TuiEventContext, TuiRect};

fn key_event(key: &str) -> Event {
    Event::KeyDown {
        keystroke: Keystroke {
            key: key.to_owned(),
            ..Default::default()
        },
        chars: key.to_owned(),
        details: KeyEventDetails::default(),
        is_composing: false,
    }
}

#[test]
fn invokes_callback_on_matching_key_and_reports_handled() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let hits = Rc::new(Cell::new(0u32));
            let counter = hits.clone();
            let mut handler =
                TuiEventHandler::new(()).on_key("enter", move |_event, _ctx, _app| {
                    counter.set(counter.get() + 1);
                });

            let area = TuiRect::new(0, 0, 4, 1);
            let mut event_ctx = TuiEventContext::default();

            let handled =
                handler.dispatch_event(&key_event("enter"), area, &mut event_ctx, app_ctx);
            assert!(handled);
            assert_eq!(hits.get(), 1);

            // A non-matching key is left unhandled for ancestors, runs no callback.
            let handled = handler.dispatch_event(&key_event("esc"), area, &mut event_ctx, app_ctx);
            assert!(!handled);
            assert_eq!(hits.get(), 1);
        });
    });
}

#[test]
fn child_consumes_the_event_before_the_wrapper() {
    App::test((), |app| async move {
        app.read(|app_ctx| {
            let inner_hits = Rc::new(Cell::new(0u32));
            let outer_hits = Rc::new(Cell::new(0u32));
            let inner_counter = inner_hits.clone();
            let outer_counter = outer_hits.clone();

            let inner = TuiEventHandler::new(()).on_key("enter", move |_, _, _| {
                inner_counter.set(inner_counter.get() + 1)
            });
            let mut outer = TuiEventHandler::new(inner).on_key("enter", move |_, _, _| {
                outer_counter.set(outer_counter.get() + 1)
            });

            let mut event_ctx = TuiEventContext::default();
            let handled = outer.dispatch_event(
                &key_event("enter"),
                TuiRect::new(0, 0, 1, 1),
                &mut event_ctx,
                app_ctx,
            );

            assert!(handled);
            assert_eq!(inner_hits.get(), 1);
            assert_eq!(outer_hits.get(), 0);
        });
    });
}
