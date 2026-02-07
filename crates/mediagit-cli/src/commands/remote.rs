use anyhow::Result;
use clap::{Parser, Subcommand};
use console::style;
use mediagit_config::Config;
use super::super::repo::find_repo_root;

/// Manage remote repositories
#[derive(Parser, Debug)]
pub struct RemoteCmd {
    #[command(subcommand)]
    pub command: RemoteSubcommand,
}

#[derive(Subcommand, Debug)]
pub enum RemoteSubcommand {
    /// Add a new remote repository
    Add {
        /// Remote name (e.g., origin, upstream)
        #[arg(value_name = "NAME")]
        name: String,

        /// Remote URL (http://, https://, file://, ssh://)
        #[arg(value_name = "URL")]
        url: String,

        /// Fetch immediately after adding
        #[arg(short, long)]
        fetch: bool,
    },

    /// Remove a remote repository
    Remove {
        /// Remote name to remove
        #[arg(value_name = "NAME")]
        name: String,
    },

    /// List remote repositories
    List {
        /// Show URLs (verbose mode)
        #[arg(short, long)]
        verbose: bool,
    },

    /// Rename a remote
    Rename {
        /// Current remote name
        #[arg(value_name = "OLD_NAME")]
        old_name: String,

        /// New remote name
        #[arg(value_name = "NEW_NAME")]
        new_name: String,
    },

    /// Show detailed information about a remote
    Show {
        /// Remote name
        #[arg(value_name = "NAME")]
        name: String,
    },

    /// Set URL for a remote
    SetUrl {
        /// Remote name
        #[arg(value_name = "NAME")]
        name: String,

        /// New URL
        #[arg(value_name = "URL")]
        url: String,

        /// Set push URL instead of fetch URL
        #[arg(long)]
        push: bool,
    },
}

impl RemoteCmd {
    pub async fn execute(&self) -> Result<()> {
        match &self.command {
            RemoteSubcommand::Add { name, url, fetch } => {
                self.add_remote(name, url, *fetch).await
            }
            RemoteSubcommand::Remove { name } => self.remove_remote(name).await,
            RemoteSubcommand::List { verbose } => self.list_remotes(*verbose).await,
            RemoteSubcommand::Rename { old_name, new_name } => {
                self.rename_remote(old_name, new_name).await
            }
            RemoteSubcommand::Show { name } => self.show_remote(name).await,
            RemoteSubcommand::SetUrl { name, url, push } => {
                self.set_url(name, url, *push).await
            }
        }
    }

    async fn add_remote(&self, name: &str, url: &str, fetch: bool) -> Result<()> {
        let repo_root = find_repo_root()?;

        // Validate remote name
        if name.is_empty() {
            anyhow::bail!("Remote name cannot be empty");
        }

        // Validate URL
        validate_url(url)?;

        // Load existing config
        let mut config = Config::load(&repo_root).await?;

        // Check if remote already exists
        if config.remotes.contains_key(name) {
            anyhow::bail!("Remote '{}' already exists", name);
        }

        // Add remote with both fetch and push URLs
        let mut remote_config = mediagit_config::RemoteConfig::new(url);
        remote_config.fetch = Some(url.to_string());
        remote_config.push = Some(url.to_string());

        config.remotes.insert(name.to_string(), remote_config);

        // Save config
        config.save(&repo_root)?;

        println!(
            "{} Added remote '{}' → {}",
            style("✓").green(),
            style(name).yellow(),
            style(url).cyan()
        );

        if fetch {
            println!(
                "{} Fetching from '{}'...",
                style("→").cyan(),
                style(name).yellow()
            );
            // Future enhancement: implement standalone fetch (pull command already works)
            println!(
                "{} Standalone fetch coming soon - use 'pull' for now",
                style("ℹ").blue()
            );
        }

        Ok(())
    }

    async fn remove_remote(&self, name: &str) -> Result<()> {
        let repo_root = find_repo_root()?;

        // Load existing config
        let mut config = Config::load(&repo_root).await?;

        // Check if remote exists
        if !config.remotes.contains_key(name) {
            anyhow::bail!("No such remote: '{}'", name);
        }

        // Remove remote
        config.remove_remote(name);

        // Save config
        config.save(&repo_root)?;

        println!(
            "{} Removed remote '{}'",
            style("✓").green(),
            style(name).yellow()
        );

        Ok(())
    }

    async fn list_remotes(&self, verbose: bool) -> Result<()> {
        let repo_root = find_repo_root()?;
        let config = Config::load(&repo_root).await?;

        if config.remotes.is_empty() {
            if verbose {
                println!("No remotes configured");
            }
            return Ok(());
        }

        let mut names: Vec<_> = config.remotes.keys().collect();
        names.sort();

        for name in names {
            if let Some(remote) = config.remotes.get(name) {
                if verbose {
                    println!("{}", style(name).yellow().bold());

                    // Show fetch URL
                    if let Some(fetch_url) = &remote.fetch {
                        println!(
                            "  Fetch URL: {}",
                            style(fetch_url).cyan()
                        );
                    } else {
                        println!(
                            "  Fetch URL: {}",
                            style(&remote.url).cyan()
                        );
                    }

                    // Show push URL
                    if let Some(push_url) = &remote.push {
                        println!(
                            "  Push URL:  {}",
                            style(push_url).cyan()
                        );
                    } else {
                        println!(
                            "  Push URL:  {}",
                            style(&remote.url).cyan()
                        );
                    }
                } else {
                    println!("{}", name);
                }
            }
        }

        Ok(())
    }

    async fn rename_remote(&self, old_name: &str, new_name: &str) -> Result<()> {
        let repo_root = find_repo_root()?;

        // Validate new name
        if new_name.is_empty() {
            anyhow::bail!("Remote name cannot be empty");
        }

        // Load existing config
        let mut config = Config::load(&repo_root).await?;

        // Check if old remote exists
        if !config.remotes.contains_key(old_name) {
            anyhow::bail!("No such remote: '{}'", old_name);
        }

        // Check if new name already exists
        if config.remotes.contains_key(new_name) {
            anyhow::bail!("Remote '{}' already exists", new_name);
        }

        // Rename by removing and re-adding
        if let Some(remote_config) = config.remotes.remove(old_name) {
            config.remotes.insert(new_name.to_string(), remote_config);
        }

        // Save config
        config.save(&repo_root)?;

        println!(
            "{} Renamed remote '{}' → '{}'",
            style("✓").green(),
            style(old_name).yellow(),
            style(new_name).yellow()
        );

        Ok(())
    }

    async fn show_remote(&self, name: &str) -> Result<()> {
        let repo_root = find_repo_root()?;
        let config = Config::load(&repo_root).await?;

        // Get remote
        let remote = config
            .remotes
            .get(name)
            .ok_or_else(|| anyhow::anyhow!("No such remote: '{}'", name))?;

        // Display remote information
        println!("{}", style(format!("* remote {}", name)).yellow().bold());

        let fetch_url = remote.fetch.as_ref().unwrap_or(&remote.url);
        println!(
            "  Fetch URL: {}",
            style(fetch_url).cyan()
        );

        let push_url = remote.push.as_ref().unwrap_or(&remote.url);
        println!(
            "  Push URL:  {}",
            style(push_url).cyan()
        );

        // Try to fetch remote refs to show HEAD and branches
        let remote_url = remote.url.clone();
        match self.fetch_remote_info(&remote_url).await {
            Ok((head_branch, branches)) => {
                // Show HEAD branch
                if let Some(head) = head_branch {
                    println!("  HEAD branch: {}", style(&head).green());
                } else {
                    println!("  HEAD branch: {}", style("(unknown)").dim());
                }

                // Show remote branches
                println!("  Remote branches:");
                if branches.is_empty() {
                    println!("    {}", style("(none)").dim());
                } else {
                    for branch in branches {
                        let short_name = branch
                            .strip_prefix("refs/heads/")
                            .unwrap_or(&branch);
                        println!("    {}", style(short_name).cyan());
                    }
                }
            }
            Err(e) => {
                // Graceful degradation - show what we can
                println!("  HEAD branch: {}", style("(could not connect)").dim());
                println!("  Remote branches:");
                println!("    {} {}", style("(could not fetch:").dim(), style(e.to_string()).dim());

                // Try to show locally cached remote tracking branches
                let storage_path = repo_root.join(".mediagit");
                let remotes_dir = storage_path.join("refs").join("remotes").join(name);
                if remotes_dir.exists() {
                    println!("  Locally tracked branches:");
                    if let Ok(entries) = std::fs::read_dir(&remotes_dir) {
                        for entry in entries.flatten() {
                            if let Some(name) = entry.file_name().to_str() {
                                println!("    {}", style(name).cyan());
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Fetch remote info (HEAD branch and list of branches)
    async fn fetch_remote_info(&self, remote_url: &str) -> Result<(Option<String>, Vec<String>)> {
        let client = mediagit_protocol::ProtocolClient::new(remote_url.to_string());
        let refs = client.get_refs().await?;

        // Find HEAD
        let head_ref = refs.refs.iter().find(|r| r.name == "HEAD");
        let head_branch = if let Some(head) = head_ref {
            // Try to find which branch HEAD points to by matching OID
            refs.refs.iter()
                .find(|r| r.name.starts_with("refs/heads/") && r.oid == head.oid)
                .map(|r| r.name.strip_prefix("refs/heads/").unwrap_or(&r.name).to_string())
        } else {
            None
        };

        // Collect branches
        let branches: Vec<String> = refs.refs
            .iter()
            .filter(|r| r.name.starts_with("refs/heads/"))
            .map(|r| r.name.clone())
            .collect();

        Ok((head_branch, branches))
    }

    async fn set_url(&self, name: &str, url: &str, push: bool) -> Result<()> {
        let repo_root = find_repo_root()?;

        // Validate URL
        validate_url(url)?;

        // Load existing config
        let mut config = Config::load(&repo_root).await?;

        // Get remote
        let remote = config
            .remotes
            .get_mut(name)
            .ok_or_else(|| anyhow::anyhow!("No such remote: '{}'", name))?;

        // Update URL
        if push {
            remote.push = Some(url.to_string());
            println!(
                "{} Changed push URL for '{}' → {}",
                style("✓").green(),
                style(name).yellow(),
                style(url).cyan()
            );
        } else {
            remote.fetch = Some(url.to_string());
            remote.url = url.to_string(); // Also update main URL
            println!(
                "{} Changed fetch URL for '{}' → {}",
                style("✓").green(),
                style(name).yellow(),
                style(url).cyan()
            );
        }

        // Save config
        config.save(&repo_root)?;

        Ok(())
    }

}

/// Validate remote URL format
pub fn validate_url(url: &str) -> Result<()> {
    // Check if URL is empty
    if url.trim().is_empty() {
        anyhow::bail!("Remote URL cannot be empty");
    }

    // Supported protocols
    let valid_protocols = ["http://", "https://", "file://", "ssh://", "git://"];

    // Check if URL starts with a valid protocol
    let has_valid_protocol = valid_protocols.iter().any(|p| url.starts_with(p));

    if !has_valid_protocol {
        anyhow::bail!(
            "Invalid URL protocol. Supported: {}",
            valid_protocols.join(", ")
        );
    }

    // Additional validation for HTTP/HTTPS URLs
    if url.starts_with("http://") || url.starts_with("https://") {
        // Basic URL structure validation
        if !url.contains("://") {
            anyhow::bail!("Invalid URL format");
        }

        // Check for at least a host after protocol
        let parts: Vec<&str> = url.splitn(2, "://").collect();
        if parts.len() != 2 || parts[1].is_empty() {
            anyhow::bail!("URL must include a host");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_url_valid_http() {
        assert!(validate_url("http://example.com/repo").is_ok());
        assert!(validate_url("https://example.com/repo").is_ok());
    }

    #[test]
    fn test_validate_url_valid_protocols() {
        assert!(validate_url("file:///path/to/repo").is_ok());
        assert!(validate_url("ssh://user@host/repo").is_ok());
        assert!(validate_url("git://host/repo").is_ok());
    }

    #[test]
    fn test_validate_url_invalid() {
        assert!(validate_url("").is_err());
        assert!(validate_url("   ").is_err());
        assert!(validate_url("ftp://example.com/repo").is_err());
        assert!(validate_url("http://").is_err());
        assert!(validate_url("invalid-url").is_err());
    }

    #[test]
    fn test_validate_url_no_host() {
        assert!(validate_url("http://").is_err());
        assert!(validate_url("https://").is_err());
    }
}
