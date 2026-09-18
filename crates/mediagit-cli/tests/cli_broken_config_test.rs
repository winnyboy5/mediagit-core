// SPDX-License-Identifier: BUSL-1.1
// Copyright (C) 2025-2026 Aswin Krishnamoorthy

//! D1: a repository whose `config.toml` cannot be read must FAIL LOUDLY and
//! leave the file alone.
//!
//! This guards a data-loss path, not an ergonomic one.
//! `create_storage_backend` used to do `Config::load(..).unwrap_or_default()`.
//! `Config::load` already returns the default for an ABSENT file, so that
//! `unwrap_or_default` could only ever swallow a real error — and
//! `resolve_repo_id` would then PERSIST the resulting default over the real
//! config. One command against an unreadable config and the repository lost:
//!
//! * `cdc_seed` — every future chunk boundary moves, destroying dedup against
//!   the repository's own existing objects
//! * `repo_namespace` — writes start landing under a different key prefix
//! * `layout_version` — claims layout v1 while the objects are v2
//! * `repo_id` — a fresh id, so the next command reports a namespace collision
//!   between the repository and itself
//!
//! It stayed latent while the parser was permissive. `deny_unknown_fields` put
//! it one typo away, which is how it was found.

#![allow(clippy::unwrap_used)]

use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::Path;
use tempfile::TempDir;

fn mediagit() -> Command {
    let mut c = Command::cargo_bin("mediagit").unwrap();
    c.env("MEDIAGIT_AUTHOR_NAME", "Test User")
        .env("MEDIAGIT_AUTHOR_EMAIL", "test@example.com")
        .env("MEDIAGIT_NO_KEYRING", "1");
    c
}

fn init_repo(dir: &Path) {
    mediagit()
        .arg("init")
        .arg("-q")
        .current_dir(dir)
        .assert()
        .success();
}

fn config_path(repo: &Path) -> std::path::PathBuf {
    repo.join(".mediagit").join("config.toml")
}

/// Both halves. A broken config must stop the command, AND the file must come
/// back untouched. Asserting only the first would pass against a build that
/// errors *after* having already rewritten the file.
#[test]
fn an_unreadable_config_stops_the_command_and_is_left_intact() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let before = fs::read_to_string(config_path(&repo)).unwrap();
    assert!(
        before.contains("cdc_seed") && before.contains("repo_namespace"),
        "the fixture must actually carry the fields at risk:\n{before}"
    );

    // A key no field accepts. Chosen as a near-miss typo rather than garbage,
    // because that is the realistic case and the one that used to pass.
    let broken = before.replace("[performance]", "[performance]\nupload_concurency = 8");
    fs::write(config_path(&repo), &broken).unwrap();

    mediagit()
        .arg("status")
        .current_dir(&repo)
        .assert()
        .failure()
        .stderr(predicate::str::contains("upload_concurency"));

    let after = fs::read_to_string(config_path(&repo)).unwrap();
    assert_eq!(
        after, broken,
        "a failed load must not rewrite config.toml — this is the data-loss half"
    );
}

/// The distinction the fix depends on: ABSENT is the default, PRESENT AND
/// BROKEN is an error. If `Config::load` ever started erroring on a missing
/// file, propagating instead of defaulting would break every fresh directory.
#[test]
fn an_absent_config_is_still_fine() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    fs::remove_file(config_path(&repo)).unwrap();

    mediagit()
        .arg("status")
        .current_dir(&repo)
        .assert()
        .success();
}

/// A config written by an older version must open without complaint. Every
/// `config.toml` this tool has ever written carries these sections, so if this
/// fails, every existing repository is bricked.
#[test]
fn a_v3_config_opens_and_is_cleaned_in_place() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().join("repo");
    fs::create_dir_all(&repo).unwrap();
    init_repo(&repo);

    let current = fs::read_to_string(config_path(&repo)).unwrap();
    let v3 = current.replace("config_version = 4", "config_version = 3")
        + r#"
[app]
name = "mediagit"
port = 8080

[observability]
log_level = "info"

[observability.metrics]
enabled = true

[security]
cors_origins = ["http://localhost:3000"]

[security.rate_limiting]
enabled = false
"#;
    fs::write(config_path(&repo), &v3).unwrap();

    mediagit()
        .arg("status")
        .current_dir(&repo)
        .assert()
        .success();

    let after = fs::read_to_string(config_path(&repo)).unwrap();
    assert!(after.contains("config_version = 4"), "{after}");
    for dead in ["[app]", "[observability]", "[security]"] {
        assert!(!after.contains(dead), "{dead} must be gone:\n{after}");
    }

    // The values that matter survived the rewrite. This is the half a
    // migration test that only checks "the dead keys are gone" would miss.
    for line in current.lines().filter(|l| {
        l.starts_with("cdc_seed") || l.starts_with("repo_namespace") || l.starts_with("repo_id")
    }) {
        assert!(after.contains(line), "{line} must survive:\n{after}");
    }

    assert!(
        repo.join(".mediagit").join("config.toml.bak").exists(),
        "the pre-migration config must be backed up"
    );
}
