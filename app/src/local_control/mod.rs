//! Running app-side server for verified Warp-terminal local control requests.
mod bridge;
pub(crate) mod confirmation_dialog;
mod handlers;
mod permissions;
mod resolver;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration as StdDuration;

use ::local_control::auth::{
    CredentialGrant, CredentialRequest, ScopedCredential, TerminalSessionProofRegistry,
};
use ::local_control::{
    ActionKind, AuthToken, ControlEndpoint, ControlError, ControlResponse, ErrorCode,
    ErrorResponseEnvelope, InstanceId, InstanceRecord, RegisteredInstance, RequestEnvelope,
    ResponseEnvelope,
};
use axum::body::Bytes;
use axum::extract::State;
use axum::http::header::{AUTHORIZATION, HOST, ORIGIN};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
pub use bridge::LocalControlBridge;
use chrono::Duration;
use futures::channel::oneshot;
use permissions::{ensure_action_allowed, ensure_feature_enabled, ensure_protocol_version};
use tokio::sync::broadcast;
use warp_core::channel::ChannelState;
use warpui::{Entity, ModelContext, ModelSpawner, SingletonEntity};

const MAX_ACTIVE_CREDENTIALS: usize = 128;

#[derive(Clone)]
struct ControlServerState {
    bridge_spawner: ModelSpawner<LocalControlBridge>,
    instance_id: InstanceId,
    expected_host: String,
    credentials: Arc<Mutex<HashMap<String, CredentialGrant>>>,
    terminal_proofs: Arc<Mutex<TerminalSessionProofRegistry>>,
    terminal_revocations: broadcast::Sender<String>,
}

async fn wait_for_confirmation(
    mut decision_receiver: oneshot::Receiver<Result<bridge::ApprovedClose, ControlError>>,
    terminal_revocations: &mut broadcast::Receiver<String>,
    terminal_session_id: &str,
) -> Result<bridge::ApprovedClose, ControlError> {
    let timeout = tokio::time::sleep(StdDuration::from_secs(60));
    tokio::pin!(timeout);
    loop {
        tokio::select! {
            decision = &mut decision_receiver => {
                return decision.map_err(|_| {
                    ControlError::new(
                        ErrorCode::UserConfirmationExpired,
                        "local-control confirmation was cancelled",
                    )
                })?;
            }
            revoked = terminal_revocations.recv() => {
                match revoked {
                    Ok(revoked_session) if revoked_session == terminal_session_id => {
                        return Err(ControlError::new(
                            ErrorCode::InvalidTerminalProof,
                            "the requesting Warp terminal closed before confirmation",
                        ));
                    }
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => {
                        return Err(ControlError::new(
                            ErrorCode::UserConfirmationExpired,
                            "local-control confirmation was cancelled",
                        ));
                    }
                }
            }
            _ = &mut timeout => {
                return Err(ControlError::new(
                    ErrorCode::UserConfirmationExpired,
                    "local-control confirmation expired",
                ));
            }
        }
    }
}
pub(crate) struct TerminalControlEnvironment {
    pub env_vars: Vec<(String, String)>,
    pub revocation: TerminalSessionRevocation,
}

pub(crate) struct TerminalSessionRevocation {
    credentials: Arc<Mutex<HashMap<String, CredentialGrant>>>,
    terminal_proofs: Arc<Mutex<TerminalSessionProofRegistry>>,
    terminal_revocations: broadcast::Sender<String>,
    terminal_session_id: String,
}

impl Drop for TerminalSessionRevocation {
    fn drop(&mut self) {
        if let Ok(mut credentials) = self.credentials.lock() {
            credentials.retain(|_, grant| grant.terminal_session_id != self.terminal_session_id);
        }
        if let Ok(mut terminal_proofs) = self.terminal_proofs.lock() {
            terminal_proofs.revoke_session(&self.terminal_session_id);
        }
        let _ = self
            .terminal_revocations
            .send(self.terminal_session_id.clone());
    }
}

pub struct LocalControlServer {
    _runtime: Option<tokio::runtime::Runtime>,
    control_endpoint: Option<ControlEndpoint>,
    registered_instance: Option<RegisteredInstance>,
    state: Option<ControlServerState>,
}

impl Entity for LocalControlServer {
    type Event = ();
}

impl SingletonEntity for LocalControlServer {}

impl LocalControlServer {
    pub fn new(ctx: &mut ModelContext<Self>) -> Self {
        let mut server = Self {
            _runtime: None,
            control_endpoint: None,
            registered_instance: None,
            state: None,
        };
        if let Err(error) = server.refresh_for_settings(ctx) {
            log::warn!("Failed to refresh local-control server state: {error:#}");
        }
        ctx.subscribe_to_model(
            &crate::settings::LocalControlSettings::handle(ctx),
            |server, _, ctx| {
                LocalControlBridge::handle(ctx).update(ctx, |bridge, ctx| {
                    bridge.cancel_all_confirmations(ctx);
                });
                server.invalidate_all_grants();
                if let Err(error) = server.refresh_for_settings(ctx) {
                    log::warn!("Failed to refresh local-control server state: {error:#}");
                }
            },
        );
        server
    }

    fn refresh_for_settings(&mut self, ctx: &mut ModelContext<Self>) -> Result<(), ControlError> {
        if !permissions::warp_control_cli_enabled()
            || !crate::settings::LocalControlSettings::as_ref(ctx).is_enabled()
        {
            self.stop(ctx);
            return Ok(());
        }
        if self._runtime.is_some() {
            return self.refresh_discovery_record();
        }
        self.start(ctx)
    }

    fn stop(&mut self, ctx: &mut ModelContext<Self>) {
        LocalControlBridge::handle(ctx).update(ctx, |bridge, ctx| {
            bridge.cancel_all_confirmations(ctx);
        });
        self.invalidate_all_grants();
        self.state = None;
        self.registered_instance = None;
        self.control_endpoint = None;
        self._runtime = None;
    }

    fn start(&mut self, ctx: &mut ModelContext<Self>) -> Result<(), ControlError> {
        if self._runtime.is_some() {
            return Err(ControlError::new(
                ErrorCode::Internal,
                "local-control server is already running",
            ));
        }
        ensure_feature_enabled()?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_io()
            .build()
            .map_err(|error| {
                ControlError::with_details(
                    ErrorCode::Internal,
                    "failed to create local-control runtime",
                    error.to_string(),
                )
            })?;
        let listener = runtime
            .block_on(tokio::net::TcpListener::bind(SocketAddr::from((
                [127, 0, 0, 1],
                0,
            ))))
            .map_err(|error| {
                ControlError::with_details(
                    ErrorCode::Internal,
                    "failed to bind local-control listener",
                    error.to_string(),
                )
            })?;
        let address = listener.local_addr().map_err(|error| {
            ControlError::with_details(
                ErrorCode::Internal,
                "failed to read local-control listener address",
                error.to_string(),
            )
        })?;
        let control_endpoint = ControlEndpoint::localhost(address.port());
        let record = discovery_record();
        let instance_id = record.instance_id.clone();
        let bridge_spawner = LocalControlBridge::handle(ctx).update(ctx, |bridge, ctx| {
            bridge.set_instance_id(instance_id.clone());
            ctx.spawner()
        });
        let registered_instance = RegisteredInstance::register(record)?;
        let (terminal_revocations, _) = broadcast::channel(128);
        let state = ControlServerState {
            bridge_spawner,
            instance_id,
            expected_host: format!("{}:{}", control_endpoint.host, control_endpoint.port),
            credentials: Arc::default(),
            terminal_proofs: Arc::default(),
            terminal_revocations,
        };
        let router = Router::new()
            .route("/v1/control", post(handle_control_request))
            .route("/v1/control/credentials", post(handle_credential_request))
            .with_state(state.clone());
        runtime.spawn(async move {
            if let Err(error) = axum::serve(listener, router).await {
                log::warn!("local-control listener stopped: {error:#}");
            }
        });
        self._runtime = Some(runtime);
        self.control_endpoint = Some(control_endpoint);
        self.registered_instance = Some(registered_instance);
        self.state = Some(state);
        Ok(())
    }

    pub(crate) fn terminal_environment(
        &self,
        terminal_session_id: impl Into<String>,
    ) -> Result<TerminalControlEnvironment, ControlError> {
        let terminal_session_id = terminal_session_id.into();
        let state = self.state.as_ref().ok_or_else(|| {
            ControlError::new(
                ErrorCode::LocalControlDisabled,
                "local-control server is not running",
            )
        })?;
        let proof = state
            .terminal_proofs
            .lock()
            .map_err(|_| {
                ControlError::new(
                    ErrorCode::Internal,
                    "local-control terminal proof registry is unavailable",
                )
            })?
            .issue(
                state.instance_id.clone(),
                terminal_session_id.clone(),
                Duration::minutes(10),
            );
        let endpoint = self.control_endpoint.as_ref().ok_or_else(|| {
            ControlError::new(
                ErrorCode::LocalControlDisabled,
                "local-control server is not running",
            )
        })?;
        Ok(TerminalControlEnvironment {
            env_vars: vec![
                (
                    ::local_control::client::CONTROL_PORT_ENV.to_owned(),
                    endpoint.port.to_string(),
                ),
                (
                    ::local_control::client::INSTANCE_ID_ENV.to_owned(),
                    state.instance_id.0.clone(),
                ),
                (
                    ::local_control::client::TERMINAL_PROOF_ID_ENV.to_owned(),
                    proof.proof_id,
                ),
                (
                    ::local_control::client::TERMINAL_SESSION_ID_ENV.to_owned(),
                    proof.terminal_session_id,
                ),
                (
                    ::local_control::client::TERMINAL_PROOF_SECRET_ENV.to_owned(),
                    proof.proof_secret,
                ),
            ],
            revocation: TerminalSessionRevocation {
                credentials: Arc::clone(&state.credentials),
                terminal_proofs: Arc::clone(&state.terminal_proofs),
                terminal_revocations: state.terminal_revocations.clone(),
                terminal_session_id,
            },
        })
    }

    pub(crate) fn invalidate_all_grants(&self) {
        if let Some(state) = &self.state {
            if let Ok(mut credentials) = state.credentials.lock() {
                credentials.clear();
            }
            if let Ok(mut terminal_proofs) = state.terminal_proofs.lock() {
                terminal_proofs.invalidate_all();
            }
        }
    }

    fn refresh_discovery_record(&mut self) -> Result<(), ControlError> {
        let Some(registered_instance) = &mut self.registered_instance else {
            return Ok(());
        };
        let mut record = discovery_record();
        record.instance_id = registered_instance.record().instance_id.clone();
        registered_instance.update(record)
    }
}

fn discovery_record() -> InstanceRecord {
    InstanceRecord::for_current_process(
        ChannelState::channel().to_string(),
        ChannelState::app_id().to_string(),
        ChannelState::app_version().map(str::to_owned),
        ActionKind::implemented_metadata(),
    )
}

async fn issue_credential(
    state: &ControlServerState,
    request: CredentialRequest,
) -> Result<ScopedCredential, ControlError> {
    ensure_feature_enabled()?;
    ensure_protocol_version(request.protocol_version)?;
    if !request.action.is_implemented() {
        return Err(ControlError::new(
            ErrorCode::UnsupportedAction,
            format!(
                "{} is not implemented by this local-control bridge",
                request.action.as_str()
            ),
        ));
    }
    {
        let terminal_proofs = state.terminal_proofs.lock().map_err(|_| {
            ControlError::new(
                ErrorCode::Internal,
                "local-control terminal proof registry is unavailable",
            )
        })?;
        request.verify_terminal_proof(&state.instance_id, &terminal_proofs)?;
    }
    state
        .bridge_spawner
        .spawn({
            let action = request.action;
            move |_, ctx| ensure_action_allowed(action, ctx)
        })
        .await
        .map_err(|_| {
            ControlError::new(
                ErrorCode::BridgeUnavailable,
                "local-control app bridge is unavailable",
            )
        })??;
    let auth_token = AuthToken::generate();
    let grant = CredentialGrant::new(
        state.instance_id.clone(),
        request.action,
        request.terminal_proof.terminal_session_id,
        Duration::minutes(5),
    );
    let mut credentials = state.credentials.lock().map_err(|_| {
        ControlError::new(
            ErrorCode::Internal,
            "local-control credential registry is unavailable",
        )
    })?;
    insert_credential(
        &mut credentials,
        auth_token.secret().to_owned(),
        grant.clone(),
    );
    Ok(ScopedCredential {
        bearer_token: auth_token.secret().to_owned(),
        grant,
    })
}

async fn handle_credential_request(
    State(state): State<ControlServerState>,
    headers: HeaderMap,
    Json(request): Json<CredentialRequest>,
) -> Response {
    if let Err(error) = validate_loopback_headers(&headers, &state.expected_host) {
        return (
            StatusCode::FORBIDDEN,
            Json(ErrorResponseEnvelope::new(error)),
        )
            .into_response();
    }
    match issue_credential(&state, request).await {
        Ok(credential) => (StatusCode::OK, Json(credential)).into_response(),
        Err(error) => (
            StatusCode::FORBIDDEN,
            Json(ErrorResponseEnvelope::new(error)),
        )
            .into_response(),
    }
}

async fn handle_control_request(
    State(state): State<ControlServerState>,
    headers: HeaderMap,
    payload: Bytes,
) -> Response {
    if let Err(error) = validate_loopback_headers(&headers, &state.expected_host) {
        return (
            StatusCode::FORBIDDEN,
            Json(ErrorResponseEnvelope::new(error)),
        )
            .into_response();
    }
    if let Err(error) = ensure_feature_enabled() {
        return (
            StatusCode::FORBIDDEN,
            Json(ErrorResponseEnvelope::new(error)),
        )
            .into_response();
    }
    let auth_header = headers
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok());
    let auth_token = match AuthToken::from_authorization_header(auth_header) {
        Ok(token) => token,
        Err(error) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponseEnvelope::new(error)),
            )
                .into_response();
        }
    };
    let grant = match state.credentials.lock() {
        Ok(mut credentials) => lookup_credential(&mut credentials, &auth_token, &state.instance_id),
        Err(_) => Err(ControlError::new(
            ErrorCode::Internal,
            "local-control credential registry is unavailable",
        )),
    };
    let grant = match grant {
        Ok(grant) => grant,
        Err(error) => {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponseEnvelope::new(error)),
            )
                .into_response();
        }
    };
    let proof_session = state
        .terminal_proofs
        .lock()
        .map_err(|_| {
            ControlError::new(
                ErrorCode::Internal,
                "local-control terminal proof registry is unavailable",
            )
        })
        .and_then(|terminal_proofs| {
            terminal_proofs.verify_session(&state.instance_id, &grant.terminal_session_id)
        });
    if let Err(error) = proof_session {
        return (
            StatusCode::UNAUTHORIZED,
            Json(ErrorResponseEnvelope::new(error)),
        )
            .into_response();
    }
    let request = match serde_json::from_slice::<RequestEnvelope>(&payload) {
        Ok(request) => request,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponseEnvelope::new(ControlError::with_details(
                    ErrorCode::InvalidRequest,
                    "failed to decode local-control request",
                    error.to_string(),
                ))),
            )
                .into_response();
        }
    };
    let request_id = request.request_id;
    let response = if request.action.kind.metadata().requires_user_confirmation {
        handle_confirmed_control_request(request, grant, auth_token, &state).await
    } else {
        match state
            .bridge_spawner
            .spawn(move |bridge, ctx| bridge.handle_request(request, grant, ctx))
            .await
        {
            Ok(response) => response,
            Err(_) => ResponseEnvelope::error(
                request_id,
                ControlError::new(
                    ErrorCode::BridgeUnavailable,
                    "local-control app bridge is unavailable",
                ),
            ),
        }
    };
    response_from_envelope(response)
}

async fn handle_confirmed_control_request(
    request: RequestEnvelope,
    grant: CredentialGrant,
    auth_token: AuthToken,
    state: &ControlServerState,
) -> ResponseEnvelope {
    let request_id = request.request_id;
    let terminal_session_id = grant.terminal_session_id.clone();
    let mut terminal_revocations = state.terminal_revocations.subscribe();
    let proof_session = state
        .terminal_proofs
        .lock()
        .map_err(|_| {
            ControlError::new(
                ErrorCode::Internal,
                "local-control terminal proof registry is unavailable",
            )
        })
        .and_then(|terminal_proofs| {
            terminal_proofs.verify_session(&state.instance_id, &terminal_session_id)
        });
    if let Err(error) = proof_session {
        return ResponseEnvelope::error(request_id, error);
    }
    let pending = match state
        .bridge_spawner
        .spawn(move |bridge, ctx| bridge.prepare_close_confirmation(request, grant, ctx))
        .await
    {
        Ok(Ok(pending)) => pending,
        Ok(Err(error)) => return ResponseEnvelope::error(request_id, error),
        Err(_) => {
            return ResponseEnvelope::error(
                request_id,
                ControlError::new(
                    ErrorCode::BridgeUnavailable,
                    "local-control app bridge is unavailable",
                ),
            );
        }
    };
    let confirmation_id = pending.confirmation_id;
    let approval = match wait_for_confirmation(
        pending.decision_receiver,
        &mut terminal_revocations,
        &terminal_session_id,
    )
    .await
    {
        Ok(approval) => approval,
        Err(error) => {
            let _ = state
                .bridge_spawner
                .spawn(move |bridge, ctx| bridge.cancel_confirmation(confirmation_id, ctx))
                .await;
            return ResponseEnvelope::error(request_id, error);
        }
    };
    let live_grant = match state.credentials.lock() {
        Ok(mut credentials) => lookup_credential(&mut credentials, &auth_token, &state.instance_id),
        Err(_) => Err(ControlError::new(
            ErrorCode::Internal,
            "local-control credential registry is unavailable",
        )),
    };
    let live_grant = match live_grant {
        Ok(live_grant) => live_grant,
        Err(error) => return ResponseEnvelope::error(request_id, error),
    };
    if live_grant.credential_id != approval.credential_id() {
        return ResponseEnvelope::error(
            request_id,
            ControlError::new(
                ErrorCode::UnauthorizedLocalClient,
                "the approved local-control credential changed before execution",
            ),
        );
    }
    let proof_session = state
        .terminal_proofs
        .lock()
        .map_err(|_| {
            ControlError::new(
                ErrorCode::Internal,
                "local-control terminal proof registry is unavailable",
            )
        })
        .and_then(|terminal_proofs| {
            terminal_proofs.verify_session(&state.instance_id, approval.terminal_session_id())
        });
    if let Err(error) = proof_session {
        return ResponseEnvelope::error(request_id, error);
    }
    if live_grant.action != approval.action_kind() {
        return ResponseEnvelope::error(
            request_id,
            ControlError::new(
                ErrorCode::InsufficientPermissions,
                "the approved local-control action no longer matches the credential",
            ),
        );
    }
    match state
        .bridge_spawner
        .spawn(move |bridge, ctx| bridge.execute_approved_close(approval, ctx))
        .await
    {
        Ok(response) => response,
        Err(_) => ResponseEnvelope::error(
            request_id,
            ControlError::new(
                ErrorCode::BridgeUnavailable,
                "local-control app bridge is unavailable",
            ),
        ),
    }
}

fn response_from_envelope(response: ResponseEnvelope) -> Response {
    let status = match &response.response {
        ControlResponse::Ok { .. } => StatusCode::OK,
        ControlResponse::Error { .. } => StatusCode::BAD_REQUEST,
    };
    (status, Json(response)).into_response()
}

fn insert_credential(
    credentials: &mut HashMap<String, CredentialGrant>,
    secret: String,
    grant: CredentialGrant,
) {
    credentials.retain(|_, grant| !grant.is_expired());
    if credentials.len() >= MAX_ACTIVE_CREDENTIALS {
        let oldest_secret = credentials
            .iter()
            .min_by_key(|(_, grant)| grant.issued_at)
            .map(|(secret, _)| secret.clone());
        if let Some(oldest_secret) = oldest_secret {
            credentials.remove(&oldest_secret);
        }
    }
    credentials.insert(secret, grant);
}

fn lookup_credential(
    credentials: &mut HashMap<String, CredentialGrant>,
    auth_token: &AuthToken,
    instance_id: &InstanceId,
) -> Result<CredentialGrant, ControlError> {
    if credentials
        .get(auth_token.secret())
        .is_some_and(CredentialGrant::is_expired)
    {
        credentials.remove(auth_token.secret());
    }
    let grant = credentials
        .get(auth_token.secret())
        .cloned()
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::UnauthorizedLocalClient,
                "local-control credential is invalid",
            )
        })?;
    grant.verify_for_action(instance_id, grant.action)?;
    Ok(grant)
}

pub(crate) fn validate_loopback_headers(
    headers: &HeaderMap,
    expected_host: &str,
) -> Result<(), ControlError> {
    if headers.contains_key(ORIGIN) {
        return Err(ControlError::new(
            ErrorCode::UnauthorizedLocalClient,
            "browser-origin local-control requests are not allowed",
        ));
    }
    let host = headers
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| {
            ControlError::new(
                ErrorCode::UnauthorizedLocalClient,
                "Host header is required for local-control requests",
            )
        })?;
    if host != expected_host {
        return Err(ControlError::new(
            ErrorCode::UnauthorizedLocalClient,
            "Host header does not match the selected local-control endpoint",
        ));
    }
    Ok(())
}

#[cfg(test)]
pub(crate) use bridge::validate_request_authority;
#[cfg(test)]
pub(crate) use permissions::{capabilities, ensure_settings_allow_action};
#[cfg(test)]
pub(crate) use resolver::{
    require_active_window_id, resolve_index_from_ids, resolve_title_from_matches,
    validate_action_params, validate_tab_create_target,
};

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
