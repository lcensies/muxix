use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Result, anyhow};
use tracing::info;

use crate::config;
use crate::multiplexer::{Multiplexer, create_backend, detect_backend};
use crate::state::StateStore;
use crate::workflow::resurrect::{ResurrectAction, plan};
use crate::workflow::{self, SetupOptions, WorkflowContext};

/// How a worktree was brought back, for honest reporting.
enum Restored {
    /// The agent's own session store had a conversation; relaunched with --continue.
    Resumed,
    /// No resumable session; relaunched fresh with the stored task prompt re-sent.
    RePrompted,
    /// No session and no stored prompt; the agent starts with a blank slate.
    Bare,
}

impl Restored {
    fn label(&self) -> &'static str {
        match self {
            Restored::Resumed => "resumed session",
            Restored::RePrompted => "re-sent task prompt",
            Restored::Bare => "started fresh (no session, no stored prompt)",
        }
    }
}

/// How a worktree's agent should be relaunched.
/// Shared with `workmux start --continue`, which reuses this resume ladder.
pub enum ResumePlan {
    /// Resume this specific session id (journalled by workmux, verified present).
    Session(String),
    /// Resume whatever the agent considers its latest conversation here.
    Latest,
    /// Nothing to resume.
    None,
}

/// Decide how (or whether) this worktree's conversation can be resumed.
///
/// The project journal is consulted first so a worktree resumes the session
/// workmux actually started in it rather than whatever happens to be newest.
/// Either way the answer is confirmed against the agent's own session store,
/// which is the only authority: the journal is a record of what workmux did,
/// and a user deleting a session behind its back must not turn into a launch
/// that exits immediately and takes the restored window down with it.
pub fn plan_resume(worktree_path: &Path, handle: &str, agent_name: &str) -> ResumePlan {
    let Some(forker) = crate::multiplexer::conversation::resolve_forker(agent_name) else {
        // No on-disk session store we can read (SQLite- or hash-keyed agents
        // like opencode/gemini/copilot). If the CLI has a continue flag, its
        // conversations are still resumable per-directory — just unverified.
        if crate::agent::profile::resolve_profile(Some(agent_name))
            .continue_flag()
            .is_some()
        {
            return ResumePlan::Latest;
        }
        return ResumePlan::None;
    };

    let journalled = crate::project_state::ProjectStateStore::open_project()
        .and_then(|store| store.get_worktree(handle))
        .ok()
        .flatten()
        .and_then(|record| {
            record
                .latest_session_for(agent_name)
                .map(|s| s.id.clone())
        });

    if let Some(id) = journalled
        && matches!(forker.find_conversation(worktree_path, &id), Ok(Some(_)))
    {
        return ResumePlan::Session(id);
    }

    match forker.find_latest_conversation(worktree_path) {
        Ok(Some(session)) => {
            // Journal what we just discovered so the next resurrect can target
            // this session by id instead of re-deriving "latest" — which drifts
            // as soon as another conversation is started in the worktree.
            if let Err(e) = crate::project_state::ProjectStateStore::open_project()
                .and_then(|store| store.record_session(handle, agent_name, &session.id))
            {
                tracing::debug!(?e, "failed to journal discovered session");
            }
            // Resume by id, not by the agent's own "latest" flag: `claude
            // --continue` only considers interactive conversations, so a
            // session workmux can see is not necessarily one it will pick.
            ResumePlan::Session(session.id)
        }
        _ => ResumePlan::None,
    }
}

/// The resume ladder as a `ResumeMode`, for callers that only relaunch the
/// agent (no stored-prompt fallback): `workmux open -c` and the dashboard.
pub fn resume_mode_for(
    worktree_path: &Path,
    handle: &str,
    agent_name: &str,
) -> crate::multiplexer::types::ResumeMode {
    use crate::multiplexer::types::ResumeMode;
    match plan_resume(worktree_path, handle, agent_name) {
        ResumePlan::Session(id) => ResumeMode::ForkSession(id),
        // Unverifiable store (opencode, gemini, ...) — the CLI's own
        // cwd-scoped continue flag is the best available.
        ResumePlan::Latest => ResumeMode::Continue,
        ResumePlan::None => ResumeMode::None,
    }
}

/// Locate the task prompt stored for a worktree.
///
/// `workmux add -p/-P` writes `.workmux/PROMPT-<branch>.md` into the worktree,
/// which is the common case. Orchestrate-spawned worktrees instead carry their
/// task text in the project's `.workmux/task-workflows/<handle>.yaml`; that is
/// checked second and best-effort, since the orchestrate path is fragile.
pub fn find_task_prompt(worktree_path: &Path, handle: &str) -> Option<PathBuf> {
    let dir = worktree_path.join(".workmux");

    if let Ok(entries) = fs::read_dir(&dir) {
        let mut prompts: Vec<PathBuf> = entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("PROMPT-") && n.ends_with(".md"))
            })
            .collect();
        prompts.sort();
        if let Some(p) = prompts.pop() {
            return Some(p);
        }
    }

    let task_workflow = dir.join("task-workflows").join(format!("{handle}.yaml"));
    task_workflow.exists().then_some(task_workflow)
}

/// Build the prompt actually sent on a re-prompt, naming the parent agent so a
/// restored child knows who spawned it and reports back to the right place.
///
/// Written next to the original rather than over it: the stored task prompt is
/// the durable record and must survive repeated resurrections unchanged.
pub fn write_resurrect_prompt(
    worktree_path: &Path,
    handle: &str,
    task_prompt: &Path,
) -> Result<PathBuf> {
    let original = fs::read_to_string(task_prompt)?;
    // The project journal owns this now; git config is only read as a fallback
    // for worktrees created before the journal existed.
    let parent = crate::project_state::ProjectStateStore::open_project()
        .and_then(|store| store.get_worktree(handle))
        .ok()
        .flatten()
        .and_then(|record| record.parent)
        .or_else(|| crate::git::get_worktree_meta(handle, "parent"));

    let mut body = String::from(
        "Your tmux window was restored after a crash and your previous \
         conversation could not be resumed, so you are starting fresh. \
         Re-establish context from the worktree itself (git log, git status, \
         uncommitted changes) before acting — work may already be partly done.\n\n",
    );
    if let Some(parent) = &parent {
        body.push_str(&format!(
            "You were spawned by the agent in worktree `{parent}`. Report \
             progress and completion back to it with \
             `workmux send {parent} \"...\"`.\n\n",
        ));
    }
    body.push_str("Your original task follows.\n\n---\n\n");
    body.push_str(&original);

    let out = worktree_path.join(".workmux/RESURRECT-PROMPT.md");
    if let Some(dir) = out.parent() {
        fs::create_dir_all(dir)?;
    }
    fs::write(&out, body)?;
    Ok(out)
}

/// Is the restored target actually still there a moment after launch?
///
/// `workflow::open` returning Ok only means the window was created. An agent
/// that exits on startup takes its pane down and tmux closes the window, so
/// success has to be confirmed against live multiplexer state, not assumed.
fn target_is_live(mux: &dyn Multiplexer, name: &str, mode: config::MuxMode) -> bool {
    let live = if mode == config::MuxMode::Session {
        mux.get_all_session_names()
    } else {
        mux.get_all_window_names()
    };
    live.map(|names| names.contains(&name.to_string()))
        .unwrap_or(false)
}

pub fn run(dry_run: bool) -> Result<()> {
    let config = config::Config::load(None)?;
    let mux = create_backend(detect_backend());
    let store = StateStore::new()?;

    let plan = plan(&store, mux.as_ref())?;

    if plan.candidates.is_empty() && plan.unmatched_states == 0 {
        println!("No agent state files found. Nothing to restore.");
        return Ok(());
    }

    // Print plan
    let to_restore: Vec<_> = plan
        .candidates
        .iter()
        .filter(|c| matches!(c.action, ResurrectAction::Restore))
        .collect();

    for candidate in &plan.candidates {
        let status = match &candidate.action {
            ResurrectAction::Restore => "restoring",
            ResurrectAction::SkipAlreadyOpen => "skipping (already open)",
            ResurrectAction::SkipMain => "skipping (main worktree)",
        };
        println!("  {:<20} -> {}", candidate.handle, status);
    }

    if plan.unmatched_states > 0 {
        println!(
            "  ({} unmatched state file(s) ignored)",
            plan.unmatched_states
        );
    }

    if to_restore.is_empty() {
        println!("\nNothing to restore.");
        return Ok(());
    }

    if dry_run {
        println!("\nDry run: would restore {} worktree(s)", to_restore.len());
        return Ok(());
    }

    // Execute restoration
    let agent_name = config.agent.clone().unwrap_or_else(|| "claude".to_string());
    let prefix = config.window_prefix().to_string();
    let context = WorkflowContext::new(config, mux, None)?;
    let mut restored: Vec<(String, Restored)> = Vec::new();
    let mut failed = Vec::new();

    for candidate in &plan.candidates {
        if !matches!(candidate.action, ResurrectAction::Restore) {
            continue;
        }

        // Choose resume vs re-prompt from the agent's real session store,
        // before launching anything.
        let resume_plan = plan_resume(&candidate.worktree_path, &candidate.handle, &agent_name);
        let resumable = !matches!(resume_plan, ResumePlan::None);
        let mut outcome = if resumable {
            Restored::Resumed
        } else {
            Restored::Bare
        };

        let mut prompt_file_path = None;
        if !resumable
            && let Some(task_prompt) =
                find_task_prompt(&candidate.worktree_path, &candidate.handle)
        {
            match write_resurrect_prompt(&candidate.worktree_path, &candidate.handle, &task_prompt)
            {
                Ok(path) => {
                    prompt_file_path = Some(path);
                    outcome = Restored::RePrompted;
                }
                Err(e) => info!(
                    handle = candidate.handle,
                    error = %e,
                    "resurrect:exec could not build re-prompt, starting bare"
                ),
            }
        }

        let options = SetupOptions {
            run_hooks: false,
            run_file_ops: false,
            run_pane_commands: true,
            prompt_file_path,
            focus_window: false,
            working_dir: None,
            config_root: None,
            open_if_exists: false,
            mode: candidate.mode,
            target_window_name: None,
            target_session_name: None,
            window_session_name: None,
            resume_mode: match &resume_plan {
                // ForkSession injects `--resume <id>`; the copy that the name
                // suggests happens only on the `add --fork` path.
                ResumePlan::Session(id) => {
                    crate::multiplexer::types::ResumeMode::ForkSession(id.clone())
                }
                ResumePlan::Latest => crate::multiplexer::types::ResumeMode::Continue,
                ResumePlan::None => crate::multiplexer::types::ResumeMode::None,
            },
        };

        info!(
            handle = candidate.handle,
            mode = ?candidate.mode,
            resumable,
            stale_keys = candidate.stale_pane_keys.len(),
            "resurrect:exec opening worktree"
        );

        match workflow::open(&candidate.handle, &context, options, false, None, None) {
            Ok(result) => {
                // New instruction cycle: clear any stale completion claim
                // before checking liveness (belt-and-braces — reconcile
                // already drops state for a pane recycled with a new pid).
                crate::state::persist_agent_completion(
                    context.mux.as_ref(),
                    &result.focus_pane_id,
                    None,
                );
                // Creation succeeding is not restoration succeeding: an agent
                // that exits on startup takes the window with it.
                std::thread::sleep(Duration::from_millis(1500));
                let target = crate::multiplexer::util::prefixed(&prefix, &result.resolved_handle);
                if !target_is_live(context.mux.as_ref(), &target, candidate.mode) {
                    info!(
                        handle = candidate.handle,
                        target, "resurrect:exec window died right after launch"
                    );
                    eprintln!(
                        "  Failed to restore '{}': agent exited immediately and the window closed",
                        candidate.handle
                    );
                    failed.push(candidate.handle.clone());
                    continue;
                }

                info!(
                    handle = candidate.handle,
                    resolved = result.resolved_handle,
                    branch = result.branch_name,
                    path = %result.worktree_path.display(),
                    outcome = outcome.label(),
                    "resurrect:exec restored successfully"
                );
                // Clean up stale state files by specific PaneKey
                for key in &candidate.stale_pane_keys {
                    info!(
                        pane_id = %key.pane_id,
                        "resurrect:exec deleting stale state file"
                    );
                    let _ = store.delete_agent(key);
                }
                restored.push((candidate.handle.clone(), outcome));
            }
            Err(e) => {
                info!(
                    handle = candidate.handle,
                    error = %e,
                    "resurrect:exec failed to restore"
                );
                eprintln!("  Failed to restore '{}': {}", candidate.handle, e);
                failed.push(candidate.handle.clone());
            }
        }
    }

    // Summary
    if !restored.is_empty() {
        println!("\n✓ Restored {} worktree(s):", restored.len());
        for (handle, outcome) in &restored {
            println!("  {:<20} {}", handle, outcome.label());
        }
    }
    if !failed.is_empty() {
        eprintln!(
            "✗ Failed to restore {} worktree(s): {}",
            failed.len(),
            failed.join(", ")
        );
        return Err(anyhow!("Failed to restore {} worktree(s)", failed.len()));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn worktree_with(files: &[(&str, &str)]) -> tempfile::TempDir {
        let tmp = tempfile::tempdir().unwrap();
        for (rel, body) in files {
            let path = tmp.path().join(rel);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, body).unwrap();
        }
        tmp
    }

    #[test]
    fn finds_manual_add_prompt() {
        let wt = worktree_with(&[(".workmux/PROMPT-feature-auth.md", "do the thing")]);
        let found = find_task_prompt(wt.path(), "feature-auth").unwrap();
        assert_eq!(fs::read_to_string(found).unwrap(), "do the thing");
    }

    #[test]
    fn falls_back_to_orchestrate_task_workflow() {
        let wt = worktree_with(&[(".workmux/task-workflows/my-task.yaml", "prompt: hi")]);
        let found = find_task_prompt(wt.path(), "my-task").unwrap();
        assert!(found.ends_with("my-task.yaml"));
    }

    #[test]
    fn no_prompt_means_none() {
        let wt = worktree_with(&[(".workmux/state.json", "{}")]);
        assert!(find_task_prompt(wt.path(), "nothing").is_none());
    }

    #[test]
    fn re_prompt_preserves_original_and_explains_restart() {
        let wt = worktree_with(&[(".workmux/PROMPT-x.md", "ORIGINAL TASK TEXT")]);
        let original = wt.path().join(".workmux/PROMPT-x.md");

        let out = write_resurrect_prompt(wt.path(), "x", &original).unwrap();
        let body = fs::read_to_string(&out).unwrap();

        assert!(body.contains("ORIGINAL TASK TEXT"), "task text must survive");
        assert!(body.contains("restored"), "must explain the restart");
        // The durable record is never overwritten by a resurrection.
        assert_eq!(
            fs::read_to_string(&original).unwrap(),
            "ORIGINAL TASK TEXT"
        );
        assert_ne!(out, original);
    }

    #[test]
    fn missing_session_store_is_not_resumable() {
        let wt = worktree_with(&[]);
        assert!(matches!(
            plan_resume(wt.path(), "nope", "claude"),
            ResumePlan::None
        ));
        // An agent with neither forker nor continue flag can never claim a
        // resumable session.
        assert!(matches!(
            plan_resume(wt.path(), "nope", "definitely-not-an-agent"),
            ResumePlan::None
        ));
    }

    #[test]
    fn continue_flag_agents_resume_unverified() {
        // opencode's sessions live in SQLite (unreadable here), but the CLI has
        // `--continue`, so the plan falls back to blind latest-resume.
        let wt = worktree_with(&[]);
        assert!(matches!(
            plan_resume(wt.path(), "x", "opencode"),
            ResumePlan::Latest
        ));
        assert!(matches!(
            plan_resume(wt.path(), "x", "copilot"),
            ResumePlan::Latest
        ));
    }

    #[test]
    fn journal_prefers_matching_agent_session() {
        use crate::project_state::types::{SessionRecord, WorktreeRecord};

        let record = WorktreeRecord {
            branch: None,
            parent: None,
            agent: Some("claude".to_string()),
            sessions: vec![
                SessionRecord {
                    agent: "opencode".to_string(),
                    id: "opencode-new".to_string(),
                    started_at: 200,
                    ended_at: None,
                },
                SessionRecord {
                    agent: "claude".to_string(),
                    id: "claude-old".to_string(),
                    started_at: 100,
                    ended_at: None,
                },
            ],
            spawned_at: None,
            last_seen: None,
        };

        // A newer session belonging to a different agent must not be offered to
        // claude — that unreachable session is what killed the restored windows.
        assert_eq!(record.latest_session_for("claude").unwrap().id, "claude-old");
        assert!(record.latest_session_for("gemini").is_none());
    }
}
