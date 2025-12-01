use serde::{Deserialize, Serialize};

/// Information about a reference (branch, tag, etc.)
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct RefInfo {
    /// Reference name (e.g., "refs/heads/main", "refs/tags/v1.0.0")
    pub name: String,
    /// Object ID (SHA-256 hash) the ref points to
    pub oid: String,
    /// For symbolic refs, the target ref name
    pub target: Option<String>,
}

/// Response for GET /info/refs
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RefsResponse {
    /// List of all references in the repository
    pub refs: Vec<RefInfo>,
    /// Protocol capabilities supported by the server
    pub capabilities: Vec<String>,
}

/// Request for POST /objects/want
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct WantRequest {
    /// Object IDs the client wants to receive
    pub want: Vec<String>,
    /// Object IDs the client already has (for delta compression)
    pub have: Vec<String>,
}

/// A single reference update operation
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RefUpdate {
    /// Reference name to update
    pub name: String,
    /// Expected old OID (None for new refs, Some for safety checks)
    pub old_oid: Option<String>,
    /// New OID to set the reference to
    pub new_oid: String,
}

/// Request for POST /refs/update
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RefUpdateRequest {
    /// List of reference updates to apply
    pub updates: Vec<RefUpdate>,
    /// Force update even if not fast-forward
    pub force: bool,
}

/// Result of a single ref update operation
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RefUpdateResult {
    /// Reference name
    pub ref_name: String,
    /// Whether the update succeeded
    pub success: bool,
    /// Error message if update failed
    pub error: Option<String>,
}

/// Response for POST /refs/update
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct RefUpdateResponse {
    /// Overall success status
    pub success: bool,
    /// Results for each ref update
    pub results: Vec<RefUpdateResult>,
}

impl RefUpdateResponse {
    /// Create a successful response
    pub fn success(ref_names: Vec<String>) -> Self {
        Self {
            success: true,
            results: ref_names
                .into_iter()
                .map(|name| RefUpdateResult {
                    ref_name: name,
                    success: true,
                    error: None,
                })
                .collect(),
        }
    }

    /// Create a failed response
    pub fn failed(ref_name: String, error: String) -> Self {
        Self {
            success: false,
            results: vec![RefUpdateResult {
                ref_name,
                success: false,
                error: Some(error),
            }],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ref_info_serialization() {
        let info = RefInfo {
            name: "refs/heads/main".to_string(),
            oid: "abc123".to_string(),
            target: None,
        };

        let json = serde_json::to_string(&info).unwrap();
        let deserialized: RefInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(info, deserialized);
    }

    #[test]
    fn test_refs_response_serialization() {
        let response = RefsResponse {
            refs: vec![RefInfo {
                name: "refs/heads/main".to_string(),
                oid: "abc123".to_string(),
                target: None,
            }],
            capabilities: vec!["pack-v1".to_string()],
        };

        let json = serde_json::to_string(&response).unwrap();
        let deserialized: RefsResponse = serde_json::from_str(&json).unwrap();
        assert_eq!(response.refs.len(), deserialized.refs.len());
    }

    #[test]
    fn test_want_request_serialization() {
        let request = WantRequest {
            want: vec!["abc123".to_string()],
            have: vec!["def456".to_string()],
        };

        let json = serde_json::to_string(&request).unwrap();
        let deserialized: WantRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(request.want, deserialized.want);
        assert_eq!(request.have, deserialized.have);
    }

    #[test]
    fn test_ref_update_request_serialization() {
        let request = RefUpdateRequest {
            updates: vec![RefUpdate {
                name: "refs/heads/main".to_string(),
                old_oid: Some("old123".to_string()),
                new_oid: "new456".to_string(),
            }],
            force: false,
        };

        let json = serde_json::to_string(&request).unwrap();
        let deserialized: RefUpdateRequest = serde_json::from_str(&json).unwrap();
        assert_eq!(request.updates.len(), deserialized.updates.len());
    }

    #[test]
    fn test_ref_update_response_success() {
        let response = RefUpdateResponse::success(vec!["refs/heads/main".to_string()]);
        assert!(response.success);
        assert_eq!(response.results.len(), 1);
        assert!(response.results[0].success);
    }

    #[test]
    fn test_ref_update_response_failed() {
        let response = RefUpdateResponse::failed(
            "refs/heads/main".to_string(),
            "not fast-forward".to_string(),
        );
        assert!(!response.success);
        assert_eq!(response.results.len(), 1);
        assert!(!response.results[0].success);
    }
}
