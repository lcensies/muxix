use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::debug;

#[derive(Debug, Deserialize)]
pub struct PrDetails {
    #[serde(rename = "headRefName")]
    pub head_ref_name: String,
    #[serde(rename = "headRepositoryOwner")]
    pub head_repository_owner: RepositoryOwner,
    pub state: String,
    #[serde(rename = "isDraft")]
    pub is_draft: bool,
    pub title: String,
    pub author: Author,
}

#[derive(Debug, Deserialize)]
pub struct RepositoryOwner {
    pub login: String,
}

#[derive(Debug, Deserialize)]
pub struct Author {
    pub login: String,
}

impl PrDetails {
    pub fn is_fork(&self, current_repo_owner: &str) -> bool {
        self.head_repository_owner.login != current_repo_owner
    }
}

/// Summary of a PR found by head ref search
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrSummary {
    pub number: u32,
    pub title: String,
    pub state: String,
    #[serde(rename = "isDraft")]
    pub is_draft: bool,
}

/// Internal struct for parsing PR list results with owner info
#[derive(Debug, Deserialize)]
struct PrListResult {
    pub number: u32,
    pub title: String,
    pub state: String,
    #[serde(rename = "isDraft")]
    pub is_draft: bool,
    #[serde(rename = "headRepositoryOwner")]
    pub head_repository_owner: RepositoryOwner,
}

/// Find a PR by its head ref (e.g., "owner:branch" format).
/// Returns None if no PR is found, or the first matching PR if found.
pub fn find_pr_by_head_ref(owner: &str, branch: &str) -> Result<Option<PrSummary>> {
    // gh pr list --head only matches branch name, not owner:branch format
    // So we query by branch and filter by owner in the results
    let output = Command::new("gh")
        .args([
            "pr",
            "list",
            "--head",
            branch,
            "--state",
            "all", // Include closed/merged PRs
            "--json",
            "number,title,state,isDraft,headRepositoryOwner",
            "--limit",
            "50", // Get enough results to handle common branch names
        ])
        .output();

    let output = match output {
        Ok(out) => out,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            debug!("github:gh CLI not found, skipping PR lookup");
            return Ok(None);
        }
        Err(e) => {
            return Err(e).context("Failed to execute gh command");
        }
    };

    if !output.status.success() {
        debug!(
            owner = owner,
            branch = branch,
            "github:pr list failed, treating as no PR found"
        );
        return Ok(None);
    }

    let json_str = String::from_utf8(output.stdout).context("gh output is not valid UTF-8")?;

    // gh pr list returns an array
    let prs: Vec<PrListResult> =
        serde_json::from_str(&json_str).context("Failed to parse gh JSON output")?;

    // Find the PR from the specified owner (case-insensitive, as GitHub usernames are case-insensitive)
    let matching_pr = prs
        .into_iter()
        .find(|pr| pr.head_repository_owner.login.eq_ignore_ascii_case(owner));

    Ok(matching_pr.map(|pr| PrSummary {
        number: pr.number,
        title: pr.title,
        state: pr.state,
        is_draft: pr.is_draft,
    }))
}

/// Fetches pull request details using the GitHub CLI
pub fn get_pr_details(pr_number: u32) -> Result<PrDetails> {
    // Fetch PR details using gh CLI
    // Note: We don't pre-check with 'which' because it doesn't respect test PATH modifications
    let output = Command::new("gh")
        .args([
            "pr",
            "view",
            &pr_number.to_string(),
            "--json",
            "headRefName,headRepositoryOwner,state,isDraft,title,author",
        ])
        .output();

    let output = match output {
        Ok(out) => out,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            debug!("github:gh CLI not found");
            return Err(anyhow!(
                "GitHub CLI (gh) is required for --pr. Install from https://cli.github.com"
            ));
        }
        Err(e) => {
            return Err(e).context("Failed to execute gh command");
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        debug!(pr = pr_number, stderr = %stderr, "github:pr view failed");
        return Err(anyhow!(
            "Failed to fetch PR #{}: {}",
            pr_number,
            stderr.trim()
        ));
    }

    let json_str = String::from_utf8(output.stdout).context("gh output is not valid UTF-8")?;

    let pr_details: PrDetails =
        serde_json::from_str(&json_str).context("Failed to parse gh JSON output")?;

    Ok(pr_details)
}

/// Internal struct for parsing batch PR list results
#[derive(Debug, Deserialize)]
struct PrBatchItem {
    number: u32,
    title: String,
    state: String,
    #[serde(rename = "isDraft")]
    is_draft: bool,
    #[serde(rename = "headRefName")]
    head_ref_name: String,
}

/// Fetch all PRs for the current repository.
pub fn list_prs() -> Result<HashMap<String, PrSummary>> {
    let output = Command::new("gh")
        .args([
            "pr",
            "list",
            "--state",
            "all",
            "--json",
            "number,title,state,isDraft,headRefName",
            "--limit",
            "200",
        ])
        .output();

    let output = match output {
        Ok(out) => out,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            debug!("github:gh CLI not found, skipping PR lookup");
            return Ok(HashMap::new());
        }
        Err(e) => {
            return Err(e).context("Failed to execute gh command");
        }
    };

    if !output.status.success() {
        debug!("github:pr list batch failed, treating as no PRs found");
        return Ok(HashMap::new());
    }

    let json_str = String::from_utf8(output.stdout).context("gh output is not valid UTF-8")?;

    let prs: Vec<PrBatchItem> =
        serde_json::from_str(&json_str).context("Failed to parse gh JSON output")?;

    let pr_map = prs
        .into_iter()
        .map(|pr| {
            (
                pr.head_ref_name,
                PrSummary {
                    number: pr.number,
                    title: pr.title,
                    state: pr.state,
                    is_draft: pr.is_draft,
                },
            )
        })
        .collect();

    Ok(pr_map)
}

/// List PRs for a specific repository
pub fn list_prs_in_repo(repo_root: &Path) -> Result<HashMap<String, PrSummary>> {
    let output = match Command::new("gh")
        .current_dir(repo_root)
        .args([
            "pr",
            "list",
            "--state",
            "all",
            "--json",
            "number,title,state,isDraft,headRefName",
            "--limit",
            "200",
        ])
        .output()
    {
        Ok(output) => output,
        Err(e) => {
            tracing::warn!("Failed to run gh pr list: {}", e);
            return Ok(HashMap::new());
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::warn!("gh pr list failed: {}", stderr);
        return Ok(HashMap::new());
    }

    let prs: Vec<PrBatchItem> = serde_json::from_slice(&output.stdout)?;

    let map = prs
        .into_iter()
        .map(|pr| {
            (
                pr.head_ref_name,
                PrSummary {
                    number: pr.number,
                    title: pr.title,
                    state: pr.state,
                    is_draft: pr.is_draft,
                },
            )
        })
        .collect();

    Ok(map)
}

/// Get the path to the PR status cache file
fn get_pr_cache_path() -> Result<PathBuf> {
    let home = home::home_dir().ok_or_else(|| anyhow!("Could not find home directory"))?;
    let cache_dir = home.join(".cache").join("workmux");
    std::fs::create_dir_all(&cache_dir)?;
    Ok(cache_dir.join("pr_status_cache.json"))
}

/// Load the PR status cache from disk
pub fn load_pr_cache() -> HashMap<PathBuf, HashMap<String, PrSummary>> {
    if let Ok(path) = get_pr_cache_path()
        && path.exists()
        && let Ok(content) = std::fs::read_to_string(&path)
    {
        return serde_json::from_str(&content).unwrap_or_default();
    }
    HashMap::new()
}

/// Save the PR status cache to disk
pub fn save_pr_cache(statuses: &HashMap<PathBuf, HashMap<String, PrSummary>>) {
    if let Ok(path) = get_pr_cache_path()
        && let Ok(content) = serde_json::to_string(statuses)
    {
        let _ = std::fs::write(path, content);
    }
}
