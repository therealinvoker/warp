//! GitHub automations settings pages.
//!
//! [`list_page`] is the settings page proper: it lists a workspace's GitHub
//! automations (from `listGithubAutomations`), hosts an inline create/edit form
//! ([`edit_page`]), and — for team admins — a workspace provider-key admin
//! section (set/remove `SetGithubProviderKey` / `RemoveGithubProviderKey`).
//!
//! Everything here is gated on [`crate::features::FeatureFlag::GithubAutomations`]
//! (compile + runtime), the tier's `githubPolicy.automationsEnabled`
//! (visibility), and [`crate::workspaces::team::Team::has_admin_permissions`]
//! (writes).

pub mod edit_page;
pub mod list_page;
