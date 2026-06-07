//! Private filesystem registry for discovering running local Warp instances.
//!
//! This module answers “which compatible instances are available?” It does not
//! listen for control requests and does not grant control authority.
//! `app/src/local_control/mod.rs` owns the
//! running app-side listeners and uses these types to publish their routing
//! metadata.
//!
//! An enabled instance publishes an owner-only JSON record containing
//! instance/build metadata and implemented actions. The record never contains
//! an actionable endpoint, broker reference, bearer token, or reusable
//! credential. A verified Warp terminal receives its issuing instance's
//! endpoint and proof material directly through its environment.
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::protocol::{ActionMetadata, ControlError, ErrorCode, PROTOCOL_VERSION};

const DISCOVERY_DIR_ENV: &str = "WARP_LOCAL_CONTROL_DISCOVERY_DIR";

/// Stable identifier for one running Warp instance.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct InstanceId(pub String);

impl InstanceId {
    pub fn new() -> Self {
        Self(format!("inst_{}", uuid::Uuid::new_v4().simple()))
    }
}

impl Default for InstanceId {
    fn default() -> Self {
        Self::new()
    }
}

/// Exact loopback HTTP route injected into a verified live Warp terminal.
///
/// The endpoint is never published in discovery metadata and does not itself
/// authorize actions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlEndpoint {
    pub host: String,
    pub port: u16,
}

impl ControlEndpoint {
    pub fn localhost(port: u16) -> Self {
        Self {
            host: "127.0.0.1".to_owned(),
            port,
        }
    }

    pub fn url(&self) -> String {
        format!("http://{}:{}/v1/control", self.host, self.port)
    }

    pub fn credential_url(&self) -> String {
        format!("http://{}:{}/v1/control/credentials", self.host, self.port)
    }
}

/// Filesystem-published routing metadata for a running Warp app process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstanceRecord {
    pub protocol_version: u32,
    pub instance_id: InstanceId,
    pub pid: u32,
    pub channel: String,
    pub app_id: String,
    pub app_version: Option<String>,
    pub started_at: DateTime<Utc>,
    pub executable_path: Option<PathBuf>,
    pub actions: Vec<ActionMetadata>,
}

impl InstanceRecord {
    pub fn for_current_process(
        channel: impl Into<String>,
        app_id: impl Into<String>,
        app_version: Option<String>,
        actions: Vec<ActionMetadata>,
    ) -> Self {
        let instance_id = InstanceId::new();
        Self {
            protocol_version: PROTOCOL_VERSION,
            instance_id,
            pid: std::process::id(),
            channel: channel.into(),
            app_id: app_id.into(),
            app_version,
            started_at: Utc::now(),
            executable_path: std::env::current_exe().ok(),
            actions,
        }
    }
}

/// RAII registration for one app-owned discovery record.
///
/// The registration publishes routing metadata for the lifetime of the running
/// server. Dropping it removes the record on graceful shutdown;
/// discovery scans prune dead-PID records left behind by crashes.
pub struct RegisteredInstance {
    record: InstanceRecord,
    path: PathBuf,
}

impl RegisteredInstance {
    /// Publishes a record in the protected per-user registry.
    pub fn register(record: InstanceRecord) -> Result<Self, ControlError> {
        let dir = discovery_dir();
        fs::create_dir_all(&dir).map_err(|err| {
            ControlError::with_details(
                ErrorCode::Internal,
                "failed to create local-control discovery directory",
                err.to_string(),
            )
        })?;
        set_private_dir_permissions(&dir)?;
        let path = record_path(&dir, &record.instance_id);
        write_record(&path, &record)?;
        Ok(Self { record, path })
    }

    pub fn record(&self) -> &InstanceRecord {
        &self.record
    }

    pub fn update(&mut self, record: InstanceRecord) -> Result<(), ControlError> {
        let path = record_path(
            self.path.parent().unwrap_or_else(|| Path::new(".")),
            &record.instance_id,
        );
        write_record(&path, &record)?;
        if path != self.path {
            let _ = fs::remove_file(&self.path);
            self.path = path;
        }
        self.record = record;
        Ok(())
    }
}

fn write_record(path: &Path, record: &InstanceRecord) -> Result<(), ControlError> {
    let bytes = serde_json::to_vec_pretty(record).map_err(|err| {
        ControlError::with_details(
            ErrorCode::Internal,
            "failed to serialize local-control discovery record",
            err.to_string(),
        )
    })?;
    fs::write(path, bytes).map_err(|err| {
        ControlError::with_details(
            ErrorCode::Internal,
            "failed to write local-control discovery record",
            err.to_string(),
        )
    })?;
    set_private_permissions(path)?;
    Ok(())
}

impl Drop for RegisteredInstance {
    // Drop-time cleanup is the best-effort fast path for graceful shutdown.
    // `list_instances_from_dir` is the robust cleanup path: it treats records
    // whose PID is no longer alive as stale, removes them, and ignores malformed
    // or unreadable records so a crash can leave at most a temporary zombie
    // reference that is pruned on the next discovery scan.
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Returns the private registry shared by app publishers and local clients.
pub fn discovery_dir() -> PathBuf {
    if let Some(path) = std::env::var_os(DISCOVERY_DIR_ENV) {
        return PathBuf::from(path);
    }
    if let Some(path) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(path).join("warp").join("local-control");
    }
    let home = std::env::var_os("HOME").unwrap_or_else(|| ".".into());
    PathBuf::from(home).join(".warp").join("local-control")
}

/// Returns compatible live instances that pass a proof-bound app ping.
///
/// The ping follows the normal proof-to-credential-to-HTTP flow and verifies
/// the responding app's instance ID, so a live PID and parseable record alone
/// are insufficient.
pub fn list_instances() -> Vec<InstanceRecord> {
    list_instances_from_dir(&discovery_dir())
        .into_iter()
        .filter(|record| crate::client::probe_instance(record).is_ok())
        .collect()
}

/// Parses structurally valid candidate records and prunes records with dead PIDs.
///
/// This lower-level scan does not contact an app endpoint; callers that need an
/// invokable instance should use [`list_instances`] so candidates also pass the
/// proof-bound probe.
pub fn list_instances_from_dir(dir: &Path) -> Vec<InstanceRecord> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut records = Vec::new();
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let contents = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let record = match serde_json::from_str::<InstanceRecord>(&contents) {
            Ok(r) => r,
            Err(_) => continue,
        };
        if record.protocol_version != PROTOCOL_VERSION {
            continue;
        }
        if !is_pid_alive(record.pid) {
            let _ = fs::remove_file(&path);
            continue;
        }
        records.push(record);
    }
    records.sort_by_key(|record| record.started_at);
    records
}

#[cfg(unix)]
fn is_pid_alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[cfg(not(unix))]
fn is_pid_alive(pid: u32) -> bool {
    std::process::Command::new("tasklist")
        .args(["/FI", &format!("PID eq {pid}"), "/NH"])
        .output()
        .map(|o| !String::from_utf8_lossy(&o.stdout).contains("No tasks"))
        .unwrap_or(true)
}

fn record_path(dir: &Path, instance_id: &InstanceId) -> PathBuf {
    dir.join(format!("{}.json", instance_id.0))
}

#[cfg(unix)]
fn set_private_dir_permissions(path: &Path) -> Result<(), ControlError> {
    let mut permissions = fs::metadata(path)
        .map_err(|err| permissions_error("read local-control discovery directory", err))?
        .permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(path, permissions)
        .map_err(|err| permissions_error("protect local-control discovery directory", err))
}

#[cfg(not(unix))]
fn set_private_dir_permissions(_path: &Path) -> Result<(), ControlError> {
    Err(ControlError::new(
        ErrorCode::LocalControlDisabled,
        "local-control discovery publication is disabled until this platform enforces record ACLs",
    ))
}

#[cfg(unix)]
fn set_private_permissions(path: &Path) -> Result<(), ControlError> {
    let mut permissions = fs::metadata(path)
        .map_err(|err| permissions_error("read local-control discovery record", err))?
        .permissions();
    permissions.set_mode(0o600);
    fs::set_permissions(path, permissions)
        .map_err(|err| permissions_error("protect local-control discovery record", err))
}

#[cfg(not(unix))]
fn set_private_permissions(_path: &Path) -> Result<(), ControlError> {
    Err(ControlError::new(
        ErrorCode::LocalControlDisabled,
        "local-control discovery publication is disabled until this platform enforces record ACLs",
    ))
}

fn permissions_error(operation: &str, error: std::io::Error) -> ControlError {
    ControlError::with_details(
        ErrorCode::Internal,
        format!("failed to {operation}"),
        error.to_string(),
    )
}

#[cfg(test)]
#[path = "discovery_tests.rs"]
mod tests;
