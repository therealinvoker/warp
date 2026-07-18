// On Windows, we don't want to display a console window when the application is running in release
// builds. See https://doc.rust-lang.org/reference/runtime.html#the-windows_subsystem-attribute.
#![cfg_attr(feature = "release_bundle", windows_subsystem = "windows")]

use anyhow::Result;
use warp_core::channel::{Channel, ChannelConfig, ChannelState, OzConfig, WarpServerConfig};
use warp_core::AppId;

// Simple wrapper around warp::run() for Warp OSS builds.
fn main() -> Result<()> {
    let mut state = ChannelState::new(
        Channel::Oss,
        ChannelConfig {
            app_id: AppId::new("dev", "warp", "WarpOss"),
            logfile_name: "warp-oss.log".into(),
            server_config: WarpServerConfig::production(),
            oz_config: OzConfig::production(),
            telemetry_config: None,
            crash_reporting_config: None,
            autoupdate_config: None,
            mcp_static_config: None,
        },
    );
    if cfg!(debug_assertions) {
        state = state.with_additional_features(warp_core::features::DEBUG_FLAGS);
    }
    // Personal-fork: enable the changelog ("What's New?") surface in the OSS
    // build. It normally ships via RELEASE_FLAGS, which the OSS channel doesn't
    // pull in. This gates the launch-time fetch, the `/changelog` command, and
    // the changelog menu entries; the content itself is served by the harness
    // backend at {WARP_SERVER_ROOT_URL}/changelog.json (see
    // app/src/autoupdate/changelog.rs).
    state = state.with_additional_features(&[warp_core::features::FeatureFlag::Changelog]);
    // Personal-fork: enable the redesigned orchestration agent-progress UI (the
    // composer-anchored "N Working" indicator, in-stream delegation cards, and
    // the read-only live progress modal, replacing the top pill bar) in the OSS
    // build. It otherwise only ships in dogfood via DOGFOOD_FLAGS.
    state = state.with_additional_features(&[warp_core::features::FeatureFlag::AgentProgressUI]);
    // Personal-fork: surface the embedded browser preview tab (and its toolbelt
    // launcher) in the OSS build, since it otherwise only ships in dogfood.
    #[cfg(target_os = "macos")]
    {
        state = state.with_additional_features(&[warp_core::features::FeatureFlag::BrowserPreview]);
    }
    ChannelState::set(state);

    warp::run()
}

// If we're not using an external plist, embed the following as the Info.plist.
#[cfg(all(not(feature = "extern_plist"), target_os = "macos"))]
// NOTE: `CFBundleIdentifier` / `CFBundleName` / `CFBundleDisplayName` are
// injected at build time from `BANG_EMBED_BUNDLE_ID` / `BANG_EMBED_BUNDLE_NAME`
// (set by app/build.rs from WARP_APP_BUNDLE_ID / WARP_APP_DISPLAY_NAME, with the
// stable identity as the default). This must stay in sync with the .app bundle's
// Info.plist because macOS uses THIS embedded plist for microphone/privacy (TCC)
// attribution and the Control Center mic indicator. See app/build.rs.
embed_plist::embed_info_plist_bytes!(concat!(
    r#"
    <?xml version="1.0" encoding="UTF-8"?>
    <!DOCTYPE plist PUBLIC "-//Apple Computer//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
    <plist version="1.0">
    <dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>English</string>
    <key>CFBundleDisplayName</key>
    <string>"#,
    env!("BANG_EMBED_BUNDLE_NAME"),
    r#"</string>
    <key>CFBundleExecutable</key>
    <string>bang</string>
    <key>CFBundleIdentifier</key>
    <string>"#,
    env!("BANG_EMBED_BUNDLE_ID"),
    r#"</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>"#,
    env!("BANG_EMBED_BUNDLE_NAME"),
    r#"</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.0</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.developer-tools</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>UIDesignRequiresCompatibility</key>
    <true/>
    <key>CFBundleURLTypes</key>
    <array><dict><key>CFBundleURLName</key><string>Custom App</string><key>CFBundleURLSchemes</key><array><string>warposs</string></array></dict></array>
    <key>NSHumanReadableCopyright</key>
    <string>© 2026, Denver Technologies, Inc</string>
    </dict>
    </plist>
"#
).as_bytes());
