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

    // Only an id the agent actually reported. The ADE does not validate what it
    // is handed -- importing a session that does not exist returns `created` with
    // a live agent id -- so a guess here would manufacture a junk agent.
    let session_id = state.provider_session_id.clone().ok_or_else(|| {
        anyhow::anyhow!(
            "agent {} has never reported its provider session id, so there is \
             nothing to import; its extension must send it (muxix signal \
             --session-id)",
            agent.to_wire()
        )
    })?;

    let provider = state.agent_kind.clone().ok_or_else(|| {
        anyhow::anyhow!("muxix does not know which provider agent {} runs", agent.to_wire())
    })?;

    let runtime = crate::agent::runtime::registry::RuntimeRegistry::for_config(&cfg)
        .get(&name)
        .ok_or_else(|| anyhow::anyhow!("agent manager '{name}' is not registered"))?;

    let imported = runtime.import(&crate::agent::runtime::ExternalSession {
        session_id,
        provider,
        cwd: state.workdir.clone(),
        title: state.pane_title.clone(),
    })?;

    println!("imported {} into {name} as {}", agent.to_wire(), imported.to_wire());
    Ok(())
}
