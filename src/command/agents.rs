//! `muxix agents` — every agent, whoever owns it.
//!
//! `muxix status` shows agents in the local repo's worktrees, which is a
//! worktree-shaped view and stays that way. This is the runtime-shaped view:
//! agents muxix started in a pane sit next to agents an ADE started and is
//! driving from someone's phone. Process ownership is exclusive; seeing them is
//! not.

use anyhow::Result;

use crate::agent::runtime::registry::RuntimeRegistry;

pub fn run(json: bool) -> Result<()> {
    let cfg = crate::config::Config::load(None).unwrap_or_default();
    let (agents, problems) = RuntimeRegistry::for_config(&cfg).list_agents();

    if json {
        let rows: Vec<serde_json::Value> = agents
            .iter()
            .map(|a| {
                serde_json::json!({
                    "ref": a.reference.to_wire(),
                    "runtime": a.reference.runtime,
                    "id": a.reference.id,
                    "status": a.status.as_str(),
                    "workdir": a.workdir,
                    "title": a.title,
                    "kind": a.kind,
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "agents": rows,
                "unavailable": problems
                    .iter()
                    .map(|(n, r)| serde_json::json!({"runtime": n, "reason": r}))
                    .collect::<Vec<_>>(),
            }))?
        );
        return Ok(());
    }

    if agents.is_empty() && problems.is_empty() {
        println!("No agents");
    }
    for a in &agents {
        let title = a.title.as_deref().unwrap_or("");
        let dir = a
            .workdir
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        println!(
            "{:<10} {:<9} {:<24} {}",
            a.reference.runtime,
            a.status.as_str(),
            a.reference.id,
            if title.is_empty() {
                dir
            } else {
                title.to_string()
            }
        );
    }
    // An unreachable runtime is reported, never silently treated as owning no
    // agents — that would hide every agent it holds.
    for (name, reason) in &problems {
        eprintln!("{name}: unavailable ({reason})");
    }
    Ok(())
}

/// `muxix agents stop <ref>` — end an agent, whichever runtime owns it.
pub fn stop(reference: &str) -> Result<()> {
    let cfg = crate::config::Config::load(None).unwrap_or_default();
    let agent = crate::agent::runtime::AgentRef::from_wire(reference);
    let runtime = RuntimeRegistry::for_config(&cfg).runtime_for(&agent)?;
    runtime.stop(&agent)?;
    println!("stopped {}", agent.to_wire());
    Ok(())
}

/// `muxix agents takeover <ref>` — resume an ADE session in the provider's own
/// CLI, in a local pane.
///
/// This is the only path that puts a second writer on a session, so it refuses
/// until writers are serialized. The ordinary session view needs none of this:
/// it is a client of the manager's daemon and works alongside every other
/// device.
pub fn takeover(reference: &str, stop_ade: bool) -> Result<()> {
    let cfg = crate::config::Config::load(None).unwrap_or_default();
    let agent = crate::agent::runtime::AgentRef::from_wire(reference);
    let ade = crate::agent::ade::for_name(&cfg, &agent.runtime)
        .ok_or_else(|| anyhow::anyhow!("'{}' is not a configured agent manager", agent.runtime))?;
    let mux = crate::multiplexer::create_backend(crate::multiplexer::detect_backend());
    let msg = crate::command::ade_takeover::execute(&ade, &agent, stop_ade, mux.as_ref())?;
    println!("{msg}");
    Ok(())
}

/// `muxix agents import <ref>` — hand a local session to an ADE, so its
/// desktop, web and phone clients can drive it.
///
/// The session itself does not move: the ADE adopts the provider session muxix
/// already has, which is the same identifier takeover resumes. That is what
/// makes one session reachable from a terminal and a phone.
pub fn import(reference: &str, ade_name: Option<&str>) -> Result<()> {
    let cfg = crate::config::Config::load(None).unwrap_or_default();
    let agent = crate::agent::runtime::AgentRef::from_wire(reference);

    if !agent.is_local() {
        anyhow::bail!(
            "{} already belongs to runtime '{}'; import adopts a local session",
            agent.to_wire(),
            agent.runtime
        );
    }

    // Pick the target ADE: the named one, else the only configured one. Guessing
    // between several would hand the session to the wrong product.
    let ades = crate::projects::sync::effective_ades(&cfg);
    let name: String = match ade_name {
        Some(want) => {
            if !ades.contains_key(want) {
                anyhow::bail!("no configured agent manager named '{want}'");
            }
            want.to_string()
        }
        None => match ades.len() {
            0 => anyhow::bail!("no agent manager is configured to import into"),
            1 => ades.keys().next().expect("length checked").clone(),
            _ => anyhow::bail!(
                "several agent managers are configured ({}); name one with --ade",
                ades.keys().cloned().collect::<Vec<_>>().join(", ")
            ),
        },
    };

    let mux = crate::multiplexer::create_backend(crate::multiplexer::detect_backend());
    let key = crate::state::PaneKey {
        backend: mux.name().to_string(),
        instance: mux.instance_id(),
        pane_id: agent.id.clone(),
    };
    let state = crate::state::StateStore::new()?
        .get_agent(&key)?
        .ok_or_else(|| anyhow::anyhow!("muxix has no record of agent {}", agent.to_wire()))?;

    let runtime = crate::agent::runtime::registry::RuntimeRegistry::for_config(&cfg)
        .get(&name)
        .ok_or_else(|| anyhow::anyhow!("agent manager '{name}' is not registered"))?;

    let imported = runtime.import(&import_session(&agent.to_wire(), &state)?)?;

    println!(
        "imported {} into {name} as {}",
        agent.to_wire(),
        imported.to_wire()
    );
    Ok(())
}

/// What an ADE needs to adopt `state`'s session, or why there is nothing to adopt.
fn import_session(
    wire: &str,
    state: &crate::state::AgentState,
) -> Result<crate::agent::runtime::ExternalSession> {
    // Only an id the agent actually reported. The ADE does not validate what it
    // is handed -- importing a session that does not exist returns `created` with
    // a live agent id -- so a guess here would manufacture a junk agent.
    let session_id = state.provider_session_id.clone().ok_or_else(|| {
        anyhow::anyhow!(
            "agent {wire} has never reported its provider session id, so there is \
             nothing to import; its extension must send it (muxix signal --session-id)"
        )
    })?;
    let provider = state
        .agent_kind
        .clone()
        .ok_or_else(|| anyhow::anyhow!("muxix does not know which provider agent {wire} runs"))?;
    Ok(crate::agent::runtime::ExternalSession {
        session_id,
        provider,
        cwd: state.workdir.clone(),
        title: state.pane_title.clone(),
    })
}

/// `muxix agents view <ref>` — open a manager's session, or focus it when it is
/// already open.
///
/// The same call the sidebar makes on select, exposed so switching to a session
/// does not require opening a TUI first.
pub fn view(reference: &str) -> Result<()> {
    let agent = crate::agent::runtime::AgentRef::from_wire(reference);
    if agent.is_local() {
        anyhow::bail!(
            "{} is a muxix agent with a pane of its own; `muxix agents switch` focuses it",
            agent.to_wire()
        );
    }
    let mux = crate::multiplexer::create_backend(crate::multiplexer::detect_backend());
    let pane = crate::command::ade_view::open_or_focus(mux.as_ref(), &agent)?;
    // Desktop hand-off opens no pane, so there is nothing to name.
    if !pane.is_empty() {
        println!("{pane}");
    }
    Ok(())
}

/// The confirmation question for deleting `agent`, naming every place it is
/// removed from and whether that can be undone. Key hints are the caller's,
/// since tmux's native prompt and the sidebar's own accept different keys.
pub fn delete_prompt(agent: &crate::multiplexer::AgentPane) -> String {
    let reference = crate::agent::runtime::AgentRef::from_wire(&agent.pane_id);
    let name = agent.pane_title.as_deref().unwrap_or(&agent.window_name);
    if reference.is_local() {
        return if agent.status == Some(crate::multiplexer::AgentStatus::Exited) {
            format!("Forget exited agent {name}? Kills its pane if it is still running.")
        } else {
            format!("Delete {name}? Kills its pane.")
        };
    }
    let cfg = crate::config::Config::load(None).unwrap_or_default();
    match crate::agent::ade::for_name(&cfg, &reference.runtime) {
        Some(ade) if ade.can_delete() => format!(
            "Delete {name} from muxix AND {}? Cannot be undone, gone from every device.",
            reference.runtime
        ),
        _ => format!(
            "Forget {name} in muxix? {} keeps its copy.",
            reference.runtime
        ),
    }
}

/// Delete an agent: from its manager first, then from muxix.
///
/// Takes the listed row rather than a bare ref because a pane id alone is not
/// proof of anything: an exited agent's pane id may since belong to an unrelated
/// process, so a pane is killed only when reconcile has just verified it is this
/// agent's.
pub fn delete_agent(
    mux: &dyn crate::multiplexer::Multiplexer,
    agent: &crate::multiplexer::AgentPane,
) -> Result<String> {
    let store = crate::state::StateStore::new()?;
    let cfg = crate::config::Config::load(None).unwrap_or_default();
    delete_agent_in(mux, agent, &store, &cfg, &|| {
        crate::command::sidebar::refresh_daemon_ade()
    })
}

/// As [`delete_agent`], against a given store and config, telling
/// `invalidate_cache` to drop the daemon's cached listing.
///
/// Everything a delete reaches outside the multiplexer is passed in: the state
/// store, the manager definitions, and the daemon signal — the last because it
/// resolves the daemon's pid from the live tmux server, which a test must not
/// touch.
fn delete_agent_in(
    mux: &dyn crate::multiplexer::Multiplexer,
    agent: &crate::multiplexer::AgentPane,
    store: &crate::state::StateStore,
    cfg: &crate::config::Config,
    invalidate_cache: &dyn Fn(),
) -> Result<String> {
    let reference = crate::agent::runtime::AgentRef::from_wire(&agent.pane_id);
    let wire = reference.to_wire();

    // Before anything that can fail, hang or complete asynchronously: a delete
    // the user confirmed must not leave the row listed, whatever answer the
    // manager gives and whatever its cached listing still reports.
    if let Err(error) = store.tombstone(&wire) {
        tracing::warn!(%wire, error = %error, "failed to record a delete tombstone");
    }

    if !reference.is_local() {
        let deleted = match crate::agent::ade::for_name(cfg, &reference.runtime) {
            Some(ade) if ade.can_delete() => Some(ade.delete(&reference)),
            _ => None,
        };
        crate::command::ade_view::forget_view(mux, store, &wire);
        // Every arm, not only the successful delete: the daemon caches the
        // manager's listing, and left alone it keeps offering this session for
        // the cache's lifetime. Selecting that stale row is what created a
        // project session and a view pane for an agent that no longer exists.
        invalidate_cache();
        return Ok(match deleted {
            Some(Ok(())) => format!("deleted {wire} in {} and muxix", reference.runtime),
            Some(Err(e)) => format!(
                "removed {wire} from muxix, but {} did not delete it: {e}",
                reference.runtime
            ),
            None => format!("forgot {wire}; {} keeps it", reference.runtime),
        });
    }

    // The stored status is not trusted: a record reads `Exited` as soon as the
    // pane's foreground command stops matching, which also happens while the
    // provider is alive. Liveness of *this* agent's pane decides instead, so a
    // recycled pane id is never killed and a falsely-exited agent still is.
    if pane_should_be_killed(mux.get_all_live_pane_info(), &agent.pane_id, agent.pane_pid) {
        let _ = mux.kill_pane(&agent.pane_id);
    }
    store.delete_agent(&crate::state::PaneKey {
        backend: mux.name().to_string(),
        instance: mux.instance_id(),
        pane_id: agent.pane_id.clone(),
    })?;
    clear_agent_identity(mux, agent);
    Ok(format!("deleted {}", agent.pane_id))
}

/// Is `pane_id` still the pane this agent recorded, and so the pane to kill?
///
/// An unanswerable query is not evidence either way, and the two mistakes are
/// not symmetric: skipping the kill leaves the agent running in a pane that is
/// now tombstoned, so muxix will never adopt or list it again. A failed query
/// therefore kills.
fn pane_should_be_killed(
    live: Result<std::collections::HashMap<String, crate::multiplexer::LivePaneInfo>>,
    pane_id: &str,
    pane_pid: u32,
) -> bool {
    match live {
        Ok(live) => pane_matches(live.get(pane_id), pane_pid),
        Err(error) => {
            tracing::warn!(
                pane_id,
                error = %error,
                "could not list panes while deleting; killing the recorded pane anyway"
            );
            true
        }
    }
}

/// Does `live` describe the pane that `pane_pid` was recorded in?
///
/// A pane id outlives the agent that ran in it and tmux reuses it, so "the pane
/// exists" is not enough — the pid has to match too. A stored pid of 0 means the
/// record was adopted before any pid was known, so the pane's existence is then
/// the only evidence there is.
fn pane_matches(live: Option<&crate::multiplexer::LivePaneInfo>, pane_pid: u32) -> bool {
    live.is_some_and(|pane| pane_pid == 0 || pane.pid.is_none_or(|pid| pid == pane_pid))
}

/// Clear what a deleted agent would still be recognised by once its record is
/// gone: the window status markers, its pane-keyed signal files, and the
/// journalled session id a later `--continue` would resume.
///
/// Stored task prompts are deliberately left in place — they are the user's
/// text, not an identity.
fn clear_agent_identity(
    mux: &dyn crate::multiplexer::Multiplexer,
    agent: &crate::multiplexer::AgentPane,
) {
    let _ = mux.clear_status(&agent.pane_id);
    for path in crate::signals::paths::pane_signal_files(&agent.pane_id) {
        let _ = std::fs::remove_file(path);
    }
    // Resolved from the agent's own worktree, not the cwd: delete may be run
    // from a pane in another project, or in none. The journal is project-wide,
    // so it has to be opened at the main worktree root the way
    // `record_session` does.
    if let Some(handle) = agent.path.file_name().and_then(|n| n.to_str())
        && let Ok(root) = crate::git::get_main_worktree_root_in(Some(&agent.path))
        && let Ok(journal) = crate::project_state::ProjectStateStore::open(&root)
        && let Err(error) = journal.forget_sessions(handle)
    {
        tracing::debug!(handle, error = %error, "failed to clear journalled sessions");
    }
}

/// `muxix _delete-agent <ref>` — what a native confirmation runs once accepted.
///
/// The row is looked up again here rather than trusted from when the prompt
/// opened: the agent may have exited or its pane been recycled while the
/// question was on screen, and `delete_agent`'s safety checks need the current
/// state. Always succeeds as a process — tmux would put a failing exit status
/// in a view-mode pane — and reports through the status line instead.
pub fn delete_by_ref(reference: &str) -> Result<()> {
    let mux = crate::multiplexer::create_backend(crate::multiplexer::detect_backend());
    let cfg = crate::config::Config::load(None).unwrap_or_default();
    // Still reported through the status line rather than as an exit status: a
    // failing process here puts tmux's own error in a view-mode pane.
    let msg = match crate::state::StateStore::new() {
        Err(e) => format!("delete failed, nothing removed: {e}"),
        Ok(store) => delete_by_ref_in(mux.as_ref(), reference, &store, &cfg, &|| {
            crate::command::sidebar::refresh_daemon_ade()
        }),
    };
    mux.display_message(&msg);
    crate::command::sidebar::refresh_daemon();
    Ok(())
}

/// What [`delete_by_ref`] reports, against a given store and config.
fn delete_by_ref_in(
    mux: &dyn crate::multiplexer::Multiplexer,
    reference: &str,
    store: &crate::state::StateStore,
    cfg: &crate::config::Config,
    invalidate_cache: &dyn Fn(),
) -> String {
    match find_listed(mux, reference, store, cfg) {
        None => format!("{reference} is no longer listed; nothing deleted"),
        Some(row) => match delete_agent_in(mux, &row, store, cfg, invalidate_cache) {
            Ok(m) => m,
            Err(e) => format!("delete failed, nothing removed: {e}"),
        },
    }
}

/// The listed row for `reference`, as reconcile or the manager sees it now.
fn find_listed(
    mux: &dyn crate::multiplexer::Multiplexer,
    reference: &str,
    store: &crate::state::StateStore,
    cfg: &crate::config::Config,
) -> Option<crate::multiplexer::AgentPane> {
    let agent = crate::agent::runtime::AgentRef::from_wire(reference);
    if agent.is_local() {
        store
            .load_reconciled_agents(mux)
            .ok()?
            .into_iter()
            .find(|a| a.pane_id == agent.id)
    } else {
        crate::agent::runtime::registry::foreign_agent_panes(cfg)
            .0
            .into_iter()
            .find(|a| a.pane_id == agent.to_wire())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::multiplexer::testing::FakeMux;

    fn state(session: Option<&str>, kind: Option<&str>) -> crate::state::AgentState {
        serde_json::from_value(serde_json::json!({
            "pane_key": {"backend": "tmux", "instance": "/tmp/s", "pane_id": "%7"},
            "workdir": "/repo/wt",
            "pane_pid": 1,
            "command": "pi",
            "updated_ts": 0,
            "pane_title": "fix parser",
            "provider_session_id": session,
            "agent_kind": kind,
        }))
        .unwrap()
    }

    fn live(pid: Option<u32>) -> crate::multiplexer::LivePaneInfo {
        crate::multiplexer::LivePaneInfo {
            pid,
            current_command: Some("zsh".to_string()),
            working_dir: std::path::PathBuf::from("/repo/wt"),
            title: None,
            session: None,
            window: None,
        }
    }

    #[test]
    fn delete_kills_a_falsely_exited_pane_and_spares_a_recycled_one() {
        // The status is never consulted: a record reads `Exited` the moment the
        // pane's foreground command stops matching, which also happens while
        // the provider is alive and owns the pane.
        assert!(pane_matches(Some(&live(Some(4242))), 4242), "same pane");
        // The pane id outlives the agent and tmux reuses it. Killing it then
        // would take down an unrelated process.
        assert!(!pane_matches(Some(&live(Some(99))), 4242), "recycled id");
        // A pane that is genuinely gone: nothing to kill, and delete must not
        // treat that as an error.
        assert!(!pane_matches(None, 4242), "pane gone");
        // Backends that expose no pid, and records adopted before one was
        // known, have only the pane's existence to go on.
        assert!(pane_matches(Some(&live(None)), 4242));
        assert!(pane_matches(Some(&live(Some(99))), 0));
    }

    #[test]
    fn an_unanswerable_pane_query_still_kills() {
        // Left unkilled, the agent keeps running in a pane that this delete
        // has just tombstoned: never re-adopted, never listed again.
        assert!(pane_should_be_killed(
            Err(anyhow::anyhow!("tmux server reloading")),
            "%7",
            4242
        ));
        assert!(!pane_should_be_killed(
            Ok(std::collections::HashMap::from([(
                "%7".to_string(),
                live(Some(99))
            )])),
            "%7",
            4242
        ));

        // The same through the whole delete, against a backend whose pane
        // query is unanswerable.
        let (store, _dir) = test_store();
        let wt = tempfile::tempdir().unwrap();
        let state = local_state(wt.path(), "%7", 4242, "claude");
        store.upsert_agent(&state).unwrap();
        let mux = FakeMux::default().failing_pane_query();
        delete_agent_in(
            &mux,
            &row(&state),
            &store,
            &crate::config::Config::default(),
            &|| {},
        )
        .unwrap();
        assert!(mux.calls().contains(&"kill_pane %7".to_string()));
    }

    /// A store of its own, so no test reads or writes the process's state.
    fn test_store() -> (crate::state::StateStore, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::state::StateStore::with_path(dir.path().to_path_buf()).unwrap();
        (store, dir)
    }

    /// A local record keyed the way [`FakeMux`] reports itself, so the store
    /// and the double agree on which pane it is.
    fn local_state(
        workdir: &std::path::Path,
        pane_id: &str,
        pid: u32,
        command: &str,
    ) -> crate::state::AgentState {
        serde_json::from_value(serde_json::json!({
            "pane_key": {"backend": "tmux", "instance": "test", "pane_id": pane_id},
            "workdir": workdir,
            "pane_pid": pid,
            "command": command,
            "updated_ts": 0,
            "agent_kind": "claude",
        }))
        .unwrap()
    }

    /// A stand-in manager CLI. Every subcommand fails, so a delete that reaches
    /// it reports the manager's refusal; `list_args` answers from `list.json`.
    fn stub_ade(dir: &std::path::Path, ade: crate::config::AdeConfig) -> crate::config::Config {
        let script = dir.join("stub-ade");
        std::fs::write(
            &script,
            format!(
                "#!/bin/sh\nif [ \"$1\" = list ]; then cat {}/list.json; exit 0; fi\n\
                 echo 'no such agent' >&2\nexit 1\n",
                dir.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&script, std::os::unix::fs::PermissionsExt::from_mode(0o755))
            .unwrap();
        let mut cfg = crate::config::Config::default();
        cfg.ade.insert(
            "stub".to_string(),
            crate::config::AdeConfig {
                command: script.to_string_lossy().into_owned(),
                list_args: vec!["list".to_string()],
                ..ade
            },
        );
        cfg
    }

    fn row(state: &crate::state::AgentState) -> crate::multiplexer::AgentPane {
        state.to_agent_pane(String::new(), String::new())
    }

    #[test]
    fn a_falsely_exited_agent_does_not_come_back_after_a_reconcile() {
        // The record reads `Exited` because the pane's foreground command
        // changed, while the provider is in fact alive and owns the pane.
        let (store, _dir) = test_store();
        let wt = tempfile::tempdir().unwrap();
        let mut state = local_state(wt.path(), "%7", 4242, "node");
        state.status = Some(crate::multiplexer::AgentStatus::Exited);
        store.upsert_agent(&state).unwrap();
        let mux = FakeMux::default().with_pane("%7", Some(4242), "claude");

        let msg = delete_agent_in(
            &mux,
            &row(&state),
            &store,
            &crate::config::Config::default(),
            &|| {},
        )
        .unwrap();

        assert_eq!(msg, "deleted %7");
        assert!(
            mux.calls().contains(&"kill_pane %7".to_string()),
            "the live provider's pane must die: {:?}",
            mux.calls()
        );
        // The double keeps the pane listed, which is the case that matters: a
        // kill that did not take still leaves a provider pane behind, and
        // reconcile would otherwise adopt it and put the row back.
        assert!(
            store.load_reconciled_agents(&mux).unwrap().is_empty(),
            "the deleted row reappeared"
        );
    }

    #[test]
    fn delete_clears_the_identities_a_new_agent_would_inherit() {
        let (store, _dir) = test_store();
        let wt = tempfile::tempdir().unwrap();
        let prompt = wt.path().join(".muxix/PROMPT-task.md");
        std::fs::create_dir_all(prompt.parent().unwrap()).unwrap();
        std::fs::write(&prompt, "the user's text").unwrap();

        // A pane id no real agent would hold: the signal files live in the
        // shared temp dir, keyed by pane id alone.
        let pane = "%99991";
        let signals = crate::signals::paths::pane_signal_files(pane);
        for path in &signals {
            std::fs::write(path, "{}").unwrap();
        }

        let state = local_state(wt.path(), pane, 4242, "claude");
        store.upsert_agent(&state).unwrap();
        let mux = FakeMux::default().with_pane(pane, Some(4242), "claude");

        delete_agent_in(
            &mux,
            &row(&state),
            &store,
            &crate::config::Config::default(),
            &|| {},
        )
        .unwrap();

        assert!(
            mux.calls().contains(&format!("clear_status {pane}")),
            "a status glyph left on the window outlives the agent: {:?}",
            mux.calls()
        );
        for path in &signals {
            assert!(
                !path.exists(),
                "{} would release a gate for the next agent in this pane",
                path.display()
            );
        }
        assert!(prompt.exists(), "the user's prompt is not an identity");
        assert!(
            store.get_agent(&state.pane_key).unwrap().is_none(),
            "the record must be gone"
        );
    }

    #[test]
    fn a_failing_manager_delete_still_removes_the_row() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _state_dir) = test_store();
        let cfg = stub_ade(
            dir.path(),
            crate::config::AdeConfig {
                delete_args: vec!["delete".to_string(), "{id}".to_string()],
                ..Default::default()
            },
        );
        let refreshed = std::cell::Cell::new(false);
        let state = local_state(dir.path(), "stub:a1", 0, "");
        let msg = delete_agent_in(&FakeMux::default(), &row(&state), &store, &cfg, &|| {
            refreshed.set(true)
        })
        .unwrap();

        assert!(
            refreshed.get(),
            "the daemon's cached listing must be dropped"
        );
        assert!(
            msg.contains("removed stub:a1 from muxix, but stub did not delete it"),
            "the user has to be told the manager refused: {msg}"
        );
        assert!(
            store.is_tombstoned("stub:a1"),
            "a failed manager delete must not leave the row listed"
        );
    }

    #[test]
    fn the_forget_only_arm_tombstones_and_invalidates_the_cache() {
        let dir = tempfile::tempdir().unwrap();
        let (store, _state_dir) = test_store();
        // A manager that cannot delete: muxix forgets its own row and says so.
        let cfg = stub_ade(
            dir.path(),
            crate::config::AdeConfig {
                delete_args: Vec::new(),
                ..Default::default()
            },
        );
        let refreshed = std::cell::Cell::new(false);
        let state = local_state(dir.path(), "stub:a1", 0, "");
        let msg = delete_agent_in(&FakeMux::default(), &row(&state), &store, &cfg, &|| {
            refreshed.set(true)
        })
        .unwrap();

        assert_eq!(msg, "forgot stub:a1; stub keeps it");
        assert!(store.is_tombstoned("stub:a1"));
        assert!(
            refreshed.get(),
            "without this the daemon keeps offering the row from its cached listing"
        );
    }

    #[test]
    fn the_in_pane_confirm_path_deletes_what_is_listed_and_nothing_else() {
        // What a native tmux confirmation runs once accepted. The row is looked
        // up again here, because the agent may have gone while the question was
        // on screen.
        let (store, _dir) = test_store();
        let wt = tempfile::tempdir().unwrap();
        let state = local_state(wt.path(), "%7", 4242, "claude");
        store.upsert_agent(&state).unwrap();
        let mux = FakeMux::default().with_pane("%7", Some(4242), "claude");
        let cfg = crate::config::Config::default();

        let gone = delete_by_ref_in(&mux, "%404", &store, &cfg, &|| {});
        assert_eq!(gone, "%404 is no longer listed; nothing deleted");
        assert!(!store.is_tombstoned("%404"), "nothing may be recorded");
        assert!(mux.calls().is_empty(), "{:?}", mux.calls());

        let refreshed = std::cell::Cell::new(false);
        let deleted = delete_by_ref_in(&mux, "%7", &store, &cfg, &|| refreshed.set(true));
        assert_eq!(deleted, "deleted %7");
        assert!(store.is_tombstoned("%7"));
        assert!(mux.calls().contains(&"kill_pane %7".to_string()));
    }

    #[test]
    fn import_refuses_an_agent_that_never_reported_its_session() {
        // The ADE would accept a made-up id and create an agent with no session.
        let err = import_session("%7", &state(None, Some("pi")))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("never reported its provider session id"),
            "{err}"
        );
    }

    #[test]
    fn import_refuses_an_agent_of_unknown_kind() {
        let err = import_session("%7", &state(Some("s1"), None))
            .unwrap_err()
            .to_string();
        assert!(err.contains("which provider"), "{err}");
    }

    #[test]
    fn import_carries_the_recorded_session_into_the_managers_command() {
        let s = import_session("%7", &state(Some("/sessions/a.jsonl"), Some("pi"))).unwrap();
        assert_eq!(s.cwd, std::path::PathBuf::from("/repo/wt"));
        assert_eq!(s.title.as_deref(), Some("fix parser"));
        let ade = crate::agent::ade::AdeRuntime::new(
            "paseo".to_string(),
            crate::config::AdeConfig::paseo(),
        );
        let args = ade.import_args_for(&s, "");
        assert_eq!(
            args,
            [
                "agent",
                "import",
                "/sessions/a.jsonl",
                "--provider",
                "pi",
                "--cwd",
                "/repo/wt",
                "--json"
            ]
        );
    }
}
