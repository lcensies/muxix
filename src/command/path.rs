use crate::git;
use anyhow::{Context, Result};

pub fn run(name: &str) -> Result<()> {
    // Smart resolution: try handle first, then branch name
    let (path, _branch) = git::find_worktree(name).with_context(|| {
        format!(
            "No worktree found with name '{}'. Use 'workmux list' to see available worktrees.",
            name
        )
    })?;
    println!("{}", path.display());
    Ok(())
}
