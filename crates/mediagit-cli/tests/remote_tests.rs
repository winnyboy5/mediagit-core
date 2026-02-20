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
use anyhow::Result;
use mediagit_config::Config;
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

/// Helper to create a test repository
fn create_test_repo() -> Result<(TempDir, PathBuf)> {
    let temp_dir = TempDir::new()?;
    let repo_path = temp_dir.path().to_path_buf();
    let mediagit_dir = repo_path.join(".mediagit");

    fs::create_dir_all(&mediagit_dir)?;

    // Create default config
    let config = Config::default();
    config.save(&repo_path)?;

    Ok((temp_dir, repo_path))
}

#[tokio::test]
async fn test_add_remote() -> Result<()> {
    let (_temp, repo_path) = create_test_repo()?;

    // Load config
    let mut config = Config::load(&repo_path).await?;

    // Add remote
    config.set_remote("origin", "https://example.com/repo.git");
    config.save(&repo_path)?;

    // Verify remote was added
    let loaded_config = Config::load(&repo_path).await?;
    assert!(loaded_config.remotes.contains_key("origin"));

    let origin = loaded_config.remotes.get("origin").unwrap();
    assert_eq!(origin.url, "https://example.com/repo.git");

    Ok(())
}

#[tokio::test]
async fn test_add_multiple_remotes() -> Result<()> {
    let (_temp, repo_path) = create_test_repo()?;

    let mut config = Config::load(&repo_path).await?;

    // Add multiple remotes
    config.set_remote("origin", "https://example.com/repo.git");
    config.set_remote("upstream", "https://upstream.com/repo.git");
    config.set_remote("backup", "file:///backup/repo");
    config.save(&repo_path)?;

    // Verify all remotes
    let loaded_config = Config::load(&repo_path).await?;
    assert_eq!(loaded_config.remotes.len(), 3);
    assert!(loaded_config.remotes.contains_key("origin"));
    assert!(loaded_config.remotes.contains_key("upstream"));
    assert!(loaded_config.remotes.contains_key("backup"));

    Ok(())
}

#[tokio::test]
async fn test_remove_remote() -> Result<()> {
    let (_temp, repo_path) = create_test_repo()?;

    let mut config = Config::load(&repo_path).await?;

    // Add and then remove remote
    config.set_remote("origin", "https://example.com/repo.git");
    config.save(&repo_path)?;

    let mut loaded_config = Config::load(&repo_path).await?;
    assert!(loaded_config.remotes.contains_key("origin"));

    // Remove remote
    loaded_config.remove_remote("origin");
    loaded_config.save(&repo_path)?;

    // Verify removal
    let final_config = Config::load(&repo_path).await?;
    assert!(!final_config.remotes.contains_key("origin"));

    Ok(())
}

#[tokio::test]
async fn test_list_remotes() -> Result<()> {
    let (_temp, repo_path) = create_test_repo()?;

    let mut config = Config::load(&repo_path).await?;

    // Add remotes
    config.set_remote("origin", "https://example.com/repo.git");
    config.set_remote("upstream", "https://upstream.com/repo.git");
    config.save(&repo_path)?;

    // List remotes
    let loaded_config = Config::load(&repo_path).await?;
    let mut remote_names = loaded_config.list_remotes();
    remote_names.sort();

    assert_eq!(remote_names, vec!["origin", "upstream"]);

    Ok(())
}

#[tokio::test]
async fn test_rename_remote() -> Result<()> {
    let (_temp, repo_path) = create_test_repo()?;

    let mut config = Config::load(&repo_path).await?;

    // Add remote
    config.set_remote("old-name", "https://example.com/repo.git");
    config.save(&repo_path)?;

    // Rename remote
    let mut loaded_config = Config::load(&repo_path).await?;
    let old_remote = loaded_config.remove_remote("old-name").unwrap();
    loaded_config.remotes.insert("new-name".to_string(), old_remote);
    loaded_config.save(&repo_path)?;

    // Verify rename
    let final_config = Config::load(&repo_path).await?;
    assert!(!final_config.remotes.contains_key("old-name"));
    assert!(final_config.remotes.contains_key("new-name"));

    Ok(())
}

#[tokio::test]
async fn test_set_url() -> Result<()> {
    let (_temp, repo_path) = create_test_repo()?;

    let mut config = Config::load(&repo_path).await?;

    // Add remote
    config.set_remote("origin", "https://example.com/repo.git");
    config.save(&repo_path)?;

    // Change URL
    let mut loaded_config = Config::load(&repo_path).await?;
    let remote = loaded_config.remotes.get_mut("origin").unwrap();
    remote.url = "https://new-example.com/repo.git".to_string();
    loaded_config.save(&repo_path)?;

    // Verify URL change
    let final_config = Config::load(&repo_path).await?;
    let origin = final_config.remotes.get("origin").unwrap();
    assert_eq!(origin.url, "https://new-example.com/repo.git");

    Ok(())
}

#[tokio::test]
async fn test_separate_fetch_push_urls() -> Result<()> {
    let (_temp, repo_path) = create_test_repo()?;

    let mut config = Config::load(&repo_path).await?;

    // Create remote with separate fetch/push URLs
    let mut remote = mediagit_config::RemoteConfig::new("https://example.com/repo.git");
    remote.fetch = Some("https://fetch.example.com/repo.git".to_string());
    remote.push = Some("https://push.example.com/repo.git".to_string());

    config.remotes.insert("origin".to_string(), remote);
    config.save(&repo_path)?;

    // Verify separate URLs
    let loaded_config = Config::load(&repo_path).await?;
    let origin = loaded_config.remotes.get("origin").unwrap();

    assert_eq!(origin.url, "https://example.com/repo.git");
    assert_eq!(origin.fetch_url(), "https://fetch.example.com/repo.git");
    assert_eq!(origin.push_url(), "https://push.example.com/repo.git");

    Ok(())
}

#[tokio::test]
async fn test_default_fetch_push_urls() -> Result<()> {
    let (_temp, repo_path) = create_test_repo()?;

    let mut config = Config::load(&repo_path).await?;

    // Create remote without separate fetch/push URLs
    let remote = mediagit_config::RemoteConfig::new("https://example.com/repo.git");
    config.remotes.insert("origin".to_string(), remote);
    config.save(&repo_path)?;

    // Verify URLs default to main URL
    let loaded_config = Config::load(&repo_path).await?;
    let origin = loaded_config.remotes.get("origin").unwrap();

    assert_eq!(origin.fetch_url(), "https://example.com/repo.git");
    assert_eq!(origin.push_url(), "https://example.com/repo.git");

    Ok(())
}

#[tokio::test]
async fn test_get_remote_url() -> Result<()> {
    let (_temp, repo_path) = create_test_repo()?;

    let mut config = Config::load(&repo_path).await?;
    config.set_remote("origin", "https://example.com/repo.git");
    config.save(&repo_path)?;

    let loaded_config = Config::load(&repo_path).await?;
    let url = loaded_config.get_remote_url("origin");

    assert!(url.is_ok());
    assert_eq!(url.unwrap(), "https://example.com/repo.git");

    Ok(())
}

#[tokio::test]
async fn test_get_nonexistent_remote() -> Result<()> {
    let (_temp, repo_path) = create_test_repo()?;

    let config = Config::load(&repo_path).await?;
    let url = config.get_remote_url("nonexistent");

    assert!(url.is_err());
    assert!(url.unwrap_err().contains("not found"));

    Ok(())
}

#[tokio::test]
async fn test_remote_persistence() -> Result<()> {
    let (_temp, repo_path) = create_test_repo()?;

    // Add remotes in one session
    {
        let mut config = Config::load(&repo_path).await?;
        config.set_remote("origin", "https://example.com/repo.git");
        config.set_remote("upstream", "https://upstream.com/repo.git");
        config.save(&repo_path)?;
    }

    // Verify persistence in new session
    {
        let loaded_config = Config::load(&repo_path).await?;
        assert_eq!(loaded_config.remotes.len(), 2);
        assert!(loaded_config.remotes.contains_key("origin"));
        assert!(loaded_config.remotes.contains_key("upstream"));
    }

    Ok(())
}

#[tokio::test]
async fn test_config_format() -> Result<()> {
    let (_temp, repo_path) = create_test_repo()?;

    let mut config = Config::load(&repo_path).await?;

    // Add remote with all fields
    let mut remote = mediagit_config::RemoteConfig::new("https://example.com/repo.git");
    remote.fetch = Some("https://fetch.example.com/repo.git".to_string());
    remote.push = Some("https://push.example.com/repo.git".to_string());
    remote.default_fetch = Some(true);

    config.remotes.insert("origin".to_string(), remote);
    config.save(&repo_path)?;

    // Read config file directly
    let config_path = repo_path.join(".mediagit/config.toml");
    let content = fs::read_to_string(config_path)?;

    // Verify TOML format contains expected sections
    assert!(content.contains("[remotes.origin]"));
    assert!(content.contains("url = \"https://example.com/repo.git\""));
    assert!(content.contains("fetch = \"https://fetch.example.com/repo.git\""));
    assert!(content.contains("push = \"https://push.example.com/repo.git\""));

    Ok(())
}

// URL validation tests are in the remote.rs module itself
// These integration tests focus on the config persistence and API
