use std::fs;
use std::path::Path;

#[cfg(unix)]
use command::blocking::Command;

use super::*;

fn record() -> InstanceRecord {
    InstanceRecord::for_current_process(
        "local",
        "dev.warp.WarpLocal",
        Some("test".to_owned()),
        crate::protocol::ActionKind::implemented_metadata(),
    )
}

#[test]
fn control_endpoint_composes_loopback_routes() {
    let endpoint = ControlEndpoint::localhost(4000);
    assert_eq!(endpoint.url(), "http://127.0.0.1:4000/v1/control");
    assert_eq!(
        endpoint.credential_url(),
        "http://127.0.0.1:4000/v1/control/credentials"
    );
}

#[test]
fn registered_instance_round_trips_discovery_record() {
    let dir = tempfile::tempdir().expect("temp dir");
    let record = record();
    let _registered = RegisteredInstance::register_in_dir_for_test(record.clone(), dir.path())
        .expect("registered");
    assert_eq!(list_instances_from_dir(dir.path()), vec![record]);
}

#[test]
fn incompatible_protocol_record_is_ignored() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut record = record();
    record.protocol_version = PROTOCOL_VERSION + 1;
    let _registered =
        RegisteredInstance::register_in_dir_for_test(record, dir.path()).expect("registered");
    assert!(list_instances_from_dir(dir.path()).is_empty());
}

#[cfg(unix)]
#[test]
fn stale_process_record_is_pruned() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut child = Command::new("true")
        .spawn()
        .expect("short-lived process starts");
    let pid = child.id();
    child.wait().expect("short-lived process exits");
    let mut record = record();
    record.pid = pid;
    let registered =
        RegisteredInstance::register_in_dir_for_test(record, dir.path()).expect("registered");
    assert!(list_instances_from_dir(dir.path()).is_empty());
    assert!(!registered.path.exists());
}

#[cfg(unix)]
#[test]
fn multiple_live_process_records_are_discovered() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut first_process = Command::new("sleep")
        .arg("10")
        .spawn()
        .expect("first process starts");
    let mut second_process = Command::new("sleep")
        .arg("10")
        .spawn()
        .expect("second process starts");
    let mut first_record = record();
    first_record.pid = first_process.id();
    let mut second_record = record();
    second_record.pid = second_process.id();
    let first_id = first_record.instance_id.clone();
    let second_id = second_record.instance_id.clone();
    let _first = RegisteredInstance::register_in_dir_for_test(first_record, dir.path())
        .expect("first registered");
    let _second = RegisteredInstance::register_in_dir_for_test(second_record, dir.path())
        .expect("second registered");
    let ids = list_instances_from_dir(dir.path())
        .into_iter()
        .map(|record| record.instance_id)
        .collect::<Vec<_>>();
    assert_eq!(ids.len(), 2);
    assert!(ids.contains(&first_id));
    assert!(ids.contains(&second_id));
    first_process.kill().expect("first process stops");
    first_process.wait().expect("first process reaped");
    second_process.kill().expect("second process stops");
    second_process.wait().expect("second process reaped");
}

#[test]
fn serialized_discovery_record_is_non_actionable_metadata() {
    let serialized = serde_json::to_value(record()).expect("serialize");
    assert!(serialized.get("endpoint").is_none());
    assert!(serialized.get("credential_broker").is_none());
    assert!(serialized.get("outside_warp_control_enabled").is_none());
    assert!(serialized.get("auth_token").is_none());
    assert!(serialized.get("bearer_token").is_none());
}

#[cfg(unix)]
#[test]
fn discovery_directory_and_record_are_owner_only_on_unix() {
    use std::os::unix::fs::PermissionsExt as _;

    let dir = tempfile::tempdir().expect("temp dir");
    let registered =
        RegisteredInstance::register_in_dir_for_test(record(), dir.path()).expect("registered");
    let dir_mode = fs::metadata(dir.path())
        .expect("directory metadata")
        .permissions()
        .mode()
        & 0o777;
    let record_mode = fs::metadata(&registered.path)
        .expect("record metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(dir_mode, 0o700);
    assert_eq!(record_mode, 0o600);
}

impl RegisteredInstance {
    fn register_in_dir_for_test(record: InstanceRecord, dir: &Path) -> Result<Self, ControlError> {
        fs::create_dir_all(dir).expect("create dir");
        set_private_dir_permissions(dir)?;
        let path = record_path(dir, &record.instance_id);
        write_record(&path, &record)?;
        Ok(Self { record, path })
    }
}
