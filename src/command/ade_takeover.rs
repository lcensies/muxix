//! Taking an ADE session over natively, and refusing to do it unsafely.
//!
//! An ADE session is also a provider session on disk, so it can be resumed in
//! the provider's own CLI (`claude --resume <handle>`) to get a native TUI.
//! That is the one path which adds a **second** writer to an append-only
//! transcript, and two processes appending to one transcript diverge in a way
//! muxix cannot reconcile.
//!
//! So takeover serializes writers for that one session, and nothing else does:
//! the ordinary session view is a client of the ADE's daemon and claims nothing,
//! which is what lets a phone, a desktop and a muxix pane use one session at
//! once. Takeover must also be reversible — a session stranded on one machine
//! fails that requirement.
//!
//! An `idle` agent is **not** an unattended one. The ADE's daemon keeps the
//! provider process resident between turns, so idle describes the turn and not
//! the absence of a writer. Inferring safety from it would be the bug this
//! module exists to avoid.

use anyhow::{Result, anyhow, bail};

use crate::agent::ade::AdeRuntime;
use crate::agent::runtime::{AgentRef, AgentRuntime, RuntimeStatus};

/// What muxix knows about taking one session over.
pub struct Takeover {
    pub handle: String,
    pub cwd: std::path::PathBuf,
    pub provider: String,
    /// The resume argv, ready to run in a pane.
    pub argv: Vec<String>,
    /// Whether the ADE still has a writer on this session.
    pub ade_holds_writer: bool,
}

/// Resolve everything takeover needs, or explain why it cannot happen.
///
/// This reads and plans only. It starts nothing: a session with a live ADE
/// writer needs a serialization choice first, and that choice is the user's.
pub fn plan(ade: &AdeRuntime, agent: &AgentRef) -> Result<Takeover> {
    let state = ade.view_state(agent)?;
    let cwd = state
        .cwd
        .ok_or_else(|| anyhow!("{agent}: the manager reports no working directory"))?;
    let provider = state
        .provider
        .ok_or_else(|| anyhow!("{agent}: the manager reports no provider"))?;

    let handle = ade.native_handle(agent, &cwd)?;

    let profile = crate::agent::profile::resolve_profile(Some(&provider));
    let flag = profile.resume_session_flag().ok_or_else(|| {
        anyhow!(
            "muxix has no verified way to resume a named {provider} session, \
             so it will not guess one; the session view keeps working"
        )
    })?;

    // A closed session has no writer left to serialize against. Every other
    // state does, resident process and all.
    let ade_holds_writer = state.status != RuntimeStatus::Done;

    Ok(Takeover {
        argv: vec![provider.clone(), flag.to_string(), handle.clone()],
        handle,
        cwd,
        provider,
        ade_holds_writer,
    })
}

/// The message the session view shows for the takeover key.
///
/// This is the first of two presses, and it changes nothing: the text names what
/// the second press will interrupt, which may be a turn running on someone
/// else's device.
pub fn offer(ade: &AdeRuntime, agent: &AgentRef) -> Result<String> {
    let plan = plan(ade, agent)?;
    if !plan.ade_holds_writer {
        return Ok("session is closed; taking over resumes it locally".to_string());
    }
    Ok(format!(
        "{} still holds this session; taking over stops its agent and interrupts \
         any turn in flight",
        ade.name()
    ))
}

/// Take the session over for real.
///
/// `stop_ade` is the serialization step. It is required whenever the ADE still
/// holds a writer, and is never implied: interrupting work someone started on
/// another device is the user's call, not a default.
pub fn execute(
    ade: &AdeRuntime,
    agent: &AgentRef,
    stop_ade: bool,
    mux: &dyn crate::multiplexer::Multiplexer,
) -> Result<String> {
    let plan = plan(ade, agent)?;

    if plan.ade_holds_writer && !stop_ade {
        bail!(
            "{} still holds a writer on this session, so resuming it locally would \
             leave two processes appending to one transcript.\n\
             \n\
             Serialize first, by preference:\n\
             \x20 1. ask {0} to release the session -- it exposes no such command today\n\
             \x20 2. re-run with --stop-ade: stops its agent, interrupting work in flight,\n\
             \x20    then `muxix agents import` hands the session back afterwards\n\
             \x20 3. do nothing and keep using the session view, which claims nothing and \
             works alongside every other device",
            ade.name()
        );
    }

    if plan.ade_holds_writer {
        // ponytail: paseo's `agent stop` is a no-op for an idle agent, and its
        // daemon keeps the provider process resident between turns, so this
        // interrupts a turn in flight but does not evict the other writer. A
        // real fix needs a release/park primitive paseo does not expose; until
        // then, driving the session from another device after takeover can still
        // append to the same transcript.
        ade.stop(agent)?;
    }

    let pane = crate::multiplexer::launch::open_argv_pane(mux, &plan.argv, &plan.cwd)?;
    Ok(format!(
        "resumed {} session {} in pane {pane}; `muxix agents import {}` returns it to {}",
        plan.provider,
        plan.handle,
        agent.to_wire(),
        ade.name()
    ))
}
