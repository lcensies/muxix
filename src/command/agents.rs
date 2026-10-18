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
            format!("Forget exited agent {name}?")
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
    let reference = crate::agent::runtime::AgentRef::from_wire(&agent.pane_id);

    if !reference.is_local() {
        let cfg = crate::config::Config::load(None).unwrap_or_default();
        let remote = match crate::agent::ade::for_name(&cfg, &reference.runtime) {
            // Manager first: if it refuses, muxix keeps its record, so the agent
            // is never forgotten here while it still exists there.
            Some(ade) if ade.can_delete() => {
                ade.delete(&reference)?;
                // The daemon caches the manager's listing; left alone it would
                // keep offering this session, and selecting it reopens a view
                // that polls a session the manager no longer has.
                crate::command::sidebar::refresh_daemon_ade();
                true
            }
            _ => false,
        };
        crate::command::ade_view::forget_view(mux, &reference.to_wire());
        return Ok(if remote {
            format!(
                "deleted {} in {} and muxix",
                reference.to_wire(),
                reference.runtime
            )
        } else {
            format!(
                "forgot {}; {} keeps it",
                reference.to_wire(),
                reference.runtime
            )
        });
    }

    if agent.status != Some(crate::multiplexer::AgentStatus::Exited) {
        let _ = mux.kill_pane(&agent.pane_id);
    }
    crate::state::StateStore::new()?.delete_agent(&crate::state::PaneKey {
        backend: mux.name().to_string(),
        instance: mux.instance_id(),
        pane_id: agent.pane_id.clone(),
    })?;
    Ok(format!("deleted {}", agent.pane_id))
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
    let msg = match find_listed(mux.as_ref(), reference) {
        None => format!("{reference} is no longer listed; nothing deleted"),
        Some(row) => match delete_agent(mux.as_ref(), &row) {
            Ok(m) => m,
            Err(e) => format!("delete failed, nothing removed: {e}"),
        },
    };
    mux.display_message(&msg);
    crate::command::sidebar::refresh_daemon();
    Ok(())
}

/// The listed row for `reference`, as reconcile or the manager sees it now.
fn find_listed(
    mux: &dyn crate::multiplexer::Multiplexer,
    reference: &str,
) -> Option<crate::multiplexer::AgentPane> {
    let agent = crate::agent::runtime::AgentRef::from_wire(reference);
    if agent.is_local() {
        crate::state::StateStore::new()
            .ok()?
            .load_reconciled_agents(mux)
            .ok()?
            .into_iter()
            .find(|a| a.pane_id == agent.id)
    } else {
        crate::agent::runtime::registry::foreign_agent_panes()
            .0
            .into_iter()
            .find(|a| a.pane_id == agent.to_wire())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
