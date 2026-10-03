use anyhow::{Context, Result};
use std::path::Path;

/// Show the current profile snapshot.
pub fn run_show() -> Result<()> {
    let config = load_config()?;
    let snapshot = crate::provision::profile::generate_snapshot(&config);
    let yaml = serde_yaml::to_string(&snapshot).context("serializing profile")?;
    print!("{}", yaml);
    Ok(())
}

/// Export the current profile snapshot to a file (or stdout when path is None).
pub fn run_export(output: Option<&Path>) -> Result<()> {
    let config = load_config()?;
    let snapshot = crate::provision::profile::generate_snapshot(&config);
    let yaml = serde_yaml::to_string(&snapshot).context("serializing profile")?;
    match output {
        Some(path) => {
            std::fs::write(path, &yaml)
                .with_context(|| format!("writing profile to {}", path.display()))?;
            println!("profile exported to {}", path.display());
        }
        None => print!("{}", yaml),
    }
    Ok(())
}

/// Show a diff between the current profile and the cached team profile (stub).
pub fn run_diff() -> Result<()> {
    use crate::provision::cache::{load_policy, CacheStatus};

    let grace = 72 * 3600;
    let team_url = match load_policy(grace)? {
        CacheStatus::Fresh(p) | CacheStatus::Stale(p) => p.team_profile_url,
        _ => None,
    };

    match team_url {
        Some(url) => {
            println!("team profile URL: {}", url);
            println!("(diff requires 'muxix provision sync' to download team profile)");
        }
        None => {
            println!("no team_profile_url in cached org policy — nothing to diff");
        }
    }
    Ok(())
}

fn load_config() -> Result<crate::config::Config> {
    crate::config::Config::load(None).context("loading muxix config")
}
