use crate::git;
use anyhow::{Context, Result, anyhow};

pub fn run(base: &str) -> Result<()> {
    if !git::branch_exists(base)? {
        return Err(anyhow!("Base reference '{}' does not exist", base));
    }

    let branch = git::get_current_branch().context("Failed to get current branch")?;

    if branch.is_empty() {
        return Err(anyhow!("Not on a branch (detached HEAD?)"));
    }

    if branch == base {
        return Err(anyhow!("Cannot set base branch to the current branch"));
    }

    git::set_branch_base(&branch, base)
        .with_context(|| format!("Failed to set base branch for '{}'", branch))?;

    println!("Set base branch for '{}' to '{}'", branch, base);
    Ok(())
}
