//! Watches the OS screenshot directory and emits an event when a new screenshot
//! is captured, so the focused terminal's agent input can auto-attach it.
//!
//! This complements the existing paste/drag-drop image intake: screenshots that
//! macOS saves directly to a file (the default behavior) never touch the
//! clipboard, so `Cmd+V` can't pick them up. This watcher fills that gap.

#[cfg(not(target_family = "wasm"))]
use std::collections::HashMap;
#[cfg(not(target_family = "wasm"))]
use std::path::Path;
use std::path::PathBuf;
#[cfg(not(target_family = "wasm"))]
use std::time::Duration;

#[cfg(not(target_family = "wasm"))]
use instant::Instant;
#[cfg(not(target_family = "wasm"))]
use notify_debouncer_full::notify::{RecursiveMode, WatchFilter};
#[cfg(not(target_family = "wasm"))]
use warp_core::features::FeatureFlag;
#[cfg(not(target_family = "wasm"))]
use warpui::ModelHandle;
use warpui::{Entity, ModelContext, SingletonEntity};
#[cfg(not(target_family = "wasm"))]
use watcher::{BulkFilesystemWatcher, BulkFilesystemWatcherEvent};

#[cfg(not(target_family = "wasm"))]
use crate::settings::AISettings;

/// Debounce window for the screenshot directory watcher. Long enough to let the
/// OS finish writing the file, short enough that attachment feels immediate.
#[cfg(not(target_family = "wasm"))]
const SCREENSHOT_WATCHER_DEBOUNCE_MILLIS: u64 = 400;

/// Window during which the same screenshot path won't be emitted twice. macOS
/// often reports one screenshot as several filesystem events (create + rename +
/// metadata write), which would otherwise attach the same image multiple times.
#[cfg(not(target_family = "wasm"))]
const SCREENSHOT_DEDUP_WINDOW_MILLIS: u64 = 3000;

/// Image file extensions we consider possible screenshots.
#[cfg(not(target_family = "wasm"))]
const SCREENSHOT_IMAGE_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "heic", "tiff", "tif", "bmp",
];

pub enum ScreenshotWatcherEvent {
    /// A new screenshot file was captured at the given path.
    ScreenshotCaptured(PathBuf),
}

#[cfg(not(target_family = "wasm"))]
pub struct ScreenshotWatcher {
    _watcher: Option<ModelHandle<BulkFilesystemWatcher>>,
    /// Paths recently emitted, used to collapse the burst of filesystem events a
    /// single screenshot produces into one attachment.
    recently_emitted: HashMap<PathBuf, Instant>,
}

#[cfg(target_family = "wasm")]
pub struct ScreenshotWatcher;

#[cfg(not(target_family = "wasm"))]
impl ScreenshotWatcher {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        // Escape hatch for dev/UI-review launches: skip the watcher entirely so
        // the startup Desktop-folder TCC prompt (from `prime_directory_access`)
        // never appears. `script/dev` sets this; production snapshots do not.
        if screenshot_watcher_disabled_by_env() {
            return Self::disabled();
        }
        if !FeatureFlag::ScreenshotAutoAttach.is_enabled() {
            return Self::disabled();
        }

        let Some(directory) = screenshot_directory() else {
            log::warn!("Screenshot auto-attach: could not resolve the screenshot directory");
            return Self::disabled();
        };

        if !directory.exists() {
            log::warn!(
                "Screenshot auto-attach: screenshot directory {} does not exist",
                directory.display()
            );
            return Self::disabled();
        }

        let watcher = ctx.add_model(|ctx| {
            BulkFilesystemWatcher::new(
                Duration::from_millis(SCREENSHOT_WATCHER_DEBOUNCE_MILLIS),
                ctx,
            )
        });
        ctx.subscribe_to_model(&watcher, Self::handle_fs_event);

        let registration = watcher.update(ctx, |watcher, _ctx| {
            watcher.register_path(
                &directory,
                WatchFilter::accept_all(),
                RecursiveMode::NonRecursive,
            )
        });
        ctx.spawn(registration, {
            let directory = directory.clone();
            move |_, result, _ctx| {
                if let Err(err) = result {
                    log::warn!(
                        "Screenshot auto-attach: failed to watch {}: {err}",
                        directory.display()
                    );
                }
            }
        });

        // On macOS the default screenshot directory (Desktop) is TCC-protected.
        // The only time we'd otherwise read it is during the screenshot flow, when
        // macOS's screenshot UI is frontmost — and TCC won't prompt for a
        // non-frontmost app, so it silently denies and the read fails. Touch the
        // directory once now, at startup, while this app is frontmost, so the
        // standard "would like to access files in your Desktop folder" prompt
        // appears. Granting it covers the later file reads too (one folder grant
        // covers listing and reading), and it persists for this signed identity.
        #[cfg(target_os = "macos")]
        prime_directory_access(directory, ctx);

        Self {
            _watcher: Some(watcher),
            recently_emitted: HashMap::new(),
        }
    }

    /// Constructs a watcher that does nothing (feature off, or the screenshot
    /// directory couldn't be resolved).
    fn disabled() -> Self {
        Self {
            _watcher: None,
            recently_emitted: HashMap::new(),
        }
    }

    fn handle_fs_event(
        &mut self,
        _: ModelHandle<BulkFilesystemWatcher>,
        event: &BulkFilesystemWatcherEvent,
        ctx: &mut ModelContext<Self>,
    ) {
        // Respect the setting (and AI enablement) live, without re-registering
        // the watcher when the user toggles it.
        if !AISettings::as_ref(ctx).is_screenshot_auto_attach_enabled(ctx) {
            return;
        }

        let now = Instant::now();
        let dedup_window = Duration::from_millis(SCREENSHOT_DEDUP_WINDOW_MILLIS);
        self.recently_emitted
            .retain(|_, emitted_at| now.duration_since(*emitted_at) < dedup_window);

        // A screenshot appears either as a freshly created file or, on some
        // systems, as a rename into the watched directory. macOS also fires
        // several events per capture, so dedupe by path within a short window.
        for path in event.added.iter().chain(event.moved.keys()) {
            if !is_screenshot_file(path) {
                continue;
            }
            if self.recently_emitted.contains_key(path) {
                continue;
            }
            self.recently_emitted.insert(path.clone(), now);
            ctx.emit(ScreenshotWatcherEvent::ScreenshotCaptured(path.clone()));
        }
    }
}

#[cfg(target_family = "wasm")]
impl ScreenshotWatcher {
    pub fn new(_ctx: &mut ModelContext<Self>) -> Self {
        Self
    }
}

impl Entity for ScreenshotWatcher {
    type Event = ScreenshotWatcherEvent;
}

impl SingletonEntity for ScreenshotWatcher {}

#[cfg(not(target_family = "wasm"))]
fn has_image_extension(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase())
        .is_some_and(|ext| SCREENSHOT_IMAGE_EXTENSIONS.contains(&ext.as_str()))
}

#[cfg(not(target_family = "wasm"))]
fn filename_looks_like_screenshot(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| {
            let lower = name.to_lowercase();
            // macOS uses "Screenshot ..." (modern) or "Screen Shot ..." (older).
            lower.starts_with("screenshot") || lower.starts_with("screen shot")
        })
        .unwrap_or(false)
}

/// macOS: a file is a screenshot if it carries the screencapture metadata
/// extended attribute (locale-independent), or its name matches the screenshot
/// naming convention as a fallback.
#[cfg(all(not(target_family = "wasm"), target_os = "macos"))]
fn is_screenshot_file(path: &Path) -> bool {
    if !path.is_file() || !has_image_extension(path) {
        return false;
    }
    has_screencapture_xattr(path) || filename_looks_like_screenshot(path)
}

/// Non-macOS desktop: fall back to the extension + filename heuristic. Screenshot
/// save locations and metadata vary by environment, so we stay conservative to
/// avoid attaching unrelated images.
#[cfg(all(not(target_family = "wasm"), not(target_os = "macos")))]
fn is_screenshot_file(path: &Path) -> bool {
    path.is_file() && has_image_extension(path) && filename_looks_like_screenshot(path)
}

/// Returns true when the file has macOS's `kMDItemIsScreenCapture` metadata
/// extended attribute, which the OS sets on every screenshot it saves.
#[cfg(all(not(target_family = "wasm"), target_os = "macos"))]
fn has_screencapture_xattr(path: &Path) -> bool {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;

    let Ok(c_path) = CString::new(path.as_os_str().as_bytes()) else {
        return false;
    };
    let Ok(name) = CString::new("com.apple.metadata:kMDItemIsScreenCapture") else {
        return false;
    };

    // Querying with a null buffer returns the attribute length (>= 0) if it
    // exists, or -1 if it does not.
    let result = unsafe {
        libc::getxattr(
            c_path.as_ptr(),
            name.as_ptr(),
            std::ptr::null_mut(),
            0,
            0,
            0,
        )
    };
    result >= 0
}

/// Performs one lightweight read of the screenshot directory at startup so macOS
/// surfaces the Desktop-folder access prompt while this app is frontmost. Without
/// this, the first (and only) read happens during the screenshot flow — when the
/// OS screenshot UI is frontmost — and TCC silently denies instead of prompting.
#[cfg(all(not(target_family = "wasm"), target_os = "macos"))]
fn prime_directory_access(directory: PathBuf, ctx: &mut ModelContext<ScreenshotWatcher>) {
    use futures::StreamExt;

    ctx.spawn(
        {
            let directory = directory.clone();
            async move {
                match async_fs::read_dir(&directory).await {
                    // Pull a single entry to force the real directory read (and thus
                    // the TCC prompt); the entry itself is irrelevant.
                    Ok(mut entries) => entries.next().await.transpose().map(|_| ()),
                    Err(err) => Err(err),
                }
            }
        },
        move |_, result, _ctx| {
            if let Err(err) = result {
                log::warn!(
                    "Screenshot auto-attach: initial read of {} failed \
                     (grant Desktop access when prompted): {err}",
                    directory.display()
                );
            }
        },
    );
}

/// Resolves the directory macOS saves screenshots to.
///
/// We deliberately do NOT read `com.apple.screencapture location` here: reading
/// another app's preferences domain raises macOS Sequoia's "would like to access
/// data from other apps" prompt on every launch (and, for dev builds, on every
/// rebuild, since re-signing invalidates the prior TCC grant). Instead we default
/// to the Desktop — where macOS saves screenshots out of the box — and let users
/// with a custom location opt in via the `WARP_SCREENSHOT_DIR` env var, which
/// never touches another app's data.
#[cfg(all(not(target_family = "wasm"), target_os = "macos"))]
fn screenshot_directory() -> Option<PathBuf> {
    screenshot_dir_override().or_else(dirs::desktop_dir)
}

#[cfg(all(not(target_family = "wasm"), not(target_os = "macos")))]
fn screenshot_directory() -> Option<PathBuf> {
    // Other platforms save screenshots in varied, harder-to-detect locations, so
    // the feature is opt-in there via `WARP_SCREENSHOT_DIR`.
    screenshot_dir_override()
}

/// Whether `WARP_DISABLE_SCREENSHOT_WATCHER` is set to a truthy value. When set,
/// the screenshot watcher is fully disabled (no filesystem watch, and — on macOS
/// — no startup Desktop-access priming), so dev/automation launches aren't
/// interrupted by the "would like to access files in your Desktop folder"
/// prompt.
#[cfg(not(target_family = "wasm"))]
fn screenshot_watcher_disabled_by_env() -> bool {
    std::env::var("WARP_DISABLE_SCREENSHOT_WATCHER")
        .map(|value| {
            let value = value.trim();
            !value.is_empty() && value != "0" && !value.eq_ignore_ascii_case("false")
        })
        .unwrap_or(false)
}

/// Opt-in override for the watched screenshot directory. Lets users point the
/// watcher at a custom location without reading `com.apple.screencapture` (which
/// would trigger the cross-app data-access prompt).
#[cfg(not(target_family = "wasm"))]
fn screenshot_dir_override() -> Option<PathBuf> {
    let raw = std::env::var("WARP_SCREENSHOT_DIR").ok()?;
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    Some(expand_tilde(raw))
}

#[cfg(not(target_family = "wasm"))]
fn expand_tilde(path: &str) -> PathBuf {
    if path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home;
        }
    }
    if let Some(stripped) = path.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    }
    PathBuf::from(path)
}
