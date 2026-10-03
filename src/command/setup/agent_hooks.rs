//! Installing declared hooks into agents' native hook config.
//!
//! Hooks are declared in agent-agnostic terms (`session-ready` / `turn-done`)
//! and translated per agent. Each agent module owns its own translation by
//! exporting a [`HookTarget`] — the config file it keeps shell hooks in and the
//! native key for each event. Claude, Codex, and Gemini all store hooks in the
//! same inner JSON shape (the one muxix's own status hooks already use), and
//! OpenCode/pi consume that shape too through Claude-hooks-compat plugins
//! (auto-injected into their plugin lists, see [`plugin_to_inject`]) — so one
//! writer serves every agent that has a target. Copilot keeps its own dialect
//! (a `version`-stamped file with flat per-event entries keyed by `bash`), so
//! the writer carries a [`HookDialect`] instead of a second implementation.
//! omp has no target: its pi heritage would point at the pi hooks compat
//! plugin, which writes pi's `settings.json` — a file omp (`config.yml`) does
//! not read. That gap reports `Skipped` so it stays visible in `setup --check`
//! rather than silent.

use anyhow::{Context as _, Result};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

use crate::agent::setup::Agent;
use crate::bootstrap::{self, BootstrapConfig, HookEvent, HookSpec};
use crate::command::setup::report::{ItemResult, Outcome, Section};

/// A plugin an agent needs before declared hooks can fire (its hook system is
/// a plugin, not a native config file).
pub struct RequiredPlugin {
    /// Install spec, in the agent's own plugin format.
    pub spec: &'static str,
    /// Substring identifying the plugin in a resolved plugin list, so pinned
    /// or scheme-prefixed declarations (`npm:x@1.2.0`) suppress injection.
    pub name_fragment: &'static str,
}

/// Where one agent keeps its shell hooks, and what it calls each event.
///
/// Owned by the agent's own setup module (`claude::declared_hook_target`, …) so
/// agent-specific knowledge stays with the agent, not in a match here.
pub struct HookTarget {
    /// The JSON config file hooks live in.
    pub file: PathBuf,
    /// The file's key for an event, or `None` when the agent cannot express it.
    pub event_key: fn(HookEvent) -> Option<&'static str>,
    /// Compat plugin auto-injected into the agent's plugin install list when
    /// hooks are declared. `None` for agents with native shell-hook config.
    pub requires_plugin: Option<RequiredPlugin>,
    /// The JSON dialect this agent's hook file uses.
    pub dialect: HookDialect,
}

/// How an agent's hook file nests and names a hook command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HookDialect {
    /// Claude/Codex/Gemini and the OpenCode/pi compat plugins:
    /// `{"hooks": {"<Event>": [{"hooks": [{"type": "command", "command": …}]}]}}`.
    #[default]
    Grouped,
    /// Copilot CLI: a `version`-stamped file whose events hold flat entries
    /// keyed by the shell to run:
    /// `{"version": 1, "hooks": {"sessionStart": [{"type": "command", "bash": …}]}}`.
    CopilotFlat,
}

/// The hook target for an agent, when it has one.
fn hook_target(agent: Agent) -> Option<HookTarget> {
    use crate::agent::setup::{claude, codex, copilot, gemini, opencode, pi};
    match agent {
        Agent::Claude => claude::declared_hook_target(),
        Agent::Codex => codex::declared_hook_target(),
        Agent::Gemini => gemini::declared_hook_target(),
        // OpenCode and pi execute Claude-format command hooks through compat
        // plugins (opencode-claude-hooks / @hsingjui/pi-hooks); the target
        // carries the plugin dependency.
        Agent::OpenCode => opencode::declared_hook_target(),
        Agent::Pi => pi::declared_hook_target(),
        Agent::Copilot => copilot::declared_hook_target(),
        // omp shares pi's extension mechanism but not its settings store: the pi
        // hooks compat plugin reads pi's `settings.json`, and omp keeps settings
        // in `config.yml`, so pointing at either file installs a hook nothing
        // will run. Reported as a skip until omp grows its own hook config.
        Agent::Omp => None,
    }
}

/// The compat plugin to inject for `agent`, iff hooks are declared for it and
/// its resolved plugin list does not already carry the plugin.
pub fn plugin_to_inject(agent: Agent, config: &BootstrapConfig) -> Option<RequiredPlugin> {
    let req = hook_target(agent)?.requires_plugin?;
    let declared = !config.hooks.is_empty()
        || config
            .skill_entries_for(agent)
            .iter()
            .any(|e| !e.hooks.is_empty());
    (declared
        && !config
            .plugins_for(agent)
            .iter()
            .any(|s| s.contains(req.name_fragment)))
    .then_some(req)
}

/// Lowercase-hex sha256 of a file.
pub fn sha256_file(path: &Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path)
        .with_context(|| format!("cannot read {} for hashing", path.display()))?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(format!("{:x}", hasher.finalize()))
}

/// One hook resolved for one agent: the rendered command and its provenance.
struct ResolvedHook {
    /// The skill it came from, or `None` for a global (config-level) hook.
    origin: Option<String>,
    event: HookEvent,
    command: String,
    sha256: Option<String>,
}

impl ResolvedHook {
    fn item_name(&self) -> String {
        match &self.origin {
            Some(skill) => format!("{skill}:{}", self.event.as_str()),
            None => format!("global:{}", self.event.as_str()),
        }
    }
}

/// Best-effort extraction of the script path a command runs, for hashing.
///
/// The command is a shell line like `bash "/path/to/script.sh"`. The hash
/// applies to the *script*, so the first existing absolute path among the
/// tokens is the candidate.
fn script_path(command: &str) -> Option<PathBuf> {
    shlex::split(command)?
        .into_iter()
        .map(PathBuf::from)
        .find(|p| p.is_absolute() && p.exists())
}

/// Every hook the config declares, rendered for one agent: skill-attached
/// hooks (with `{{ skill_install_dir }}` bound to that skill's destination)
/// plus global hooks from `bootstrap.hooks`.
fn resolved_hooks(config: &BootstrapConfig, agent: Agent) -> Result<Vec<ResolvedHook>> {
    let mut out = Vec::new();

    let mut push = |origin: Option<String>,
                    event: HookEvent,
                    spec: &HookSpec,
                    install_dir: Option<&Path>|
     -> Result<()> {
        let command = bootstrap::render_hook_command(&spec.command, agent, config, install_dir)?;
        out.push(ResolvedHook {
            origin,
            event,
            command,
            sha256: spec.sha256.clone(),
        });
        Ok(())
    };

    for entry in config.skill_entries_for(agent) {
        if entry.hooks.is_empty() {
            continue;
        }
        let name = bootstrap::skill_name_from_source(&entry.source)?;
        let Some(base_dir) = crate::skills::skills_dir(agent) else {
            continue;
        };
        let install_dir = base_dir.join(&name);
        for (event, specs) in &entry.hooks {
            for spec in specs {
                push(Some(name.clone()), *event, spec, Some(&install_dir))?;
            }
        }
    }

    // Global hooks are not tied to a skill, so `{{ skill_install_dir }}` has
    // no meaning in them; rendering with no dir makes such a reference a
    // template error rather than a silently empty path.
    for (event, specs) in &config.hooks {
        for spec in specs {
            push(None, *event, spec, None)?;
        }
    }

    Ok(out)
}

/// Verify a hook's pinned hash against the installed script.
///
/// `sha256` pins the source at install time; ongoing tamper detection is the
/// skills section's content comparison. Both together, neither alone.
fn verify(hook: &ResolvedHook) -> Result<(), String> {
    let Some(expected) = &hook.sha256 else {
        return Ok(());
    };
    let Some(script) = script_path(&hook.command) else {
        return Err(format!(
            "sha256 declared but no existing script path found in command `{}` -- \
             is the skill installed?",
            hook.command
        ));
    };
    match sha256_file(&script) {
        Ok(actual) if actual == expected.to_lowercase() => Ok(()),
        Ok(actual) => Err(format!(
            "sha256 mismatch for {}: expected {expected}, actual {actual}",
            script.display()
        )),
        Err(e) => Err(e.to_string()),
    }
}

/// Install (or, with `dry_run`, report) the declared hooks for one agent.
pub fn apply_for_agent(agent: Agent, config: &BootstrapConfig, dry_run: bool) -> Vec<ItemResult> {
    let agent_name = Some(agent.name());
    let hooks = match resolved_hooks(config, agent) {
        Ok(h) => h,
        Err(e) => {
            return vec![ItemResult::failed(
                Section::AgentHooks,
                agent_name,
                "hooks",
                e.to_string(),
            )];
        }
    };
    if hooks.is_empty() {
        return Vec::new();
    }

    let Some(target) = hook_target(agent) else {
        return hooks
            .iter()
            .map(|h| {
                ItemResult::skipped(
                    Section::AgentHooks,
                    agent_name,
                    h.item_name(),
                    "no declarative hook support for this agent yet",
                )
            })
            .collect();
    };

    apply_to_target(agent, &target, &hooks, dry_run)
}

/// The shared writer: merge hooks into one agent's JSON hook file.
///
/// Ownership is by exact command match — entries are only ever added, and an
/// entry is "ours" iff its command equals what the config renders. Nothing
/// else in the file is touched, and a file that fails to parse is never
/// overwritten.
fn apply_to_target(
    agent: Agent,
    target: &HookTarget,
    hooks: &[ResolvedHook],
    dry_run: bool,
) -> Vec<ItemResult> {
    let agent_name = Some(agent.name());
    let path = &target.file;

    let mut settings: Value = match std::fs::read_to_string(path) {
        Ok(content) => match serde_json::from_str(&content) {
            Ok(v) => v,
            Err(e) => {
                return hooks
                    .iter()
                    .map(|h| {
                        ItemResult::failed(
                            Section::AgentHooks,
                            agent_name,
                            h.item_name(),
                            format!("{} is not valid JSON: {e}", path.display()),
                        )
                    })
                    .collect();
            }
        },
        Err(_) => json!({}),
    };

    let mut out = Vec::new();
    let mut changed = false;

    for hook in hooks {
        let item_name = hook.item_name();

        let Some(event_key) = (target.event_key)(hook.event) else {
            out.push(ItemResult::skipped(
                Section::AgentHooks,
                agent_name,
                item_name,
                format!("{} cannot express {}", agent.name(), hook.event.as_str()),
            ));
            continue;
        };

        if let Err(why) = verify(hook) {
            out.push(ItemResult::failed(
                Section::AgentHooks,
                agent_name,
                item_name,
                why,
            ));
            continue;
        }

        if has_hook(&settings, target.dialect, event_key, &hook.command) {
            out.push(
                ItemResult::new(Section::AgentHooks, agent_name, item_name, Outcome::UpToDate)
                    .managed_at(&hook.command),
            );
            continue;
        }

        if !dry_run {
            add_hook(&mut settings, target.dialect, event_key, &hook.command);
            changed = true;
        }
        out.push(
            ItemResult::new(Section::AgentHooks, agent_name, item_name, Outcome::Installed)
                .managed_at(&hook.command)
                .with_detail(format!("hooks.{event_key} in {}", path.display())),
        );
    }

    if changed {
        let write = || -> Result<()> {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(path, serde_json::to_string_pretty(&settings)?)?;
            Ok(())
        };
        if let Err(e) = write() {
            out.push(ItemResult::failed(
                Section::AgentHooks,
                agent_name,
                "hook config",
                e.to_string(),
            ));
        }
    }

    out
}

/// Whether the settings document already carries `command` under `event_key`.
///
/// The shape is shared by Claude's settings.json, Codex's hooks.json, and
/// Gemini's settings.json:
/// `{"hooks": {"<Event>": [{"hooks": [{"type": "command", "command": ...}]}]}}`
fn has_hook(settings: &Value, dialect: HookDialect, event_key: &str, command: &str) -> bool {
    let mut entries = settings["hooks"][event_key].as_array().into_iter().flatten();
    match dialect {
        HookDialect::Grouped => entries
            .filter_map(|group| group["hooks"].as_array())
            .flatten()
            .any(|h| h["command"].as_str() == Some(command)),
        HookDialect::CopilotFlat => entries.any(|h| h["bash"].as_str() == Some(command)),
    }
}

/// Merge one hook command into the settings document.
fn add_hook(settings: &mut Value, dialect: HookDialect, event_key: &str, command: &str) {
    if !settings.is_object() {
        *settings = json!({});
    }
    let root = settings.as_object_mut().expect("just ensured object");
    if dialect == HookDialect::CopilotFlat {
        // Copilot rejects a hook file without its schema version.
        root.entry("version").or_insert_with(|| json!(1));
    }
    let hooks = root.entry("hooks").or_insert_with(|| json!({}));
    if !hooks.is_object() {
        *hooks = json!({});
    }
    let entries = hooks
        .as_object_mut()
        .expect("just ensured object")
        .entry(event_key)
        .or_insert_with(|| json!([]));
    if !entries.is_array() {
        *entries = json!([]);
    }
    let entry = match dialect {
        HookDialect::Grouped => json!({
            "hooks": [{ "type": "command", "command": command }]
        }),
        HookDialect::CopilotFlat => json!({ "type": "command", "bash": command }),
    };
    entries
        .as_array_mut()
        .expect("just ensured array")
        .push(entry);
}

/// Remove one exact hook command from an agent's hook config.
///
/// The inverse of [`add_hook`], with the same ownership rule: only an entry
/// whose command matches exactly is touched. Groups and event keys left empty
/// by the removal are deleted too, so a config that started without them ends
/// up byte-identical rather than littered with empty arrays. An unparsable
/// file is never rewritten.
pub fn remove_command(agent: Agent, command: &str) -> Result<String> {
    let Some(target) = hook_target(agent) else {
        anyhow::bail!("{} has no hook config", agent.name());
    };
    remove_command_in(&target.file, target.dialect, command)
}

/// File-parameterized core of [`remove_command`], so it can be tested against
/// a scratch config instead of a real agent's.
fn remove_command_in(path: &Path, dialect: HookDialect, command: &str) -> Result<String> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return Ok(format!("no hook config at {}", path.display()));
    };
    let mut settings: Value = serde_json::from_str(&content)
        .with_context(|| format!("{} is not valid JSON", path.display()))?;

    let Some(hooks) = settings.get_mut("hooks").and_then(|h| h.as_object_mut()) else {
        return Ok(format!("no hook to remove in {}", path.display()));
    };

    let mut removed = false;
    for entries in hooks.values_mut() {
        let Some(entries) = entries.as_array_mut() else {
            continue;
        };
        if dialect == HookDialect::CopilotFlat {
            let before = entries.len();
            entries.retain(|h| h["bash"].as_str() != Some(command));
            removed |= entries.len() != before;
            continue;
        }
        for group in entries.iter_mut() {
            if let Some(list) = group.get_mut("hooks").and_then(|h| h.as_array_mut()) {
                let before = list.len();
                list.retain(|h| h["command"].as_str() != Some(command));
                removed |= list.len() != before;
            }
        }
        entries.retain(|g| !g["hooks"].as_array().is_some_and(|l| l.is_empty()));
    }
    hooks.retain(|_, groups| !groups.as_array().is_some_and(|g| g.is_empty()));
    let hooks_empty = hooks.is_empty();

    if !removed {
        return Ok(format!("hook already absent from {}", path.display()));
    }
    if hooks_empty {
        settings.as_object_mut().map(|o| o.remove("hooks"));
    }

    std::fs::write(path, serde_json::to_string_pretty(&settings)?)
        .with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(format!("removed hook from {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_injection_gated_on_hooks_and_existing_entries() {
        let mut config = BootstrapConfig::default();
        // No hooks declared -> nothing to inject.
        assert!(plugin_to_inject(Agent::Pi, &config).is_none());

        config.hooks.insert(
            HookEvent::TurnDone,
            vec![HookSpec {
                command: "bash x.sh".into(),
                sha256: None,
            }],
        );
        let req = plugin_to_inject(Agent::Pi, &config).expect("declared hooks inject");
        assert_eq!(req.spec, "npm:@hsingjui/pi-hooks");
        assert!(
            plugin_to_inject(Agent::OpenCode, &config)
                .is_some_and(|r| r.spec == "opencode-claude-hooks")
        );
        // Claude has native hook config -> never injects.
        assert!(plugin_to_inject(Agent::Claude, &config).is_none());

        // A pinned/prefixed entry already carrying the fragment suppresses it.
        config
            .plugins
            .push("npm:@hsingjui/pi-hooks@1.2.0".into());
        assert!(plugin_to_inject(Agent::Pi, &config).is_none());
    }

    #[test]
    fn writer_preserves_pi_settings_siblings() {
        let tmp = std::env::temp_dir().join(format!(
            "muxix-pi-settings-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let file = tmp.join("settings.json");
        std::fs::write(&file, r#"{"model":"kimi-k3","theme":"dark"}"#).unwrap();

        fn keys(event: HookEvent) -> Option<&'static str> {
            match event {
                HookEvent::SessionReady => Some("SessionStart"),
                HookEvent::TurnDone => Some("Stop"),
            }
        }
        let target = HookTarget {
            file: file.clone(),
            event_key: keys,
            requires_plugin: None,
            dialect: HookDialect::Grouped,
        };
        let hook = ResolvedHook {
            origin: Some("s".into()),
            event: HookEvent::TurnDone,
            command: "bash go.sh".into(),
            sha256: None,
        };

        let r = apply_to_target(Agent::Pi, &target, &[hook], false);
        assert_eq!(r[0].outcome, Outcome::Installed, "{r:?}");
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(v["model"], "kimi-k3");
        assert_eq!(v["theme"], "dark");
        assert!(has_hook(&v, HookDialect::Grouped, "Stop", "bash go.sh"));

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn add_then_detect_round_trips() {
        let mut settings = json!({});
        assert!(!has_hook(&settings, HookDialect::Grouped, "Stop", "bash x.sh"));
        add_hook(&mut settings, HookDialect::Grouped, "Stop", "bash x.sh");
        assert!(has_hook(&settings, HookDialect::Grouped, "Stop", "bash x.sh"));
        assert!(!has_hook(&settings, HookDialect::Grouped, "Stop", "bash other.sh"));
    }

    #[test]
    fn copilot_dialect_is_version_stamped_and_flat() {
        // Copilot's own file shape: {version, hooks: {event: [{type, bash}]}} --
        // a Grouped entry written here would never run.
        let mut settings = json!({});
        assert!(!has_hook(
            &settings,
            HookDialect::CopilotFlat,
            "sessionStart",
            "bash go.sh"
        ));
        add_hook(
            &mut settings,
            HookDialect::CopilotFlat,
            "sessionStart",
            "bash go.sh",
        );
        assert_eq!(settings["version"], 1);
        assert_eq!(settings["hooks"]["sessionStart"][0]["type"], "command");
        assert_eq!(settings["hooks"]["sessionStart"][0]["bash"], "bash go.sh");
        assert!(has_hook(
            &settings,
            HookDialect::CopilotFlat,
            "sessionStart",
            "bash go.sh"
        ));
        // The two dialects do not see each other's entries.
        assert!(!has_hook(
            &settings,
            HookDialect::Grouped,
            "sessionStart",
            "bash go.sh"
        ));
    }

    #[test]
    fn copilot_target_maps_both_events_and_omp_has_none() {
        let target = crate::agent::setup::copilot::declared_hook_target()
            .expect("copilot has a hook target");
        assert_eq!(target.dialect, HookDialect::CopilotFlat);
        assert!(target.file.ends_with("hooks/muxix.json"), "{:?}", target.file);
        assert_eq!((target.event_key)(HookEvent::SessionReady), Some("sessionStart"));
        assert_eq!((target.event_key)(HookEvent::TurnDone), Some("agentStop"));

        // omp: no hook config muxix can write (see hook_target).
        assert!(hook_target(Agent::Omp).is_none());
    }

    #[test]
    fn add_preserves_existing_settings() {
        let mut settings = json!({
            "model": "opus",
            "hooks": { "Stop": [ { "hooks": [{"type":"command","command":"existing"}] } ] }
        });
        add_hook(&mut settings, HookDialect::Grouped, "Stop", "bash new.sh");
        assert_eq!(settings["model"], "opus");
        assert!(has_hook(&settings, HookDialect::Grouped, "Stop", "existing"));
        assert!(has_hook(&settings, HookDialect::Grouped, "Stop", "bash new.sh"));
    }

    /// Removal is the inverse of `add_hook`: the named command goes, the
    /// group it emptied goes with it, and a foreign command in the same file
    /// is never touched.
    #[test]
    fn remove_deletes_only_the_named_command_and_its_empty_group() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("settings.json");

        let mut settings = json!({"model": "opus"});
        add_hook(&mut settings, HookDialect::Grouped, "Stop", "bash ours.sh");
        add_hook(&mut settings, HookDialect::Grouped, "Stop", "bash theirs.sh");
        add_hook(&mut settings, HookDialect::Grouped, "SessionStart", "bash only.sh");
        std::fs::write(&file, serde_json::to_string_pretty(&settings).unwrap()).unwrap();

        remove_command_in(&file, HookDialect::Grouped, "bash ours.sh").unwrap();
        remove_command_in(&file, HookDialect::Grouped, "bash only.sh").unwrap();

        let v: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(v["model"], "opus", "siblings survive");
        assert!(!has_hook(&v, HookDialect::Grouped, "Stop", "bash ours.sh"));
        assert!(has_hook(&v, HookDialect::Grouped, "Stop", "bash theirs.sh"));
        assert!(
            v["hooks"].get("SessionStart").is_none(),
            "an event key with no groups left must be dropped, got {v}"
        );
    }

    #[test]
    fn removing_the_last_hook_drops_the_hooks_key() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("settings.json");
        let mut settings = json!({"theme": "dark"});
        add_hook(&mut settings, HookDialect::Grouped, "Stop", "bash x.sh");
        std::fs::write(&file, serde_json::to_string_pretty(&settings).unwrap()).unwrap();

        remove_command_in(&file, HookDialect::Grouped, "bash x.sh").unwrap();

        let v: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(v, json!({"theme": "dark"}));
    }

    #[test]
    fn removing_an_absent_hook_is_not_an_error_and_rewrites_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("settings.json");
        let original = r#"{"model":"opus"}"#;
        std::fs::write(&file, original).unwrap();

        remove_command_in(&file, HookDialect::Grouped, "bash nope.sh").unwrap();

        assert_eq!(std::fs::read_to_string(&file).unwrap(), original);
    }

    #[test]
    fn removal_never_rewrites_an_unparsable_file() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("settings.json");
        std::fs::write(&file, "{not json").unwrap();

        assert!(remove_command_in(&file, HookDialect::Grouped, "bash x.sh").is_err());
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "{not json");
    }

    #[test]
    fn sha256_matches_known_vector() {
        let tmp = std::env::temp_dir().join(format!(
            "muxix-sha-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::write(&tmp, b"abc").unwrap();
        assert_eq!(
            sha256_file(&tmp).unwrap(),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let _ = std::fs::remove_file(&tmp);
    }

    #[test]
    fn script_path_finds_the_existing_token() {
        let tmp = std::env::temp_dir().join(format!(
            "muxix-sp-{}-{:?}.sh",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::write(&tmp, "#!/bin/sh\n").unwrap();
        let cmd = format!("bash \"{}\" --flag", tmp.display());
        assert_eq!(script_path(&cmd), Some(tmp.clone()));
        assert_eq!(script_path("bash /nonexistent/x.sh"), None);
        let _ = std::fs::remove_file(&tmp);
    }

    /// End to end against a scratch target: install, idempotence, drift, and a
    /// hash mismatch failing closed. Exercises the shared writer exactly as an
    /// agent target would.
    #[test]
    fn writer_end_to_end() {
        let tmp = std::env::temp_dir().join(format!(
            "muxix-hooks-e2e-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        let script = tmp.join("go.sh");
        std::fs::write(&script, "#!/bin/sh\necho hi\n").unwrap();
        let good_hash = sha256_file(&script).unwrap();

        fn keys(event: HookEvent) -> Option<&'static str> {
            match event {
                HookEvent::SessionReady => Some("SessionStart"),
                HookEvent::TurnDone => Some("Stop"),
            }
        }
        let target = HookTarget {
            file: tmp.join("settings.json"),
            event_key: keys,
            requires_plugin: None,
            dialect: HookDialect::Grouped,
        };
        let hook = |sha: Option<String>| ResolvedHook {
            origin: Some("s".into()),
            event: HookEvent::TurnDone,
            command: format!("bash \"{}\"", script.display()),
            sha256: sha,
        };

        // Install.
        let r = apply_to_target(Agent::Claude, &target, &[hook(Some(good_hash.clone()))], false);
        assert_eq!(r[0].outcome, Outcome::Installed, "{r:?}");
        // Idempotent.
        let r = apply_to_target(Agent::Claude, &target, &[hook(None)], false);
        assert_eq!(r[0].outcome, Outcome::UpToDate, "{r:?}");
        // Dry-run drift: remove the file, check reports Installed, writes nothing.
        std::fs::remove_file(target.file.clone()).unwrap();
        let r = apply_to_target(Agent::Claude, &target, &[hook(None)], true);
        assert_eq!(r[0].outcome, Outcome::Installed, "{r:?}");
        assert!(!target.file.exists(), "dry run must not write");
        // Tampered script fails closed.
        std::fs::write(&script, "#!/bin/sh\necho evil\n").unwrap();
        let r = apply_to_target(Agent::Claude, &target, &[hook(Some(good_hash))], false);
        assert_eq!(r[0].outcome, Outcome::Failed, "{r:?}");
        assert!(!target.file.exists(), "a failed hook must not be written");
        let detail = r[0].detail.as_deref().unwrap();
        assert!(detail.contains("expected"), "{detail}");

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
