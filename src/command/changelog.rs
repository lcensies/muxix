use anyhow::Result;

const CHANGELOG: &str = include_str!("../../CHANGELOG.md");

pub fn run() -> Result<()> {
    crate::markdown::display(CHANGELOG, CHANGELOG);
    Ok(())
}
