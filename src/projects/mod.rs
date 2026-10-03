//! Cross-project tracking and launching (`workmux project ...`, `workmux start`).
//!
//! Named `projects` (plural) to stay clear of `project_state`, the per-project
//! journal in `.workmux/state`.

pub mod registry;
pub mod start;
pub mod sync;

use std::path::Path;

use anyhow::Result;

use registry::{AddOutcome, Registry};

pub fn cli_add(dir: &Path) -> Result<()> {
    let mut reg = Registry::load()?;
    match reg.add(dir)? {
        AddOutcome::Added(e) => println!("✓ Tracking project '{}' ({})", e.name, e.root.display()),
        AddOutcome::AlreadyTracked(e) => {
            println!("Project '{}' already tracked ({})", e.name, e.root.display())
        }
    }
    Ok(())
}

pub fn cli_rm(target: &str) -> Result<()> {
    let mut reg = Registry::load()?;
    match reg.remove(target)? {
        Some(e) => println!("✓ Untracked project '{}' ({})", e.name, e.root.display()),
        None => println!("No tracked project matches '{}'", target),
    }
    Ok(())
}

pub fn cli_list() -> Result<()> {
    let reg = Registry::load()?;
    if reg.projects.is_empty() {
        println!("No tracked projects. Add one with 'workmux project add <dir>'.");
        return Ok(());
    }
    for p in &reg.projects {
        println!("{}\t{}", p.name, p.root.display());
    }
    Ok(())
}
