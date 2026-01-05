use anyhow::Result;
use std::io::{IsTerminal, Write};
use std::process::{Command, Stdio};

/// The README.md content embedded at compile time
const README: &str = include_str!("../../README.md");

pub fn run() -> Result<()> {
    // If stdout is not a terminal (e.g., piped), print directly
    if !std::io::stdout().is_terminal() {
        print!("{README}");
        return Ok(());
    }

    // Try to use a pager for better UX
    let pager = std::env::var("PAGER").unwrap_or_else(|_| "less".to_string());
    let mut parts = pager.split_whitespace();
    let cmd = parts.next().unwrap_or("less");
    let args: Vec<&str> = parts.collect();

    if let Ok(mut child) = Command::new(cmd).args(&args).stdin(Stdio::piped()).spawn() {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(README.as_bytes());
        }
        let _ = child.wait();
    } else {
        // Fallback if pager fails
        print!("{README}");
    }

    Ok(())
}
