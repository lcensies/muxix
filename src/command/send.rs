use std::io::{IsTerminal, Read};

use anyhow::{Result, anyhow};

use crate::config;
use crate::multiplexer::{create_backend, detect_backend};
use crate::workflow;

pub fn run(name: &str, text: Option<&str>, file: Option<&str>) -> Result<()> {
    let cfg = config::Config::load(None).unwrap_or_default();

    // A namespaced reference (`<runtime>:<id>`, as printed by `muxix agents`)
    // addresses an agent another runtime owns. Route it there: the project's
    // configured runtime decides who *creates* agents, not who we may talk to.
    let reference = crate::agent::runtime::AgentRef::from_wire(name);
    if !reference.is_local() {
        let content = read_content(text, file)?;
        let registry = crate::agent::runtime::registry::RuntimeRegistry::for_config(&cfg);
        let runtime = registry.runtime_for(&reference)?;
        if !runtime
            .features()
            .has(crate::agent::runtime::Feature::Send)
        {
            return Err(crate::agent::runtime::unsupported(
                runtime.name(),
                crate::agent::runtime::Feature::Send,
            ));
        }
        return runtime.send(&reference, &content);
    }

    let mux = create_backend(detect_backend());
    let (_path, agent) = workflow::resolve_worktree_agent(name, mux.as_ref())?;

    // Determine content: positional arg > --file > stdin
    let content = if let Some(t) = text {
        t.to_string()
    } else if let Some(f) = file {
        std::fs::read_to_string(f)?
    } else {
        // Guard: don't block on interactive TTY
        if std::io::stdin().is_terminal() {
            return Err(anyhow!(
                "No content to send. Provide text argument, --file, or pipe stdin"
            ));
        }
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        buf
    };

    // Strip trailing newline
    let content = content.trim_end_matches('\n');

    if content.is_empty() {
        return Err(anyhow!("No content to send"));
    }

    // Single-line: use send_keys_to_agent (handles Claude's ! prefix delay)
    // Multi-line: use paste_multiline (already sends Enter in both backends)
    if content.contains('\n') {
        mux.paste_multiline(&agent.pane_id, content)?;
    } else {
        mux.send_keys_to_agent(&agent.pane_id, content, cfg.agent.as_deref())?;
    }

    // New instruction: clear any stale completion claim (D2). Best-effort.
    crate::state::persist_agent_completion(mux.as_ref(), &agent.pane_id, None);

    Ok(())
}

/// Content from the positional arg, `--file`, or stdin — the same precedence
/// the local path uses, factored out so a runtime-routed send behaves the same.
fn read_content(text: Option<&str>, file: Option<&str>) -> Result<String> {
    let content = if let Some(t) = text {
        t.to_string()
    } else if let Some(f) = file {
        std::fs::read_to_string(f)?
    } else {
        if std::io::stdin().is_terminal() {
            return Err(anyhow!(
                "No content to send. Provide text argument, --file, or pipe stdin"
            ));
        }
        let mut buf = String::new();
        std::io::stdin().read_to_string(&mut buf)?;
        buf
    };
    let content = content.trim_end_matches('\n').to_string();
    if content.is_empty() {
        return Err(anyhow!("No content to send"));
    }
    Ok(content)
}

#[cfg(test)]
mod tests {
    use crate::agent::runtime::AgentRef;

    #[test]
    fn worktree_names_are_not_mistaken_for_runtime_refs() {
        // Handles are bare words; only a `<runtime>:<id>` form routes elsewhere.
        assert!(AgentRef::from_wire("my-feature").is_local());
        assert!(AgentRef::from_wire("%3").is_local());
        assert!(!AgentRef::from_wire("paseo:agt_1").is_local());
    }

    // Serializes tests that mutate XDG_STATE_HOME, matching the pattern in
    // command/setup/tests.rs.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn send_clears_completion_after_delivering_keys() {
        // Exercises the state-store clear directly (D2): skips the tmux part
        // since `run()` requires a live pane to resolve an agent.
        use crate::multiplexer::{create_backend, detect_backend};
        use crate::state::{AgentState, Completion, CompletionKind, PaneKey, StateStore};
        use std::path::PathBuf;

        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let prev = std::env::var("XDG_STATE_HOME").ok();
        unsafe {
            std::env::set_var("XDG_STATE_HOME", dir.path());
        }

        let mux = create_backend(detect_backend());
        let key = PaneKey {
            backend: mux.name().to_string(),
            instance: mux.instance_id(),
            pane_id: "%wmtest-send-clear".to_string(),
        };
        let state = AgentState {
            agent_id: "test-agent".to_string(),
            pane_key: key.clone(),
            workdir: PathBuf::from("/tmp"),
            status: None,
            status_ts: None,
            pane_title: None,
            pane_pid: 1,
            command: "node".to_string(),
            updated_ts: 1,
            window_name: None,
            session_name: None,
            boot_id: None,
            agent_kind: None,
            sandbox_id: None,
            checkpoint_path: None,
            checkpoint_ts: None,
            pipeline_node_id: None,
            pipeline_node_title: None,
            runtime: None,
            completion: Some(Completion {
                kind: CompletionKind::Completed,
                feedback: Some("all tests pass".to_string()),
                ts: 1,
            }),
        };
        let store = StateStore::new().unwrap();
        store.upsert_agent(&state).unwrap();

        crate::state::persist_agent_completion(mux.as_ref(), &key.pane_id, None);

        let after = store.get_agent(&key).unwrap().unwrap();
        assert_eq!(after.completion, None);

        unsafe {
            match prev {
                Some(v) => std::env::set_var("XDG_STATE_HOME", v),
                None => std::env::remove_var("XDG_STATE_HOME"),
            }
        }
    }
}
