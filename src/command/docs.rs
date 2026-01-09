use anyhow::Result;

const README: &str = include_str!("../../README.md");

pub fn run() -> Result<()> {
    crate::markdown::display(README, README);
    Ok(())
}
