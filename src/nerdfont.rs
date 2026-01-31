//! Nerdfont detection and icon helpers.
//!
//! Provides automatic detection of nerdfont support and fallback icons for
//! users without nerdfonts installed.

use anyhow::Result;
use console::style;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::PathBuf;
use std::sync::OnceLock;

/// Cached nerdfont setting to avoid repeated config lookups.
static NERDFONT_ENABLED: OnceLock<bool> = OnceLock::new();

/// Icons for PR status display.
#[derive(Clone, Copy)]
pub struct PrIcons {
    pub draft: &'static str,
    pub open: &'static str,
    pub merged: &'static str,
    pub closed: &'static str,
}

/// Icons for git status display.
#[derive(Clone, Copy)]
pub struct GitIcons {
    pub diff: &'static str,
    pub conflict: &'static str,
}

const NERDFONT_PR_ICONS: PrIcons = PrIcons {
    draft: "\u{f177}",  // nf-oct-git_pull_request_draft
    open: "\u{f407}",   // nf-oct-git_pull_request
    merged: "\u{f419}", // nf-oct-git_merge
    closed: "\u{f406}", // nf-oct-git_pull_request_closed
};

const FALLBACK_PR_ICONS: PrIcons = PrIcons {
    draft: "○",
    open: "●",
    merged: "◆",
    closed: "×",
};

/// Icons for CI/CD check status display.
#[derive(Clone, Copy)]
pub struct CheckIcons {
    pub success: &'static str,
    pub failure: &'static str,
    pub pending: &'static str,
}

const NERDFONT_CHECK_ICONS: CheckIcons = CheckIcons {
    success: "\u{f0134}", // 󰄴 nf-md-check_circle
    failure: "\u{f0159}", // 󰅙 nf-md-close_circle
    pending: "\u{f1423}", // 󱐣 nf-md-timer_sand
};

const FALLBACK_CHECK_ICONS: CheckIcons = CheckIcons {
    success: "✓",
    failure: "×",
    pending: "◷",
};

const NERDFONT_GIT_ICONS: GitIcons = GitIcons {
    diff: "\u{f03eb}",     // nf-md-file_document_edit_outline
    conflict: "\u{f002a}", // nf-md-alert
};

const FALLBACK_GIT_ICONS: GitIcons = GitIcons {
    diff: "*",
    conflict: "!",
};

/// Git branch icon used in the setup prompt.
const GIT_BRANCH_ICON: &str = "\u{e725}"; // nf-dev-git_branch

/// Initialize the nerdfont setting from config or detection.
/// Should be called early in the CLI flow.
pub fn init(config_nerdfont: Option<bool>, config_has_pua: bool) {
    let enabled = config_nerdfont.unwrap_or(config_has_pua);
    let _ = NERDFONT_ENABLED.set(enabled);
}

/// Check if nerdfonts are enabled.
pub fn is_enabled() -> bool {
    *NERDFONT_ENABLED.get().unwrap_or(&false)
}

/// Get PR status icons based on nerdfont setting.
pub fn pr_icons() -> PrIcons {
    if is_enabled() {
        NERDFONT_PR_ICONS
    } else {
        FALLBACK_PR_ICONS
    }
}

/// Get check status icons based on nerdfont setting.
pub fn check_icons() -> CheckIcons {
    if is_enabled() {
        NERDFONT_CHECK_ICONS
    } else {
        FALLBACK_CHECK_ICONS
    }
}

/// Get git status icons based on nerdfont setting.
pub fn git_icons() -> GitIcons {
    if is_enabled() {
        NERDFONT_GIT_ICONS
    } else {
        FALLBACK_GIT_ICONS
    }
}

/// Check if a string contains characters in Private Use Area ranges.
/// PUA ranges: U+E000-U+F8FF (BMP PUA), U+F0000-U+FFFFF (Supplementary PUA-A)
pub fn contains_pua(s: &str) -> bool {
    s.chars().any(|c| {
        let cp = c as u32;
        (0xE000..=0xF8FF).contains(&cp) || (0xF0000..=0xFFFFF).contains(&cp)
    })
}

/// Check if the config contains any PUA characters in string values.
/// This indicates the user has nerdfonts configured.
pub fn config_has_pua(config: &crate::config::Config) -> bool {
    // Check status_icons
    if let Some(ref working) = config.status_icons.working
        && contains_pua(working)
    {
        return true;
    }
    if let Some(ref waiting) = config.status_icons.waiting
        && contains_pua(waiting)
    {
        return true;
    }
    if let Some(ref done) = config.status_icons.done
        && contains_pua(done)
    {
        return true;
    }

    // Check window_prefix
    if let Some(ref prefix) = config.window_prefix
        && contains_pua(prefix)
    {
        return true;
    }

    // Check worktree_prefix
    if let Some(ref prefix) = config.worktree_prefix
        && contains_pua(prefix)
    {
        return true;
    }

    false
}

/// Get the path to the global config file.
/// Prefers existing .yml file to avoid shadowing, otherwise defaults to .yaml.
fn global_config_path() -> Option<PathBuf> {
    let home = home::home_dir()?;
    let yaml = home.join(".config/workmux/config.yaml");
    let yml = home.join(".config/workmux/config.yml");

    // Prefer existing .yml file to avoid shadowing user's config
    if yml.exists() && !yaml.exists() {
        Some(yml)
    } else {
        Some(yaml)
    }
}

/// Prompt the user to indicate if they have nerdfonts installed.
/// Returns None if stdin is not a TTY (non-interactive) or in CI/test environments.
pub fn prompt_setup() -> Result<Option<bool>> {
    // Skip prompt in CI or test environments
    if std::env::var("CI").is_ok() || std::env::var("WORKMUX_TEST").is_ok() {
        return Ok(None);
    }

    // Only prompt in interactive mode
    if !io::stdin().is_terminal() {
        return Ok(None);
    }

    let dim = style("│").dim();
    let corner_top = style("┌").dim();
    let corner_bottom = style("└─").dim();

    // Print the prompt box
    println!();
    println!("{} {}", corner_top, style("Nerdfont Setup").bold().cyan());
    println!("{}", dim);
    println!(
        "{}  Does this look like a git branch icon?  {}  {}",
        dim,
        style("→").yellow(),
        style(GIT_BRANCH_ICON).green()
    );
    println!("{}", dim);
    let prompt_line = format!(
        "{} {}{}{} Yes  {}{}{} No: ",
        corner_bottom,
        style("[").bold().cyan(),
        style("y").bold(),
        style("]").bold().cyan(),
        style("[").bold().cyan(),
        style("n").bold(),
        style("]").bold().cyan(),
    );

    // Loop until valid input
    let enabled = loop {
        print!("{}", prompt_line);
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let answer = input.trim().to_lowercase();

        match answer.as_str() {
            "y" | "yes" => break true,
            "n" | "no" => break false,
            _ => {
                println!("{}", style("  Please enter y or n").dim());
            }
        }
    };

    // Show confirmation first so user knows their choice is active
    if enabled {
        println!("{}", style("✔ Nerdfont icons enabled").green());
    } else {
        println!("{}", style("✔ Using Unicode fallbacks").green());
    }

    // Save to global config
    if let Err(e) = save_nerdfont_preference(enabled) {
        // Config file might be read-only (e.g., symlink to Nix store)
        println!(
            "  {}",
            style(format!("Could not save preference: {}", e)).yellow()
        );
        println!(
            "  {}",
            style(format!(
                "Add 'nerdfont: {}' to ~/.config/workmux/config.yaml to persist this setting",
                enabled
            ))
            .dim()
        );
    } else if !enabled {
        println!(
            "  {}",
            style("Set nerdfont: true in ~/.config/workmux/config.yaml to enable later").dim()
        );
    }
    println!();

    Ok(Some(enabled))
}

/// Save the nerdfont preference to the global config file.
fn save_nerdfont_preference(enabled: bool) -> Result<()> {
    let config_path = global_config_path()
        .ok_or_else(|| anyhow::anyhow!("Could not determine home directory"))?;

    // Ensure the config directory exists
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Read existing config or create empty
    let mut config_content = if config_path.exists() {
        fs::read_to_string(&config_path)?
    } else {
        String::new()
    };

    // Check if nerdfont key already exists
    if config_content.contains("nerdfont:") {
        // Update existing value
        let re = regex::Regex::new(r"(?m)^nerdfont:.*$")?;
        config_content = re
            .replace(&config_content, format!("nerdfont: {}", enabled))
            .to_string();
    } else {
        // Add nerdfont key
        if !config_content.is_empty() && !config_content.ends_with('\n') {
            config_content.push('\n');
        }
        config_content.push_str(&format!("nerdfont: {}\n", enabled));
    }

    fs::write(&config_path, config_content)?;

    Ok(())
}

/// Run the nerdfont setup check.
/// Returns the nerdfont setting (true/false) or None if not determined.
pub fn check_and_prompt(config: &crate::config::Config) -> Result<Option<bool>> {
    // If nerdfont is already configured, use that value
    if let Some(enabled) = config.nerdfont {
        return Ok(Some(enabled));
    }

    // If config contains PUA characters, assume nerdfonts are available
    if config_has_pua(config) {
        return Ok(Some(true));
    }

    // Otherwise, prompt the user
    prompt_setup()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_pua_detects_bmp_pua() {
        assert!(contains_pua("\u{E000}"));
        assert!(contains_pua("\u{F8FF}"));
        assert!(contains_pua("text \u{E725} more")); // git branch icon
    }

    #[test]
    fn contains_pua_detects_supplementary_pua() {
        assert!(contains_pua("\u{F0000}"));
        assert!(contains_pua("\u{FFFFF}"));
        assert!(contains_pua("\u{f03eb}")); // file edit icon
    }

    #[test]
    fn contains_pua_rejects_normal_text() {
        assert!(!contains_pua("hello world"));
        assert!(!contains_pua("✓ ✗ → ↑ ↓"));
        assert!(!contains_pua("●○◆×"));
    }

    #[test]
    fn contains_pua_handles_empty_string() {
        assert!(!contains_pua(""));
    }
}
