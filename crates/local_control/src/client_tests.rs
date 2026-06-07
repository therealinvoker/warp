use chrono::Utc;
use uuid::Uuid;

use super::*;
use crate::discovery::InstanceId;

#[test]
fn probe_rejects_mismatched_instance_identity() {
    let instance = InstanceRecord {
        protocol_version: crate::PROTOCOL_VERSION,
        instance_id: InstanceId("inst_expected".to_owned()),
        pid: std::process::id(),
        channel: "local".to_owned(),
        app_id: "dev.warp.WarpLocal".to_owned(),
        app_version: None,
        started_at: Utc::now(),
        executable_path: None,
        actions: vec![ActionKind::AppPing.metadata()],
    };
    let error = validate_probe_response(
        &instance,
        ResponseEnvelope::ok(
            Uuid::new_v4(),
            serde_json::json!({ "instance_id": "inst_other" }),
        ),
    )
    .expect_err("mismatched live identity is rejected");
    assert_eq!(error.code, ErrorCode::TransportUnavailable);
}
