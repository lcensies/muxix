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
