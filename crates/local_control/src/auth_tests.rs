use chrono::Duration;

use super::*;
use crate::discovery::InstanceId;

#[test]
fn rejects_missing_authorization_header() {
    let token = AuthToken::from_secret("secret");
    let error = token
        .verify_authorization_header(None)
        .expect_err("rejected");
    assert_eq!(error.code, ErrorCode::UnauthorizedLocalClient);
}

#[test]
fn rejects_malformed_authorization_header() {
    let token = AuthToken::from_secret("secret");
    let error = token
        .verify_authorization_header(Some("Basic secret"))
        .expect_err("rejected");
    assert_eq!(error.code, ErrorCode::UnauthorizedLocalClient);
}

#[test]
fn rejects_wrong_bearer_token() {
    let token = AuthToken::from_secret("secret");
    let error = token
        .verify_authorization_header(Some("Bearer wrong"))
        .expect_err("rejected");
    assert_eq!(error.code, ErrorCode::UnauthorizedLocalClient);
}

#[test]
fn accepts_matching_bearer_token() {
    AuthToken::from_secret("secret")
        .verify_authorization_header(Some("Bearer secret"))
        .expect("accepted");
}

#[test]
fn scoped_credential_allows_only_granted_action() {
    let grant = CredentialGrant::new(
        InstanceId("inst_test".to_owned()),
        ActionKind::TabCreate,
        "session-1",
        Duration::minutes(5),
    );
    grant
        .verify_for_action(&grant.instance_id, ActionKind::TabCreate)
        .expect("tab.create grant is accepted");
    let error = grant
        .verify_for_action(&grant.instance_id, ActionKind::WindowCreate)
        .expect_err("other actions are rejected");
    assert_eq!(error.code, ErrorCode::InsufficientPermissions);
}

#[test]
fn scoped_credential_rejects_different_instance() {
    let grant = CredentialGrant::new(
        InstanceId("inst_test".to_owned()),
        ActionKind::TabCreate,
        "session-1",
        Duration::minutes(5),
    );
    let error = grant
        .verify_for_action(&InstanceId("inst_other".to_owned()), ActionKind::TabCreate)
        .expect_err("other instance is rejected");
    assert_eq!(error.code, ErrorCode::UnauthorizedLocalClient);
}

#[test]
fn scoped_credential_rejects_expired_grant() {
    let grant = CredentialGrant::new(
        InstanceId("inst_test".to_owned()),
        ActionKind::TabCreate,
        "session-1",
        Duration::minutes(-1),
    );
    let error = grant
        .verify_for_action(&grant.instance_id, ActionKind::TabCreate)
        .expect_err("expired grant is rejected");
    assert_eq!(error.code, ErrorCode::UnauthorizedLocalClient);
}

#[test]
fn scoped_credential_allows_confirmation_required_action_scope() {
    let grant = CredentialGrant::new(
        InstanceId("inst_test".to_owned()),
        ActionKind::WindowClose,
        "session-1",
        Duration::minutes(5),
    );
    grant
        .verify_for_action(&grant.instance_id, ActionKind::WindowClose)
        .expect("exact-action credential is separate from one-shot confirmation");
}

#[test]
fn credential_request_accepts_registry_verified_terminal_proof() {
    let instance_id = InstanceId("inst_test".to_owned());
    let mut registry = TerminalSessionProofRegistry::default();
    let proof = registry.issue(instance_id.clone(), "session-1", Duration::minutes(5));
    CredentialRequest::new(ActionKind::TabCreate, proof)
        .verify_terminal_proof(&instance_id, &registry)
        .expect("verified terminal proof is accepted");
}

#[test]
fn registry_rejects_terminal_proof_for_wrong_instance() {
    let instance_id = InstanceId("inst_test".to_owned());
    let mut registry = TerminalSessionProofRegistry::default();
    let proof = registry.issue(instance_id, "session-1", Duration::minutes(5));
    let error = CredentialRequest::new(ActionKind::TabCreate, proof)
        .verify_terminal_proof(&InstanceId("other_instance".to_owned()), &registry)
        .expect_err("wrong instance is rejected");
    assert_eq!(error.code, ErrorCode::InvalidTerminalProof);
}

#[test]
fn registry_rejects_revoked_terminal_proof() {
    let instance_id = InstanceId("inst_test".to_owned());
    let mut registry = TerminalSessionProofRegistry::default();
    let proof = registry.issue(instance_id.clone(), "session-1", Duration::minutes(5));
    registry.revoke_session("session-1");
    let error = CredentialRequest::new(ActionKind::TabCreate, proof)
        .verify_terminal_proof(&instance_id, &registry)
        .expect_err("revoked proof is rejected");
    assert_eq!(error.code, ErrorCode::InvalidTerminalProof);
    let error = registry
        .verify_session(&instance_id, "session-1")
        .expect_err("revoked session is rejected");
    assert_eq!(error.code, ErrorCode::InvalidTerminalProof);
}
