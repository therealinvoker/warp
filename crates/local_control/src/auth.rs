//! Credential request, issuance, and validation types for local control.
use std::collections::HashMap;

use base64::Engine as _;
use chrono::{DateTime, Duration, Utc};
use rand::RngCore as _;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::discovery::InstanceId;
use crate::protocol::{ActionKind, ControlError, ErrorCode};

/// Bearer token used to authorize a single scoped local-control credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthToken(String);

impl AuthToken {
    pub fn generate() -> Self {
        let mut bytes = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        Self(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes))
    }

    pub fn from_secret(secret: impl Into<String>) -> Self {
        Self(secret.into())
    }

    pub fn secret(&self) -> &str {
        &self.0
    }

    pub fn authorization_value(&self) -> String {
        format!("Bearer {}", self.0)
    }

    pub fn from_authorization_header(value: Option<&str>) -> Result<Self, ControlError> {
        let Some(value) = value else {
            return Err(ControlError::new(
                ErrorCode::UnauthorizedLocalClient,
                "Authorization header is required",
            ));
        };
        let Some(token) = value.strip_prefix("Bearer ") else {
            return Err(ControlError::new(
                ErrorCode::UnauthorizedLocalClient,
                "Authorization header must use the Bearer scheme",
            ));
        };
        Ok(Self::from_secret(token))
    }

    pub fn verify_authorization_header(&self, value: Option<&str>) -> Result<(), ControlError> {
        let token = Self::from_authorization_header(value)?;
        if token != *self {
            return Err(ControlError::new(
                ErrorCode::UnauthorizedLocalClient,
                "Authorization token is invalid",
            ));
        }
        Ok(())
    }
}

/// App-issued proof material for one Warp-managed terminal session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalSessionProof {
    pub proof_id: String,
    pub terminal_session_id: String,
    pub proof_secret: String,
}

#[derive(Debug, Clone)]
struct TerminalSessionProofEntry {
    instance_id: InstanceId,
    terminal_session_id: String,
    proof_secret: String,
    expires_at: DateTime<Utc>,
    revoked: bool,
}

/// In-memory verifier for app-issued terminal-session proof material.
#[derive(Debug, Default, Clone)]
pub struct TerminalSessionProofRegistry {
    entries: HashMap<String, TerminalSessionProofEntry>,
}

impl TerminalSessionProofRegistry {
    pub fn issue(
        &mut self,
        instance_id: InstanceId,
        terminal_session_id: impl Into<String>,
        ttl: Duration,
    ) -> TerminalSessionProof {
        let terminal_session_id = terminal_session_id.into();
        let proof = TerminalSessionProof {
            proof_id: format!("term_proof_{}", Uuid::new_v4().simple()),
            terminal_session_id: terminal_session_id.clone(),
            proof_secret: AuthToken::generate().secret().to_owned(),
        };
        self.entries.insert(
            proof.proof_id.clone(),
            TerminalSessionProofEntry {
                instance_id,
                terminal_session_id,
                proof_secret: proof.proof_secret.clone(),
                expires_at: Utc::now() + ttl,
                revoked: false,
            },
        );
        proof
    }

    pub fn revoke_session(&mut self, terminal_session_id: &str) {
        for entry in self.entries.values_mut() {
            if entry.terminal_session_id == terminal_session_id {
                entry.revoked = true;
            }
        }
    }

    pub fn invalidate_all(&mut self) {
        self.entries.clear();
    }
    pub fn verify_session(
        &self,
        instance_id: &InstanceId,
        terminal_session_id: &str,
    ) -> Result<(), ControlError> {
        if self.entries.values().any(|entry| {
            &entry.instance_id == instance_id
                && entry.terminal_session_id == terminal_session_id
                && !entry.revoked
                && Utc::now() < entry.expires_at
        }) {
            return Ok(());
        }
        Err(ControlError::new(
            ErrorCode::InvalidTerminalProof,
            "Warp terminal proof session is expired or revoked",
        ))
    }

    pub fn verify(
        &self,
        instance_id: &InstanceId,
        proof: &TerminalSessionProof,
    ) -> Result<(), ControlError> {
        let Some(entry) = self.entries.get(&proof.proof_id) else {
            return Err(ControlError::new(
                ErrorCode::InvalidTerminalProof,
                "Warp terminal proof is unknown or has been invalidated",
            ));
        };
        if entry.revoked || Utc::now() >= entry.expires_at {
            return Err(ControlError::new(
                ErrorCode::InvalidTerminalProof,
                "Warp terminal proof is expired or revoked",
            ));
        }
        if &entry.instance_id != instance_id
            || entry.terminal_session_id != proof.terminal_session_id
            || entry.proof_secret != proof.proof_secret
        {
            return Err(ControlError::new(
                ErrorCode::InvalidTerminalProof,
                "Warp terminal proof does not match the issuing session",
            ));
        }
        Ok(())
    }
}

/// Request for a short-lived credential scoped to one action and verified terminal session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialRequest {
    pub protocol_version: u32,
    pub request_id: Uuid,
    pub action: ActionKind,
    pub terminal_proof: TerminalSessionProof,
}

impl CredentialRequest {
    pub fn new(action: ActionKind, terminal_proof: TerminalSessionProof) -> Self {
        Self {
            protocol_version: crate::protocol::PROTOCOL_VERSION,
            request_id: Uuid::new_v4(),
            action,
            terminal_proof,
        }
    }

    pub fn verify_terminal_proof(
        &self,
        instance_id: &InstanceId,
        registry: &TerminalSessionProofRegistry,
    ) -> Result<(), ControlError> {
        registry.verify(instance_id, &self.terminal_proof)
    }
}

/// Client-facing credential response containing a bearer secret and its grant metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopedCredential {
    pub bearer_token: String,
    pub grant: CredentialGrant,
}

impl ScopedCredential {
    pub fn authorization_value(&self) -> String {
        format!("Bearer {}", self.bearer_token)
    }
}

/// Authorization grant issued for one action from one verified terminal session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialGrant {
    pub credential_id: String,
    pub instance_id: InstanceId,
    pub action: ActionKind,
    pub terminal_session_id: String,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl CredentialGrant {
    pub fn new(
        instance_id: InstanceId,
        action: ActionKind,
        terminal_session_id: impl Into<String>,
        ttl: Duration,
    ) -> Self {
        let issued_at = Utc::now();
        Self {
            credential_id: format!("cred_{}", Uuid::new_v4().simple()),
            instance_id,
            action,
            terminal_session_id: terminal_session_id.into(),
            issued_at,
            expires_at: issued_at + ttl,
        }
    }

    pub fn is_expired(&self) -> bool {
        Utc::now() >= self.expires_at
    }

    pub fn verify_for_action(
        &self,
        instance_id: &InstanceId,
        action: ActionKind,
    ) -> Result<(), ControlError> {
        if self.is_expired() {
            return Err(ControlError::new(
                ErrorCode::UnauthorizedLocalClient,
                "local-control credential has expired",
            ));
        }
        if &self.instance_id != instance_id {
            return Err(ControlError::new(
                ErrorCode::UnauthorizedLocalClient,
                "local-control credential belongs to a different Warp instance",
            ));
        }
        if self.action != action {
            return Err(ControlError::new(
                ErrorCode::InsufficientPermissions,
                format!(
                    "credential for {} cannot invoke {}",
                    self.action.as_str(),
                    action.as_str()
                ),
            ));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "auth_tests.rs"]
mod tests;
