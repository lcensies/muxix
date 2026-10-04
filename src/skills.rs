//! Bundled skill installation.
//!
//! Embeds all muxix SKILL.md files at compile time and writes them
//! to the appropriate platform-specific skills directories.

use anyhow::{Context, Result};
use console::style;
use similar::{ChangeTag, TextDiff};
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use crate::agent::setup::Agent;

pub struct BundledSkill {
    pub name: &'static str,
    pub content: &'static str,
}

pub const BUNDLED_SKILLS: &[BundledSkill] = &[
    BundledSkill {
        name: "merge",
        content: include_str!("../skills/merge/SKILL.md"),
    },
    BundledSkill {
        name: "rebase",
        content: include_str!("../skills/rebase/SKILL.md"),
    },
    BundledSkill {
        name: "worktree",
        content: include_str!("../skills/worktree/SKILL.md"),
    },
    BundledSkill {
        name: "open-pr",
        content: include_str!("../skills/open-pr/SKILL.md"),
    },
    BundledSkill {
        name: "muxix",
        content: include_str!("../skills/muxix/SKILL.md"),
    },
    BundledSkill {
        name: "agent-packages",
        content: include_str!("../skills/agent-packages/SKILL.md"),
    },
];

/// Return the skills base directory for a given agent.
/// Returns None if the agent doesn't support skills.
pub fn skills_dir(agent: Agent) -> Option<PathBuf> {
    use crate::agent::setup::{copilot, gemini, omp, pi, prime};
    let home = home::home_dir()?;
    match agent {
        Agent::Claude => {
            let base = std::env::var_os("CLAUDE_CONFIG_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".claude"));
            Some(base.join("skills"))
        }
        Agent::OpenCode => Some(home.join(".config/opencode/skills")),
        Agent::Pi => Some(pi::agent_dir()?.join("skills")),
        Agent::Omp => Some(omp::agent_dir()?.join("skills")),
        Agent::Prime => Some(prime::agent_dir()?.join("skills")),
        Agent::Gemini => Some(gemini::skills_dir()?),
        Agent::Copilot => copilot::skills_dir(),
        // Codex reads USER-scope skills from `$HOME/.agents/skills` — the shared
        // cross-agent location, NOT a path under `CODEX_HOME`. A profile that
        // redirects `CODEX_HOME` therefore cannot isolate Codex's skills.
        Agent::Codex => Some(home.join(".agents/skills")),
    }
}

/// Check if any bundled skills are missing for the given agent.
pub fn needs_install(agent: Agent) -> bool {
    let Some(base_dir) = skills_dir(agent) else {
        return false;
    };

    BUNDLED_SKILLS
        .iter()
        .any(|skill| !base_dir.join(skill.name).join("SKILL.md").exists())
}

enum InstallOutcome {
    Installed,
    AlreadyUpToDate,
    Updated,
    Skipped,
}

/// Install all bundled skills to the given agent's skills directory.
pub fn install_skills(agent: Agent) -> Result<String> {
    let Some(base_dir) = skills_dir(agent) else {
        return Ok(format!("{} does not support skills", agent.name()));
    };

    let mut installed = 0u32;
    let mut up_to_date = 0u32;
    let mut updated = 0u32;
    let mut skipped = 0u32;

    for skill in BUNDLED_SKILLS {
        let dir = base_dir.join(skill.name);
        let path = dir.join("SKILL.md");

        let outcome = if path.exists() {
            let existing = fs::read_to_string(&path)
                .with_context(|| format!("Failed to read {}", path.display()))?;

            if existing == skill.content {
                InstallOutcome::AlreadyUpToDate
            } else {
                print_skill_diff(skill.name, &existing, skill.content);
                if confirm_overwrite(skill.name)? {
                    fs::write(&path, skill.content)
                        .with_context(|| format!("Failed to write {}", path.display()))?;
                    println!("  {} updated {}/SKILL.md", style("✓").green(), skill.name);
                    InstallOutcome::Updated
                } else {
                    InstallOutcome::Skipped
                }
            }
        } else {
            fs::create_dir_all(&dir)
                .with_context(|| format!("Failed to create {}", dir.display()))?;
            fs::write(&path, skill.content)
                .with_context(|| format!("Failed to write {}", path.display()))?;
            println!("  {} installed {}/SKILL.md", style("✓").green(), skill.name);
            InstallOutcome::Installed
        };

        match outcome {
            InstallOutcome::Installed => installed += 1,
            InstallOutcome::AlreadyUpToDate => up_to_date += 1,
            InstallOutcome::Updated => updated += 1,
            InstallOutcome::Skipped => skipped += 1,
        }
    }

    let mut parts = Vec::new();
    if installed > 0 {
        parts.push(format!("{installed} installed"));
    }
    if updated > 0 {
        parts.push(format!("{updated} updated"));
    }
    if up_to_date > 0 {
        parts.push(format!("{up_to_date} up to date"));
    }
    if skipped > 0 {
        parts.push(format!("{skipped} skipped"));
    }

    Ok(format!(
        "Skills for {} ({}): {}",
        agent.name(),
        base_dir.display(),
        parts.join(", ")
    ))
}

fn print_skill_diff(name: &str, old: &str, new: &str) {
    println!();
    println!(
        "  {} {}/SKILL.md differs from bundled version:",
        style("~").yellow(),
        name
    );
    println!();

    let diff = TextDiff::from_lines(old, new);
    for (idx, group) in diff.grouped_ops(3).iter().enumerate() {
        if idx > 0 {
            println!("    {}", style("~~~").dim());
        }
        for op in group {
            for change in diff.iter_changes(op) {
                let line = change.value().trim_end_matches('\n');
                match change.tag() {
                    ChangeTag::Insert => {
                        println!("    {}", style(format!("+{line}")).green());
                    }
                    ChangeTag::Delete => {
                        println!("    {}", style(format!("-{line}")).red());
                    }
                    ChangeTag::Equal => {
                        println!("    {}", style(format!(" {line}")).dim());
                    }
                }
            }
        }
    }
    println!();
}

fn confirm_overwrite(name: &str) -> Result<bool> {
    let prompt = format!(
        "  Overwrite {}/SKILL.md with bundled version? {}{}{} ",
        name,
        style("[").bold().cyan(),
        style("y/N").bold(),
        style("]").bold().cyan(),
    );

    loop {
        print!("{}", prompt);
        io::stdout().flush()?;

        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
        let answer = input.trim().to_lowercase();

        match answer.as_str() {
            "" | "n" | "no" => return Ok(false),
            "y" | "yes" => return Ok(true),
            _ => println!("    {}", style("Please enter y or n").dim()),
        }
    }
}

/// Install the bundled skills for `agent`, reporting one result per skill.
///
/// Unlike [`install_skills`], this never prompts: it is the path used by
/// non-interactive `muxix setup` and by `--check`. A locally-modified skill is
/// overwritten rather than queried, because in declarative mode the bundled
/// content is the source of truth — the interactive path still asks.
///
/// With `dry_run` set, outcomes are computed and nothing is written.
pub fn install_bundled(
    agent: Agent,
    dry_run: bool,
) -> Result<Vec<crate::command::setup::ItemResult>> {
    use crate::command::setup::{ItemResult, Outcome, Section};

    let Some(base_dir) = skills_dir(agent) else {
        return Ok(vec![ItemResult::skipped(
            Section::Skills,
            Some(agent.name()),
            "bundled skills",
            format!("{} does not support skills", agent.name()),
        )]);
    };

    let mut out = Vec::new();
    for skill in BUNDLED_SKILLS {
        let dir = base_dir.join(skill.name);
        let path = dir.join("SKILL.md");
        let existing = fs::read_to_string(&path).ok();

        let outcome = match existing {
            Some(ref e) if e == skill.content => Outcome::UpToDate,
            Some(_) => Outcome::Updated,
            None => Outcome::Installed,
        };

        if outcome != Outcome::UpToDate && !dry_run {
            fs::create_dir_all(&dir)
                .with_context(|| format!("Failed to create {}", dir.display()))?;
            fs::write(&path, skill.content)
                .with_context(|| format!("Failed to write {}", path.display()))?;
        }

        out.push(ItemResult::new(
            Section::Skills,
            Some(agent.name()),
            skill.name,
            outcome,
        ));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bundled_skills_not_empty() {
        assert_eq!(BUNDLED_SKILLS.len(), 6);
        for skill in BUNDLED_SKILLS {
            assert!(!skill.name.is_empty(), "skill name should not be empty");
            assert!(
                !skill.content.is_empty(),
                "skill {} content should not be empty",
                skill.name
            );
            assert!(
                skill.content.starts_with("---"),
                "skill {} should start with YAML frontmatter",
                skill.name
            );
        }
    }

    #[test]
    fn test_skills_dir_claude() {
        // Without CLAUDE_CONFIG_DIR, this resolves to $HOME/.claude/skills.
        // With it set, the env var should win. We can't safely mutate process
        // env in parallel tests, so just exercise the unset-or-set branch
        // generically and assert the trailing component.
        let dir = skills_dir(Agent::Claude);
        assert!(dir.is_some());
        let path = dir.unwrap();
        assert!(path.ends_with("skills"));
    }

    #[test]
    fn test_skills_dir_claude_respects_env() {
        // Safety: serial within this test; we restore the original value.
        let prev = std::env::var_os("CLAUDE_CONFIG_DIR");
        // SAFETY: tests in this module that read CLAUDE_CONFIG_DIR are
        // intentionally isolated; cargo test runs may interleave, but no
        // other test in this crate mutates this var.
        unsafe {
            std::env::set_var("CLAUDE_CONFIG_DIR", "/tmp/muxix-test-claude-cfg");
        }
        let dir = skills_dir(Agent::Claude).unwrap();
        assert_eq!(dir, PathBuf::from("/tmp/muxix-test-claude-cfg/skills"));
        unsafe {
            match prev {
                Some(v) => std::env::set_var("CLAUDE_CONFIG_DIR", v),
                None => std::env::remove_var("CLAUDE_CONFIG_DIR"),
            }
        }
    }

    #[test]
    fn test_skills_dir_opencode() {
        let dir = skills_dir(Agent::OpenCode);
        assert!(dir.is_some());
    }

    #[test]
    fn test_skills_dir_pi() {
        let dir = skills_dir(Agent::Pi);
        assert!(dir.is_some());
        let path = dir.unwrap();
        assert!(path.ends_with(".pi/agent/skills"));
    }

    #[test]
    fn skills_dir_codex_is_the_shared_agents_location() {
        // Codex reads USER-scope skills from $HOME/.agents/skills, NOT from
        // under CODEX_HOME -- a profile cannot isolate them.
        let dir = skills_dir(Agent::Codex).expect("codex has a skills dir");
        assert!(dir.ends_with(".agents/skills"), "{dir:?}");
    }

    #[test]
    fn skills_dir_gemini_is_user_scope() {
        let dir = skills_dir(Agent::Gemini).expect("gemini has a skills dir");
        assert!(dir.ends_with(".gemini/skills"), "{dir:?}");
    }

    #[test]
    fn skills_dir_copilot_follows_copilot_home() {
        let dir = skills_dir(Agent::Copilot).expect("copilot has a skills dir");
        assert!(dir.ends_with("skills"), "{dir:?}");
        assert!(
            dir.parent().is_some_and(
                |p| p.ends_with(".copilot") || std::env::var_os("COPILOT_HOME").is_some()
            )
        );
    }

    #[test]
    fn every_agent_has_an_explicit_skills_decision() {
        // No agent may fall through a catch-all arm: each one is either a real
        // directory or a documented `None`.
        for agent in Agent::ALL {
            let dir = skills_dir(agent);
            assert!(dir.is_some(), "{} has no skills dir decision", agent.name());
        }
    }

    #[test]
    fn test_bundled_skill_names() {
        let names: Vec<_> = BUNDLED_SKILLS.iter().map(|s| s.name).collect();
        assert!(names.contains(&"merge"));
        assert!(names.contains(&"rebase"));
        assert!(names.contains(&"worktree"));
        assert!(names.contains(&"open-pr"));
        assert!(names.contains(&"muxix"));
    }
}
