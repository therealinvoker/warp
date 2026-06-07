//! Blocking client helpers used by the `warpctrl` CLI.
use crate::auth::{CredentialRequest, ScopedCredential, TerminalSessionProof};
use crate::discovery::{ControlEndpoint, InstanceRecord};
use crate::protocol::{
    Action, ActionKind, ControlError, ControlResponse, ErrorCode, ErrorResponseEnvelope,
    RequestEnvelope, ResponseEnvelope,
};

pub const TERMINAL_PROOF_ID_ENV: &str = "WARPCTRL_TERMINAL_PROOF_ID";
pub const TERMINAL_SESSION_ID_ENV: &str = "WARPCTRL_TERMINAL_SESSION_ID";
pub const TERMINAL_PROOF_SECRET_ENV: &str = "WARPCTRL_TERMINAL_PROOF_SECRET";
pub const CONTROL_PORT_ENV: &str = "WARPCTRL_CONTROL_PORT";
pub const INSTANCE_ID_ENV: &str = "WARPCTRL_INSTANCE_ID";

/// Requests an action-scoped credential and sends one authenticated control request.
pub fn send_request(
    instance: &InstanceRecord,
    request: &RequestEnvelope,
) -> Result<ResponseEnvelope, ControlError> {
    ensure_issuing_instance(instance)?;
    let credential = request_credential(request.action.kind)?;
    let endpoint = control_endpoint_from_environment()?;
    let client = reqwest::blocking::Client::new();
    let response = client
        .post(endpoint.url())
        .header("Authorization", credential.authorization_value())
        .json(request)
        .send()
        .map_err(|err| {
            ControlError::with_details(
                ErrorCode::TransportUnavailable,
                "failed to send local-control request",
                err.to_string(),
            )
        })?;
    let status = response.status();
    let text = response.text().map_err(|err| {
        ControlError::with_details(
            ErrorCode::TransportUnavailable,
            "failed to read local-control response",
            err.to_string(),
        )
    })?;
    if let Ok(envelope) = serde_json::from_str::<ResponseEnvelope>(&text) {
        if let ControlResponse::Error { error } = &envelope.response {
            return Err(error.clone());
        }
        return Ok(envelope);
    }
    if let Ok(envelope) = serde_json::from_str::<ErrorResponseEnvelope>(&text) {
        return Err(envelope.error);
    }
    Err(ControlError::with_details(
        ErrorCode::TransportUnavailable,
        format!("local-control request failed with HTTP {status}"),
        text,
    ))
}

pub fn terminal_proof_from_environment() -> Result<TerminalSessionProof, ControlError> {
    let proof_id = std::env::var(TERMINAL_PROOF_ID_ENV).ok();
    let terminal_session_id = std::env::var(TERMINAL_SESSION_ID_ENV).ok();
    let proof_secret = std::env::var(TERMINAL_PROOF_SECRET_ENV).ok();
    match (proof_id, terminal_session_id, proof_secret) {
        (Some(proof_id), Some(terminal_session_id), Some(proof_secret)) => {
            Ok(TerminalSessionProof {
                proof_id,
                terminal_session_id,
                proof_secret,
            })
        }
        _ => Err(ControlError::new(
            ErrorCode::ExecutionContextNotAllowed,
            "warpctrl requires a verified live Warp-terminal session",
        )),
    }
}

fn control_endpoint_from_environment() -> Result<ControlEndpoint, ControlError> {
    let port = std::env::var(CONTROL_PORT_ENV)
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::ExecutionContextNotAllowed,
                "warpctrl requires an app-injected local-control endpoint",
            )
        })?;
    Ok(ControlEndpoint::localhost(port))
}

fn ensure_issuing_instance(instance: &InstanceRecord) -> Result<(), ControlError> {
    let instance_id = std::env::var(INSTANCE_ID_ENV).map_err(|_| {
        ControlError::new(
            ErrorCode::ExecutionContextNotAllowed,
            "warpctrl requires an app-injected instance identity",
        )
    })?;
    if instance.instance_id.0 != instance_id {
        return Err(ControlError::new(
            ErrorCode::InvalidTerminalProof,
            "warpctrl can control only the proof-issuing Warp instance",
        ));
    }
    Ok(())
}

fn request_credential_over_http(
    endpoint: &ControlEndpoint,
    request: &CredentialRequest,
) -> Result<String, ControlError> {
    let response = reqwest::blocking::Client::new()
        .post(endpoint.credential_url())
        .json(request)
        .send()
        .map_err(|err| {
            ControlError::with_details(
                ErrorCode::TransportUnavailable,
                "failed to request inside-Warp local-control credential",
                err.to_string(),
            )
        })?;
    response.text().map_err(|err| {
        ControlError::with_details(
            ErrorCode::TransportUnavailable,
            "failed to read local-control credential response",
            err.to_string(),
        )
    })
}

/// Requests and decodes a short-lived credential for one action.
pub fn request_credential(action: ActionKind) -> Result<ScopedCredential, ControlError> {
    let request = CredentialRequest::new(action, terminal_proof_from_environment()?);
    let text = request_credential_over_http(&control_endpoint_from_environment()?, &request)?;
    if let Ok(credential) = serde_json::from_str::<ScopedCredential>(&text) {
        return Ok(credential);
    }
    if let Ok(envelope) = serde_json::from_str::<ErrorResponseEnvelope>(&text) {
        return Err(envelope.error);
    }
    Err(ControlError::new(
        ErrorCode::TransportUnavailable,
        "local-control credential endpoint returned an invalid response",
    ))
}

/// Authenticates an app-ping request and verifies the selected instance is live.
pub fn probe_instance(instance: &InstanceRecord) -> Result<(), ControlError> {
    let response = send_request(
        instance,
        &RequestEnvelope::new(Action::new(ActionKind::AppPing)),
    )?;
    validate_probe_response(instance, response)
}

fn validate_probe_response(
    instance: &InstanceRecord,
    response: ResponseEnvelope,
) -> Result<(), ControlError> {
    let ControlResponse::Ok { data } = response.response else {
        return Err(ControlError::new(
            ErrorCode::TransportUnavailable,
            "local-control health probe returned an error response",
        ));
    };
    if data.get("instance_id").and_then(serde_json::Value::as_str)
        != Some(instance.instance_id.0.as_str())
    {
        return Err(ControlError::new(
            ErrorCode::TransportUnavailable,
            "local-control health probe returned a different instance identity",
        ));
    }
    Ok(())
}

#[cfg(test)]
#[path = "client_tests.rs"]
mod tests;
