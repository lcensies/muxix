use crate::multiplexer::{create_backend, detect_backend};
use crate::{config, nerdfont, workflow};
use anyhow::Result;
use pathdiff::diff_paths;
use tabled::{
    Table, Tabled,
    settings::{Padding, Style, disable::Remove, object::Columns},
};

#[derive(Tabled)]
struct WorktreeRow {
    #[tabled(rename = "BRANCH")]
    branch: String,
    #[tabled(rename = "PR")]
    pr_status: String,
    #[tabled(rename = "MUX")]
    mux_status: String,
    #[tabled(rename = "UNMERGED")]
    unmerged_status: String,
    #[tabled(rename = "PATH")]
    path_str: String,
}

fn format_pr_status(pr_info: Option<crate::github::PrSummary>) -> String {
    pr_info
        .map(|pr| {
            let icons = nerdfont::pr_icons();
            // GitHub-style colors: green for open, gray for draft, purple for merged, red for closed
            let (icon, color) = match pr.state.as_str() {
                "OPEN" if pr.is_draft => (icons.draft, "\x1b[90m"), // gray
                "OPEN" => (icons.open, "\x1b[32m"),                 // green
                "MERGED" => (icons.merged, "\x1b[35m"),             // purple/magenta
                "CLOSED" => (icons.closed, "\x1b[31m"),             // red
                _ => (icons.open, "\x1b[32m"),
            };
            format!("#{} {}{}\x1b[0m", pr.number, color, icon)
        })
        .unwrap_or_else(|| "-".to_string())
}

pub fn run(show_pr: bool) -> Result<()> {
    let config = config::Config::load(None)?;
    let mux = create_backend(detect_backend());
    let worktrees = workflow::list(&config, mux.as_ref(), show_pr)?;

    if worktrees.is_empty() {
        println!("No worktrees found");
        return Ok(());
    }

    let current_dir = std::env::current_dir()?;

    let display_data: Vec<WorktreeRow> = worktrees
        .into_iter()
        .map(|wt| {
            let path_str = diff_paths(&wt.path, &current_dir)
                .map(|p| {
                    let s = p.display().to_string();
                    if s.is_empty() || s == "." {
                        "(here)".to_string()
                    } else {
                        s
                    }
                })
                .unwrap_or_else(|| wt.path.display().to_string());

            WorktreeRow {
                branch: wt.branch,
                pr_status: format_pr_status(wt.pr_info),
                mux_status: if wt.has_mux_window {
                    "✓".to_string()
                } else {
                    "-".to_string()
                },
                unmerged_status: if wt.has_unmerged {
                    "●".to_string()
                } else {
                    "-".to_string()
                },
                path_str,
            }
        })
        .collect();

    let mut table = Table::new(display_data);
    table
        .with(Style::blank())
        .modify(Columns::new(0..5), Padding::new(0, 1, 0, 0));

    // Hide PR column if --pr flag not used (column 1)
    if !show_pr {
        table.with(Remove::column(Columns::new(1..2)));
    }

    println!("{table}");

    Ok(())
}
