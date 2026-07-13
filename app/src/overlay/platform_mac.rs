//! macOS implementations of the overlay platform traits.
//!
//! The native puck (`native/overlay_puck.m`) is driven through the FFI below.
//! Puck clicks come back into the app via `bang_overlay_puck_clicked`, which
//! bridges from the AppKit main thread into an `AppContext` using a `WeakApp`
//! captured at startup (see `install_overlay_app_bridge`).

use std::cell::RefCell;
use std::ffi::CString;

use warpui::{SingletonEntity, WeakApp};

use super::platform::OverlayWindow;

// Implemented in app/src/overlay/native/overlay_puck.m (compiled by build.rs).
// All calls must happen on the main thread.
extern "C" {
    fn bang_overlay_puck_show();
    fn bang_overlay_puck_hide();
    fn bang_overlay_puck_set_listening(listening: bool);
    fn bang_overlay_puck_set_paused(paused: bool);
    fn bang_overlay_puck_set_level(level: f64);
    fn bang_overlay_puck_set_thinking(thinking: bool);
    fn bang_overlay_box_show();
    fn bang_overlay_box_hide();
    fn bang_overlay_box_set_text(utf8: *const std::os::raw::c_char);
    fn bang_overlay_box_set_editable(editable: bool);
    fn bang_overlay_box_set_bg(r: f64, g: f64, b: f64, a: f64);
    fn bang_overlay_box_set_auto_submit(on: bool);
}

thread_local! {
    /// Main-thread handle back into the app, used by the puck click callback.
    /// `WeakApp` is `Rc`-based, hence thread-local rather than a `static`.
    static OVERLAY_APP: RefCell<Option<WeakApp>> = const { RefCell::new(None) };
}

/// Capture a `WeakApp` so native puck clicks can re-enter app context. Call once
/// at startup on the main thread.
pub fn install_overlay_app_bridge(weak_app: WeakApp) {
    OVERLAY_APP.with(|slot| *slot.borrow_mut() = Some(weak_app));
}

/// Called from `overlay_puck.m` when a puck is clicked (not dragged). `kind` is
/// 0 for the mic puck (pause/resume) and 1 for the submit puck.
///
/// # Safety
/// Invoked by AppKit on the main thread. Must not be called re-entrantly while
/// an `AppContext` borrow is held (AppKit dispatches these between frames, as
/// with the global-hotkey/menu callbacks).
#[no_mangle]
pub extern "C-unwind" fn bang_overlay_puck_clicked(kind: i32) {
    let weak = OVERLAY_APP.with(|slot| slot.borrow().clone());
    let Some(weak) = weak else {
        return;
    };
    let Some(mut app) = weak.upgrade() else {
        return;
    };
    app.update(|ctx| {
        let input = super::OverlayController::handle(ctx)
            .as_ref(ctx)
            .active_input();
        let Some(input) = input.and_then(|weak| weak.upgrade(ctx)) else {
            return;
        };
        input.update(ctx, |input, ctx| match kind {
            k if k == 1 => input.overlay_submit(ctx),
            _ => input.overlay_toggle_pause(ctx),
        });
    });
}

/// Called from `overlay_puck.m` when the user edits the result box (dictation
/// mode). Routes the new text back into the composer.
///
/// # Safety
/// `utf8` is a valid NUL-terminated C string owned by AppKit for the duration of
/// the call; invoked on the main thread.
#[no_mangle]
pub extern "C-unwind" fn bang_overlay_box_edited(utf8: *const std::os::raw::c_char) {
    if utf8.is_null() {
        return;
    }
    // SAFETY: caller guarantees a valid NUL-terminated string for this call.
    let text = match unsafe { std::ffi::CStr::from_ptr(utf8) }.to_str() {
        Ok(text) => text.to_owned(),
        Err(_) => return,
    };
    let weak = OVERLAY_APP.with(|slot| slot.borrow().clone());
    let Some(weak) = weak else {
        return;
    };
    let Some(mut app) = weak.upgrade() else {
        return;
    };
    app.update(|ctx| {
        let input = super::OverlayController::handle(ctx)
            .as_ref(ctx)
            .active_input();
        let Some(input) = input.and_then(|weak| weak.upgrade(ctx)) else {
            return;
        };
        input.update(ctx, |input, ctx| input.overlay_box_edited(&text, ctx));
    });
}

/// Always-on-top circular puck windows (see `overlay_puck.m`).
#[derive(Default)]
pub struct MacOverlayWindow {
    visible: bool,
}

impl OverlayWindow for MacOverlayWindow {
    fn show(&mut self) {
        self.visible = true;
        // SAFETY: FFI to the puck helper; safe on the main thread, which is where
        // the overlay controller runs.
        unsafe { bang_overlay_puck_show() }
    }

    fn hide(&mut self) {
        self.visible = false;
        // SAFETY: see `show`.
        unsafe { bang_overlay_puck_hide() }
    }

    fn is_visible(&self) -> bool {
        self.visible
    }

    fn set_listening(&mut self, listening: bool) {
        // SAFETY: see `show`.
        unsafe { bang_overlay_puck_set_listening(listening) }
    }

    fn set_paused(&mut self, paused: bool) {
        // SAFETY: see `show`.
        unsafe { bang_overlay_puck_set_paused(paused) }
    }

    fn set_level(&mut self, level: f32) {
        // SAFETY: see `show`.
        unsafe { bang_overlay_puck_set_level(level as f64) }
    }

    fn set_thinking(&mut self, thinking: bool) {
        // SAFETY: see `show`.
        unsafe { bang_overlay_puck_set_thinking(thinking) }
    }

    fn show_result_box(&mut self) {
        // SAFETY: see `show`.
        unsafe { bang_overlay_box_show() }
    }

    fn hide_result_box(&mut self) {
        // SAFETY: see `show`.
        unsafe { bang_overlay_box_hide() }
    }

    fn set_result_text(&mut self, text: &str) {
        // `NSString stringWithUTF8String` needs a NUL-terminated C string; strip
        // any interior NULs so `CString::new` can't fail.
        let sanitized = text.replace('\0', "");
        if let Ok(c) = CString::new(sanitized) {
            // SAFETY: `c` outlives the call; ObjC copies the string.
            unsafe { bang_overlay_box_set_text(c.as_ptr()) }
        }
    }

    fn set_box_editable(&mut self, editable: bool) {
        // SAFETY: see `show`.
        unsafe { bang_overlay_box_set_editable(editable) }
    }

    fn set_box_background(&mut self, r: f32, g: f32, b: f32, a: f32) {
        // SAFETY: see `show`.
        unsafe { bang_overlay_box_set_bg(r as f64, g as f64, b as f64, a as f64) }
    }

    fn set_box_auto_submit(&mut self, on: bool) {
        // SAFETY: see `show`.
        unsafe { bang_overlay_box_set_auto_submit(on) }
    }
}

/// Called from `overlay_puck.m` when the box's auto-submit toggle is clicked.
/// Flips the persisted setting and pushes the new state back to the toggle.
///
/// # Safety
/// Invoked by AppKit on the main thread (see `bang_overlay_puck_clicked`).
#[no_mangle]
pub extern "C-unwind" fn bang_overlay_auto_submit_clicked() {
    use settings::ToggleableSetting;
    let weak = OVERLAY_APP.with(|slot| slot.borrow().clone());
    let Some(weak) = weak else {
        return;
    };
    let Some(mut app) = weak.upgrade() else {
        return;
    };
    app.update(|ctx| {
        let on = match crate::settings::AISettings::handle(ctx).update(ctx, |settings, ctx| {
            settings
                .voice_overlay_auto_submit
                .toggle_and_save_value(ctx)
        }) {
            Ok(value) => value,
            Err(_) => *crate::settings::AISettings::as_ref(ctx).voice_overlay_auto_submit,
        };
        super::OverlayController::handle(ctx)
            .update(ctx, |controller, _| controller.set_box_auto_submit(on));
    });
}
