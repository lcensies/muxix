use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
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

/// Aggregated status of PR checks
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum CheckState {
    /// All checks passed
    Success,
    /// Some checks failed (passed/total)
    Failure { passed: u32, total: u32 },
    /// Checks still running (passed/total)
    Pending { passed: u32, total: u32 },
}

/// Summary of a PR found by head ref search
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrSummary {
    pub number: u32,
    pub title: String,
    pub state: String,
    #[serde(rename = "isDraft")]
    pub is_draft: bool,
    /// Aggregated check status (None if no checks configured)
    #[serde(default)]
    pub checks: Option<CheckState>,
}

/// Handles both CheckRun (status/conclusion) and StatusContext (state) from GitHub API
#[derive(Debug, Deserialize)]
struct CheckRollupItem {
    #[serde(alias = "state")]
    status: Option<String>,
    conclusion: Option<String>,
}

/// Aggregate check results into a single CheckState
fn aggregate_checks(checks: &[CheckRollupItem]) -> Option<CheckState> {
    if checks.is_empty() {
        return None;
    }

    let mut passed = 0u32;
    let mut failed = 0u32;
    let mut pending = 0u32;
    let mut skipped = 0u32;

    for check in checks {
        let status = check.status.as_deref().unwrap_or("");
        let conclusion = check.conclusion.as_deref().unwrap_or("");

        match (status, conclusion) {
            // Success states
            (_, "SUCCESS") | ("SUCCESS", _) => passed += 1,
            // Failure states (expanded to catch all failure-like conclusions)
            (_, "FAILURE" | "CANCELLED" | "TIMED_OUT" | "STARTUP_FAILURE" | "ACTION_REQUIRED")
            | ("FAILURE" | "ERROR", _) => failed += 1,
            // Neutral/skipped - track but don't count toward active total
            (_, "NEUTRAL" | "SKIPPED") => skipped += 1,
            // Pending states (expanded)
            ("IN_PROGRESS" | "QUEUED" | "PENDING" | "REQUESTED" | "WAITING", _) => pending += 1,
            _ => {}
        }
    }

    let total = passed + failed + pending;

    // If no active checks but some were skipped, treat as success (GitHub behavior)
    if total == 0 {
        return if skipped > 0 {
            Some(CheckState::Success)
        } else {
            None
        };
    }

    Some(if failed > 0 {
        CheckState::Failure { passed, total }
    } else if pending > 0 {
        CheckState::Pending { passed, total }
    } else {
        CheckState::Success
    })
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
        checks: None,
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
    #[serde(rename = "statusCheckRollup", default)]
    status_check_rollup: Vec<CheckRollupItem>,
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
            "number,title,state,isDraft,headRefName,statusCheckRollup",
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
                    checks: aggregate_checks(&pr.status_check_rollup),
                },
            )
        })
        .collect();

    Ok(pr_map)
}

/// Fetch PR status for specific branches using a single GraphQL query.
/// Falls back to per-branch REST calls if GraphQL fails.
pub fn list_prs_for_branches(
    repo_root: &Path,
    branches: &[String],
) -> Result<HashMap<String, PrSummary>> {
    if branches.is_empty() {
        return Ok(HashMap::new());
    }

    match list_prs_for_branches_graphql(repo_root, branches) {
        Ok(map) => Ok(map),
        Err(e) => {
            debug!("github:graphql batch failed, falling back to per-branch REST: {e}");
            list_prs_for_branches_rest(repo_root, branches)
        }
    }
}

/// Sanitize a branch name into a valid GraphQL alias (alphanumeric + underscore).
fn branch_to_alias(index: usize, branch: &str) -> String {
    // Use a prefix + index to guarantee uniqueness, since sanitizing could create collisions
    let sanitized: String = branch
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("br{}_{}", index, sanitized)
}

/// Build a GraphQL query fragment for a single branch alias.
fn build_branch_fragment(alias: &str, branch: &str) -> String {
    // Escape any quotes in branch name for safety
    let escaped = branch.replace('\\', "\\\\").replace('"', "\\\"");
    format!(
        r#"    {alias}: pullRequests(headRefName: "{escaped}", first: 1, states: [OPEN, MERGED, CLOSED], orderBy: {{field: CREATED_AT, direction: DESC}}) {{
      nodes {{
        number title state isDraft headRefName
        commits(last: 1) {{ nodes {{ commit {{ statusCheckRollup {{ contexts(first: 100) {{
          nodes {{ __typename ... on CheckRun {{ name status conclusion }} ... on StatusContext {{ context state }} }}
        }} }} }} }} }}
      }}
    }}"#
    )
}

/// GraphQL response structures
#[derive(Debug, Deserialize)]
struct GraphqlResponse {
    data: Option<GraphqlData>,
    errors: Option<Vec<GraphqlError>>,
}

#[derive(Debug, Deserialize)]
struct GraphqlError {
    message: String,
}

/// The `data.repository` value is a map of alias -> PullRequestConnection
#[derive(Debug, Deserialize)]
struct GraphqlData {
    repository: HashMap<String, GraphqlPrConnection>,
}

#[derive(Debug, Deserialize)]
struct GraphqlPrConnection {
    nodes: Vec<GraphqlPrNode>,
}

#[derive(Debug, Deserialize)]
struct GraphqlPrNode {
    number: u32,
    title: String,
    state: String,
    #[serde(rename = "isDraft")]
    is_draft: bool,
    #[serde(rename = "headRefName")]
    head_ref_name: String,
    commits: GraphqlCommits,
}

#[derive(Debug, Deserialize)]
struct GraphqlCommits {
    nodes: Vec<GraphqlCommitNode>,
}

#[derive(Debug, Deserialize)]
struct GraphqlCommitNode {
    commit: GraphqlCommit,
}

#[derive(Debug, Deserialize)]
struct GraphqlCommit {
    #[serde(rename = "statusCheckRollup")]
    status_check_rollup: Option<GraphqlCheckRollup>,
}

#[derive(Debug, Deserialize)]
struct GraphqlCheckRollup {
    contexts: GraphqlCheckContexts,
}

#[derive(Debug, Deserialize)]
struct GraphqlCheckContexts {
    nodes: Vec<GraphqlCheckNode>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "__typename")]
enum GraphqlCheckNode {
    CheckRun {
        status: Option<String>,
        conclusion: Option<String>,
    },
    StatusContext {
        state: Option<String>,
    },
}

impl GraphqlCheckNode {
    fn to_rollup_item(&self) -> CheckRollupItem {
        match self {
            GraphqlCheckNode::CheckRun { status, conclusion } => CheckRollupItem {
                status: status.clone(),
                conclusion: conclusion.clone(),
            },
            GraphqlCheckNode::StatusContext { state } => CheckRollupItem {
                status: state.clone(),
                conclusion: None,
            },
        }
    }
}

/// Repository context resolved by `gh`, matching its own repo detection logic
/// (respects `gh repo set-default`, fork conventions, GHES hosts).
#[derive(Debug, Deserialize)]
struct RepoContext {
    name: String,
    owner: RepositoryOwner,
    url: String,
}

/// Get the repo owner, name, and API hostname using `gh repo view`.
/// This delegates repo resolution to `gh` so it works correctly with forks,
/// `gh repo set-default`, and GitHub Enterprise.
fn get_repo_context(repo_root: &Path) -> Result<(String, String, String)> {
    let output = Command::new("gh")
        .current_dir(repo_root)
        .args(["repo", "view", "--json", "owner,name,url"])
        .output()
        .context("Failed to run gh repo view")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("gh repo view failed: {stderr}"));
    }

    let ctx: RepoContext =
        serde_json::from_slice(&output.stdout).context("Failed to parse gh repo view output")?;

    // Extract hostname from the repo URL for GHES support
    let hostname = ctx
        .url
        .strip_prefix("https://")
        .or_else(|| ctx.url.strip_prefix("http://"))
        .and_then(|s| s.split('/').next())
        .unwrap_or("github.com")
        .to_string();

    Ok((ctx.owner.login, ctx.name, hostname))
}

/// Fetch PR status for multiple branches in a single GraphQL API call.
fn list_prs_for_branches_graphql(
    repo_root: &Path,
    branches: &[String],
) -> Result<HashMap<String, PrSummary>> {
    let (owner, repo_name, hostname) = get_repo_context(repo_root)?;

    // Build query fragments with one alias per branch
    let fragments: Vec<String> = branches
        .iter()
        .enumerate()
        .map(|(i, branch)| {
            let alias = branch_to_alias(i, branch);
            build_branch_fragment(&alias, branch)
        })
        .collect();

    // Use GraphQL variables for owner/name to avoid injection from crafted repo names
    let query = format!(
        "query($owner: String!, $name: String!) {{ repository(owner: $owner, name: $name) {{\n{}\n  }} }}",
        fragments.join("\n")
    );

    let body = serde_json::to_vec(&serde_json::json!({
        "query": query,
        "variables": {
            "owner": owner,
            "name": repo_name,
        }
    }))
    .context("JSON serialize")?;

    let mut child = Command::new("gh")
        .current_dir(repo_root)
        .args(["api", "graphql", "--hostname", &hostname, "--input", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to spawn gh api graphql")?;

    child
        .stdin
        .take()
        .expect("stdin was piped")
        .write_all(&body)
        .context("Failed to write to gh stdin")?;

    let output = child
        .wait_with_output()
        .context("Failed to wait for gh api graphql")?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(anyhow!("gh api graphql failed: {stderr}"));
    }

    let response: GraphqlResponse =
        serde_json::from_slice(&output.stdout).context("Failed to parse GraphQL response")?;

    if let Some(errors) = &response.errors
        && !errors.is_empty()
    {
        let msgs: Vec<&str> = errors.iter().map(|e| e.message.as_str()).collect();
        return Err(anyhow!("GraphQL errors: {}", msgs.join("; ")));
    }

    let data = response
        .data
        .ok_or_else(|| anyhow!("No data in GraphQL response"))?;
    let repo = data.repository;

    let mut map = HashMap::new();
    for (_alias, connection) in repo {
        for node in connection.nodes {
            let checks: Vec<CheckRollupItem> = node
                .commits
                .nodes
                .first()
                .and_then(|c| c.commit.status_check_rollup.as_ref())
                .map(|rollup| {
                    rollup
                        .contexts
                        .nodes
                        .iter()
                        .map(|n| n.to_rollup_item())
                        .collect()
                })
                .unwrap_or_default();

            map.insert(
                node.head_ref_name,
                PrSummary {
                    number: node.number,
                    title: node.title,
                    state: node.state,
                    is_draft: node.is_draft,
                    checks: aggregate_checks(&checks),
                },
            );
        }
    }

    Ok(map)
}

/// Fallback: fetch PR status one branch at a time using REST-style gh pr list.
fn list_prs_for_branches_rest(
    repo_root: &Path,
    branches: &[String],
) -> Result<HashMap<String, PrSummary>> {
    let mut map = HashMap::new();

    for branch in branches {
        let output = match Command::new("gh")
            .current_dir(repo_root)
            .args([
                "pr",
                "list",
                "--head",
                branch,
                "--state",
                "all",
                "--json",
                "number,title,state,isDraft,headRefName,statusCheckRollup",
                "--limit",
                "1",
            ])
            .output()
        {
            Ok(output) => output,
            Err(_) => continue,
        };

        if !output.status.success() {
            continue;
        }

        let prs: Vec<PrBatchItem> = match serde_json::from_slice(&output.stdout) {
            Ok(prs) => prs,
            Err(_) => continue,
        };

        if let Some(pr) = prs.into_iter().next() {
            map.insert(
                pr.head_ref_name,
                PrSummary {
                    number: pr.number,
                    title: pr.title,
                    state: pr.state,
                    is_draft: pr.is_draft,
                    checks: aggregate_checks(&pr.status_check_rollup),
                },
            );
        }
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    fn check_item(status: Option<&str>, conclusion: Option<&str>) -> CheckRollupItem {
        CheckRollupItem {
            status: status.map(String::from),
            conclusion: conclusion.map(String::from),
        }
    }

    #[test]
    fn aggregate_checks_empty() {
        assert_eq!(aggregate_checks(&[]), None);
    }

    #[test]
    fn aggregate_checks_all_success() {
        let checks = vec![
            check_item(Some("COMPLETED"), Some("SUCCESS")),
            check_item(Some("COMPLETED"), Some("SUCCESS")),
        ];
        assert_eq!(aggregate_checks(&checks), Some(CheckState::Success));
    }

    #[test]
    fn aggregate_checks_with_failure() {
        let checks = vec![
            check_item(Some("COMPLETED"), Some("SUCCESS")),
            check_item(Some("COMPLETED"), Some("FAILURE")),
        ];
        assert_eq!(
            aggregate_checks(&checks),
            Some(CheckState::Failure {
                passed: 1,
                total: 2
            })
        );
    }

    #[test]
    fn aggregate_checks_with_pending() {
        let checks = vec![
            check_item(Some("COMPLETED"), Some("SUCCESS")),
            check_item(Some("IN_PROGRESS"), None),
        ];
        assert_eq!(
            aggregate_checks(&checks),
            Some(CheckState::Pending {
                passed: 1,
                total: 2
            })
        );
    }

    #[test]
    fn aggregate_checks_failure_takes_priority_over_pending() {
        let checks = vec![
            check_item(Some("COMPLETED"), Some("SUCCESS")),
            check_item(Some("COMPLETED"), Some("FAILURE")),
            check_item(Some("IN_PROGRESS"), None),
        ];
        assert_eq!(
            aggregate_checks(&checks),
            Some(CheckState::Failure {
                passed: 1,
                total: 3
            })
        );
    }

    #[test]
    fn aggregate_checks_status_context_success() {
        // StatusContext uses "state" field (aliased to status) with values like SUCCESS
        let checks = vec![check_item(Some("SUCCESS"), None)];
        assert_eq!(aggregate_checks(&checks), Some(CheckState::Success));
    }

    #[test]
    fn aggregate_checks_status_context_pending() {
        let checks = vec![check_item(Some("PENDING"), None)];
        assert_eq!(
            aggregate_checks(&checks),
            Some(CheckState::Pending {
                passed: 0,
                total: 1
            })
        );
    }

    #[test]
    fn aggregate_checks_status_context_error() {
        let checks = vec![check_item(Some("ERROR"), None)];
        assert_eq!(
            aggregate_checks(&checks),
            Some(CheckState::Failure {
                passed: 0,
                total: 1
            })
        );
    }

    #[test]
    fn aggregate_checks_all_skipped_returns_success() {
        let checks = vec![
            check_item(Some("COMPLETED"), Some("SKIPPED")),
            check_item(Some("COMPLETED"), Some("NEUTRAL")),
        ];
        assert_eq!(aggregate_checks(&checks), Some(CheckState::Success));
    }

    #[test]
    fn aggregate_checks_skipped_not_counted_in_total() {
        let checks = vec![
            check_item(Some("COMPLETED"), Some("SUCCESS")),
            check_item(Some("COMPLETED"), Some("SKIPPED")),
            check_item(Some("IN_PROGRESS"), None),
        ];
        // Only SUCCESS and IN_PROGRESS count toward total (2), not SKIPPED
        assert_eq!(
            aggregate_checks(&checks),
            Some(CheckState::Pending {
                passed: 1,
                total: 2
            })
        );
    }

    #[test]
    fn aggregate_checks_cancelled_is_failure() {
        let checks = vec![check_item(Some("COMPLETED"), Some("CANCELLED"))];
        assert_eq!(
            aggregate_checks(&checks),
            Some(CheckState::Failure {
                passed: 0,
                total: 1
            })
        );
    }

    #[test]
    fn aggregate_checks_timed_out_is_failure() {
        let checks = vec![check_item(Some("COMPLETED"), Some("TIMED_OUT"))];
        assert_eq!(
            aggregate_checks(&checks),
            Some(CheckState::Failure {
                passed: 0,
                total: 1
            })
        );
    }

    #[test]
    fn aggregate_checks_mixed_check_types() {
        // Mix of CheckRun (status/conclusion) and StatusContext (state only)
        let checks = vec![
            check_item(Some("COMPLETED"), Some("SUCCESS")), // CheckRun success
            check_item(Some("IN_PROGRESS"), None),          // CheckRun pending
            check_item(Some("SUCCESS"), None),              // StatusContext success
        ];
        assert_eq!(
            aggregate_checks(&checks),
            Some(CheckState::Pending {
                passed: 2,
                total: 3
            })
        );
    }

    #[test]
    fn aggregate_checks_queued_is_pending() {
        let checks = vec![check_item(Some("QUEUED"), None)];
        assert_eq!(
            aggregate_checks(&checks),
            Some(CheckState::Pending {
                passed: 0,
                total: 1
            })
        );
    }

    #[test]
    fn aggregate_checks_waiting_is_pending() {
        let checks = vec![check_item(Some("WAITING"), None)];
        assert_eq!(
            aggregate_checks(&checks),
            Some(CheckState::Pending {
                passed: 0,
                total: 1
            })
        );
    }

    #[test]
    fn branch_to_alias_sanitizes_hyphens() {
        let alias = branch_to_alias(0, "my-feature-branch");
        assert_eq!(alias, "br0_my_feature_branch");
    }

    #[test]
    fn branch_to_alias_sanitizes_slashes() {
        let alias = branch_to_alias(3, "feat/add-thing");
        assert_eq!(alias, "br3_feat_add_thing");
    }

    #[test]
    fn branch_to_alias_index_prevents_collisions() {
        // "a-b" and "a_b" would collide without the index prefix
        let a1 = branch_to_alias(0, "a-b");
        let a2 = branch_to_alias(1, "a_b");
        assert_ne!(a1, a2);
    }

    #[test]
    fn graphql_check_node_to_rollup_item_check_run() {
        let node = GraphqlCheckNode::CheckRun {
            status: Some("COMPLETED".to_string()),
            conclusion: Some("SUCCESS".to_string()),
        };
        let item = node.to_rollup_item();
        assert_eq!(item.status.as_deref(), Some("COMPLETED"));
        assert_eq!(item.conclusion.as_deref(), Some("SUCCESS"));
    }

    #[test]
    fn graphql_check_node_to_rollup_item_status_context() {
        let node = GraphqlCheckNode::StatusContext {
            state: Some("PENDING".to_string()),
        };
        let item = node.to_rollup_item();
        assert_eq!(item.status.as_deref(), Some("PENDING"));
        assert_eq!(item.conclusion, None);
    }
}
