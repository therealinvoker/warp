//! A small, hand-rolled client for the subset of the GitHub REST API that
//! Warp's GitHub integration needs.
//!
//! The client ([`GithubClient`]) is deliberately thin: it owns a
//! [`reqwest::Client`] configured with GitHub's required headers and a short
//! timeout, delegates auth to a [`TokenProvider`], retries once on `401` after
//! invalidating the cached token, and honors `Retry-After` once on rate-limit
//! responses. See the `melodic-weaving-pretzel` plan (G1) for context.

mod client;
mod error;
pub mod token;
pub mod types;

pub use client::{GithubClient, DEFAULT_BASE_URL, GITHUB_API_VERSION};
pub use error::{Error, Result};
pub use token::{GithubToken, TokenProvider};

#[cfg(test)]
mod tests;
