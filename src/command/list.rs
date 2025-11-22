use crate::{config, workflow};
use anyhow::Result;
use tabled::{
    Table, Tabled,
    settings::{Padding, Style, object::Columns},
};

#[derive(Tabled)]
struct WorktreeRow {
    #[tabled(rename = "BRANCH")]
    branch: String,
    #[tabled(rename = "TMUX")]
    tmux_status: String,
    #[tabled(rename = "UNMERGED")]
    unmerged_status: String,
    #[tabled(rename = "PATH")]
    path_str: String,
}

pub fn run() -> Result<()> {
    let config = config::Config::load(None)?;
    let worktrees = workflow::list(&config)?;

    if worktrees.is_empty() {
        println!("No worktrees found");
        return Ok(());
    }

    let display_data: Vec<WorktreeRow> = worktrees
        .into_iter()
        .map(|wt| WorktreeRow {
            branch: wt.branch,
            path_str: wt.path.display().to_string(),
            tmux_status: if wt.has_tmux {
                "✓".to_string()
            } else {
                "-".to_string()
            },
            unmerged_status: if wt.has_unmerged {
                "●".to_string()
            } else {
                "-".to_string()
            },
        })
        .collect();

    let mut table = Table::new(display_data);
    table
        .with(Style::blank())
        .modify(Columns::new(0..3), Padding::new(0, 5, 0, 0));

    println!("{table}");

    Ok(())
}
