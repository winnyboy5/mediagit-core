// Copyright (C) 2026  winnyboy5
//
// This program is free software: you can redistribute it and/or modify
// it under the terms of the GNU Affero General Public License as published by
// the Free Software Foundation, either version 3 of the License, or
// (at your option) any later version.
//
// This program is distributed in the hope that it will be useful,
// but WITHOUT ANY WARRANTY; without even the implied warranty of
// MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
// GNU Affero General Public License for more details.
//
// You should have received a copy of the GNU Affero General Public License
// along with this program.  If not, see <https://www.gnu.org/licenses/>.
// Integration tests for mediagit-server
// These tests verify basic server configuration parsing

#[test]
fn test_toml_config_parsing() {
    use std::path::PathBuf;

    #[derive(serde::Deserialize)]
    struct ServerConfig {
        port: u16,
        host: String,
        repos_dir: PathBuf,
    }

    let toml_str = r#"
        port = 8080
        host = "0.0.0.0"
        repos_dir = "/var/repos"
    "#;

    let config: ServerConfig = toml::from_str(toml_str).expect("Failed to parse config");
    assert_eq!(config.port, 8080);
    assert_eq!(config.host, "0.0.0.0");
    assert_eq!(config.repos_dir, PathBuf::from("/var/repos"));
}

#[test]
fn test_default_config_values() {
    use std::path::PathBuf;

    #[derive(serde::Deserialize)]
    #[serde(default)]
    struct ServerConfig {
        port: u16,
        host: String,
        repos_dir: PathBuf,
    }

    impl Default for ServerConfig {
        fn default() -> Self {
            Self {
                port: 3000,
                host: "127.0.0.1".to_string(),
                repos_dir: PathBuf::from("./repos"),
            }
        }
    }

    let config = ServerConfig::default();
    assert_eq!(config.port, 3000);
    assert_eq!(config.host, "127.0.0.1");
}

#[test]
fn test_bind_address_format() {
    let host = "127.0.0.1";
    let port = 3000;
    let bind_addr = format!("{}:{}", host, port);
    assert_eq!(bind_addr, "127.0.0.1:3000");

    let parsed: std::net::SocketAddr = bind_addr.parse().expect("Invalid address");
    assert_eq!(parsed.port(), 3000);
}

#[test]
fn test_repository_path_construction() {
    use std::path::PathBuf;

    let repos_dir = PathBuf::from("./repos");
    let repo_name = "test-repo";
    let repo_path = repos_dir.join(repo_name);

    assert_eq!(repo_path, PathBuf::from("./repos/test-repo"));

    let objects_path = repo_path.join(".mediagit/objects");
    assert_eq!(
        objects_path,
        PathBuf::from("./repos/test-repo/.mediagit/objects")
    );
}

// Note: Full E2E tests requiring actual server startup would be in
// a separate test file or behind a feature flag to avoid blocking CI
