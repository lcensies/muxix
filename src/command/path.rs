use crate::git;
use anyhow::{Result, anyhow};

pub fn run(name: &str) -> Result<()> {
    // Smart resolution: try handle first, then branch name
    let (path, _branch) = git::find_worktree(name).map_err(|_| {
        anyhow!(
            "Worktree '{}' not found. Use 'workmux list' to see available worktrees.",
            name
        )
    })?;
    println!("{}", path.display());
    Ok(())
}
