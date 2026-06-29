use warpui::keymap::Context;
use warpui::App;

use crate::util::bindings::keybinding_name_to_keystroke;

/// The branch-selector shortcut is registered as an *editable* binding with no
/// default keystroke, so it appears in Settings → Keyboard shortcuts but does
/// nothing until the user assigns a key.
#[test]
fn test_open_branch_selector_binding_is_editable_with_no_default() {
    App::test((), |mut app| async move {
        app.update(crate::code_review::init);

        app.update(|ctx| {
            let binding = ctx
                .editable_bindings()
                .find(|binding| binding.name == "code_review:open_branch_selector")
                .expect(
                    "code_review:open_branch_selector should be registered as an editable binding",
                );

            // No default keystroke: the trigger stays empty until the user
            // assigns one in keyboard settings.
            assert!(
                binding.trigger.is_empty(),
                "code_review:open_branch_selector should have no default keystroke"
            );
            assert_eq!(
                None,
                keybinding_name_to_keystroke("code_review:open_branch_selector", ctx),
                "an unassigned editable binding should resolve to no keystroke"
            );

            let mut editing_context = Context::default();
            editing_context.set.insert("CodeReviewView");
            assert!(
                !binding.in_context(&editing_context),
                "code_review:open_branch_selector should not be active while editing"
            );

            let mut not_editing_context = Context::default();
            not_editing_context.set.insert("CodeReviewView");
            not_editing_context.set.insert("CodeReviewView_NotEditing");
            assert!(
                binding.in_context(&not_editing_context),
                "code_review:open_branch_selector should be active in the non-editing code review context"
            );
        });
    });
}
