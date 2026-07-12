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

//! Client-auth credential resolution tests (M2 Step 1).
//!
//! Precedence: per-remote config (`remotes.<name>.token`/`.api_key`) → env
//! `MEDIAGIT_TOKEN`/`MEDIAGIT_API_KEY` → none.
//!
//! `MEDIAGIT_TOKEN`/`MEDIAGIT_API_KEY` are not touched by any other test in
//! this workspace (verified via grep), so mutating them here is safe as
//! long as this stays a single sequential test function — parallel test
//! *threads* within this binary never race on these two var names.

use mediagit_cli::repo::resolve_credentials;
use mediagit_config::{Config, RemoteConfig};
use mediagit_protocol::Credentials;
use tempfile::TempDir;

fn config_with_remote(remote: RemoteConfig) -> Config {
    let mut config = Config::default();
    config.remotes.insert("origin".to_string(), remote);
    config
}

#[test]
fn credential_precedence_config_env_none() {
    let temp_dir = TempDir::new().unwrap();
    let repo_root = temp_dir.path();

    // Ensure a clean slate regardless of the outer environment.
    std::env::remove_var("MEDIAGIT_TOKEN");
    std::env::remove_var("MEDIAGIT_API_KEY");

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

    // 4. Config token set AND env token set -> config wins.
    let mut remote = RemoteConfig::new("http://localhost:3000/repo");
    remote.token = Some("config-token".to_string());
    let config_with_token = config_with_remote(remote);
    std::env::set_var("MEDIAGIT_TOKEN", "env-token-should-lose");
    assert_eq!(
        resolve_credentials(repo_root, &config_with_token, "origin"),
        Credentials::Bearer("config-token".to_string())
    );

    // 5. Config api_key set (no config token), env token also set -> config
    //    api_key still wins (per-remote config beats env unconditionally).
    let mut remote = RemoteConfig::new("http://localhost:3000/repo");
    remote.api_key = Some("config-key".to_string());
    let config_with_key = config_with_remote(remote);
    assert_eq!(
        resolve_credentials(repo_root, &config_with_key, "origin"),
        Credentials::ApiKey("config-key".to_string())
    );

    // 6. Unknown remote name falls through to env, ignoring the "origin"
    //    entry entirely.
    assert_eq!(
        resolve_credentials(repo_root, &config_with_token, "some-other-remote"),
        Credentials::Bearer("env-token-should-lose".to_string())
    );

    // Cleanup so this doesn't leak into any test that runs after it in the
    // same process.
    std::env::remove_var("MEDIAGIT_TOKEN");
    std::env::remove_var("MEDIAGIT_API_KEY");
}
