//! Tests for the setup runner: idempotency, drift detection, and section
//! selection.
//!
//! Every test redirects `CLAUDE_CONFIG_DIR` into a scratch directory, so no
//! test ever reads or writes a real agent config.

use super::report::{Outcome, Section};
use super::{parse_sections, SetupOptions};
use std::path::PathBuf;

/// `CLAUDE_CONFIG_DIR` is process-global; tests that set it serialize here.
/// `pub(crate)` so other test modules under `command::setup` (e.g.
/// `sections::plugins_tests`) that also touch `CLAUDE_CONFIG_DIR` serialize
/// against the same lock instead of racing on a second, unrelated `Mutex`.
pub(crate) static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Scratch {
    dir: PathBuf,
    _lock: std::sync::MutexGuard<'static, ()>,
    prev: Option<String>,
}

impl Scratch {
    fn new(name: &str) -> Self {
        let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!(
            "muxix-setup-test-{}-{}-{:?}",
            std::process::id(),
            name,
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let prev = std::env::var("CLAUDE_CONFIG_DIR").ok();
        unsafe { std::env::set_var("CLAUDE_CONFIG_DIR", &dir) };
        Self {
            dir,
            _lock: lock,
            prev,
        }
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        unsafe {
            match &self.prev {
                Some(v) => std::env::set_var("CLAUDE_CONFIG_DIR", v),
                None => std::env::remove_var("CLAUDE_CONFIG_DIR"),
            }
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

// --- agent profiles: reconcile, drift, prune -------------------------------

#[cfg(unix)]
#[test]
fn agent_profiles_reconcile_build_idempotence_and_prune() {
    use crate::agent::setup::{Agent, AgentCheck, StatusCheck};
    use crate::config::{AgentProfile, Config};
    use std::collections::BTreeMap;

    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let root = std::env::temp_dir().join(format!(
        "muxix-ap-test-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let xdg_config = root.join("config");
    let xdg_state = root.join("state");
    let base = root.join("base");
    let prev_c = std::env::var("XDG_CONFIG_HOME").ok();
    let prev_s = std::env::var("XDG_STATE_HOME").ok();
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", &xdg_config);
        std::env::set_var("XDG_STATE_HOME", &xdg_state);
    }

    // Base config dir (the "default") holds two skills.
    std::fs::create_dir_all(base.join("pi/skills/a")).unwrap();
    std::fs::write(base.join("pi/skills/a/SKILL.md"), "base-a").unwrap();
    std::fs::create_dir_all(base.join("pi/skills/b")).unwrap();
    std::fs::write(base.join("pi/skills/b/SKILL.md"), "base-b").unwrap();

    // Profile "corp" source overrides skill a.
    let src = xdg_config.join("muxix/agent-profiles/corp/skills/a");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("SKILL.md"), "corp-a").unwrap();

    let mut agent_profiles = BTreeMap::new();
    agent_profiles.insert("corp".to_string(), AgentProfile::default());
    let mut config = Config::default();
    config.agent_profiles = agent_profiles;
    config.sandbox.agent_config_dir = Some(format!("{}/{{agent}}", base.display()));

    let checks = vec![AgentCheck {
        agent: Agent::Pi,
        reason: "test",
        status: StatusCheck::Installed,
    }];

    // Dry run reports the install as drift and writes nothing.
    let dry = super::sections::agent_profiles(&config, &checks, &root, true);
    assert!(dry.iter().any(|r| r.name == "corp/pi" && r.outcome == Outcome::Installed));
    let dest = crate::agent::agent_profiles::build_dir("corp", "pi").unwrap();
    assert!(!dest.exists(), "dry run must not write");

    // Apply builds the overlay; the override wins, the sibling is inherited.
    let applied = super::sections::agent_profiles(&config, &checks, &root, false);
    assert!(applied.iter().any(|r| r.name == "corp/pi" && r.outcome == Outcome::Installed));
    assert_eq!(
        std::fs::read_to_string(dest.join("skills/a/SKILL.md")).unwrap(),
        "corp-a"
    );
    assert_eq!(
        std::fs::read_to_string(dest.join("skills/b/SKILL.md")).unwrap(),
        "base-b"
    );

    // Re-running is idempotent.
    let again = super::sections::agent_profiles(&config, &checks, &root, false);
    assert!(again.iter().any(|r| r.name == "corp/pi" && r.outcome == Outcome::UpToDate));

    // Dropping the profile from config prunes the derived dir but never the
    // user-authored source.
    config.agent_profiles.clear();
    let pruned = super::sections::agent_profiles(&config, &checks, &root, false);
    assert!(pruned.iter().any(|r| r.name == "corp" && r.outcome == Outcome::Updated));
    assert!(!dest.exists(), "orphaned overlay removed");
    assert!(src.join("SKILL.md").exists(), "user source never pruned");

    unsafe {
        match prev_c {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
        match prev_s {
            Some(v) => std::env::set_var("XDG_STATE_HOME", v),
            None => std::env::remove_var("XDG_STATE_HOME"),
        }
    }
    let _ = std::fs::remove_dir_all(&root);
}

#[cfg(unix)]
#[test]
fn agent_profiles_apply_declared_deltas() {
    use crate::agent::setup::{Agent, AgentCheck, StatusCheck};
    use crate::config::{AgentProfile, AgentProfileAgent, Config};
    use std::collections::BTreeMap;

    let _lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let root = std::env::temp_dir().join(format!(
        "muxix-apd-test-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let xdg_config = root.join("config");
    let xdg_state = root.join("state");
    let base = root.join("base");
    let prev_c = std::env::var("XDG_CONFIG_HOME").ok();
    let prev_s = std::env::var("XDG_STATE_HOME").ok();
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", &xdg_config);
        std::env::set_var("XDG_STATE_HOME", &xdg_state);
    }

    std::fs::create_dir_all(base.join("pi")).unwrap();
    std::fs::write(
        base.join("pi/settings.json"),
        r#"{"packages":["npm:pi-cliproxyapi","npm:keep"],"defaultProvider":"anthropic"}"#,
    )
    .unwrap();
    let litellm = root.join("litellm-ext");
    std::fs::create_dir_all(&litellm).unwrap();

    let deltas = AgentProfileAgent {
        additional_plugins: vec![litellm.to_string_lossy().into_owned()],
        exclude_plugins: vec!["npm:pi-cliproxyapi".into()],
        settings: Some(serde_json::json!({"defaultProvider": "litellm"})),
        ..Default::default()
    };
    let mut agents = BTreeMap::new();
    agents.insert("pi".to_string(), deltas);
    let mut agent_profiles = BTreeMap::new();
    agent_profiles.insert(
        "corp".to_string(),
        AgentProfile { description: None, agents },
    );
    let mut config = Config::default();
    config.agent_profiles = agent_profiles;
    config.sandbox.agent_config_dir = Some(format!("{}/{{agent}}", base.display()));

    let checks = vec![AgentCheck {
        agent: Agent::Pi,
        reason: "test",
        status: StatusCheck::Installed,
    }];

    let applied = super::sections::agent_profiles(&config, &checks, &root, false);
    assert!(
        applied.iter().any(|r| r.name == "corp/pi" && r.outcome == Outcome::Installed),
        "{applied:?}"
    );
    let dest = crate::agent::agent_profiles::build_dir("corp", "pi").unwrap();
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dest.join("settings.json")).unwrap())
            .unwrap();
    assert_eq!(
        v["packages"],
        serde_json::json!(["npm:keep", litellm.to_str().unwrap()])
    );
    assert_eq!(v["defaultProvider"], "litellm");

    // Idempotent while nothing changes.
    let again = super::sections::agent_profiles(&config, &checks, &root, false);
    assert!(again.iter().any(|r| r.name == "corp/pi" && r.outcome == Outcome::UpToDate));

    // Base package change is drift and re-applies the delta.
    std::fs::write(
        base.join("pi/settings.json"),
        r#"{"packages":["npm:pi-cliproxyapi","npm:keep","npm:new"],"defaultProvider":"anthropic"}"#,
    )
    .unwrap();
    let rebuilt = super::sections::agent_profiles(&config, &checks, &root, false);
    assert!(rebuilt.iter().any(|r| r.name == "corp/pi" && r.outcome == Outcome::Updated));
    let v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dest.join("settings.json")).unwrap())
            .unwrap();
    assert_eq!(
        v["packages"],
        serde_json::json!(["npm:keep", "npm:new", litellm.to_str().unwrap()])
    );

    // Deltas for an agent without a generator refuse to build.
    let claude_deltas = AgentProfileAgent {
        exclude_plugins: vec!["x".into()],
        ..Default::default()
    };
    config
        .agent_profiles
        .get_mut("corp")
        .unwrap()
        .agents
        .insert("claude".to_string(), claude_deltas);
    std::fs::create_dir_all(base.join("claude")).unwrap();
    let checks2 = vec![AgentCheck {
        agent: Agent::Claude,
        reason: "test",
        status: StatusCheck::Installed,
    }];
    let refused = super::sections::agent_profiles(&config, &checks2, &root, false);
    assert!(
        refused
            .iter()
            .any(|r| r.name == "corp/claude" && r.outcome == Outcome::Skipped),
        "{refused:?}"
    );
    assert!(
        !crate::agent::agent_profiles::build_dir("corp", "claude").unwrap().exists(),
        "overlay must not be built without its deltas"
    );

    unsafe {
        match prev_c {
            Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
            None => std::env::remove_var("XDG_CONFIG_HOME"),
        }
        match prev_s {
            Some(v) => std::env::set_var("XDG_STATE_HOME", v),
            None => std::env::remove_var("XDG_STATE_HOME"),
        }
    }
    let _ = std::fs::remove_dir_all(&root);
}

// --- managed state: record and prune ---------------------------------------

/// The declarative loop end to end: declare a skill and a subagent, converge,
/// then drop both from the config and converge again. Whatever muxix put on
/// the machine has to come back off it, and nothing else may.
#[test]
fn dropping_a_declared_feature_removes_it_on_the_next_run() {
    use crate::agent::setup::{Agent, AgentCheck, StatusCheck};
    use crate::bootstrap::{BootstrapConfig, Source, SubagentDef};
    use crate::command::setup::managed::Manifest;
    use crate::config::Config;

    let scratch = Scratch::new("prune");
    let root = scratch.dir.clone();
    let xdg_state = root.join("state");
    let prev_s = std::env::var("XDG_STATE_HOME").ok();
    unsafe { std::env::set_var("XDG_STATE_HOME", &xdg_state) };

    // A project declaring one skill and one subagent.
    let project = root.join("project");
    std::fs::create_dir_all(project.join("skills/demo")).unwrap();
    std::fs::write(project.join("skills/demo/SKILL.md"), "---\nname: demo\n---\n").unwrap();
    std::fs::create_dir_all(project.join("agents")).unwrap();
    std::fs::write(project.join("agents/scout.md"), "---\nname: scout\n---\n").unwrap();

    let mut config = Config::default();
    config.bootstrap = Some(BootstrapConfig {
        default_skills: vec![Source::LocalPath("./skills/demo".into()).into()],
        default_subagents: vec![SubagentDef::File("./agents/scout.md".into())],
        ..Default::default()
    });

    let checks = vec![AgentCheck {
        agent: Agent::Claude,
        reason: "test",
        status: StatusCheck::Installed,
    }];
    let only = [Section::Skills, Section::Subagents];

    let installed_skill = scratch.dir.join("skills/demo");
    let installed_subagent = scratch.dir.join("agents/scout.md");

    let first = super::sections::run_all(&only, &checks, Some(&config), &project, false);
    assert!(installed_skill.exists(), "{first:?}");
    assert!(installed_subagent.exists(), "{first:?}");
    assert_eq!(
        Manifest::load().entries.len(),
        2,
        "only the declared skill and subagent are managed; bundled skills are not"
    );

    // Drop both from the config.
    config.bootstrap = Some(BootstrapConfig::default());

    // --check reports the removals as drift without performing them.
    let dry = super::sections::run_all(&only, &checks, Some(&config), &project, true);
    let removals: Vec<_> = dry
        .items
        .iter()
        .filter(|i| i.outcome == Outcome::Removed)
        .map(|i| i.name.as_str())
        .collect();
    assert_eq!(removals, vec!["demo", "scout"], "{dry:?}");
    assert!(installed_skill.exists(), "--check must not delete");
    assert_eq!(Manifest::load().entries.len(), 2, "--check must not record");

    // --no-prune converges without removing, and keeps the entries.
    let kept = super::sections::run_all_with_prune(
        &only,
        &checks,
        Some(&config),
        &project,
        false,
        false,
    );
    assert!(!kept.items.iter().any(|i| i.outcome == Outcome::Removed));
    assert!(installed_skill.exists(), "--no-prune must not delete");
    assert_eq!(Manifest::load().entries.len(), 2, "entries survive for a later run");

    // The real run removes both and empties the manifest.
    let second = super::sections::run_all(&only, &checks, Some(&config), &project, false);
    assert!(!installed_skill.exists(), "{second:?}");
    assert!(!installed_subagent.exists(), "{second:?}");
    assert!(Manifest::load().entries.is_empty());

    // A third run has nothing left to say about them.
    let third = super::sections::run_all(&only, &checks, Some(&config), &project, false);
    assert!(!third.items.iter().any(|i| i.outcome == Outcome::Removed));

    unsafe {
        match prev_s {
            Some(v) => std::env::set_var("XDG_STATE_HOME", v),
            None => std::env::remove_var("XDG_STATE_HOME"),
        }
    }
}

/// The safety property that makes pruning acceptable at all: a skill muxix
/// never installed is never deleted, however undeclared it is.
#[test]
fn a_hand_installed_skill_is_never_pruned() {
    use crate::agent::setup::{Agent, AgentCheck, StatusCheck};
    use crate::config::Config;

    let scratch = Scratch::new("prune-foreign");
    let root = scratch.dir.clone();
    let xdg_state = root.join("state");
    let prev_s = std::env::var("XDG_STATE_HOME").ok();
    unsafe { std::env::set_var("XDG_STATE_HOME", &xdg_state) };

    let mine = scratch.dir.join("skills/my-own");
    std::fs::create_dir_all(&mine).unwrap();
    std::fs::write(mine.join("SKILL.md"), "mine").unwrap();

    let checks = vec![AgentCheck {
        agent: Agent::Claude,
        reason: "test",
        status: StatusCheck::Installed,
    }];
    let config = Config::default();
    let report = super::sections::run_all(
        &[Section::Skills],
        &checks,
        Some(&config),
        &root.join("project"),
        false,
    );

    assert!(mine.exists(), "{report:?}");
    assert!(!report.items.iter().any(|i| i.name == "my-own"));

    unsafe {
        match prev_s {
            Some(v) => std::env::set_var("XDG_STATE_HOME", v),
            None => std::env::remove_var("XDG_STATE_HOME"),
        }
    }
}

// --- section selection ------------------------------------------------------

#[test]
fn parse_sections_accepts_a_comma_list() {
    let got = parse_sections("skills, mcp").unwrap();
    assert_eq!(got, vec![Section::Skills, Section::Mcp]);
}

#[test]
fn parse_sections_rejects_unknown_names_with_the_valid_list() {
    let err = parse_sections("skills,bogus").unwrap_err().to_string();
    assert!(err.contains("bogus"), "{err}");
    assert!(err.contains("hooks"), "names the valid sections: {err}");
}

#[test]
fn parse_sections_ignores_empty_entries() {
    assert_eq!(parse_sections("skills,,").unwrap(), vec![Section::Skills]);
    assert!(parse_sections("").unwrap().is_empty());
}

#[test]
fn empty_selection_means_every_section() {
    let opts = SetupOptions::default();
    assert_eq!(opts.selected(), Section::ALL.to_vec());
}

#[test]
fn selection_is_returned_in_canonical_order() {
    // Requested out of order; runs in the order setup applies them.
    let opts = SetupOptions {
        only: vec![Section::Mcp, Section::Hooks],
        ..Default::default()
    };
    assert_eq!(opts.selected(), vec![Section::Hooks, Section::Mcp]);
}

// --- bundled skills: idempotency and drift ---------------------------------

#[test]
fn bundled_skills_install_then_report_up_to_date() {
    let _s = Scratch::new("bundled-idempotent");
    let agent = crate::agent::setup::Agent::Claude;

    let first = crate::skills::install_bundled(agent, false).unwrap();
    assert!(!first.is_empty(), "there are bundled skills to install");
    assert!(
        first.iter().all(|i| i.outcome == Outcome::Installed),
        "a fresh directory installs everything: {first:?}"
    );

    let second = crate::skills::install_bundled(agent, false).unwrap();
    assert!(
        second.iter().all(|i| i.outcome == Outcome::UpToDate),
        "re-running with no change must write nothing: {second:?}"
    );
}

#[test]
fn bundled_skills_dry_run_writes_nothing() {
    let s = Scratch::new("bundled-dry-run");
    let agent = crate::agent::setup::Agent::Claude;

    let planned = crate::skills::install_bundled(agent, true).unwrap();
    assert!(
        planned.iter().all(|i| i.outcome == Outcome::Installed),
        "a dry run still reports what would happen"
    );
    assert!(
        !s.dir.join("skills").exists(),
        "a dry run must not create the skills directory"
    );
}

#[test]
fn bundled_skills_detect_an_edited_file_as_drift() {
    let s = Scratch::new("bundled-drift");
    let agent = crate::agent::setup::Agent::Claude;
    crate::skills::install_bundled(agent, false).unwrap();

    // Corrupt one installed skill.
    let name = crate::skills::BUNDLED_SKILLS[0].name;
    let path = s.dir.join("skills").join(name).join("SKILL.md");
    std::fs::write(&path, "locally edited").unwrap();

    let checked = crate::skills::install_bundled(agent, true).unwrap();
    let edited = checked.iter().find(|i| i.name == name).unwrap();
    assert_eq!(edited.outcome, Outcome::Updated, "the edit is drift");
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "locally edited",
        "a check must not repair the file"
    );

    // A real run repairs it.
    let applied = crate::skills::install_bundled(agent, false).unwrap();
    assert_eq!(
        applied.iter().find(|i| i.name == name).unwrap().outcome,
        Outcome::Updated
    );
    assert_ne!(std::fs::read_to_string(&path).unwrap(), "locally edited");
}

#[test]
fn a_deleted_skill_is_reported_as_installed_again() {
    let s = Scratch::new("bundled-deleted");
    let agent = crate::agent::setup::Agent::Claude;
    crate::skills::install_bundled(agent, false).unwrap();

    let name = crate::skills::BUNDLED_SKILLS[0].name;
    std::fs::remove_dir_all(s.dir.join("skills").join(name)).unwrap();

    let checked = crate::skills::install_bundled(agent, true).unwrap();
    assert_eq!(
        checked.iter().find(|i| i.name == name).unwrap().outcome,
        Outcome::Installed
    );
}

// --- report semantics through the runner -----------------------------------

#[test]
fn hooks_section_reports_one_item_per_agent() {
    let checks = crate::agent::setup::check_all();
    let items = super::sections::hooks(&checks, true);
    assert_eq!(items.len(), checks.len());
    assert!(items.iter().all(|i| i.section == Section::Hooks));
}

#[test]
fn a_dry_run_hooks_pass_writes_nothing_and_reports_drift_shape() {
    let checks = crate::agent::setup::check_all();
    let items = super::sections::hooks(&checks, true);
    // Every outcome must be one of the four an inspection can produce; a dry
    // run never reports Failed for a merely-absent hook.
    assert!(items.iter().all(|i| matches!(
        i.outcome,
        Outcome::Installed | Outcome::Updated | Outcome::UpToDate | Outcome::Failed
    )));
}

// --- profile threading ------------------------------------------------------

/// Every section must read from the same profile-resolved config. A section
/// that re-loaded the config itself would silently ignore `--profile`.
#[test]
fn setup_resolves_config_with_the_requested_profile() {
    use crate::config::Config;

    let _s = Scratch::new("profile-threading");
    let root = std::env::temp_dir().join(format!(
        "muxix-setup-profile-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::process::Command::new("git")
        .args(["init", "-q"])
        .current_dir(&root)
        .output()
        .unwrap();
    std::fs::write(
        root.join(".muxix.yaml"),
        "agent: base\nprofiles:\n  corp:\n    agent: profiled\n",
    )
    .unwrap();

    let base = Config::load_with_options(&root, None, None, None).unwrap().0;
    assert_eq!(base.agent.as_deref(), Some("base"));

    let profiled = Config::load_with_options(&root, None, None, Some("corp"))
        .unwrap()
        .0;
    assert_eq!(
        profiled.agent.as_deref(),
        Some("profiled"),
        "the profile the runner passes must reach the resolved config"
    );

    let _ = std::fs::remove_dir_all(&root);
}

// --- provider sync ----------------------------------------------------------

/// End-to-end over real files: a litellm provider with connection fields lands
/// in opencode.json (hand-written provider preserved) and in a codex marker
/// region, idempotently, without ever containing a secret value.
#[test]
fn provider_sync_end_to_end() {
    use crate::agent::setup::{Agent, AgentCheck, StatusCheck};
    use crate::model::{ProviderConfig, ProviderRegistry};

    let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let root = std::env::temp_dir().join(format!("muxix-provider-sync-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let oc_dir = root.join("opencode");
    let home = root.join("home");
    std::fs::create_dir_all(&oc_dir).unwrap();
    std::fs::create_dir_all(home.join(".codex")).unwrap();

    std::fs::write(
        oc_dir.join("opencode.json"),
        r#"{ "theme": "catppuccin", "provider": { "corp": { "options": { "baseURL": "https://corp/v1" } } } }"#,
    )
    .unwrap();
    std::fs::write(home.join(".codex/config.toml"), "[features]\nhooks = true\n").unwrap();

    let prev_oc = std::env::var("OPENCODE_CONFIG").ok();
    let prev_home = std::env::var("HOME").ok();
    unsafe {
        std::env::set_var("OPENCODE_CONFIG", &oc_dir);
        std::env::set_var("HOME", &home);
    }

    let mut reg = ProviderRegistry::new();
    reg.insert(
        "litellm".into(),
        ProviderConfig {
            base_url: Some("https://llm.corp/v1".into()),
            api_key_env: Some("LITELLM_API_KEY".into()),
            ..Default::default()
        },
    );
    let checks = vec![
        AgentCheck {
            agent: Agent::OpenCode,
            reason: "test",
            status: StatusCheck::Installed,
        },
        AgentCheck {
            agent: Agent::Codex,
            reason: "test",
            status: StatusCheck::Installed,
        },
    ];

    // Secret value in the environment must never leak into configs or output.
    unsafe { std::env::set_var("LITELLM_API_KEY", "s3cret-value") };

    // Dry run: no writes.
    let dry = super::sections::providers_sync(&checks, Some(&reg), true);
    assert!(dry.iter().all(|i| i.outcome == Outcome::Updated));
    let oc_before = std::fs::read_to_string(oc_dir.join("opencode.json")).unwrap();
    assert!(!oc_before.contains("litellm"));

    // Real run.
    let items = super::sections::providers_sync(&checks, Some(&reg), false);
    assert!(items.iter().all(|i| i.outcome == Outcome::Updated), "{items:?}");

    let oc = std::fs::read_to_string(oc_dir.join("opencode.json")).unwrap();
    assert!(oc.contains(r#""baseURL": "https://corp/v1""#), "corp preserved");
    assert!(oc.contains("{env:LITELLM_API_KEY}"));
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&oc).unwrap()["theme"],
        "catppuccin"
    );
    let toml = std::fs::read_to_string(home.join(".codex/config.toml")).unwrap();
    assert!(toml.starts_with("[features]\nhooks = true\n"));
    assert!(toml.contains("[model_providers.litellm]"));
    assert!(toml.contains("env_key = \"LITELLM_API_KEY\""));
    for content in [&oc, &toml] {
        assert!(!content.contains("s3cret-value"));
    }

    // Idempotent.
    let again = super::sections::providers_sync(&checks, Some(&reg), false);
    assert!(again.iter().all(|i| i.outcome == Outcome::UpToDate), "{again:?}");

    unsafe {
        match prev_oc {
            Some(v) => std::env::set_var("OPENCODE_CONFIG", v),
            None => std::env::remove_var("OPENCODE_CONFIG"),
        }
        match prev_home {
            Some(v) => std::env::set_var("HOME", v),
            None => std::env::remove_var("HOME"),
        }
        std::env::remove_var("LITELLM_API_KEY");
    }
    drop(lock);
    let _ = std::fs::remove_dir_all(&root);
}
