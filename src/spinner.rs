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

/// Run a command with a spinner, streaming its output above the spinner line.
///
/// Shows a spinner with the given message while the command runs. Lines from
/// the command's stdout and stderr are printed above the spinner in real time.
/// On completion, the spinner shows success/failure.
pub fn with_streaming_command(msg: &str, mut cmd: std::process::Command) -> Result<()> {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;

    let pb = create_spinner(msg);

    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            pb.finish_with_message(format!("✘ {}", msg));
            anyhow::anyhow!("Failed to spawn command: {}", e)
        })?;

    // Stream stdout and stderr in separate threads, printing above the spinner
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let pb_out = pb.clone();
    let pb_err = pb.clone();

    let stdout_thread = std::thread::spawn(move || {
        if let Some(stdout) = stdout {
            for line in BufReader::new(stdout).lines() {
                if let Ok(line) = line
                    && !line.trim().is_empty() {
                        pb_out.println(&line);
                    }
            }
        }
    });

    let stderr_thread = std::thread::spawn(move || {
        if let Some(stderr) = stderr {
            for line in BufReader::new(stderr).lines() {
                if let Ok(line) = line
                    && !line.trim().is_empty() {
                        pb_err.println(&line);
                    }
            }
        }
    });

    stdout_thread.join().ok();
    stderr_thread.join().ok();

    let status = child.wait().map_err(|e| {
        pb.finish_with_message(format!("✘ {}", msg));
        anyhow::anyhow!("Failed to wait for command: {}", e)
    })?;

    if status.success() {
        pb.finish_with_message(format!("✔ {}", msg));
        Ok(())
    } else {
        pb.finish_with_message(format!("✘ {}", msg));
        anyhow::bail!("{} (exit code: {})", msg, status.code().unwrap_or(-1))
    }
}
