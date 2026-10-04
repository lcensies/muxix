//! `muxix start` — launch every tracked project: one tmux session per
//! project (base layout), plus one window per muxix worktree. With
//! `--continue`, relaunch the last coding agent in each of them via the
//! resurrect resume machinery.
//!
//! `muxix project open <name>` does the same for a single tracked project
//! and then focuses its session.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use tracing::info;

use crate::command::resurrect::{
    ResumePlan, find_task_prompt, plan_resume, write_resurrect_prompt,
};
use crate::config::{Config, MuxMode, ProjectOpenFilter};
use crate::multiplexer::types::{CreateSessionParams, CreateWindowInSessionParams, ResumeMode};
use crate::multiplexer::{Multiplexer, create_backend, detect_backend};
use crate::state::StateStore;
use crate::util::canon_or_self;
use crate::workflow::{self, SetupOptions, WorkflowContext};
use crate::{config, git};

use super::open_filter;
use super::registry::{ProjectEntry, Registry};

/// A window of a project's base session layout.
struct BaseWindow {
    name: Option<String>,
    command: Option<String>,
}

pub fn run(continue_session: bool, filter: Option<ProjectOpenFilter>) -> Result<()> {
    let registry = Registry::load()?;
    if registry.projects.is_empty() {
        println!("No tracked projects. Add one with 'muxix project add <dir>'.");
        return Ok(());
    }

    let mut failed = 0usize;
    for entry in &registry.projects {
        if !entry.root.is_dir() {
            eprintln!(
                "⚠ Skipping '{}': directory {} no longer exists",
                entry.name,
                entry.root.display()
            );
            continue;
        }
        if let Err(e) = start_project(entry, continue_session, filter) {
            eprintln!("✗ Failed to start '{}': {:#}", entry.name, e);
            failed += 1;
        }
    }
    if failed > 0 {
        bail!("Failed to start {} project(s)", failed);
    }
    Ok(())
}

/// Start one tracked project and focus its session.
pub fn open(
    target: &str,
    continue_session: bool,
    filter: Option<ProjectOpenFilter>,
) -> Result<()> {
    let registry = Registry::load()?;
    let Some(entry) = registry.find(target).cloned() else {
        bail!("No tracked project matches '{target}'. Add it with 'muxix project add <dir>'.");
    };
    if !entry.root.is_dir() {
        bail!(
            "Project '{}' root {} no longer exists",
            entry.name,
            entry.root.display()
        );
    }
    start_project(&entry, continue_session, filter)?;
    focus_session(&entry.name)
}

/// Switch the client when we are already inside the multiplexer; attach
/// (replacing this process) when invoked from a plain shell.
fn focus_session(name: &str) -> Result<()> {
    let mux = create_backend(detect_backend());
    if mux.name() != "tmux" || std::env::var_os("TMUX").is_some() {
        return mux.switch_to_session("", name);
    }
    use std::os::unix::process::CommandExt;
    let err = std::process::Command::new("tmux")
        .args(["attach-session", "-t", name])
        .exec();
    Err(err).context("Failed to exec 'tmux attach-session'")
}

fn start_project(
    entry: &ProjectEntry,
    continue_session: bool,
    filter: Option<ProjectOpenFilter>,
) -> Result<()> {
    // Per-project context: config discovery, the project journal
    // (ProjectStateStore::open_project) and git helpers all resolve from cwd.
    std::env::set_current_dir(&entry.root)
        .with_context(|| format!("Could not enter {}", entry.root.display()))?;

    let (cfg, config_location) = Config::load_with_location_from(&entry.root, None)?;
    // ponytail: only the session strategy exists; the match keeps this honest
    // when a window strategy lands.
    match cfg.project_mux.unwrap_or_default() {
        config::ProjectMux::Session => {}
    }

    let mux = create_backend(detect_backend());

    // Base session with layout, created once; an existing session is left alone.
    if mux.session_exists(&entry.name)? {
        println!("• {}: session already running", entry.name);
    } else {
        let windows = resolve_layout(&entry.name, &entry.root, &cfg);
        let first = windows.first().expect("layout always has one window");
        let root_pane = mux.create_session(CreateSessionParams {
            prefix: "",
            name: &entry.name,
            cwd: &entry.root,
            initial_window_name: first.name.as_deref(),
        })?;
        if let Some(cmd) = &first.command {
            mux.send_keys(&root_pane, cmd)?;
        }
        for w in &windows[1..] {
            let pane = mux.create_window_in_session(CreateWindowInSessionParams {
                session_name: &entry.name,
                name: w.name.as_deref(),
                cwd: &entry.root,
            })?;
            if let Some(cmd) = &w.command {
                mux.send_keys(&pane, cmd)?;
            }
        }
        println!(
            "✓ {}: started session ({} window(s))",
            entry.name,
            windows.len()
        );

        // Resume the last agent in the project-root window. Only on a freshly
        // created session (typing into a live one is destructive) and only if
        // the layout didn't already claim that pane with its own command.
        if continue_session && first.command.is_none() {
            resume_root_agent(entry, &cfg, mux.as_ref(), &root_pane);
        }
    }

    // Worktree windows — only meaningful for git projects.
    if !git::is_git_repo_in(Some(&entry.root)).unwrap_or(false) {
        return Ok(());
    }
    let context = WorkflowContext::new_in(&entry.root, cfg, mux, config_location)?;
    open_worktree_windows(entry, &context, continue_session, filter)
}

/// `filter` overrides the config's `project_open` when the caller passed
/// `--worktrees`.
fn open_worktree_windows(
    entry: &ProjectEntry,
    context: &WorkflowContext,
    continue_session: bool,
    filter: Option<ProjectOpenFilter>,
) -> Result<()> {
    let canon_main = canon_or_self(&context.main_worktree_root);
    let agent_name = context
        .config
        .agent
        .clone()
        .unwrap_or_else(|| "claude".to_string());
    let default_mode = context.config.mode();

    let worktrees: Vec<(PathBuf, String)> = git::list_worktrees_in(Some(&entry.root))?
        .into_iter()
        .filter(|(path, _)| canon_or_self(path) != canon_main)
        .collect();

    let filter = filter.or(context.config.project_open).unwrap_or_default();
    let days = context.config.project_open_days();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // One pass over the agent state store for the whole project; `all` needs none.
    let states = if filter == ProjectOpenFilter::All {
        HashMap::new()
    } else {
        open_filter::states_by_handle(
            &StateStore::new()?,
            context.mux.name(),
            &context.mux.instance_id(),
            worktrees.iter().map(|(path, _)| path.as_path()),
        )?
    };
    let mut skipped = 0usize;

    for (path, _branch) in worktrees {
        let canon_path = canon_or_self(&path);
        let handle = match path.file_name() {
            Some(n) => n.to_string_lossy().to_string(),
            None => continue,
        };

        // newest_repo_ts spawns git, so only `recent` pays for it.
        let repo_ts = (filter == ProjectOpenFilter::Recent)
            .then(|| open_filter::newest_repo_ts(&path))
            .flatten();
        let worktree_states = states.get(&handle).map_or(&[][..], Vec::as_slice);
        if !open_filter::keep(filter, days, worktree_states, repo_ts, now) {
            info!(handle, filter = filter.as_str(), "start: worktree filtered");
            skipped += 1;
            continue;
        }

        // Mirror workflow::open's mode resolution so we only attach a parent
        // session in window mode (a session-mode worktree keeps its own session).
        let mode =
            git::get_worktree_mode_opt_in(&handle, Some(&entry.root)).unwrap_or(default_mode);

        let mut options = SetupOptions::new(false, false, continue_session);
        options.focus_window = false;
        options.mode = mode;
        if mode == MuxMode::Window {
            options.window_session_name = Some(entry.name.clone());
        }
        if continue_session {
            apply_resume(&mut options, &canon_path, &handle, &agent_name);
        }

        match workflow::open(&handle, context, options, false, None, None) {
            Ok(result) if result.did_switch => {
                info!(handle, "start: window already open, skipped");
            }
            Ok(_) => println!("  ↳ {}: opened worktree window '{}'", entry.name, handle),
            Err(e) => eprintln!(
                "  ✗ {}: could not open worktree '{}': {:#}",
                entry.name, handle, e
            ),
        }
    }
    if skipped > 0 {
        println!(
            "• {}: skipped {} worktree(s) (project_open: {})",
            entry.name,
            skipped,
            filter.as_str()
        );
    }
    Ok(())
}

/// Resolve how a worktree's agent should come back: resume a session, re-send
/// the stored task prompt, or start bare. Same ladder as `muxix resurrect`.
fn apply_resume(options: &mut SetupOptions, worktree_path: &Path, handle: &str, agent_name: &str) {
    if crate::multiplexer::conversation::resolve_forker(agent_name).is_none()
        && crate::agent::profile::resolve_profile(Some(agent_name))
            .continue_flag()
            .is_none()
    {
        eprintln!(
            "⚠ '{}': agent '{}' has no resume support, launching without --continue",
            handle, agent_name
        );
    }
    let resume_plan = plan_resume(worktree_path, handle, agent_name);
    options.resume_mode = match &resume_plan {
        ResumePlan::Session(id) => ResumeMode::ForkSession(id.clone()),
        ResumePlan::Latest => ResumeMode::Continue,
        ResumePlan::None => ResumeMode::None,
    };
    if matches!(resume_plan, ResumePlan::None)
        && let Some(task_prompt) = find_task_prompt(worktree_path, handle)
        && let Ok(path) = write_resurrect_prompt(worktree_path, handle, &task_prompt)
    {
        options.prompt_file_path = Some(path);
    }
}

/// Relaunch the last agent conversation found at the project root, if any.
/// Verified via the agent's session store when readable; otherwise fall back
/// to the CLI's continue flag (cwd-scoped resume). No support at all → the
/// window stays a plain shell.
fn resume_root_agent(entry: &ProjectEntry, cfg: &Config, mux: &dyn Multiplexer, pane_id: &str) {
    let agent_name = cfg.agent.clone().unwrap_or_else(|| "claude".to_string());
    let profile = crate::agent::profile::resolve_profile(Some(&agent_name));

    let cmd = match crate::multiplexer::conversation::resolve_forker(&agent_name) {
        Some(forker) => match forker.find_latest_conversation(&entry.root) {
            Ok(Some(session)) => Some(format!(
                "{} {}",
                agent_name,
                forker.resume_args(&session.id).join(" ")
            )),
            _ => None, // verified: nothing to resume here
        },
        None => profile
            .continue_flag()
            .map(|flag| format!("{} {}", agent_name, flag)),
    };

    let Some(cmd) = cmd else {
        if profile.continue_flag().is_none() {
            eprintln!(
                "⚠ {}: agent '{}' has no resume support, starting without --continue",
                entry.name, agent_name
            );
        }
        return;
    };
    if let Err(e) = mux.send_keys(pane_id, &cmd) {
        eprintln!("⚠ {}: could not launch agent: {:#}", entry.name, e);
    } else {
        println!("  ↺ {}: resumed {} in project root", entry.name, agent_name);
    }
}

/// Base layout for a project session. Priority: project `.muxix.yaml`
/// `windows:` → `~/.config/tmuxrs/<name>.yml` → `~/.config/tmuxinator/<name>.yml`
/// → single default window at the project root.
fn resolve_layout(name: &str, _root: &Path, cfg: &Config) -> Vec<BaseWindow> {
    if let Some(windows) = &cfg.windows
        && !windows.is_empty()
    {
        return windows
            .iter()
            .map(|w| BaseWindow {
                name: w.name.clone(),
                // Base-session windows take only the window name from muxix
                // config; pane commands stay a worktree concern.
                command: None,
            })
            .collect();
    }

    for dir in ["tmuxrs", "tmuxinator"] {
        if let Some(windows) = load_tmuxinator_layout(dir, name) {
            return windows;
        }
    }

    vec![BaseWindow {
        name: None,
        command: None,
    }]
}

/// Lenient reader for tmuxrs/tmuxinator YAML: only `windows:` entries of the
/// form `- name: command` are honored; anything else (nested panes, ERB,
/// hooks) is ignored. Parse failure → warn and fall back.
fn load_tmuxinator_layout(dir: &str, name: &str) -> Option<Vec<BaseWindow>> {
    let path = crate::xdg::config_dir()
        .ok()?
        .parent()?
        .join(dir)
        .join(format!("{name}.yml"));
    let content = std::fs::read_to_string(&path).ok()?;
    let doc: serde_yaml::Value = match serde_yaml::from_str(&content) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("⚠ Ignoring unparseable {}: {}", path.display(), e);
            return None;
        }
    };
    let windows: Vec<BaseWindow> = doc
        .get("windows")?
        .as_sequence()?
        .iter()
        .filter_map(|w| {
            let map = w.as_mapping()?;
            let (k, v) = map.iter().next()?;
            Some(BaseWindow {
                name: k.as_str().map(str::to_string),
                command: v.as_str().map(str::to_string),
            })
        })
        .collect();
    if windows.is_empty() {
        None
    } else {
        Some(windows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_layout_is_single_plain_window() {
        let cfg = Config::default();
        let layout = resolve_layout("no-such-project-xyz", Path::new("/tmp"), &cfg);
        assert_eq!(layout.len(), 1);
        assert!(layout[0].name.is_none());
        assert!(layout[0].command.is_none());
    }

    #[test]
    fn resume_ladder_falls_back_to_stored_prompt_then_bare() {
        let tmp = tempfile::tempdir().unwrap();

        // No session, no prompt → bare shell (ResumeMode::None, no prompt file)
        let mut options = SetupOptions::new(false, false, true);
        apply_resume(&mut options, tmp.path(), "wt", "claude");
        assert_eq!(options.resume_mode, ResumeMode::None);
        assert!(options.prompt_file_path.is_none());

        // No session but stored task prompt → re-prompt
        std::fs::create_dir_all(tmp.path().join(".muxix")).unwrap();
        std::fs::write(tmp.path().join(".muxix/PROMPT-wt.md"), "task").unwrap();
        let mut options = SetupOptions::new(false, false, true);
        apply_resume(&mut options, tmp.path(), "wt", "claude");
        assert_eq!(options.resume_mode, ResumeMode::None);
        assert!(options.prompt_file_path.is_some());
    }

    #[test]
    fn muxix_windows_config_wins() {
        let cfg: Config =
            serde_yaml::from_str("windows:\n  - name: editor\n  - name: shell\n").unwrap();
        let layout = resolve_layout("x", Path::new("/tmp"), &cfg);
        assert_eq!(layout.len(), 2);
        assert_eq!(layout[0].name.as_deref(), Some("editor"));
    }
}
