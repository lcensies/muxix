//! Per-section execution for `workmux setup`.
//!
//! Every section runs through one shape: given the resolved config and a
//! `dry_run` flag, produce [`ItemResult`]s describing what happened or would
//! happen. `--check` is exactly `dry_run = true`, so check and apply can never
//! disagree — they are the same code path.

use anyhow::Result;
use std::path::Path;

use super::report::{ItemResult, Outcome, Section, SetupReport};
use crate::agent::setup::{self, Agent, StatusCheck};
use crate::bootstrap;
use crate::config::Config;

/// Map a bootstrap install result onto the shared outcome vocabulary.
///
/// `target` resolves the item's name to the path it was installed at, so the
/// managed-state manifest knows how to remove it later.
fn from_skill_install(
    section: Section,
    agent: Agent,
    result: bootstrap::SkillInstall,
    target: impl Fn(&str) -> Option<std::path::PathBuf>,
) -> ItemResult {
    let agent_name = Some(agent.name());
    let managed = |item: ItemResult, name: &str| match target(name) {
        Some(p) => item.managed_at(p.display().to_string()),
        None => item,
    };
    match result {
        bootstrap::SkillInstall::Installed(name) => managed(
            ItemResult::new(section, agent_name, name.clone(), Outcome::Installed),
            &name,
        ),
        bootstrap::SkillInstall::Updated(name) => managed(
            ItemResult::new(section, agent_name, name.clone(), Outcome::Updated),
            &name,
        ),
        bootstrap::SkillInstall::UpToDate(name) => managed(
            ItemResult::new(section, agent_name, name.clone(), Outcome::UpToDate),
            &name,
        ),
        bootstrap::SkillInstall::Skipped(name, why) => {
            ItemResult::skipped(section, agent_name, name, why)
        }
    }
}

/// Status-tracking hooks.
///
/// `check_all` already inspects without writing, so this section is naturally
/// idempotent: an agent reporting `Installed` needs no work.
pub fn hooks(checks: &[setup::AgentCheck], dry_run: bool) -> Vec<ItemResult> {
    let mut out = Vec::new();
    for check in checks {
        let agent = Some(check.agent.name());
        match &check.status {
            StatusCheck::Installed => {
                out.push(ItemResult::new(
                    Section::Hooks,
                    agent,
                    "status hooks",
                    Outcome::UpToDate,
                ));
            }
            StatusCheck::Stale { missing } => {
                if dry_run {
                    out.push(
                        ItemResult::new(Section::Hooks, agent, "status hooks", Outcome::Updated)
                            .with_detail(format!("missing: {}", missing.join(", "))),
                    );
                } else {
                    out.push(install_hooks(check.agent, Outcome::Updated));
                }
            }
            StatusCheck::NotInstalled => {
                if dry_run {
                    out.push(ItemResult::new(
                        Section::Hooks,
                        agent,
                        "status hooks",
                        Outcome::Installed,
                    ));
                } else {
                    out.push(install_hooks(check.agent, Outcome::Installed));
                }
            }
            StatusCheck::Error(e) => {
                out.push(ItemResult::failed(
                    Section::Hooks,
                    agent,
                    "status hooks",
                    e.clone(),
                ));
            }
        }
    }
    out
}

fn install_hooks(agent: Agent, outcome: Outcome) -> ItemResult {
    match setup::install(agent) {
        Ok(msg) => {
            ItemResult::new(Section::Hooks, Some(agent.name()), "status hooks", outcome)
                .with_detail(msg)
        }
        Err(e) => ItemResult::failed(
            Section::Hooks,
            Some(agent.name()),
            "status hooks",
            e.to_string(),
        ),
    }
}

/// Skills: the bundled set plus anything the project declares.
pub fn skills(
    checks: &[setup::AgentCheck],
    config: Option<&bootstrap::BootstrapConfig>,
    project_root: &Path,
    dry_run: bool,
) -> Vec<ItemResult> {
    let mut out = Vec::new();

    for check in checks {
        let agent = check.agent;
        if crate::skills::skills_dir(agent).is_none() {
            out.push(ItemResult::skipped(
                Section::Skills,
                Some(agent.name()),
                "bundled skills",
                "agent has no skills directory",
            ));
            continue;
        }
        out.extend(crate::skills::install_bundled(agent, dry_run).unwrap_or_else(|e| {
            vec![ItemResult::failed(
                Section::Skills,
                Some(agent.name()),
                "bundled skills",
                e.to_string(),
            )]
        }));
    }

    let Some(config) = config else {
        return out;
    };
    for check in checks {
        let agent = check.agent;
        // Bundled skills above are deliberately left unmanaged: they are
        // always declared, so they can never become prune candidates.
        let base = crate::skills::skills_dir(agent);
        match bootstrap::install_skills_for_agent(agent, config, project_root, dry_run) {
            Ok(results) => out.extend(results.into_iter().map(|r| {
                from_skill_install(Section::Skills, agent, r, |name| {
                    base.as_ref().map(|d| d.join(name))
                })
            })),
            Err(e) => out.push(ItemResult::failed(
                Section::Skills,
                Some(agent.name()),
                "project skills",
                e.to_string(),
            )),
        }
    }
    out
}

/// Project-declared subagents.
pub fn subagents(
    checks: &[setup::AgentCheck],
    config: &bootstrap::BootstrapConfig,
    project_root: &Path,
    providers: Option<&crate::model::ProviderRegistry>,
    dry_run: bool,
) -> Vec<ItemResult> {
    let mut out = Vec::new();
    for check in checks {
        let agent = check.agent;
        let base = bootstrap::subagents_dir(agent);
        match bootstrap::install_subagents_for_agent(
            agent,
            config,
            project_root,
            providers,
            dry_run,
        ) {
            Ok(results) => out.extend(results.into_iter().map(|r| {
                from_skill_install(Section::Subagents, agent, r, |name| {
                    base.as_ref().map(|d| d.join(format!("{name}.md")))
                })
            })),
            Err(e) => out.push(ItemResult::failed(
                Section::Subagents,
                Some(agent.name()),
                "subagents",
                e.to_string(),
            )),
        }
    }
    out
}

/// Agent plugins.
///
/// Each agent installs through its own CLI, which has no dry-run mode and no
/// way to ask "is this already installed?" without running it. So `--check`
/// reports plugins as skipped rather than guessing — silently claiming "no
/// drift" would be worse than admitting the section cannot be inspected.
pub fn plugins(
    checks: &[setup::AgentCheck],
    config: &bootstrap::BootstrapConfig,
    project_root: &Path,
    dry_run: bool,
) -> Vec<ItemResult> {
    // One worker thread per agent so a cold machine pays the slowest agent
    // instead of the sum of all agents. Results are joined in `checks`
    // order (the canonical agent order), so the report is byte-identical to
    // the old sequential loop regardless of thread scheduling.
    std::thread::scope(|scope| {
        let handles: Vec<_> = checks
            .iter()
            .map(|check| {
                scope.spawn(move || {
                    // A panic in one agent's worker (bug in an installer, bad
                    // unwrap, etc.) must not take the whole section down with
                    // it -- the other agents' installs already ran or are
                    // running concurrently. Catch it here, at the boundary,
                    // and report it against this agent like any other
                    // failure; a normal `Err` from an installer already flows
                    // through `plugin_items_for_agent` as `ItemResult::failed`
                    // without needing this catch.
                    let agent = check.agent;
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        plugin_items_for_agent(check, config, project_root, dry_run)
                    }))
                    .unwrap_or_else(|payload| {
                        vec![ItemResult::failed(
                            Section::Plugins,
                            Some(agent.name()),
                            "plugin worker",
                            format!("worker panicked: {}", panic_message(&payload)),
                        )]
                    })
                })
            })
            .collect();
        handles
            .into_iter()
            // Each worker already caught its own panics above, so `join`
            // only fails if the panic somehow escaped that catch (e.g. a
            // double panic while unwinding); fall back to an empty result
            // for that agent rather than panicking the parent thread and
            // losing every other agent's results.
            .flat_map(|h| h.join().unwrap_or_default())
            .collect()
    })
}

/// Best-effort text for a caught panic payload, for embedding in a report
/// item's failure detail.
fn panic_message(payload: &Box<dyn std::any::Any + Send>) -> String {
    if let Some(s) = payload.downcast_ref::<&str>() {
        s.to_string()
    } else if let Some(s) = payload.downcast_ref::<String>() {
        s.clone()
    } else {
        "non-string panic payload".to_string()
    }
}

/// Plugin work for a single agent: the body of the old per-agent loop
/// iteration, run inside its own worker thread by [`plugins`].
fn plugin_items_for_agent(
    check: &setup::AgentCheck,
    config: &bootstrap::BootstrapConfig,
    project_root: &Path,
    dry_run: bool,
) -> Vec<ItemResult> {
    use crate::agent::setup::{claude, omp, opencode, pi};

    let mut out = Vec::new();
    let mut specs = config.plugins_for(check.agent);
        // An agent whose hook system is a compat plugin gets that plugin
        // appended when hooks are declared for it and nothing in the list
        // already matches.
        let injected = super::agent_hooks::plugin_to_inject(check.agent, config)
            .map(|req| req.spec.to_string());
        specs.extend(injected.clone());
        if specs.is_empty() {
            return out;
        }
        let agent = Some(check.agent.name());

        // Best-effort "already installed" probe: reads the agent's own config
        // where installs are recorded, so a re-run can skip shelling out to the
        // agent CLI (the slow part) when the pinned spec is already present.
        // `None` = cannot inspect cheaply; fall back to the old always-install
        // behavior for that agent.
        let installed_check: Option<Box<dyn Fn(&str) -> bool + '_>> = match check.agent {
            Agent::Pi => Some(Box::new(move |s: &str| pi::plugin_installed(s, project_root))),
            Agent::OpenCode => Some(Box::new(opencode::plugin_installed)),
            Agent::Claude => Some(Box::new(claude::plugin_installed)),
            _ => None,
        };

        // Each agent installs through its own CLI, so the spec string must
        // already be in that agent's format (see docs/guide/bootstrap.md).
        let install: fn(&str) -> Result<String> = match check.agent {
            Agent::Pi => pi::install_plugin_from_url,
            Agent::Omp => omp::install_plugin_from_url,
            Agent::Claude => claude::install_plugin,
            Agent::OpenCode => opencode::install_plugin,
            Agent::Codex | Agent::Copilot | Agent::Gemini => {
                for spec in &specs {
                    out.push(ItemResult::skipped(
                        Section::Plugins,
                        agent,
                        spec.clone(),
                        "no plugin installer for this agent; use a prompt component",
                    ));
                }
                return out;
            }
        };

        for spec in &specs {
            let provenance = (injected.as_deref() == Some(spec))
                .then_some("auto-added: required by declared hooks");

            // Skip the expensive CLI install when the spec is already recorded
            // in the agent's config. Pinned specs don't change, so "installed"
            // means "no update to apply" here.
            if installed_check
                .as_deref()
                .is_some_and(|is_installed| is_installed(spec))
            {
                out.push(
                    ItemResult::new(Section::Plugins, agent, spec.clone(), Outcome::UpToDate)
                        .managed_at(spec.clone())
                        .with_detail("already installed"),
                );
                continue;
            }

            if dry_run {
                // Inspectable agents now report real drift (would install);
                // the rest still admit they cannot be inspected dry.
                if installed_check.is_some() {
                    out.push(
                        ItemResult::new(
                            Section::Plugins,
                            agent,
                            spec.clone(),
                            Outcome::Installed,
                        )
                        .managed_at(spec.clone()),
                    );
                } else {
                    out.push(ItemResult::skipped(
                        Section::Plugins,
                        agent,
                        spec.clone(),
                        "cannot be inspected without running the agent CLI",
                    ));
                }
                continue;
            }
            match install(spec) {
                Ok(msg) => out.push(
                    ItemResult::new(Section::Plugins, agent, spec.clone(), Outcome::Installed)
                        .managed_at(spec.clone())
                        .with_detail(match provenance {
                            Some(p) => format!("{p}; {msg}"),
                            None => msg,
                        }),
                ),
                Err(e) => out.push(ItemResult::failed(
                    Section::Plugins,
                    agent,
                    spec.clone(),
                    e.to_string(),
                )),
            }
        }
    out
}

/// Prompt components merged into each agent's own prompt file.
pub fn prompts(
    checks: &[setup::AgentCheck],
    config: &bootstrap::BootstrapConfig,
    project_root: &Path,
    dry_run: bool,
) -> Vec<ItemResult> {
    let mut out = Vec::new();
    for check in checks {
        let agent = Some(check.agent.name());
        let prompt = match bootstrap::apply_prompt_to_agent(check.agent, config, project_root) {
            Ok(Some(p)) => p,
            Ok(None) => continue,
            Err(e) => {
                out.push(ItemResult::failed(
                    Section::Prompts,
                    agent,
                    "prompt components",
                    e.to_string(),
                ));
                continue;
            }
        };

        if dry_run {
            // Each agent stores its prompt in its own format, and the
            // bootstrappers expose no read-back. Reporting "skipped" is honest;
            // claiming "no drift" without looking would not be.
            out.push(ItemResult::skipped(
                Section::Prompts,
                agent,
                "prompt components",
                "prompt drift is not inspected",
            ));
            continue;
        }

        match setup::bootstrap(check.agent, &prompt, Some(config)) {
            Ok(setup::BootstrapOutcome::Updated) => out.push(ItemResult::new(
                Section::Prompts,
                agent,
                "prompt components",
                Outcome::Updated,
            )),
            Ok(setup::BootstrapOutcome::UpToDate) => out.push(ItemResult::new(
                Section::Prompts,
                agent,
                "prompt components",
                Outcome::UpToDate,
            )),
            Ok(setup::BootstrapOutcome::Unsupported) => out.push(ItemResult::skipped(
                Section::Prompts,
                agent,
                "prompt components",
                "agent has no prompt bootstrap support",
            )),
            Err(e) => out.push(ItemResult::failed(
                Section::Prompts,
                agent,
                "prompt components",
                e.to_string(),
            )),
        }
    }
    out
}

/// Registry providers with connection details rendered into each supported
/// agent's native provider config. Agents without a confirmed provider config
/// format are silently skipped.
pub fn providers_sync(
    checks: &[setup::AgentCheck],
    providers: Option<&crate::model::ProviderRegistry>,
    dry_run: bool,
) -> Vec<ItemResult> {
    let Some(registry) = providers.filter(|r| r.values().any(|c| c.has_connection())) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for check in checks {
        let sync: fn(&crate::model::ProviderRegistry, bool) -> Result<Vec<String>> =
            match check.agent {
                Agent::OpenCode => setup::opencode::sync_providers,
                Agent::Codex => setup::codex::sync_providers,
                _ => continue,
            };
        let agent = Some(check.agent.name());
        match sync(registry, dry_run) {
            Ok(msgs) if msgs.is_empty() => out.push(ItemResult::new(
                Section::Providers,
                agent,
                "provider sync",
                Outcome::UpToDate,
            )),
            Ok(msgs) => out.extend(msgs.into_iter().map(|msg| {
                let outcome = if msg.starts_with("Skipped") {
                    Outcome::Skipped
                } else {
                    Outcome::Updated
                };
                ItemResult::new(Section::Providers, agent, "provider sync", outcome)
                    .with_detail(msg)
            })),
            Err(e) => out.push(ItemResult::failed(
                Section::Providers,
                agent,
                "provider sync",
                e.to_string(),
            )),
        }
    }
    out
}

/// The theme written into each agent's own config.
pub fn theme(
    checks: &[setup::AgentCheck],
    config: &bootstrap::BootstrapConfig,
    dry_run: bool,
) -> Vec<ItemResult> {
    let mut out = Vec::new();
    for check in checks {
        let Some(name) = config.theme_for(check.agent) else {
            continue;
        };
        let agent = Some(check.agent.name());

        if dry_run {
            // No write path here: report the declared theme as pending rather
            // than reaching into each agent's config format a second time.
            out.push(
                ItemResult::new(Section::Theme, agent, name.to_string(), Outcome::Skipped)
                    .with_detail("theme drift is not inspected"),
            );
            continue;
        }

        match bootstrap::apply_theme_for_agent(check.agent, config) {
            Ok(Some(msg)) => out.push(
                ItemResult::new(Section::Theme, agent, name.to_string(), Outcome::Updated)
                    .with_detail(msg),
            ),
            Ok(None) => out.push(ItemResult::new(
                Section::Theme,
                agent,
                name.to_string(),
                Outcome::UpToDate,
            )),
            Err(e) => out.push(ItemResult::failed(
                Section::Theme,
                agent,
                name.to_string(),
                e.to_string(),
            )),
        }
    }
    out
}

/// Declared merge patches applied to each agent's own settings file.
///
/// RFC 7386, so a key the patch does not name survives — including keys the
/// agent writes for itself (pi's `packages`, its last-seen version). Drift is
/// judged on the parsed value, not the file text: re-running with an unchanged
/// patch must not rewrite a file whose formatting differs from ours.
pub fn agent_settings(
    checks: &[setup::AgentCheck],
    config: &bootstrap::BootstrapConfig,
    dry_run: bool,
) -> Vec<ItemResult> {
    let mut out = Vec::new();
    for check in checks {
        let Some(patch) = config.settings_for(check.agent) else {
            continue;
        };
        let agent = Some(check.agent.name());
        let Some(path) = setup::settings_file(check.agent) else {
            out.push(ItemResult::skipped(
                Section::AgentSettings,
                agent,
                "settings",
                "no JSON settings file known for this agent",
            ));
            continue;
        };

        let existing = std::fs::read_to_string(&path).ok();
        let before: serde_json::Value = match existing.as_deref().map(str::trim) {
            None | Some("") => serde_json::json!({}),
            Some(text) => match serde_json::from_str(text) {
                Ok(v) => v,
                Err(e) => {
                    out.push(ItemResult::failed(
                        Section::AgentSettings,
                        agent,
                        "settings",
                        format!("{}: {e}", path.display()),
                    ));
                    continue;
                }
            },
        };

        let mut after = before.clone();
        crate::agent::agent_profiles::json_merge_patch(&mut after, patch);

        let changed: Vec<&str> = match patch.as_object() {
            Some(keys) => keys
                .keys()
                .filter(|k| before.get(k.as_str()) != after.get(k.as_str()))
                .map(String::as_str)
                .collect(),
            // RFC 7386 allows a non-object patch, which replaces the whole
            // document; no key to name, so name the document.
            None => vec!["<document>"],
        };

        if changed.is_empty() {
            out.push(ItemResult::new(
                Section::AgentSettings,
                agent,
                "settings",
                Outcome::UpToDate,
            ));
            continue;
        }

        let outcome = if existing.is_none() {
            Outcome::Installed
        } else {
            Outcome::Updated
        };
        let item = ItemResult::new(Section::AgentSettings, agent, "settings", outcome)
            .with_detail(format!("{} in {}", changed.join(", "), path.display()));

        if dry_run {
            out.push(item);
            continue;
        }

        let write = || -> Result<()> {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            crate::util::write_atomic(
                &path,
                &format!("{}\n", serde_json::to_string_pretty(&after)?),
            )
        };
        match write() {
            Ok(()) => out.push(item),
            Err(e) => out.push(ItemResult::failed(
                Section::AgentSettings,
                agent,
                "settings",
                e.to_string(),
            )),
        }
    }
    out
}

/// MCP servers rendered into each agent's native config.
pub fn mcp(config: &Config, dry_run: bool) -> Vec<ItemResult> {
    if config.mcp.as_ref().is_none_or(|m| m.is_empty()) {
        return Vec::new();
    }
    let Ok(repo_root) = crate::git::get_main_worktree_root() else {
        return vec![ItemResult::skipped(
            Section::Mcp,
            None,
            "mcp sync",
            "not inside a git repository",
        )];
    };

    if dry_run {
        return vec![ItemResult::skipped(
            Section::Mcp,
            None,
            "mcp sync",
            "mcp drift is not inspected",
        )];
    }

    match crate::mcp::sync_agent_mcp_configs(&repo_root, config) {
        Ok(written) if written.is_empty() => vec![ItemResult::new(
            Section::Mcp,
            None,
            "mcp sync",
            Outcome::UpToDate,
        )],
        Ok(written) => written
            .into_iter()
            .map(|p| {
                ItemResult::new(
                    Section::Mcp,
                    None,
                    p.display().to_string(),
                    Outcome::Updated,
                )
            })
            .collect(),
        Err(e) => vec![ItemResult::failed(
            Section::Mcp,
            None,
            "mcp sync",
            e.to_string(),
        )],
    }
}

#[cfg(test)]
mod plugins_tests {
    use super::*;
    use crate::agent::setup::StatusCheck;

    // CLAUDE_CONFIG_DIR is process-global; serialize against the same lock
    // `command::setup::tests::Scratch` uses, so this module never races
    // another test's env-var swap.
    fn with_claude_dir(dir: &Path, f: impl FnOnce()) {
        let _g = super::super::tests::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var_os("CLAUDE_CONFIG_DIR");
        unsafe { std::env::set_var("CLAUDE_CONFIG_DIR", dir) };
        f();
        unsafe {
            match prev {
                Some(v) => std::env::set_var("CLAUDE_CONFIG_DIR", v),
                None => std::env::remove_var("CLAUDE_CONFIG_DIR"),
            }
        }
    }

    fn claude_check() -> setup::AgentCheck {
        setup::AgentCheck {
            agent: Agent::Claude,
            reason: "test",
            status: StatusCheck::Installed,
        }
    }

    /// `--check` (dry_run) on a missing Claude plugin must report it as drift
    /// (`Installed` outcome), not `skipped` -- the whole point of wiring the
    /// Claude probe into `installed_check`.
    #[test]
    fn check_reports_missing_claude_plugin_as_drift_not_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        // No plugins/installed_plugins.json at all: nothing is installed.
        with_claude_dir(tmp.path(), || {
            let config = bootstrap::BootstrapConfig {
                default_plugins: vec!["ponytail@ponytail".to_string()],
                ..Default::default()
            };
            let project_root = Path::new("/does/not/matter");
            let results = plugins(&[claude_check()], &config, project_root, true);

            assert_eq!(results.len(), 1);
            let item = &results[0];
            assert_eq!(item.outcome, Outcome::Installed, "expected drift, got {item:?}");
            assert_ne!(item.outcome, Outcome::Skipped);
        });
    }

    /// A Claude plugin already recorded at `scope: user` reports up-to-date
    /// under `--check`, and installs nothing.
    #[test]
    fn check_reports_installed_claude_plugin_as_up_to_date() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("plugins")).unwrap();
        std::fs::write(
            tmp.path().join("plugins/installed_plugins.json"),
            r#"{"version":2,"plugins":{"ponytail@ponytail":[{"scope":"user"}]}}"#,
        )
        .unwrap();
        with_claude_dir(tmp.path(), || {
            let config = bootstrap::BootstrapConfig {
                default_plugins: vec!["ponytail@ponytail".to_string()],
                ..Default::default()
            };
            let project_root = Path::new("/does/not/matter");
            let results = plugins(&[claude_check()], &config, project_root, true);

            assert_eq!(results.len(), 1);
            assert_eq!(results[0].outcome, Outcome::UpToDate);
        });
    }

    fn check_for(agent: Agent) -> setup::AgentCheck {
        setup::AgentCheck {
            agent,
            reason: "test",
            status: StatusCheck::Installed,
        }
    }

    /// The plugins section now runs one worker thread per agent
    /// (`std::thread::scope`), but the emitted report must still read as if
    /// it were produced by the old sequential loop: items grouped by agent,
    /// agents in the same order as `checks`, specs within an agent in the
    /// same order `plugins_for` returned them. Codex/Copilot/Gemini have no
    /// installer, so each hits the synchronous "skipped" path with no CLI
    /// spawn or filesystem probe -- deterministic regardless of thread
    /// scheduling, which is exactly what this test needs to isolate the
    /// merge-order logic from execution-order nondeterminism.
    #[test]
    fn plugins_emits_items_in_canonical_agent_order_when_run_concurrently() {
        let config = bootstrap::BootstrapConfig {
            default_plugins: vec!["a-plugin".to_string(), "b-plugin".to_string()],
            ..Default::default()
        };
        let project_root = Path::new("/does/not/matter");
        // Deliberately not alphabetical / not `Agent::ALL` order, so a bug
        // that reorders by completion time or by agent sort would show up.
        let checks = [
            check_for(Agent::Gemini),
            check_for(Agent::Codex),
            check_for(Agent::Copilot),
        ];

        let results = plugins(&checks, &config, project_root, false);

        let got: Vec<(&str, &str)> = results
            .iter()
            .map(|r| (r.agent.as_deref().unwrap(), r.name.as_str()))
            .collect();
        let want = vec![
            (Agent::Gemini.name(), "a-plugin"),
            (Agent::Gemini.name(), "b-plugin"),
            (Agent::Codex.name(), "a-plugin"),
            (Agent::Codex.name(), "b-plugin"),
            (Agent::Copilot.name(), "a-plugin"),
            (Agent::Copilot.name(), "b-plugin"),
        ];
        assert_eq!(got, want);
    }

    /// One agent's installer failing (here: `pi install` unreachable because
    /// `PATH` has nothing on it) must not take down the section: the other
    /// agent's items still come back, and the failing agent's own item
    /// reports the failure rather than the run silently losing it.
    ///
    /// This is the hermetic stand-in for a worker panic: both paths flow
    /// through the same per-agent boundary in `plugins()`, and unlike an
    /// actual panic, an installer `Err` can be forced deterministically
    /// without unwind-safety games.
    ///
    /// `PATH` is process-global and other tests run concurrently in this
    /// process, so this only *prepends* a directory with a fake `pi` that
    /// exits non-zero -- everything else already on `PATH` (git, real
    /// binaries other tests spawn) stays reachable. Only `pi.rs`'s own
    /// `Command::new("pi")` is affected.
    #[cfg(unix)]
    #[test]
    fn plugins_reports_failing_agent_without_aborting_the_section() {
        use std::os::unix::fs::PermissionsExt;

        let _g = super::super::tests::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let tmp = tempfile::tempdir().unwrap();
        let prev_agent_dir = std::env::var_os("PI_CODING_AGENT_DIR");
        let prev_path = std::env::var_os("PATH");

        let bin_dir = tmp.path().join("bin");
        std::fs::create_dir_all(&bin_dir).unwrap();
        let fake_pi = bin_dir.join("pi");
        std::fs::write(&fake_pi, "#!/bin/sh\nexit 1\n").unwrap();
        let mut perms = std::fs::metadata(&fake_pi).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake_pi, perms).unwrap();
        let new_path = match &prev_path {
            Some(p) => format!("{}:{}", bin_dir.display(), p.to_string_lossy()),
            None => bin_dir.display().to_string(),
        };

        unsafe {
            // No settings.json here, so pi::plugin_installed reports
            // not-installed and the section proceeds to (attempt) install.
            std::env::set_var("PI_CODING_AGENT_DIR", tmp.path());
            std::env::set_var("PATH", &new_path);
        }

        let config = bootstrap::BootstrapConfig {
            default_plugins: vec!["./vendor/x".to_string()],
            ..Default::default()
        };
        let checks = [check_for(Agent::Pi), check_for(Agent::Codex)];
        let results = plugins(&checks, &config, tmp.path(), false);

        unsafe {
            match prev_agent_dir {
                Some(v) => std::env::set_var("PI_CODING_AGENT_DIR", v),
                None => std::env::remove_var("PI_CODING_AGENT_DIR"),
            }
            match prev_path {
                Some(v) => std::env::set_var("PATH", v),
                None => std::env::remove_var("PATH"),
            }
        }

        let pi_item = results
            .iter()
            .find(|r| r.agent.as_deref() == Some(Agent::Pi.name()))
            .expect("pi agent must still report an item");
        assert_eq!(pi_item.outcome, Outcome::Failed, "expected {pi_item:?} to be Failed");

        let codex_item = results
            .iter()
            .find(|r| r.agent.as_deref() == Some(Agent::Codex.name()));
        assert!(
            codex_item.is_some(),
            "codex's result must still appear; the failing pi worker must not abort the section"
        );
    }
}

/// Run every selected section and collect one report.
///
/// A failing item never aborts the run: the remaining sections still execute,
/// and the failure surfaces through [`SetupReport::exit_code`].
pub fn run_all(
    selected: &[Section],
    checks: &[setup::AgentCheck],
    config: Option<&Config>,
    project_root: &Path,
    dry_run: bool,
) -> SetupReport {
    run_all_with_prune(selected, checks, config, project_root, dry_run, true)
}

/// [`run_all`], with pruning of no-longer-declared features switchable off
/// (`workmux setup --no-prune`).
pub fn run_all_with_prune(
    selected: &[Section],
    checks: &[setup::AgentCheck],
    config: Option<&Config>,
    project_root: &Path,
    dry_run: bool,
    prune: bool,
) -> SetupReport {
    let mut report = SetupReport {
        check_only: dry_run,
        ..Default::default()
    };

    let bootstrap_cfg = config.and_then(|c| c.bootstrap.as_ref());
    let providers = config.and_then(|c| c.providers.as_ref());

    for section in selected {
        match section {
            Section::Hooks => report.extend(hooks(checks, dry_run)),
            Section::Skills => {
                report.extend(skills(checks, bootstrap_cfg, project_root, dry_run))
            }
            Section::AgentHooks => {
                if let Some(cfg) = bootstrap_cfg {
                    for check in checks {
                        report.extend(super::agent_hooks::apply_for_agent(
                            check.agent,
                            cfg,
                            dry_run,
                        ));
                    }
                }
            }
            Section::Subagents => {
                if let Some(cfg) = bootstrap_cfg {
                    report.extend(subagents(checks, cfg, project_root, providers, dry_run));
                }
            }
            Section::Plugins => {
                if let Some(cfg) = bootstrap_cfg {
                    report.extend(plugins(checks, cfg, project_root, dry_run));
                }
            }
            Section::AgentSettings => {
                if let Some(cfg) = bootstrap_cfg {
                    report.extend(agent_settings(checks, cfg, dry_run));
                }
            }
            Section::Providers => report.extend(providers_sync(checks, providers, dry_run)),
            Section::Prompts => {
                if let Some(cfg) = bootstrap_cfg {
                    report.extend(prompts(checks, cfg, project_root, dry_run));
                }
            }
            Section::Theme => {
                if let Some(cfg) = bootstrap_cfg {
                    report.extend(theme(checks, cfg, dry_run));
                }
            }
            Section::Mcp => {
                if let Some(cfg) = config {
                    report.extend(mcp(cfg, dry_run));
                }
            }
            Section::AgentProfiles => {
                if let Some(cfg) = config {
                    report.extend(agent_profiles(cfg, checks, project_root, dry_run));
                }
            }
            Section::Deps => {
                if let Some(cfg) = config {
                    report.extend(deps(cfg, dry_run));
                }
            }
        }
    }

    reconcile_managed(&mut report, selected, config, project_root, dry_run, prune);

    report
}

/// Remove harness features workmux installed that the config no longer
/// declares, then persist what this run left on the machine.
///
/// Runs last, after every section has reported: the report *is* the desired
/// state, and anything the manifest claims for this project that the report
/// does not mention has fallen out of the config.
fn reconcile_managed(
    report: &mut SetupReport,
    selected: &[Section],
    config: Option<&Config>,
    project_root: &Path,
    dry_run: bool,
    prune: bool,
) {
    use super::managed::{self, Manifest};

    let mut manifest = Manifest::load();
    let installed = managed::installed_keys(&report.items);
    let mut stale = manifest.stale_for(project_root, &installed, selected);
    // deps_strict: the prefix is fully declarative, hand installs go too.
    if selected.contains(&Section::Deps)
        && config.and_then(|c| c.bootstrap.as_ref()).is_some_and(|b| b.deps_strict)
        && let Ok(prefix) = crate::deps::npm_prefix(
            config.and_then(|c| c.bootstrap.as_ref()).and_then(|b| b.npm_prefix.as_deref()),
        )
    {
        let declared: std::collections::BTreeSet<String> = report
            .items
            .iter()
            // Every reported name, failed ones included: a conflicting pin is
            // still a declaration, not a hand install.
            .filter(|i| i.section == Section::Deps)
            .map(|i| i.name.clone())
            .collect();
        for entry in managed::undeclared_npm(&prefix, &declared, project_root) {
            if !stale.contains(&entry) {
                stale.push(entry);
            }
        }
    }
    // Computed even under --no-prune: the stale set is also the keep set, and
    // dropping the entries instead of the features would make the removal
    // unrecoverable on the next run.
    let removals = if prune {
        managed::prune(&stale, dry_run)
    } else {
        Vec::new()
    };

    // A removal that failed is still on the machine, so its entry survives to
    // be retried; --no-prune keeps every stale entry for the same reason.
    let failed: std::collections::BTreeSet<(Section, String)> = removals
        .iter()
        .filter(|r| r.outcome == Outcome::Failed)
        .map(|r| (r.section, r.name.clone()))
        .collect();
    let keep: Vec<_> = stale
        .iter()
        .filter(|e| !prune || failed.contains(&(e.section, e.name.clone())))
        .cloned()
        .collect();

    report.extend(removals);

    if dry_run {
        return;
    }
    let current = managed::entries_from_report(&report.items, project_root);
    manifest.record(project_root, selected, current, &keep);
    if let Err(e) = manifest.save() {
        // No section owns the manifest; attribute it to the last one that ran
        // so it lands at the tail of the report rather than inventing a
        // section that never executes.
        report.push(ItemResult::failed(
            *selected.last().unwrap_or(&Section::Hooks),
            None,
            "managed-state manifest",
            e.to_string(),
        ));
    }
}

/// Dependencies of skills and MCP servers: install declared npm packages into
/// the user prefix, assert declared executables on PATH.
///
/// One item per npm package name (versions deduped across declaring entities,
/// a conflict is one `Failed` item) and one per distinct executable.
pub fn deps(config: &Config, dry_run: bool) -> Vec<ItemResult> {
    use crate::deps::{self, NpmSpec};
    use std::collections::BTreeMap;

    // (kind, entity name, requires), skills deduped by source across agents.
    let mut declared: Vec<(&'static str, String, crate::deps::Requires)> = Vec::new();
    if let Some(cfg) = config.bootstrap.as_ref() {
        let mut seen = std::collections::BTreeSet::new();
        for agent in Agent::ALL {
            for entry in cfg.skill_entries_for(agent) {
                if entry.requires.is_empty() || !seen.insert(entry.display()) {
                    continue;
                }
                let name = bootstrap::skill_name_from_source(&entry.source)
                    .unwrap_or_else(|_| entry.display());
                declared.push(("skill", name, entry.requires));
            }
        }
    }
    if let Some(mcp) = config.mcp.as_ref() {
        for (name, server) in mcp {
            if let Some(req) = server.requires.as_ref().filter(|r| !r.is_empty())
                && server.is_enabled()
            {
                declared.push(("mcp", name.clone(), req.clone()));
            }
        }
    }
    if declared.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    let by = |kind: &str, name: &str| format!("required by {kind} {name}");

    // name -> (spec, declaring entities); a second, different pin is a conflict.
    let mut npm: BTreeMap<String, (NpmSpec, Vec<String>, bool)> = BTreeMap::new();
    let mut bins: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (kind, entity, req) in &declared {
        let who = format!("{kind} {entity}");
        for raw in &req.npm {
            let spec = match NpmSpec::parse(raw) {
                Ok(s) => s,
                Err(e) => {
                    out.push(ItemResult::failed(Section::Deps, None, raw.clone(), format!("{e} ({})", by(kind, entity))));
                    continue;
                }
            };
            let slot = npm.entry(spec.name.clone()).or_insert_with(|| (spec.clone(), Vec::new(), false));
            if slot.0.version != spec.version && slot.0.is_pinned() && spec.is_pinned() {
                slot.2 = true;
            } else if spec.is_pinned() {
                slot.0 = spec;
            }
            slot.1.push(who.clone());
        }
        for b in &req.bin {
            bins.entry(b.clone()).or_default().push(who.clone());
        }
    }

    let prefix = if npm.is_empty() {
        None
    } else {
        let cfg_prefix = config.bootstrap.as_ref().and_then(|b| b.npm_prefix.as_deref());
        match deps::npm_prefix(cfg_prefix).and_then(|p| {
            if !dry_run {
                deps::ensure_prefix(&p)?;
            }
            Ok(p)
        }) {
            Ok(p) => {
                if !deps::prefix_bin_on_path(&p) {
                    out.push(ItemResult::skipped(
                        Section::Deps,
                        None,
                        "npm prefix",
                        format!("{} is not on PATH; installed bins are unreachable", p.join("bin").display()),
                    ));
                }
                if deps::which("npm").is_none() {
                    for (name, (_, who, _)) in &npm {
                        out.push(ItemResult::failed(Section::Deps, None, name.clone(), format!("npm not on PATH ({})", who.join(", "))));
                    }
                    npm.clear();
                }
                Some(p)
            }
            Err(e) => {
                for (name, (_, who, _)) in &npm {
                    out.push(ItemResult::failed(Section::Deps, None, name.clone(), format!("{e} ({})", who.join(", "))));
                }
                npm.clear();
                None
            }
        }
    };

    if let Some(prefix) = prefix.as_deref() {
        for (name, (spec, who, conflict)) in &npm {
            let target = deps::package_dir(prefix, name).to_string_lossy().into_owned();
            let who = who.join(", ");
            if *conflict {
                out.push(ItemResult::failed(Section::Deps, None, name.clone(), format!("conflicting pins ({who})")));
                continue;
            }
            let installed = deps::installed_version(prefix, name);
            let unpinned = if spec.is_pinned() { "" } else { "; unpinned" };
            let outcome = match (&installed, &spec.version) {
                (None, _) => Outcome::Installed,
                (Some(_), None) => Outcome::UpToDate,
                (Some(have), Some(want)) if have == want => Outcome::UpToDate,
                (Some(_), Some(_)) => Outcome::Updated,
            };
            let item = ItemResult::new(Section::Deps, None, name.clone(), outcome).managed_at(target);
            let item = match outcome {
                Outcome::UpToDate => item.with_detail(format!("{}{unpinned} ({who})", installed.as_deref().unwrap_or(""))),
                _ if dry_run => item.with_detail(format!("would install {spec}{unpinned} ({who})")),
                _ => match deps::npm_install(prefix, spec) {
                    Ok(()) => item.with_detail(format!("{spec}{unpinned} ({who})")),
                    Err(e) => ItemResult::failed(Section::Deps, None, name.clone(), format!("{e} ({who})")),
                },
            };
            out.push(item);
        }
    }

    for (bin, who) in &bins {
        let who = who.join(", ");
        out.push(match deps::which(bin) {
            Some(p) => ItemResult::new(Section::Deps, None, bin.clone(), Outcome::UpToDate)
                .with_detail(format!("{} ({who})", p.display())),
            None => ItemResult::failed(
                Section::Deps,
                None,
                bin.clone(),
                format!("not on PATH ({who}); provide it via the system, e.g. home.packages"),
            ),
        });
    }
    out
}

/// Agent config profiles: reconcile the derived overlay dirs against the
/// declared `agent_profiles:` set.
///
/// For every declared profile and every detected, profileable agent, rebuild
/// the overlay (base ⊕ profile source ⊕ declarative deltas) when it has
/// drifted. Profile dirs no longer declared are pruned — safe because the
/// derived tree is reconstructable and the user-authored source tree is never
/// touched. Deltas are generated for pi only; declaring them for another agent
/// skips that agent's overlay rather than building it without them.
pub fn agent_profiles(
    config: &Config,
    checks: &[setup::AgentCheck],
    project_root: &Path,
    dry_run: bool,
) -> Vec<ItemResult> {
    use crate::agent::agent_profiles as ap;
    let sec = Section::AgentProfiles;
    let mut out = Vec::new();

    let declared = &config.agent_profiles;

    // Prune derived dirs for profiles that are no longer declared.
    if let Ok(root) = ap::build_root() {
        if let Ok(rd) = std::fs::read_dir(&root) {
            for entry in rd.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if entry.path().is_dir() && !declared.contains_key(&name) {
                    if dry_run {
                        out.push(
                            ItemResult::new(sec, None, name, Outcome::Updated)
                                .with_detail("orphaned; would remove"),
                        );
                    } else {
                        match std::fs::remove_dir_all(entry.path()) {
                            Ok(()) => out.push(
                                ItemResult::new(sec, None, name, Outcome::Updated)
                                    .with_detail("removed orphaned overlay"),
                            ),
                            Err(e) => {
                                out.push(ItemResult::failed(sec, None, name, e.to_string()))
                            }
                        }
                    }
                }
            }
        }
    }

    if declared.is_empty() {
        return out;
    }

    let bootstrap_cfg = config.bootstrap.as_ref();

    // Detected agents that can actually be profiled (have a config-dir env var).
    let agents: Vec<Agent> = checks
        .iter()
        .map(|c| c.agent)
        .filter(|a| ap::config_dir_env(a.profile_id()).is_some())
        .collect();

    for (profile, decl) in declared {
        let source = match ap::source_dir(profile) {
            Ok(p) => p,
            Err(e) => {
                out.push(ItemResult::failed(sec, None, profile.clone(), e.to_string()));
                continue;
            }
        };
        for agent in &agents {
            let agent_id = agent.profile_id();
            let name = format!("{profile}/{agent_id}");
            let Some(base) = config.sandbox.resolved_agent_config_dir(agent_id) else {
                out.push(ItemResult::skipped(
                    sec,
                    Some(agent.name()),
                    name,
                    "no base config dir",
                ));
                continue;
            };
            let dest = match ap::build_dir(profile, agent_id) {
                Ok(p) => p,
                Err(e) => {
                    out.push(ItemResult::failed(sec, Some(agent.name()), name, e.to_string()));
                    continue;
                }
            };

            let agent_deltas = decl.agents.get(agent_id);
            let delta = match agent_deltas {
                Some(d) if !d.is_empty() && *agent != Agent::Pi => {
                    // Never build an overlay that silently drops declared deltas.
                    out.push(ItemResult::skipped(
                        sec,
                        Some(agent.name()),
                        name,
                        "bootstrap deltas are only supported for pi; overlay not built",
                    ));
                    continue;
                }
                Some(d) if !d.is_empty() => {
                    match ap::pi_delta_plan(&base, &source, d, bootstrap_cfg, project_root) {
                        Ok(dp) => dp,
                        Err(e) => {
                            out.push(ItemResult::failed(
                                sec,
                                Some(agent.name()),
                                name,
                                e.to_string(),
                            ));
                            continue;
                        }
                    }
                }
                _ => ap::DeltaPlan::default(),
            };
            for w in &delta.warnings {
                out.push(
                    ItemResult::new(sec, Some(agent.name()), name.clone(), Outcome::UpToDate)
                        .with_detail(format!("warning: {w}")),
                );
            }

            let plan = ap::plan(&base, &source, &delta);
            if ap::in_sync(&dest, &plan) {
                out.push(ItemResult::new(sec, Some(agent.name()), name, Outcome::UpToDate));
                continue;
            }
            let fresh = !dest.exists();
            let outcome = if fresh { Outcome::Installed } else { Outcome::Updated };
            if dry_run {
                out.push(ItemResult::new(sec, Some(agent.name()), name, outcome));
            } else {
                match ap::materialize(&dest, &plan) {
                    Ok(_) => out.push(ItemResult::new(sec, Some(agent.name()), name, outcome)),
                    Err(e) => {
                        out.push(ItemResult::failed(sec, Some(agent.name()), name, e.to_string()))
                    }
                }
            }
        }
    }

    out
}

#[cfg(test)]
mod agent_settings_tests {
    use super::*;
    use crate::agent::setup::StatusCheck;

    /// Run `f` with pi's agent dir pointed at `dir`. `PI_CODING_AGENT_DIR` is
    /// process-global, so this shares the setup-wide env lock.
    fn with_pi_dir<T>(dir: &Path, f: impl FnOnce() -> T) -> T {
        let _g = super::super::tests::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var_os("PI_CODING_AGENT_DIR");
        unsafe { std::env::set_var("PI_CODING_AGENT_DIR", dir) };
        let out = f();
        unsafe {
            match prev {
                Some(v) => std::env::set_var("PI_CODING_AGENT_DIR", v),
                None => std::env::remove_var("PI_CODING_AGENT_DIR"),
            }
        }
        out
    }

    fn config(patch: serde_json::Value) -> bootstrap::BootstrapConfig {
        bootstrap::BootstrapConfig {
            agents: [(
                "pi".to_string(),
                bootstrap::AgentBootstrapOverrides {
                    settings: Some(patch),
                    ..Default::default()
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        }
    }

    fn pi_check() -> setup::AgentCheck {
        setup::AgentCheck {
            agent: Agent::Pi,
            reason: "test",
            status: StatusCheck::Installed,
        }
    }

    fn run(dir: &Path, patch: serde_json::Value, dry_run: bool) -> Vec<ItemResult> {
        with_pi_dir(dir, || agent_settings(&[pi_check()], &config(patch), dry_run))
    }

    fn settings(dir: &Path) -> serde_json::Value {
        serde_json::from_str(&std::fs::read_to_string(dir.join("settings.json")).unwrap()).unwrap()
    }

    #[test]
    fn patches_keys_and_preserves_the_rest() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("settings.json"),
            r#"{"packages":["npm:keep"],"defaultTools":["read","bash"],"theme":"dark"}"#,
        )
        .unwrap();

        let out = run(
            tmp.path(),
            serde_json::json!({
                "defaultTools": ["read", "edit"],
                "defaultThinkingLevel": "high",
                "theme": null,
            }),
            false,
        );

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].outcome, Outcome::Updated, "{:?}", out[0]);
        let got = settings(tmp.path());
        assert_eq!(got["defaultTools"], serde_json::json!(["read", "edit"]));
        assert_eq!(got["defaultThinkingLevel"], "high");
        assert!(got.get("theme").is_none(), "null deletes the key");
        assert_eq!(
            got["packages"],
            serde_json::json!(["npm:keep"]),
            "agent-owned keys survive"
        );
    }

    #[test]
    fn second_run_is_a_no_op() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("settings.json"), r#"{"packages":[]}"#).unwrap();
        let patch = serde_json::json!({"defaultTools": ["read"]});

        run(tmp.path(), patch.clone(), false);
        let first = std::fs::read_to_string(tmp.path().join("settings.json")).unwrap();
        let out = run(tmp.path(), patch, false);

        assert_eq!(out[0].outcome, Outcome::UpToDate, "{:?}", out[0]);
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("settings.json")).unwrap(),
            first
        );
    }

    #[test]
    fn creates_the_file_when_absent() {
        let tmp = tempfile::tempdir().unwrap();
        let out = run(tmp.path(), serde_json::json!({"defaultTools": []}), false);

        assert_eq!(out[0].outcome, Outcome::Installed, "{:?}", out[0]);
        assert_eq!(settings(tmp.path()), serde_json::json!({"defaultTools": []}));
    }

    #[test]
    fn corrupt_file_fails_without_writing() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("settings.json"), "{not json").unwrap();

        let out = run(tmp.path(), serde_json::json!({"a": 1}), false);

        assert_eq!(out[0].outcome, Outcome::Failed, "{:?}", out[0]);
        assert_eq!(
            std::fs::read_to_string(tmp.path().join("settings.json")).unwrap(),
            "{not json"
        );
    }

    #[test]
    fn dry_run_reports_drift_without_writing() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("settings.json"), r#"{"a":1}"#).unwrap();

        let out = run(tmp.path(), serde_json::json!({"a": 2}), true);

        assert!(out[0].outcome.is_drift(), "{:?}", out[0]);
        assert!(out[0].detail.as_deref().unwrap().contains('a'));
        assert_eq!(settings(tmp.path()), serde_json::json!({"a": 1}));
    }

    /// Same section, a different agent and a different settings file: the
    /// per-agent path resolver is the only thing that varies.
    #[test]
    fn patches_claude_settings_too() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("settings.json"),
            r#"{"permissions":{"allow":["Bash"]}}"#,
        )
        .unwrap();

        let config = bootstrap::BootstrapConfig {
            agents: [(
                "claude code".to_string(),
                bootstrap::AgentBootstrapOverrides {
                    settings: Some(serde_json::json!({"autoCompact": true})),
                    ..Default::default()
                },
            )]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        let check = setup::AgentCheck {
            agent: Agent::Claude,
            reason: "test",
            status: StatusCheck::Installed,
        };

        let out = {
            let _g = super::super::tests::ENV_LOCK
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let prev = std::env::var_os("CLAUDE_CONFIG_DIR");
            unsafe { std::env::set_var("CLAUDE_CONFIG_DIR", tmp.path()) };
            let out = agent_settings(&[check], &config, false);
            unsafe {
                match prev {
                    Some(v) => std::env::set_var("CLAUDE_CONFIG_DIR", v),
                    None => std::env::remove_var("CLAUDE_CONFIG_DIR"),
                }
            }
            out
        };

        assert_eq!(out[0].outcome, Outcome::Updated, "{:?}", out[0]);
        let got = settings(tmp.path());
        assert_eq!(got["autoCompact"], true);
        assert_eq!(got["permissions"]["allow"], serde_json::json!(["Bash"]));
    }

    #[test]
    fn agent_without_a_known_settings_file_is_skipped() {
        let out = agent_settings(
            &[setup::AgentCheck {
                // Codex config is TOML, so a JSON merge patch cannot apply.
                agent: Agent::Codex,
                reason: "test",
                status: StatusCheck::Installed,
            }],
            &bootstrap::BootstrapConfig {
                agents: [(
                    "codex".to_string(),
                    bootstrap::AgentBootstrapOverrides {
                        settings: Some(serde_json::json!({"a": 1})),
                        ..Default::default()
                    },
                )]
                .into_iter()
                .collect(),
                ..Default::default()
            },
            false,
        );

        assert_eq!(out.len(), 1);
        assert_eq!(out[0].outcome, Outcome::Skipped, "{:?}", out[0]);
    }
}

#[cfg(test)]
mod deps_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// A fake `npm` on PATH that logs its argv and fakes the package dir the
    /// real one would create, so install/uninstall/version paths are exercised
    /// without the network.
    fn with_fake_npm<T>(tmp: &Path, f: impl FnOnce() -> T) -> T {
        let _g = super::super::tests::ENV_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let bin = tmp.join("shim");
        std::fs::create_dir_all(&bin).unwrap();
        let npm = bin.join("npm");
        std::fs::write(
            &npm,
            format!(
                "#!/bin/sh\necho \"$@\" >> {log}\ncmd=$1; shift 3; prefix=$1; shift\nspec=$1\ncase $cmd in\n  install) name=${{spec%@*}}; [ -z \"$name\" ] && name=$spec; ver=${{spec##*@}}; d=$prefix/lib/node_modules/$name; mkdir -p $d; echo \"{{\\\"version\\\":\\\"$ver\\\"}}\" > $d/package.json;;\n  uninstall) rm -rf $prefix/lib/node_modules/$spec;;\nesac\n",
                log = tmp.join("npm.log").display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&npm, std::fs::Permissions::from_mode(0o755)).unwrap();
        let prev = std::env::var_os("PATH");
        let path = std::env::join_paths(
            std::iter::once(bin).chain(prev.iter().flat_map(std::env::split_paths)),
        )
        .unwrap();
        unsafe { std::env::set_var("PATH", &path) };
        let out = f();
        unsafe {
            match prev {
                Some(v) => std::env::set_var("PATH", v),
                None => std::env::remove_var("PATH"),
            }
        }
        out
    }

    fn config(prefix: &Path, yaml: &str) -> Config {
        let yaml = format!("bootstrap:\n  npm_prefix: {}\n{yaml}", prefix.display());
        serde_yaml::from_str(&yaml).unwrap()
    }

    fn by_name<'a>(items: &'a [ItemResult], name: &str) -> &'a ItemResult {
        items.iter().find(|i| i.name == name).unwrap_or_else(|| panic!("no item {name} in {items:?}"))
    }

    #[test]
    fn npm_install_update_prune_and_bin_assert() {
        let tmp = tempfile::tempdir().unwrap();
        let prefix = tmp.path().join("np");
        let log = tmp.path().join("npm.log");
        with_fake_npm(tmp.path(), || {
            let cfg = config(
                &prefix,
                "  default_skills:\n    - path: ./skills/x\n      requires:\n        npm: [\"foo@1.0.0\", \"@s/bar\"]\n        bin: [sh, definitely-not-a-binary-xyz]\nmcp:\n  srv:\n    command: foo\n    requires: {npm: [\"foo@1.0.0\"]}\n",
            );

            // --check: drift, no npm invocation.
            let items = deps(&cfg, true);
            assert_eq!(by_name(&items, "foo").outcome, Outcome::Installed);
            assert!(by_name(&items, "foo").detail.as_deref().unwrap().contains("skill x, mcp srv"));
            assert!(by_name(&items, "@s/bar").detail.as_deref().unwrap().contains("unpinned"));
            assert_eq!(by_name(&items, "sh").outcome, Outcome::UpToDate);
            let missing = by_name(&items, "definitely-not-a-binary-xyz");
            assert_eq!(missing.outcome, Outcome::Failed);
            assert!(missing.detail.as_deref().unwrap().contains("home.packages"));
            assert!(!log.exists());

            // apply: installs both, records targets.
            let items = deps(&cfg, false);
            assert_eq!(by_name(&items, "foo").outcome, Outcome::Installed, "{:?}", by_name(&items, "foo"));
            assert!(by_name(&items, "foo").managed.as_deref().unwrap().ends_with("lib/node_modules/foo"));
            assert_eq!(crate::deps::installed_version(&prefix, "foo").as_deref(), Some("1.0.0"));
            assert_eq!(deps(&cfg, false).iter().filter(|i| i.outcome == Outcome::UpToDate).count(), 3);

            // version bump → updated.
            let cfg2 = config(&prefix, "  default_skills:\n    - path: ./skills/x\n      requires: {npm: [\"foo@2.0.0\"]}\n");
            assert_eq!(by_name(&deps(&cfg2, false), "foo").outcome, Outcome::Updated);
            assert_eq!(crate::deps::installed_version(&prefix, "foo").as_deref(), Some("2.0.0"));

            // conflicting pins → one failure, nothing installed.
            let cfg3 = config(&prefix, "  default_skills:\n    - path: ./skills/x\n      requires: {npm: [\"baz@1.0.0\"]}\n    - path: ./skills/y\n      requires: {npm: [\"baz@2.0.0\"]}\n");
            let items = deps(&cfg3, false);
            assert_eq!(by_name(&items, "baz").outcome, Outcome::Failed);
            assert!(crate::deps::installed_version(&prefix, "baz").is_none());

            // prune: manifest entry for a package nobody declares any more.
            let entry = super::super::managed::ManagedEntry {
                section: Section::Deps,
                agent: None,
                name: "foo".into(),
                project: tmp.path().to_path_buf(),
                target: crate::deps::package_dir(&prefix, "foo").to_string_lossy().into_owned(),
            };
            let removed = super::super::managed::prune(&[entry], false);
            assert_eq!(removed[0].outcome, Outcome::Removed, "{removed:?}");
            assert!(crate::deps::installed_version(&prefix, "foo").is_none());

            // strict: hand-installed package surfaces as a stale entry; default does not.
            let hand = crate::deps::package_dir(&prefix, "hand");
            std::fs::create_dir_all(&hand).unwrap();
            std::fs::write(hand.join("package.json"), "{\"version\":\"0.0.1\"}").unwrap();
            let declared = ["@s/bar".to_string()].into_iter().collect();
            let stale = super::super::managed::undeclared_npm(&prefix, &declared, tmp.path());
            assert_eq!(stale.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), ["hand"]);
        });
        let calls = std::fs::read_to_string(&log).unwrap();
        assert!(calls.contains("install -g --prefix"), "{calls}");
        assert!(calls.contains("uninstall -g --prefix"), "{calls}");
    }

    #[test]
    fn deps_without_requires_is_silent_and_npm_missing_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let cfg = config(tmp.path(), "  default_skills: [./skills/x]\n");
        assert!(deps(&cfg, true).is_empty());

        let _g = super::super::tests::ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var_os("PATH");
        unsafe { std::env::set_var("PATH", tmp.path()) };
        let cfg = config(tmp.path(), "  default_skills:\n    - path: ./skills/x\n      requires: {npm: [foo]}\n");
        let items = deps(&cfg, true);
        unsafe {
            match prev {
                Some(v) => std::env::set_var("PATH", v),
                None => std::env::remove_var("PATH"),
            }
        }
        let foo = by_name(&items, "foo");
        assert_eq!(foo.outcome, Outcome::Failed);
        assert!(foo.detail.as_deref().unwrap().contains("npm not on PATH"));
    }
}
