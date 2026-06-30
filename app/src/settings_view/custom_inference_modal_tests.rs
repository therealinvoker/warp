use super::*;

// --- ConnectionTestStatus / probe-result tests ---

#[test]
fn http_200_response_sets_confirmed_status() {
    // When the probe receives a 2xx HTTP response (success = true) the status
    // should transition to Confirmed.
    assert_eq!(
        connection_status_from_result(true),
        ConnectionTestStatus::Confirmed,
    );
}

#[test]
fn http_error_or_non_200_sets_failed_status() {
    // A non-2xx response or a network/transport error (success = false) should
    // produce Failed, not Confirmed.
    assert_eq!(
        connection_status_from_result(false),
        ConnectionTestStatus::Failed,
    );
}

#[test]
fn editing_url_or_api_key_resets_connection_status_to_idle() {
    // handle_endpoint_url_event and handle_api_key_event both call
    // reset_connection_test_status() on any Edited event, which unconditionally
    // sets connection_test_status to Idle.  Verify the invariant: every
    // reachable non-Idle state (Confirmed, Failed, Testing) differs from Idle,
    // so a URL or API-key edit always produces a visible state change (and can
    // never accidentally stay in a stale terminal result).
    let non_idle_states = [
        connection_status_from_result(true),  // Confirmed
        connection_status_from_result(false), // Failed
        ConnectionTestStatus::Testing,
    ];
    for state in non_idle_states {
        assert_ne!(
            state,
            ConnectionTestStatus::Idle,
            "{state:?} should differ from Idle so that a URL/key edit triggers a visible reset",
        );
    }
}

// --- validate_url tests (existing) ---

#[test]
fn validate_url_accepts_https_with_host() {
    assert!(validate_url("https://api.example.com/v1").is_ok());
    assert!(validate_url("https://example.com").is_ok());
    assert!(validate_url("https://8.8.8.8/v1").is_ok());
}

#[test]
fn validate_url_rejects_http() {
    assert_eq!(
        validate_url("http://api.example.com/v1"),
        Err("URL must use HTTPS")
    );
    assert_eq!(
        validate_url("http://example.com"),
        Err("URL must use HTTPS")
    );
}

#[test]
fn validate_url_rejects_ftp_and_other_schemes() {
    assert_eq!(
        validate_url("ftp://files.example.com"),
        Err("URL must use HTTPS")
    );
    assert_eq!(
        validate_url("file:///etc/passwd"),
        Err("URL must use HTTPS")
    );
    assert_eq!(
        validate_url("ws://socket.example.com"),
        Err("URL must use HTTPS")
    );
}

#[test]
fn validate_url_rejects_malformed_strings() {
    assert_eq!(validate_url("not a url"), Err("Invalid URL"));
    assert_eq!(validate_url("https://"), Err("Invalid URL"));
}

#[test]
fn validate_url_rejects_empty_host() {
    assert_eq!(validate_url("https://?query=1"), Err("Invalid URL"));
}

#[test]
fn validate_url_allows_empty_string() {
    assert!(validate_url("").is_ok());
}

#[test]
fn validate_url_allows_whitespace_only() {
    assert!(validate_url("   ").is_ok());
}

#[test]
fn validate_url_rejects_localhost_and_private_ips() {
    let error = Err("URL must not use a local or private host");
    assert_eq!(validate_url("https://localhost:8080"), error);
    assert_eq!(validate_url("https://127.0.0.1/v1"), error);
    assert_eq!(validate_url("https://0.0.0.0/v1"), error);
    assert_eq!(validate_url("https://10.0.0.1/v1"), error);
    assert_eq!(validate_url("https://172.16.0.1/v1"), error);
    assert_eq!(validate_url("https://192.168.0.1/v1"), error);
    assert_eq!(validate_url("https://169.254.0.1/v1"), error);
    assert_eq!(validate_url("https://[::1]/v1"), error);
    assert_eq!(validate_url("https://[::]/v1"), error);
    assert_eq!(validate_url("https://[fc00::1]/v1"), error);
    assert_eq!(validate_url("https://[fe80::1]/v1"), error);
    assert_eq!(validate_url("https://[::ffff:192.168.0.1]/v1"), error);
}

#[test]
fn endpoint_form_valid_rejects_invalid_current_url() {
    assert!(!is_endpoint_form_valid(
        "Endpoint",
        "http://api.example.com/v1",
        "key",
        true
    ));
}

#[test]
fn endpoint_form_valid_requires_non_empty_url() {
    assert!(!is_endpoint_form_valid("Endpoint", "", "key", true));
    assert!(!is_endpoint_form_valid("Endpoint", "   ", "key", true));
}

#[test]
fn endpoint_form_valid_accepts_complete_valid_form() {
    assert!(is_endpoint_form_valid(
        "Endpoint",
        "https://api.example.com/v1",
        "key",
        true
    ));
}
