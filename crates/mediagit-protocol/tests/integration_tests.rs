// Integration tests for mediagit-protocol
// These tests verify protocol serialization and basic client functionality

use mediagit_protocol::{
    RefInfo, RefUpdate, RefUpdateRequest, RefUpdateResponse, RefUpdateResult, RefsResponse,
    WantRequest,
};
use serde_json;

#[test]
fn test_ref_info_serialization() {
    let ref_info = RefInfo {
        name: "refs/heads/main".to_string(),
        oid: "abc123def456".to_string(),
        target: None,
    };

    // Serialize to JSON
    let json = serde_json::to_string(&ref_info).expect("Failed to serialize");
    assert!(json.contains("refs/heads/main"));
    assert!(json.contains("abc123def456"));

    // Deserialize back
    let deserialized: RefInfo = serde_json::from_str(&json).expect("Failed to deserialize");
    assert_eq!(deserialized.name, "refs/heads/main");
    assert_eq!(deserialized.oid, "abc123def456");
    assert_eq!(deserialized.target, None);
}

#[test]
fn test_ref_info_symbolic() {
    let ref_info = RefInfo {
        name: "HEAD".to_string(),
        oid: String::new(),
        target: Some("refs/heads/main".to_string()),
    };

    let json = serde_json::to_string(&ref_info).expect("Failed to serialize");
    let deserialized: RefInfo = serde_json::from_str(&json).expect("Failed to deserialize");

    assert_eq!(deserialized.name, "HEAD");
    assert_eq!(deserialized.target, Some("refs/heads/main".to_string()));
}

#[test]
fn test_refs_response() {
    let refs_response = RefsResponse {
        refs: vec![
            RefInfo {
                name: "HEAD".to_string(),
                oid: String::new(),
                target: Some("refs/heads/main".to_string()),
            },
            RefInfo {
                name: "refs/heads/main".to_string(),
                oid: "abc123".to_string(),
                target: None,
            },
        ],
        capabilities: vec!["pack-v1".to_string()],
    };

    let json = serde_json::to_string(&refs_response).expect("Failed to serialize");
    let deserialized: RefsResponse =
        serde_json::from_str(&json).expect("Failed to deserialize");

    assert_eq!(deserialized.refs.len(), 2);
    assert_eq!(deserialized.capabilities.len(), 1);
    assert_eq!(deserialized.capabilities[0], "pack-v1");
}

#[test]
fn test_ref_update() {
    let update = RefUpdate {
        name: "refs/heads/main".to_string(),
        old_oid: Some("abc123".to_string()),
        new_oid: "def456".to_string(),
    };

    let json = serde_json::to_string(&update).expect("Failed to serialize");
    let deserialized: RefUpdate = serde_json::from_str(&json).expect("Failed to deserialize");

    assert_eq!(deserialized.name, "refs/heads/main");
    assert_eq!(deserialized.old_oid, Some("abc123".to_string()));
    assert_eq!(deserialized.new_oid, "def456");
}

#[test]
fn test_ref_update_request() {
    let request = RefUpdateRequest {
        updates: vec![RefUpdate {
            name: "refs/heads/main".to_string(),
            old_oid: None,
            new_oid: "abc123".to_string(),
        }],
        force: false,
    };

    let json = serde_json::to_string(&request).expect("Failed to serialize");
    let deserialized: RefUpdateRequest =
        serde_json::from_str(&json).expect("Failed to deserialize");

    assert_eq!(deserialized.updates.len(), 1);
    assert_eq!(deserialized.force, false);
}

#[test]
fn test_ref_update_response() {
    let response = RefUpdateResponse {
        success: true,
        results: vec![RefUpdateResult {
            ref_name: "refs/heads/main".to_string(),
            success: true,
            error: None,
        }],
    };

    let json = serde_json::to_string(&response).expect("Failed to serialize");
    let deserialized: RefUpdateResponse =
        serde_json::from_str(&json).expect("Failed to deserialize");

    assert_eq!(deserialized.success, true);
    assert_eq!(deserialized.results.len(), 1);
    assert_eq!(deserialized.results[0].success, true);
    assert_eq!(deserialized.results[0].error, None);
}

#[test]
fn test_ref_update_response_with_error() {
    let response = RefUpdateResponse {
        success: false,
        results: vec![RefUpdateResult {
            ref_name: "refs/heads/main".to_string(),
            success: false,
            error: Some("not fast-forward".to_string()),
        }],
    };

    let json = serde_json::to_string(&response).expect("Failed to serialize");
    let deserialized: RefUpdateResponse =
        serde_json::from_str(&json).expect("Failed to deserialize");

    assert_eq!(deserialized.success, false);
    assert_eq!(deserialized.results[0].success, false);
    assert_eq!(
        deserialized.results[0].error,
        Some("not fast-forward".to_string())
    );
}

#[test]
fn test_want_request() {
    let request = WantRequest {
        want: vec!["abc123".to_string(), "def456".to_string()],
        have: vec!["ghi789".to_string()],
    };

    let json = serde_json::to_string(&request).expect("Failed to serialize");
    let deserialized: WantRequest = serde_json::from_str(&json).expect("Failed to deserialize");

    assert_eq!(deserialized.want.len(), 2);
    assert_eq!(deserialized.have.len(), 1);
    assert_eq!(deserialized.want[0], "abc123");
    assert_eq!(deserialized.have[0], "ghi789");
}

#[test]
fn test_protocol_client_creation() {
    let client = mediagit_protocol::ProtocolClient::new("http://localhost:3000/test-repo");
    // Just verify it can be created
    drop(client);
}

#[test]
fn test_multiple_ref_updates() {
    let request = RefUpdateRequest {
        updates: vec![
            RefUpdate {
                name: "refs/heads/main".to_string(),
                old_oid: Some("abc123".to_string()),
                new_oid: "def456".to_string(),
            },
            RefUpdate {
                name: "refs/heads/feature".to_string(),
                old_oid: None,
                new_oid: "ghi789".to_string(),
            },
        ],
        force: false,
    };

    let json = serde_json::to_string(&request).expect("Failed to serialize");
    let deserialized: RefUpdateRequest =
        serde_json::from_str(&json).expect("Failed to deserialize");

    assert_eq!(deserialized.updates.len(), 2);
    assert_eq!(deserialized.updates[0].name, "refs/heads/main");
    assert_eq!(deserialized.updates[1].name, "refs/heads/feature");
}

#[test]
fn test_force_push() {
    let request = RefUpdateRequest {
        updates: vec![RefUpdate {
            name: "refs/heads/main".to_string(),
            old_oid: Some("abc123".to_string()),
            new_oid: "xyz999".to_string(),
        }],
        force: true,
    };

    let json = serde_json::to_string(&request).expect("Failed to serialize");
    let deserialized: RefUpdateRequest =
        serde_json::from_str(&json).expect("Failed to deserialize");

    assert_eq!(deserialized.force, true);
}
