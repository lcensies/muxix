use crate::git;
use anyhow::Result;

pub fn run(branch_name: &str) -> Result<()> {
    let path = git::get_worktree_path(branch_name)?;
    println!("{}", path.display());
    Ok(())
}
