use anyhow::{Context, Result, anyhow};
use git_url_parse::GitUrl;
use git_url_parse::types::provider::GenericProvider;
use tracing::info;

use crate::cmd::Cmd;

/// Return a list of configured git remotes
pub fn list_remotes() -> Result<Vec<String>> {
    let output = Cmd::new("git")
        .arg("remote")
        .run_and_capture_stdout()
        .context("Failed to list git remotes")?;

    Ok(output
        .lines()
        .map(|line| line.trim())
        .filter(|line| !line.is_empty())
        .map(|line| line.to_string())
        .collect())
}

/// Check if a remote exists
pub fn remote_exists(remote: &str) -> Result<bool> {
    Ok(list_remotes()?.into_iter().any(|name| name == remote))
}

/// Fetch updates from the given remote
pub fn fetch_remote(remote: &str) -> Result<()> {
    Cmd::new("git")
        .args(&["fetch", remote])
        .run()
        .with_context(|| format!("Failed to fetch from remote '{}'", remote))?;
    Ok(())
}

/// Fetch from remote with prune to update remote-tracking refs
pub fn fetch_prune() -> Result<()> {
    Cmd::new("git")
        .args(&["fetch", "--prune"])
        .run()
        .context("Failed to fetch with prune")?;
    Ok(())
}

/// Add a git remote if it doesn't exist
pub fn add_remote(name: &str, url: &str) -> Result<()> {
    Cmd::new("git")
        .args(&["remote", "add", name, url])
        .run()
        .with_context(|| format!("Failed to add remote '{}' with URL '{}'", name, url))?;
    Ok(())
}

/// Set the URL for an existing git remote
pub fn set_remote_url(name: &str, url: &str) -> Result<()> {
    Cmd::new("git")
        .args(&["remote", "set-url", name, url])
        .run()
        .with_context(|| format!("Failed to set URL for remote '{}' to '{}'", name, url))?;
    Ok(())
}

/// Get the remote URL for a given remote name
/// Note: Returns the configured URL, not the resolved URL after insteadOf substitution
pub fn get_remote_url(remote: &str) -> Result<String> {
    // Use git config to get the raw URL, not the insteadOf-resolved one
    // git remote get-url resolves insteadOf, which breaks our owner parsing in tests
    Cmd::new("git")
        .args(&["config", "--get", &format!("remote.{}.url", remote)])
        .run_and_capture_stdout()
        .with_context(|| format!("Failed to get URL for remote '{}'", remote))
}

/// Ensure a remote exists for a specific fork owner.
/// Returns the name of the remote (e.g., "origin" or "fork-username").
/// If the remote needs to be created, it constructs the URL based on the origin URL's scheme.
pub fn ensure_fork_remote(fork_owner: &str) -> Result<String> {
    // If the fork owner is the same as the origin owner, just use origin
    let current_owner = get_repo_owner().unwrap_or_default();
    if !current_owner.is_empty() && fork_owner == current_owner {
        return Ok("origin".to_string());
    }

    let remote_name = format!("fork-{}", fork_owner);

    // Construct fork URL based on origin URL format, preserving host and protocol
    let origin_url = get_remote_url("origin")?;
    let parsed_url = GitUrl::parse(&origin_url).with_context(|| {
        format!(
            "Failed to parse origin URL for fork remote construction: {}",
            origin_url
        )
    })?;

    let host = parsed_url.host().unwrap_or("github.com");
    let scheme = parsed_url.scheme().unwrap_or("ssh");

    let provider: GenericProvider = parsed_url
        .provider_info()
        .with_context(|| "Failed to extract provider info from origin URL")?;
    let repo_name = provider.repo();

    let fork_url = match scheme {
        "https" => format!("https://{}/{}/{}.git", host, fork_owner, repo_name),
        "http" => format!("http://{}/{}/{}.git", host, fork_owner, repo_name),
        _ => {
            // SSH or other schemes
            format!("git@{}:{}/{}.git", host, fork_owner, repo_name)
        }
    };

    // Check if remote exists and update URL if needed
    if remote_exists(&remote_name)? {
        let current_url = get_remote_url(&remote_name)?;
        if current_url != fork_url {
            info!(remote = %remote_name, url = %fork_url, "git:updating fork remote URL");
            set_remote_url(&remote_name, &fork_url)
                .with_context(|| format!("Failed to update remote for fork '{}'", fork_owner))?;
        }
    } else {
        info!(remote = %remote_name, url = %fork_url, "git:adding fork remote");
        add_remote(&remote_name, &fork_url)
            .with_context(|| format!("Failed to add remote for fork '{}'", fork_owner))?;
    }

    Ok(remote_name)
}

/// Parse the repository owner from a git remote URL
/// Supports both HTTPS and SSH formats for github.com and GitHub Enterprise domains
fn parse_owner_from_git_url(url: &str) -> Option<&str> {
    if let Some(https_part) = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
    {
        // HTTPS format: https://github.com/owner/repo.git or https://github.enterprise.com/owner/repo.git
        https_part.split('/').nth(1)
    } else if url.starts_with("git@") {
        // SSH format: git@github.com:owner/repo.git or git@github.enterprise.com:owner/repo.git
        url.split(':')
            .nth(1)
            .and_then(|path| path.split('/').next())
    } else {
        None
    }
}

/// Get the repository owner from the origin remote URL
pub fn get_repo_owner() -> Result<String> {
    let url = get_remote_url("origin")?;

    parse_owner_from_git_url(&url)
        .ok_or_else(|| anyhow!("Could not parse repository owner from origin URL: {}", url))
        .map(|s| s.to_string())
}

#[cfg(test)]
mod tests {
    use super::parse_owner_from_git_url;

    #[test]
    fn test_parse_repo_owner_https_github_com() {
        assert_eq!(
            parse_owner_from_git_url("https://github.com/owner/repo.git"),
            Some("owner")
        );
    }

    #[test]
    fn test_parse_repo_owner_https_github_com_no_git_suffix() {
        assert_eq!(
            parse_owner_from_git_url("https://github.com/owner/repo"),
            Some("owner")
        );
    }

    #[test]
    fn test_parse_repo_owner_http_github_com() {
        assert_eq!(
            parse_owner_from_git_url("http://github.com/owner/repo.git"),
            Some("owner")
        );
    }

    #[test]
    fn test_parse_repo_owner_ssh_github_com() {
        assert_eq!(
            parse_owner_from_git_url("git@github.com:owner/repo.git"),
            Some("owner")
        );
    }

    #[test]
    fn test_parse_repo_owner_ssh_github_com_no_git_suffix() {
        assert_eq!(
            parse_owner_from_git_url("git@github.com:owner/repo"),
            Some("owner")
        );
    }

    #[test]
    fn test_parse_repo_owner_https_github_enterprise() {
        assert_eq!(
            parse_owner_from_git_url("https://github.enterprise.com/owner/repo.git"),
            Some("owner")
        );
    }

    #[test]
    fn test_parse_repo_owner_ssh_github_enterprise() {
        assert_eq!(
            parse_owner_from_git_url("git@github.enterprise.net:org/project.git"),
            Some("org")
        );
    }

    #[test]
    fn test_parse_repo_owner_https_github_enterprise_subdomain() {
        assert_eq!(
            parse_owner_from_git_url("https://github.company.internal/team/project.git"),
            Some("team")
        );
    }

    #[test]
    fn test_parse_repo_owner_with_nested_path() {
        assert_eq!(
            parse_owner_from_git_url("https://github.com/owner/repo/subpath"),
            Some("owner")
        );
    }

    #[test]
    fn test_parse_repo_owner_ssh_with_nested_path() {
        assert_eq!(
            parse_owner_from_git_url("git@github.com:owner/repo/subpath"),
            Some("owner")
        );
    }

    #[test]
    fn test_parse_repo_owner_invalid_format() {
        assert_eq!(parse_owner_from_git_url("not-a-valid-url"), None);
    }

    #[test]
    fn test_parse_repo_owner_local_path() {
        assert_eq!(parse_owner_from_git_url("/local/path/to/repo"), None);
    }

    #[test]
    fn test_parse_repo_owner_file_protocol() {
        assert_eq!(parse_owner_from_git_url("file:///local/path/to/repo"), None);
    }
}
