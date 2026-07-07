//! Error type for the GitHub client.

use reqwest::StatusCode;

/// Result alias for the crate.
pub type Result<T> = std::result::Result<T, Error>;

/// Errors returned by [`crate::GithubClient`].
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The [`crate::TokenProvider`] failed to supply a token.
    #[error("failed to obtain GitHub token: {0}")]
    Token(#[source] anyhow::Error),

    /// The token could not be encoded as an HTTP header (e.g. it contained
    /// invalid characters).
    #[error("GitHub token is not a valid HTTP header value")]
    InvalidToken,

    /// A transport-level or (de)serialization error from reqwest.
    #[error("GitHub request failed: {0}")]
    Http(#[source] reqwest::Error),

    /// The API returned a non-success status.
    #[error("GitHub API returned {status}: {message}")]
    Status { status: StatusCode, message: String },
}

impl Error {
    /// The HTTP status code, if this error came from a non-success response.
    pub fn status(&self) -> Option<StatusCode> {
        match self {
            Error::Status { status, .. } => Some(*status),
            Error::Http(e) => e.status(),
            _ => None,
        }
    }

    /// Whether this error represents a "not found" (404) from the API.
    pub fn is_not_found(&self) -> bool {
        self.status() == Some(StatusCode::NOT_FOUND)
    }
}
