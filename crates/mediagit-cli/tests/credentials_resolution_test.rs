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

//! Client-auth credential resolution tests (M2 Step 1 + I10 OS-keychain tier).
//!
//! Precedence: env `MEDIAGIT_TOKEN`/`MEDIAGIT_API_KEY` -> OS keychain
//! (skippable via `MEDIAGIT_NO_KEYRING`) -> per-remote config
//! (`remotes.<name>.token`/`.api_key`) -> none.
//!
//! `credential_precedence_env_keychain_config_none` always sets
//! `MEDIAGIT_NO_KEYRING=1` so it never touches the real OS credential store
//! -- running `cargo test` shouldn't write real Windows Credential
//! Manager/macOS Keychain/Secret Service entries on a dev machine or CI
//! runner. The keychain tier itself is covered by
//! `keychain_tier_write_through_and_read_back`, which is opt-in
//! (`MEDIAGIT_TEST_REAL_KEYRING=1`) and deletes the one entry it creates.
//!
//! `MEDIAGIT_TOKEN`/`MEDIAGIT_API_KEY`/`MEDIAGIT_NO_KEYRING` are not touched
//! by any other test in this workspace (verified via grep). Both tests below
//! share `ENV_LOCK` so they never race on these process-global vars even
//! though `cargo test` runs `#[test]` functions in this binary on parallel
//! threads.

use mediagit_cli::repo::{remember_credentials, resolve_credentials};
use mediagit_config::{Config, RemoteConfig};
use mediagit_protocol::Credentials;
use std::sync::Mutex;
use tempfile::TempDir;

static ENV_LOCK: Mutex<()> = Mutex::new(());

fn config_with_remote(remote: RemoteConfig) -> Config {
    let mut config = Config::default();
    config.remotes.insert("origin".to_string(), remote);
    config
}

#[test]
fn credential_precedence_env_keychain_config_none() {
    let _guard = ENV_LOCK.lock().unwrap();
    let temp_dir = TempDir::new().unwrap();
    let repo_root = temp_dir.path();

    // Ensure a clean slate regardless of the outer environment. Keychain
    // disabled throughout this test -- it only exercises the env/config
    // tiers and their ordering relative to each other.
    std::env::remove_var("MEDIAGIT_TOKEN");
    std::env::remove_var("MEDIAGIT_API_KEY");
    std::env::set_var("MEDIAGIT_NO_KEYRING", "1");

    // 1. No config, no env -> None.
    let empty_config = Config::default();
    assert_eq!(
        resolve_credentials(repo_root, &empty_config, "origin"),
        Credentials::None
    );

    // 2. Env token set, no config -> Bearer from env.
    std::env::set_var("MEDIAGIT_TOKEN", "env-token");
    assert_eq!(
        resolve_credentials(repo_root, &empty_config, "origin"),
        Credentials::Bearer("env-token".to_string())
    );

    // 3. Env api key set (token unset), no config -> ApiKey from env.
    std::env::remove_var("MEDIAGIT_TOKEN");
    std::env::set_var("MEDIAGIT_API_KEY", "env-key");
    assert_eq!(
        resolve_credentials(repo_root, &empty_config, "origin"),
        Credentials::ApiKey("env-key".to_string())
    );
    std::env::remove_var("MEDIAGIT_API_KEY");

    // 4. Config token set AND env token set -> env wins now (I10 reordered
    //    env/explicit ahead of the file tier so env always overrides a
    //    checked-in or stale config.toml value).
    let mut remote = RemoteConfig::new("http://localhost:3000/repo");
    remote.token = Some("config-token".to_string());
    let config_with_token = config_with_remote(remote);
    std::env::set_var("MEDIAGIT_TOKEN", "env-token-wins");
    assert_eq!(
        resolve_credentials(repo_root, &config_with_token, "origin"),
        Credentials::Bearer("env-token-wins".to_string())
    );

    // 5. Config token set, no env -> config wins (file is still the last
    //    fallback tier, reached once env and keychain are both empty).
    std::env::remove_var("MEDIAGIT_TOKEN");
    assert_eq!(
        resolve_credentials(repo_root, &config_with_token, "origin"),
        Credentials::Bearer("config-token".to_string())
    );

    // 6. Config api_key set (no config token, no env) -> config api_key.
    let mut remote = RemoteConfig::new("http://localhost:3000/repo");
    remote.api_key = Some("config-key".to_string());
    let config_with_key = config_with_remote(remote);
    assert_eq!(
        resolve_credentials(repo_root, &config_with_key, "origin"),
        Credentials::ApiKey("config-key".to_string())
    );

    // 7. Unknown remote name falls through to env, ignoring the "origin"
    //    entry entirely.
    std::env::set_var("MEDIAGIT_TOKEN", "env-token-wins");
    assert_eq!(
        resolve_credentials(repo_root, &config_with_token, "some-other-remote"),
        Credentials::Bearer("env-token-wins".to_string())
    );

    // Cleanup so this doesn't leak into any test that runs after it in the
    // same process.
    std::env::remove_var("MEDIAGIT_TOKEN");
    std::env::remove_var("MEDIAGIT_API_KEY");
    std::env::remove_var("MEDIAGIT_NO_KEYRING");
}

/// Exercises the real OS keychain: `remember_credentials` write-through
/// followed by `resolve_credentials` reading it back with no env/config
/// value to shadow it. Skipped by default (opt-in via
/// `MEDIAGIT_TEST_REAL_KEYRING=1`) since it writes an entry into whatever
/// credential store this machine has (Windows Credential Manager here) --
/// not something a routine `cargo test` run should do unprompted. Deletes
/// its one entry when done, pass or fail.
#[test]
fn keychain_tier_write_through_and_read_back() {
    if std::env::var_os("MEDIAGIT_TEST_REAL_KEYRING").is_none() {
        eprintln!(
            "skipping keychain_tier_write_through_and_read_back: set MEDIAGIT_TEST_REAL_KEYRING=1 to run against the real OS keychain"
        );
        return;
    }
    let _guard = ENV_LOCK.lock().unwrap();
    let temp_dir = TempDir::new().unwrap();
    let repo_root = temp_dir.path();

    std::env::remove_var("MEDIAGIT_TOKEN");
    std::env::remove_var("MEDIAGIT_API_KEY");
    std::env::remove_var("MEDIAGIT_NO_KEYRING");

    let account = "http://localhost:9999/mediagit-i10-keyring-test-repo";
    let mut remote = RemoteConfig::new(account);
    remote.token = None; // nothing in config.toml -- only the keychain has it
    let config = config_with_remote(remote);

    // Belt-and-suspenders: delete any leftover entry from a previous failed
    // run before asserting on a fresh write.
    let cleanup = || {
        if let Ok(entry) = keyring::Entry::new("mediagit", account) {
            let _ = entry.delete_credential();
        }
    };
    cleanup();

    remember_credentials(
        &config,
        "origin",
        &Credentials::Bearer("keychain-token".to_string()),
    );
    let result = resolve_credentials(repo_root, &config, "origin");

    cleanup();

    assert_eq!(result, Credentials::Bearer("keychain-token".to_string()));
}
