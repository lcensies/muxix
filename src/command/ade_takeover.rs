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

/// Take the session over for real.
///
/// `stop_ade` is the serialization step, and it stops the manager's agent for
/// this one id -- never every session on the host. On the command line it must be
/// asked for with `--stop-ade`; the session view's takeover key passes it, since
/// pressing a takeover key is itself the request.
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
        //
        // Do NOT reach for `agent archive` or `agent delete` to make this bite
        // harder. The session must stay resumable in the manager afterwards, and
        // archive has no inverse in paseo's CLI (`agent update` cannot unset it;
        // `agent ls -a` only lists archived agents), while delete is a hard
        // delete. Takeover is a loan, not a transfer.
        ade.stop(agent)?;
    }

    // Its own placement, not the view's: a resumed provider session owns a
    // transcript and must not be closed by closing the view that launched it.
    let (placement, window) = match ade.view() {
        Some(v) => (
            v.takeover_placement,
            crate::command::ade_view::render_window_name(&v.window_name, agent),
        ),
        None => (crate::config::AdeViewPlacement::default(), None),
    };
    let pane = crate::multiplexer::launch::open_argv_pane(
        mux,
        &plan.argv,
        &plan.cwd,
        placement,
        window.as_deref(),
    )?;
    Ok(format!(
        "resumed {} session {} in pane {pane}; `muxix agents import {}` returns it to {}",
        plan.provider,
        plan.handle,
        agent.to_wire(),
        ade.name()
    ))
}
