use anyhow::Result;
use indicatif::{ProgressBar, ProgressStyle};
use std::time::Duration;

/// Create a spinner with consistent styling.
fn create_spinner(msg: &str) -> ProgressBar {
    let pb = ProgressBar::new_spinner();
    pb.enable_steady_tick(Duration::from_millis(120));
    pb.set_style(
        ProgressStyle::default_spinner()
            .tick_strings(&["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"])
            .template("{spinner:.blue} {msg}")
            .unwrap(),
    );
    pb.set_message(msg.to_string());
    pb
}

/// Run an operation with a spinner, showing success/failure.
pub fn with_spinner<T, F>(msg: &str, op: F) -> Result<T>
where
    F: FnOnce() -> Result<T>,
{
    let pb = create_spinner(msg);
    let result = op();
    match &result {
        Ok(_) => pb.finish_with_message(format!("✔ {}", msg)),
        Err(_) => pb.finish_with_message(format!("✘ {}", msg)),
    }
    result
}
