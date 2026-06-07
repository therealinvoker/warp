use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use ::local_control::auth::{CredentialGrant, CredentialRequest, TerminalSessionProofRegistry};
use ::local_control::protocol::{
    Action, ActionKind, PaneSelector, PaneTarget, TabSelector, TabTarget, TargetSelector,
    WindowSelector, WindowTarget,
};
use ::local_control::{AuthToken, ControlResponse, ErrorCode, InstanceId, RequestEnvelope};
use axum::http::header::{HOST, ORIGIN};
use axum::http::{HeaderMap, HeaderValue};
use chrono::Duration;
use settings::Setting as _;
use warp_core::features::FeatureFlag;
use warpui::SingletonEntity as _;

use super::{
    capabilities, ensure_feature_enabled, ensure_protocol_version, ensure_settings_allow_action,
    insert_credential, issue_credential, lookup_credential, require_active_window_id,
    resolve_index_from_ids, resolve_title_from_matches, validate_action_params,
    validate_loopback_headers, validate_request_authority, validate_tab_create_target,
    ControlServerState, LocalControlBridge, LocalControlServer, TerminalSessionRevocation,
    MAX_ACTIVE_CREDENTIALS,
};
use crate::settings::{LocalControlMode, LocalControlModeSetting, LocalControlSettings};

fn settings_with_mode(mode: LocalControlMode) -> LocalControlSettings {
    LocalControlSettings {
        local_control_mode: LocalControlModeSetting::new(Some(mode)),
    }
}

#[test]
fn protocol_version_helper_rejects_unsupported_versions() {
    ensure_protocol_version(::local_control::PROTOCOL_VERSION)
        .expect("current version is accepted");
    let error = ensure_protocol_version(::local_control::PROTOCOL_VERSION + 1)
        .expect_err("future protocol version is rejected");
    assert_eq!(error.code, ErrorCode::ProtocolVersionUnsupported);
}

#[test]
fn feature_flag_disabled_denies_local_control() {
    let _flag = FeatureFlag::WarpControlCli.override_enabled(false);
    let error = ensure_feature_enabled().expect_err("feature flag disabled");
    assert_eq!(error.code, ErrorCode::LocalControlDisabled);
}

#[test]
fn binary_setting_allows_retained_actions_when_enabled() {
    let error = ensure_settings_allow_action(
        &settings_with_mode(LocalControlMode::Disabled),
        ActionKind::AppPing,
    )
    .expect_err("disabled setting rejects local control");
    assert_eq!(error.code, ErrorCode::LocalControlDisabled);
    ensure_settings_allow_action(
        &settings_with_mode(LocalControlMode::Enabled),
        ActionKind::AppPing,
    )
    .expect("enabled setting accepts ordinary actions");
    ensure_settings_allow_action(
        &settings_with_mode(LocalControlMode::Enabled),
        ActionKind::WindowClose,
    )
    .expect("confirmation happens after the binary action policy check");
}

#[test]
fn capabilities_advertises_all_retained_actions() {
    assert_eq!(capabilities(), ActionKind::ALL.to_vec());
}

#[test]
fn loopback_headers_reject_origin_and_host_mismatch() {
    let expected_host = "127.0.0.1:1234";
    let mut headers = HeaderMap::new();
    headers.insert(HOST, HeaderValue::from_static(expected_host));
    validate_loopback_headers(&headers, expected_host).expect("matching host is accepted");
    headers.insert(ORIGIN, HeaderValue::from_static("https://example.com"));
    let error = validate_loopback_headers(&headers, expected_host).expect_err("origin is rejected");
    assert_eq!(error.code, ErrorCode::UnauthorizedLocalClient);
    headers.remove(ORIGIN);
    headers.insert(HOST, HeaderValue::from_static("localhost:1234"));
    let error =
        validate_loopback_headers(&headers, expected_host).expect_err("host mismatch is rejected");
    assert_eq!(error.code, ErrorCode::UnauthorizedLocalClient);
}

#[test]
fn duplicate_server_start_is_rejected() {
    warpui::App::test((), |mut app| async move {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");
        let server = app.add_model(|_| LocalControlServer {
            _runtime: Some(runtime),
            control_endpoint: None,
            registered_instance: None,
            state: None,
        });
        let error = server
            .update(&mut app, |server, ctx| server.start(ctx))
            .expect_err("duplicate start should fail");
        assert_eq!(error.code, ErrorCode::Internal);
        server
            .update(&mut app, |server, _| server._runtime.take())
            .expect("existing runtime should remain active")
            .shutdown_background();
    });
}

#[test]
fn tab_create_accepts_default_and_window_targets() {
    validate_tab_create_target(&TargetSelector::default()).expect("default target is accepted");
    validate_tab_create_target(&TargetSelector {
        window: Some(WindowTarget::Id {
            id: WindowSelector("window".to_owned()),
        }),
        tab: None,
        pane: None,
        session: None,
    })
    .expect("window id target is accepted");
}

#[test]
fn tab_create_rejects_lower_level_targets() {
    let error = validate_tab_create_target(&TargetSelector {
        window: None,
        tab: Some(TabTarget::Id {
            id: TabSelector("tab".to_owned()),
        }),
        pane: None,
        session: None,
    })
    .expect_err("concrete tab target is rejected");
    assert_eq!(error.code, ErrorCode::InvalidSelector);
    let error = validate_tab_create_target(&TargetSelector {
        window: None,
        tab: None,
        pane: Some(PaneTarget::Id {
            id: PaneSelector("pane".to_owned()),
        }),
        session: None,
    })
    .expect_err("concrete pane target is rejected");
    assert_eq!(error.code, ErrorCode::InvalidSelector);
}

#[test]
fn resolver_distinguishes_missing_and_ambiguous_targets() {
    let active = warpui::WindowId::from_usize(1);
    assert_eq!(
        require_active_window_id(Some(active)).expect("active"),
        active
    );
    assert_eq!(
        require_active_window_id(None)
            .expect_err("missing active window")
            .code,
        ErrorCode::MissingTarget
    );
    assert_eq!(
        resolve_index_from_ids(std::iter::empty(), 0, ActionKind::TabCreate)
            .expect_err("zero-match index is missing")
            .code,
        ErrorCode::MissingTarget
    );
    let matches = [
        warpui::WindowId::from_usize(1),
        warpui::WindowId::from_usize(2),
    ];
    assert_eq!(
        resolve_title_from_matches(&matches, ActionKind::TabCreate)
            .expect_err("multi-match title is ambiguous")
            .code,
        ErrorCode::AmbiguousTarget
    );
}

#[test]
fn action_params_are_validated() {
    let error = validate_action_params(&Action {
        kind: ActionKind::TabCreate,
        params: serde_json::json!({ "unexpected": true }),
    })
    .expect_err("tab.create params reject unknown fields");
    assert_eq!(error.code, ErrorCode::InvalidParams);
    validate_action_params(&Action::new(ActionKind::TabCreate))
        .expect("empty tab.create params are accepted");
}

#[test]
fn bridge_checks_grant_before_action_params() {
    let instance_id = InstanceId("inst_test".to_owned());
    let grant = CredentialGrant::new(
        instance_id.clone(),
        ActionKind::AppPing,
        "session-1",
        Duration::minutes(5),
    );
    let error = validate_request_authority(
        &instance_id,
        &Action {
            kind: ActionKind::AppVersion,
            params: serde_json::json!({ "unexpected": true }),
        },
        &grant,
    )
    .expect_err("wrong-action grant is rejected before params");
    assert_eq!(error.code, ErrorCode::InsufficientPermissions);
}

#[test]
fn direct_bridge_dispatch_cannot_bypass_close_confirmation() {
    let _flag = FeatureFlag::WarpControlCli.override_enabled(true);
    warpui::App::test((), |mut app| async move {
        crate::test_util::settings::initialize_settings_for_tests(&mut app);
        app.update(|ctx| {
            LocalControlSettings::handle(ctx).update(ctx, |settings, ctx| {
                settings
                    .local_control_mode
                    .set_value(LocalControlMode::Enabled, ctx)
            })
        })
        .expect("local control should enable");
        let instance_id = InstanceId("inst_test".to_owned());
        let grant = CredentialGrant::new(
            instance_id.clone(),
            ActionKind::WindowClose,
            "session-1",
            Duration::minutes(5),
        );
        let bridge = app.add_singleton_model(LocalControlBridge::new);
        let response = bridge.update(&mut app, |bridge, ctx| {
            bridge.set_instance_id(instance_id);
            bridge.handle_request(
                RequestEnvelope::new(Action::new(ActionKind::WindowClose)),
                grant,
                ctx,
            )
        });
        let ControlResponse::Error { error } = response.response else {
            panic!("direct close dispatch must fail");
        };
        assert_eq!(error.code, ErrorCode::UserConfirmationRequired);
    });
}

#[test]
fn credential_insertion_prunes_expired_and_caps_active_grants() {
    let mut credentials = HashMap::new();
    let instance_id = InstanceId("inst_test".to_owned());
    insert_credential(
        &mut credentials,
        "expired".to_owned(),
        CredentialGrant::new(
            instance_id.clone(),
            ActionKind::TabCreate,
            "session-1",
            Duration::minutes(-1),
        ),
    );
    for index in 0..=MAX_ACTIVE_CREDENTIALS {
        insert_credential(
            &mut credentials,
            format!("active-{index}"),
            CredentialGrant::new(
                instance_id.clone(),
                ActionKind::TabCreate,
                "session-1",
                Duration::minutes(5),
            ),
        );
    }
    assert!(!credentials.contains_key("expired"));
    assert_eq!(credentials.len(), MAX_ACTIVE_CREDENTIALS);
}

#[test]
fn expired_credential_is_rejected_and_pruned() {
    let mut credentials = HashMap::new();
    let token = AuthToken::from_secret("expired");
    credentials.insert(
        token.secret().to_owned(),
        CredentialGrant::new(
            InstanceId("inst_test".to_owned()),
            ActionKind::TabCreate,
            "session-1",
            Duration::minutes(-1),
        ),
    );
    let error = lookup_credential(
        &mut credentials,
        &token,
        &InstanceId("inst_test".to_owned()),
    )
    .expect_err("expired grant is rejected");
    assert_eq!(error.code, ErrorCode::UnauthorizedLocalClient);
    assert!(!credentials.contains_key(token.secret()));
}

#[test]
fn terminal_session_revocation_invalidates_proofs_and_credentials() {
    let instance_id = InstanceId("inst_test".to_owned());
    let credentials = Arc::new(Mutex::new(HashMap::from([(
        "secret".to_owned(),
        CredentialGrant::new(
            instance_id.clone(),
            ActionKind::TabCreate,
            "session-1",
            Duration::minutes(5),
        ),
    )])));
    let terminal_proofs = Arc::new(Mutex::new(TerminalSessionProofRegistry::default()));
    let proof = terminal_proofs
        .lock()
        .expect("terminal proof registry")
        .issue(instance_id.clone(), "session-1", Duration::minutes(5));
    let (terminal_revocations, _) = tokio::sync::broadcast::channel(1);
    drop(TerminalSessionRevocation {
        credentials: Arc::clone(&credentials),
        terminal_proofs: Arc::clone(&terminal_proofs),
        terminal_revocations,
        terminal_session_id: "session-1".to_owned(),
    });
    assert!(credentials.lock().expect("credential registry").is_empty());
    let error = terminal_proofs
        .lock()
        .expect("terminal proof registry")
        .verify(&instance_id, &proof)
        .expect_err("revoked proof is rejected");
    assert_eq!(error.code, ErrorCode::InvalidTerminalProof);
}

#[test]
fn credential_issuance_requires_valid_proof_and_allows_confirmation_actions() {
    let _flag = FeatureFlag::WarpControlCli.override_enabled(true);
    warpui::App::test((), |mut app| async move {
        crate::test_util::settings::initialize_settings_for_tests(&mut app);
        app.update(|ctx| {
            LocalControlSettings::handle(ctx).update(ctx, |settings, ctx| {
                settings
                    .local_control_mode
                    .set_value(LocalControlMode::Enabled, ctx)
            })
        })
        .expect("local control should enable");
        let instance_id = InstanceId("inst_test".to_owned());
        let bridge = app.add_singleton_model(LocalControlBridge::new);
        let state = bridge.update(&mut app, |bridge, ctx| {
            bridge.set_instance_id(instance_id.clone());
            ControlServerState {
                bridge_spawner: ctx.spawner(),
                instance_id: instance_id.clone(),
                expected_host: "127.0.0.1:1234".to_owned(),
                credentials: Default::default(),
                terminal_proofs: Default::default(),
                terminal_revocations: tokio::sync::broadcast::channel(128).0,
            }
        });
        let proof = state
            .terminal_proofs
            .lock()
            .expect("terminal proof registry")
            .issue(instance_id, "session-1", Duration::minutes(5));
        let credential = issue_credential(
            &state,
            CredentialRequest::new(ActionKind::AppPing, proof.clone()),
        )
        .await
        .expect("valid proof receives exact-action credential");
        assert_eq!(credential.grant.action, ActionKind::AppPing);
        assert_eq!(credential.grant.terminal_session_id, "session-1");
        let mut invalid_proof = proof.clone();
        invalid_proof.proof_secret = "wrong".to_owned();
        let error = issue_credential(
            &state,
            CredentialRequest::new(ActionKind::AppPing, invalid_proof),
        )
        .await
        .expect_err("invalid proof is rejected");
        assert_eq!(error.code, ErrorCode::InvalidTerminalProof);
        let credential = issue_credential(
            &state,
            CredentialRequest::new(ActionKind::WindowClose, proof),
        )
        .await
        .expect("confirmation-required action receives an exact-action credential");
        assert_eq!(credential.grant.action, ActionKind::WindowClose);
    });
}
