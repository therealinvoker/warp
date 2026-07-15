use std::iter;
use std::sync::Arc;

use anyhow::Result;
use channel_versions::{Changelog, ChannelVersions};
use rand::distributions::Alphanumeric;
use rand::{thread_rng, Rng as _};

use super::channel_versions::fetch_channel_versions;
use super::release_assets_directory_url;
use crate::channel::{Channel, ChannelState};
use crate::server::server_api::ServerApi;

pub async fn get_current_changelog(server_api: Arc<ServerApi>) -> Result<Option<Changelog>> {
    let rand: String = {
        let mut rng = thread_rng();
        iter::repeat(())
            .map(|()| rng.sample(Alphanumeric))
            .map(char::from)
            .take(7)
            .collect()
    };

    let channel = ChannelState::channel();

    if should_fetch_changelog_json(channel) {
        log::info!("Attempting to fetch changelog.json");
        match fetch_current_changelog(server_api.http_client(), rand.as_str()).await {
            changelog_result @ Ok(_) => {
                return changelog_result.map(Option::Some);
            }
            Err(error) => log::error!("Failed to fetch changelog.json: {error}"),
        };
    }

    let versions: ChannelVersions =
        fetch_channel_versions(rand.as_str(), server_api, true, false).await?;

    let res = versions.changelogs.and_then(|changelogs| {
        match channel {
            Channel::Stable => Some(changelogs.stable),
            Channel::Preview => Some(changelogs.preview),
            Channel::Dev | Channel::Local => Some(changelogs.dev),
            // Integration tests and the open-source build don't support autoupdate.
            Channel::Integration | Channel::Oss => None,
        }
        .and_then(|versions| {
            ChannelState::app_version()
                .and_then(|running_version| versions.get(running_version))
                .cloned()
        })
    });
    Ok(res)
}

/// Fetches the changelog for the running release bundle, using the given http
/// client and cache-busting nonce.
async fn fetch_current_changelog(client: &http_client::Client, nonce: &str) -> Result<Changelog> {
    let app_version = ChannelState::app_version().unwrap_or_default();
    let url = format!(
        "{}?r={}",
        changelog_url(ChannelState::channel(), app_version),
        nonce
    );
    let res = client.get(url.as_str()).send().await?;
    let changelog: Changelog = res.json().await?;
    log::info!("Received changelog.json for {app_version}");
    Ok(changelog)
}

/// Returns the URL to the changelog for the given version of this release
/// bundle.
fn changelog_url(channel: Channel, version: &str) -> String {
    // The Bang (OSS) fork has no per-version GCS release bucket, and
    // `release_assets_directory_url` is `unreachable!` for the OSS channel. Its
    // changelog is served by the harness backend at a fixed path off the
    // configured server root (WARP_SERVER_ROOT_URL). The client appends a
    // cache-busting `?r=` query param (see `fetch_current_changelog`).
    if channel == Channel::Oss {
        return format!(
            "{}/changelog.json",
            ChannelState::server_root_url().trim_end_matches('/')
        );
    }
    format!(
        "{}/changelog.json",
        release_assets_directory_url(channel, version)
    )
}

/// Returns whether the app should fetch changelog.json for the current
/// build (true), or use the changelog information embedded in
/// channel_versions.json (false).
pub fn should_fetch_changelog_json(channel: Channel) -> bool {
    // Dev fetches a per-release changelog.json from GCS. The OSS/Bang fork fetches
    // a single changelog.json served by its harness backend (see `changelog_url`),
    // since the channel_versions changelog map explicitly excludes `Channel::Oss`
    // (see `get_current_changelog`).
    matches!(channel, Channel::Dev | Channel::Oss)
}
