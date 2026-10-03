//! `muxix project-state …` — read/write CLI over the per-project runtime
//! state store, intended for preflight checks and harness bash gates.
//!
//! Mutating subcommands (`acquire`/`heartbeat`/`done`/`release`) take an
//! `--owner` so a gate can coordinate the `Absent` → `InProgress` → `Done`
//! lifecycle of a setup step. Outcomes are printed as a single lowercase word
//! to stdout (so bash can `case` on them) and the process exits 0 on success,
//! making the commands safe under `set -e`.

use anyhow::{Context, Result};
use clap::Subcommand;

use crate::project_state::{AcquireOutcome, DEFAULT_CAPABILITY_TTL_SECS, ProjectStateStore};

#[derive(Subcommand, Debug)]
pub enum ProjectStateCommand {
    /// Compare-and-swap acquire a capability for an owner. Prints one of
    /// `acquired` / `reclaimed` / `held` / `done`.
    Acquire {
        /// Capability name (e.g. "build", "test").
        name: String,
        /// Opaque, stable owner id for this actor (e.g. the gate's `$$` or task id).
        #[arg(long)]
        owner: String,
        /// Heartbeat staleness TTL in seconds before a crashed owner is reclaimable.
        #[arg(long, default_value_t = DEFAULT_CAPABILITY_TTL_SECS)]
        ttl: u64,
    },
    /// Refresh the heartbeat for a capability you own. Prints `ok` or `lost`.
    Heartbeat {
        name: String,
        #[arg(long)]
        owner: String,
    },
    /// Mark a capability `done`. Prints `ok` or `denied` (a different live
    /// owner holds it).
    Done {
        name: String,
        #[arg(long)]
        owner: String,
    },
    /// Reset a capability back to `absent`. Prints `ok` or `denied`.
    Release {
        name: String,
        #[arg(long)]
        owner: String,
    },
    /// Print a capability's current status (`absent` / `in_progress` / `done`).
    GetCapability { name: String },
    /// Record a fact discovered/produced by setup (e.g. `test_command`).
    SetFact { key: String, value: String },
    /// Print a fact's value. Exits non-zero if the fact is not set.
    GetFact { key: String },
    /// Print the whole state document as pretty JSON.
    Show,
}

pub fn run(command: ProjectStateCommand) -> Result<()> {
    let cwd = std::env::current_dir().context("Failed to determine current directory")?;
    let store = ProjectStateStore::open(&cwd)?;

    match command {
        ProjectStateCommand::Acquire { name, owner, ttl } => {
            let outcome = store.acquire(&name, &owner, ttl)?;
            println!("{}", outcome.as_str());
            if let AcquireOutcome::Reclaimed { previous_owner } = &outcome
                && let Some(prev) = previous_owner
            {
                tracing::info!(capability = %name, %prev, "reclaimed stale capability");
            }
        }
        ProjectStateCommand::Heartbeat { name, owner } => {
            let ok = store.heartbeat(&name, &owner)?;
            println!("{}", if ok { "ok" } else { "lost" });
        }
        ProjectStateCommand::Done { name, owner } => {
            let ok = store.complete(&name, &owner)?;
            println!("{}", if ok { "ok" } else { "denied" });
        }
        ProjectStateCommand::Release { name, owner } => {
            let ok = store.release(&name, &owner)?;
            println!("{}", if ok { "ok" } else { "denied" });
        }
        ProjectStateCommand::GetCapability { name } => {
            println!("{}", store.get_capability(&name)?.status.as_str());
        }
        ProjectStateCommand::SetFact { key, value } => {
            store.set_fact(&key, &value)?;
        }
        ProjectStateCommand::GetFact { key } => match store.get_fact(&key)? {
            Some(value) => println!("{value}"),
            None => std::process::exit(1),
        },
        ProjectStateCommand::Show => {
            println!("{}", store.show()?);
        }
    }
    Ok(())
}
