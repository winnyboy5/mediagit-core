// MediaGit - Git for Media Files
// Copyright (C) 2025 MediaGit Contributors
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published
// by the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the
// GNU Affero General Public License for more details.

//! The wire contract, frozen (Phase 9).
//!
//! 1.0 adds a web UI, and that UI will read these shapes. A field quietly
//! renamed or retyped in a refactor breaks every deployed client and every
//! browser tab pointed at an older server, and nothing in the type system
//! notices — `serde` will happily emit a different document from a struct that
//! still compiles.
//!
//! This is the same discipline `dev-tests/compat-fixture` applies to the
//! *storage* format, applied to the *wire* format: a canonical instance of each
//! frozen type is serialized and compared against a literal expected document.
//! A promise in a doc file is not a freeze; a test that fails is.
//!
//! ## Changing a frozen shape
//!
//! **Adding a field is the only backward-compatible change**, and only with
//! `#[serde(default)]` so older senders that omit it still deserialize. Any
//! other change — remove, rename, retype, or make a defaulted field required —
//! is a breaking protocol change and needs a new capability token (see
//! `api_version_is_advertised` below), not an edit to the expected string here.
//!
//! These tests deliberately fail on *additions* too. That is not pedantry: it
//! forces whoever adds the field to come here, confirm they added
//! `#[serde(default)]`, and record the new shape — which is exactly the review
//! that does not happen when a test only checks the fields it already knew
//! about.

use mediagit_protocol::types::*;

/// Compare a value's JSON against the frozen document, with a message that says
/// what to do rather than just printing two blobs.
fn assert_frozen<T: serde::Serialize>(what: &str, value: &T, expected: &str) {
    // Compared as parsed `Value`, not as text. JSON objects are unordered by
    // spec, so a struct-field reorder changes the string while changing nothing
    // a client can observe — a text comparison would fail on that and train
    // people to edit the expected blob without reading why it moved. Every
    // change that *does* matter (added, removed, renamed, retyped) still fails.
    let actual: serde_json::Value =
        serde_json::to_value(value).expect("frozen types must serialize");
    let expected: serde_json::Value =
        serde_json::from_str(expected).expect("the expected document must be valid JSON");
    assert_eq!(
        actual, expected,
        "\n\nThe wire shape of `{what}` changed.\n\
         If you ADDED a field: confirm it is `#[serde(default)]` (older senders \
         omit it), then update the expected document here.\n\
         If you removed, renamed or retyped a field: that breaks every deployed \
         client and the 1.0 UI. Add a capability token instead.\n"
    );
}

#[test]
fn refs_response_shape_is_frozen() {
    assert_frozen(
        "RefInfo",
        &RefInfo {
            name: "refs/heads/main".to_string(),
            oid: "ab".repeat(32),
            target: None,
        },
        r#"{"name":"refs/heads/main","oid":"abababababababababababababababababababababababababababababababab","target":null}"#,
    );

    assert_frozen(
        "RefsResponse",
        &RefsResponse {
            refs: vec![],
            capabilities: vec!["pack-v1".to_string()],
        },
        r#"{"refs":[],"capabilities":["pack-v1"]}"#,
    );
}

#[test]
fn want_negotiation_shape_is_frozen() {
    assert_frozen(
        "WantRequest",
        &WantRequest {
            want: vec!["aa".repeat(32)],
            have: vec![],
        },
        r#"{"want":["aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"],"have":[]}"#,
    );

    assert_frozen(
        "WantResponse",
        &WantResponse {
            request_id: "req-1".to_string(),
        },
        r#"{"request_id":"req-1"}"#,
    );
}

#[test]
fn ref_update_shape_is_frozen() {
    assert_frozen(
        "RefUpdate",
        &RefUpdate {
            name: "refs/heads/main".to_string(),
            old_oid: None,
            new_oid: "cd".repeat(32),
            delete: false,
        },
        r#"{"name":"refs/heads/main","old_oid":null,"new_oid":"cdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcdcd","delete":false}"#,
    );

    assert_frozen(
        "RefUpdateRequest",
        &RefUpdateRequest {
            updates: vec![],
            force: false,
            force_with_lease: false,
        },
        r#"{"updates":[],"force":false,"force_with_lease":false}"#,
    );

    assert_frozen(
        "RefUpdateResult",
        &RefUpdateResult {
            ref_name: "refs/heads/main".to_string(),
            success: true,
            error: None,
        },
        r#"{"ref_name":"refs/heads/main","success":true,"error":null}"#,
    );

    assert_frozen(
        "RefUpdateResponse",
        &RefUpdateResponse {
            success: true,
            results: vec![],
        },
        r#"{"success":true,"results":[]}"#,
    );
}

/// The fields carrying `#[serde(default)]` must stay optional on the wire.
///
/// `force_with_lease` and `delete` were both added after clients shipped. If
/// someone drops the attribute, the struct still compiles and every existing
/// test still passes — but every older client's push starts failing to
/// deserialize server-side, which reads as a network fault, not a protocol
/// break. This is the only test that would catch it.
#[test]
fn fields_added_after_v1_stay_optional_for_older_senders() {
    let legacy_update = r#"{"name":"refs/heads/main","old_oid":null,"new_oid":"aa"}"#;
    let parsed: RefUpdate =
        serde_json::from_str(legacy_update).expect("a sender predating `delete` must still parse");
    assert!(!parsed.delete, "omitted `delete` must mean false");

    let legacy_request = r#"{"updates":[],"force":false}"#;
    let parsed: RefUpdateRequest = serde_json::from_str(legacy_request)
        .expect("a sender predating `force_with_lease` must still parse");
    assert!(
        !parsed.force_with_lease,
        "omitted `force_with_lease` must mean false — defaulting it to true \
         would silently upgrade every legacy push to a force push"
    );
}

/// Unknown fields must be ignored, not rejected.
///
/// This is what makes additive evolution possible at all: a *newer* server
/// sending a field an older client has never heard of must not break that
/// client. If someone adds `#[serde(deny_unknown_fields)]` for strictness, the
/// contract stops being extensible and every future addition becomes breaking.
#[test]
fn unknown_fields_are_ignored_so_the_contract_can_grow() {
    let from_a_future_server = r#"{"refs":[],"capabilities":[],"pagination_token":"xyz"}"#;
    let parsed: RefsResponse = serde_json::from_str(from_a_future_server)
        .expect("an older client must tolerate fields added by a newer server");
    assert!(parsed.refs.is_empty());
}
