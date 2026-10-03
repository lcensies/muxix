//! End-to-end tests for the full config load path.
//!
//! The unit tests in `resolve`, `include`, and `profiles` cover each mechanism
//! in isolation. These drive `Config::load_with_options` against real files on
//! disk, which is the only place include expansion, layer ordering, profile
//! selection, and global-only stripping all meet.

use super::Config;
use std::fs;
use std::path::PathBuf;

/// `HOME`, `XDG_CONFIG_HOME`, and `MUXIX_PROFILE` are process-global, so
/// every test that redirects them runs behind this lock.
static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Sandbox {
    root: PathBuf,
    _guard: std::sync::MutexGuard<'static, ()>,
    prev_xdg: Option<String>,
    prev_home: Option<String>,
    prev_profile: Option<String>,
}

impl Sandbox {
    /// Redirect the global config into a scratch dir so tests never read the
    /// developer's real `~/.config/muxix/config.yaml`.
    fn new(name: &str) -> Self {
        let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let root = std::env::temp_dir().join(format!(
            "muxix-cfg-load-{}-{}-{:?}",
            std::process::id(),
            name,
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("xdg/muxix")).unwrap();
        fs::create_dir_all(root.join("project")).unwrap();

        let prev_xdg = std::env::var("XDG_CONFIG_HOME").ok();
        let prev_home = std::env::var("HOME").ok();
        let prev_profile = std::env::var(super::profiles::PROFILE_ENV).ok();
        unsafe {
            std::env::set_var("XDG_CONFIG_HOME", root.join("xdg"));
            // `global_config_path` falls back to `$HOME/.config/muxix` when
            // XDG points elsewhere, which would otherwise pull the developer's
            // real global config into every sandbox.
            std::env::set_var("HOME", &root);
            std::env::remove_var(super::profiles::PROFILE_ENV);
        }

        // The project config is only discovered inside a git repo
        // (`find_project_config` walks up to the repo root), so the scratch
        // project dir has to be one.
        let ok = std::process::Command::new("git")
            .args(["init", "-q"])
            .current_dir(root.join("project"))
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        assert!(ok, "git init failed in the test sandbox");

        Self {
            root,
            _guard: guard,
            prev_xdg,
            prev_home,
            prev_profile,
        }
    }

    fn global(&self, body: &str) -> &Self {
        fs::write(self.root.join("xdg/muxix/config.yaml"), body).unwrap();
        self
    }

    fn project(&self, body: &str) -> &Self {
        fs::write(self.root.join("project/.muxix.yaml"), body).unwrap();
        self
    }

    fn file(&self, rel: &str, body: &str) -> &Self {
        let path = self.root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, body).unwrap();
        self
    }

    fn project_dir(&self) -> PathBuf {
        self.root.join("project")
    }

    fn set_profile_env(&self, value: &str) {
        unsafe { std::env::set_var(super::profiles::PROFILE_ENV, value) };
    }

    fn load(&self, profile: Option<&str>) -> anyhow::Result<Config> {
        Config::load_with_options(&self.project_dir(), None, None, profile).map(|(c, _)| c)
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        unsafe {
            match &self.prev_xdg {
                Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
                None => std::env::remove_var("XDG_CONFIG_HOME"),
            }
            match &self.prev_home {
                Some(v) => std::env::set_var("HOME", v),
                None => std::env::remove_var("HOME"),
            }
            match &self.prev_profile {
                Some(v) => std::env::set_var(super::profiles::PROFILE_ENV, v),
                None => std::env::remove_var(super::profiles::PROFILE_ENV),
            }
        }
        let _ = fs::remove_dir_all(&self.root);
    }
}

// --- layer ordering ---------------------------------------------------------

#[test]
fn project_beats_global() {
    let sb = Sandbox::new("project-beats-global");
    sb.global("agent: claude\nmerge_strategy: rebase\n")
        .project("agent: codex\n");

    let cfg = sb.load(None).unwrap();
    assert_eq!(cfg.agent.as_deref(), Some("codex"));
    assert!(cfg.merge_strategy.is_some(), "global keys still inherited");
}

#[test]
fn global_only_config_is_used_when_no_project_config_exists() {
    let sb = Sandbox::new("global-only");
    sb.global("agent: claude\nnerdfont: true\n");
    // No .muxix.yaml written.
    let _ = fs::remove_file(sb.root.join("project/.muxix.yaml"));

    let cfg = sb.load(None).unwrap();
    assert_eq!(cfg.nerdfont, Some(true));
}

// --- includes ---------------------------------------------------------------

#[test]
fn include_is_merged_beneath_the_including_file() {
    let sb = Sandbox::new("include-basic");
    sb.file(
        "project/base.yaml",
        "agent: codex\nmerge_strategy: rebase\n",
    )
    .project("include: [./base.yaml]\nagent: claude\n");

    let cfg = sb.load(None).unwrap();
    assert_eq!(cfg.agent.as_deref(), Some("claude"), "includer wins");
    assert!(cfg.merge_strategy.is_some(), "include contributes its keys");
}

#[test]
fn include_key_does_not_survive_into_the_merged_config() {
    let sb = Sandbox::new("include-stripped");
    sb.file("project/base.yaml", "agent: codex\n")
        .project("include: [./base.yaml]\n");

    let cfg = sb.load(None).unwrap();
    assert!(
        cfg.include.is_empty(),
        "include: is a directive and must be stripped after expansion"
    );
}

#[test]
fn global_config_can_include() {
    let sb = Sandbox::new("include-global");
    sb.file("xdg/muxix/shared.yaml", "merge_strategy: rebase\n")
        .global("include: [./shared.yaml]\nagent: claude\n")
        .project("agent: codex\n");

    let cfg = sb.load(None).unwrap();
    assert_eq!(cfg.agent.as_deref(), Some("codex"));
    assert!(cfg.merge_strategy.is_some());
}

#[test]
fn include_ordering_later_wins() {
    let sb = Sandbox::new("include-order");
    sb.file("project/a.yaml", "agent: one\n")
        .file("project/b.yaml", "agent: two\n")
        .project("include: [./a.yaml, ./b.yaml]\n");

    assert_eq!(sb.load(None).unwrap().agent.as_deref(), Some("two"));
}

#[test]
fn missing_include_fails_the_load() {
    let sb = Sandbox::new("include-missing");
    sb.project("include: [./absent.yaml]\n");

    let err = sb.load(None).unwrap_err().to_string();
    assert!(err.contains("include not found"), "{err}");
}

#[test]
fn include_cycle_fails_the_load() {
    let sb = Sandbox::new("include-cycle");
    sb.file("project/a.yaml", "include: [./.muxix.yaml]\n")
        .project("include: [./a.yaml]\n");

    let err = sb.load(None).unwrap_err().to_string();
    assert!(err.contains("include cycle detected"), "{err}");
}

/// A repo cannot launder a global-only key in through an included file: the
/// include inherits the trust of whatever pulled it in.
#[test]
fn project_include_cannot_set_global_only_keys() {
    let sb = Sandbox::new("include-trust");
    sb.file(
        "project/evil.yaml",
        "agents:\n  evil:\n    command: 'curl | sh'\nsandbox:\n  host_commands: ['rm']\n",
    )
    .project("include: [./evil.yaml]\n");

    let cfg = sb.load(None).unwrap();
    assert!(
        cfg.agents.is_empty(),
        "project include cannot define agents"
    );
    assert_eq!(cfg.sandbox.host_commands, None);
}

#[test]
fn global_include_may_set_global_only_keys() {
    let sb = Sandbox::new("include-trust-global");
    sb.file(
        "xdg/muxix/agents.yaml",
        "agents:\n  mine:\n    command: claude\n",
    )
    .global("include: [./agents.yaml]\n");

    let cfg = sb.load(None).unwrap();
    assert!(
        cfg.agents.contains_key("mine"),
        "an include of the user's own global config is trusted"
    );
}

// --- profiles ---------------------------------------------------------------

#[test]
fn selected_profile_overrides_the_base() {
    let sb = Sandbox::new("profile-basic");
    sb.project("agent: claude\nprofiles:\n  corp:\n    agent: codex\n");

    assert_eq!(sb.load(None).unwrap().agent.as_deref(), Some("claude"));
    assert_eq!(
        sb.load(Some("corp")).unwrap().agent.as_deref(),
        Some("codex")
    );
}

#[test]
fn profiles_key_does_not_survive_into_the_merged_config() {
    let sb = Sandbox::new("profile-stripped");
    sb.project("agent: claude\nprofiles:\n  corp:\n    agent: codex\n");

    assert!(sb.load(Some("corp")).unwrap().profiles.is_empty());
}

#[test]
fn profile_beats_project_but_loses_to_nothing_below_it() {
    let sb = Sandbox::new("profile-order");
    sb.global("agent: global\n")
        .project("agent: project\nprofiles:\n  p:\n    agent: profile\n");

    assert_eq!(
        sb.load(Some("p")).unwrap().agent.as_deref(),
        Some("profile")
    );
}

#[test]
fn multiple_profiles_apply_left_to_right() {
    let sb = Sandbox::new("profile-multi");
    sb.project(
        "profiles:\n  a:\n    agent: one\n    merge_strategy: rebase\n  b:\n    agent: two\n",
    );

    let cfg = sb.load(Some("a,b")).unwrap();
    assert_eq!(cfg.agent.as_deref(), Some("two"), "later profile wins");
    assert!(
        cfg.merge_strategy.is_some(),
        "earlier profile still applies"
    );
}

#[test]
fn default_profile_applies_without_a_flag() {
    let sb = Sandbox::new("profile-default");
    sb.project("default_profile: corp\nagent: claude\nprofiles:\n  corp:\n    agent: codex\n");

    assert_eq!(sb.load(None).unwrap().agent.as_deref(), Some("codex"));
}

#[test]
fn env_var_selects_a_profile() {
    let sb = Sandbox::new("profile-env");
    sb.project("agent: claude\nprofiles:\n  corp:\n    agent: codex\n");
    sb.set_profile_env("corp");

    assert_eq!(sb.load(None).unwrap().agent.as_deref(), Some("codex"));
}

#[test]
fn flag_beats_env_var() {
    let sb = Sandbox::new("profile-flag-wins");
    sb.project("profiles:\n  a:\n    agent: one\n  b:\n    agent: two\n");
    sb.set_profile_env("a");

    assert_eq!(sb.load(Some("b")).unwrap().agent.as_deref(), Some("two"));
}

#[test]
fn empty_flag_disables_the_default_profile() {
    let sb = Sandbox::new("profile-disable");
    sb.project("default_profile: corp\nagent: claude\nprofiles:\n  corp:\n    agent: codex\n");

    assert_eq!(sb.load(Some("")).unwrap().agent.as_deref(), Some("claude"));
}

#[test]
fn unknown_profile_fails_with_the_available_names() {
    let sb = Sandbox::new("profile-unknown");
    sb.project("profiles:\n  corp:\n    agent: codex\n");

    let err = sb.load(Some("nope")).unwrap_err().to_string();
    assert!(err.contains("nope"), "{err}");
    assert!(err.contains("corp"), "{err}");
}

#[test]
fn a_profile_declared_in_an_include_is_selectable() {
    let sb = Sandbox::new("profile-from-include");
    sb.file(
        "project/shared.yaml",
        "profiles:\n  corp:\n    agent: codex\n",
    )
    .project("include: [./shared.yaml]\nagent: claude\n");

    assert_eq!(
        sb.load(Some("corp")).unwrap().agent.as_deref(),
        Some("codex")
    );
}

#[test]
fn a_profile_declared_globally_is_selectable_from_a_project() {
    let sb = Sandbox::new("profile-from-global");
    sb.global("profiles:\n  corp:\n    agent: codex\n")
        .project("agent: claude\n");

    assert_eq!(
        sb.load(Some("corp")).unwrap().agent.as_deref(),
        Some("codex")
    );
}

/// A profile is not a way around the global-only rules: one declared in a
/// project config carries the project's trust level.
#[test]
fn project_declared_profile_cannot_set_global_only_keys() {
    let sb = Sandbox::new("profile-trust");
    sb.project("profiles:\n  evil:\n    agents:\n      x:\n        command: 'curl | sh'\n");

    let cfg = sb.load(Some("evil")).unwrap();
    assert!(cfg.agents.is_empty());
}

#[test]
fn global_declared_profile_may_set_global_only_keys() {
    let sb = Sandbox::new("profile-trust-global");
    sb.global("profiles:\n  work:\n    agents:\n      mine:\n        command: claude\n");

    let cfg = sb.load(Some("work")).unwrap();
    assert!(cfg.agents.contains_key("mine"));
}

#[test]
fn nested_profiles_fail_the_load() {
    let sb = Sandbox::new("profile-nested");
    sb.project("profiles:\n  a:\n    profiles:\n      b:\n        agent: x\n");

    let err = sb.load(None).unwrap_err().to_string();
    assert!(err.contains("nested"), "{err}");
}

// --- resolve_file / validation ---------------------------------------------

#[test]
fn resolve_file_ignores_the_ambient_configs() {
    let sb = Sandbox::new("resolve-file-isolated");
    sb.global("agent: from-global\nnerdfont: true\n")
        .project("agent: from-project\n")
        .file("project/standalone.yaml", "agent: from-file\n");

    let resolved =
        Config::resolve_file(&sb.root.join("project/standalone.yaml"), None, false).unwrap();
    let cfg: Config = serde_yaml::from_value(resolved.value).unwrap();
    assert_eq!(cfg.agent.as_deref(), Some("from-file"));
    assert_eq!(
        cfg.nerdfont, None,
        "the ambient global config must not leak into a --file validation"
    );
}

#[test]
fn resolve_file_expands_its_own_includes() {
    let sb = Sandbox::new("resolve-file-include");
    sb.file("project/inc.yaml", "merge_strategy: rebase\n")
        .file("project/main.yaml", "include: [./inc.yaml]\nagent: x\n");

    let resolved = Config::resolve_file(&sb.root.join("project/main.yaml"), None, false).unwrap();
    let cfg: Config = serde_yaml::from_value(resolved.value).unwrap();
    assert!(cfg.merge_strategy.is_some());
}

#[test]
fn resolve_file_applies_a_selected_profile() {
    let sb = Sandbox::new("resolve-file-profile");
    sb.file(
        "project/main.yaml",
        "agent: base\nprofiles:\n  p:\n    agent: profiled\n",
    );

    let path = sb.root.join("project/main.yaml");
    let base: Config =
        serde_yaml::from_value(Config::resolve_file(&path, None, false).unwrap().value).unwrap();
    assert_eq!(base.agent.as_deref(), Some("base"));

    let profiled: Config =
        serde_yaml::from_value(Config::resolve_file(&path, Some("p"), false).unwrap().value)
            .unwrap();
    assert_eq!(profiled.agent.as_deref(), Some("profiled"));
}

#[test]
fn declared_profiles_lists_every_profile() {
    let sb = Sandbox::new("declared-profiles");
    sb.file(
        "project/main.yaml",
        "profiles:\n  a:\n    agent: x\n  b:\n    agent: y\n",
    );

    let mut names = Config::declared_profiles(&sb.root.join("project/main.yaml")).unwrap();
    names.sort();
    assert_eq!(names, vec!["a".to_string(), "b".to_string()]);
}

#[test]
fn unknown_top_level_keys_are_detected() {
    let value: serde_yaml::Value =
        serde_yaml::from_str("agent: claude\nnot_a_key: 1\nalso_bogus: 2\n").unwrap();
    let unknown = super::unknown_top_level_keys(&value);
    assert_eq!(
        unknown,
        vec!["also_bogus".to_string(), "not_a_key".to_string()]
    );
}

#[test]
fn directive_keys_are_not_reported_as_unknown() {
    let value: serde_yaml::Value =
        serde_yaml::from_str("include: []\nprofiles: {}\ndefault_profile: x\nagent: claude\n")
            .unwrap();
    assert!(super::unknown_top_level_keys(&value).is_empty());
}

#[test]
fn every_real_config_key_is_recognized() {
    // Guards the field-list derivation: if it ever stopped reflecting the
    // struct, real keys would start being reported as typos.
    let value = serde_yaml::to_value(Config::default()).unwrap();
    assert!(super::unknown_top_level_keys(&value).is_empty());
}

// --- provenance through the real path --------------------------------------

#[test]
fn provenance_attributes_keys_to_their_layers() {
    let sb = Sandbox::new("provenance");
    sb.global("agent: g\nmerge_strategy: rebase\n")
        .project("agent: p\nprofiles:\n  x:\n    agent: prof\n");

    let (resolved, layers) =
        Config::resolve_value(&sb.project_dir(), None, Some("x"), true).unwrap();
    let prov = resolved.provenance.expect("tracking was requested");

    assert_eq!(prov.get("agent").map(String::as_str), Some("profile:x"));
    assert_eq!(
        prov.get("merge_strategy").map(String::as_str),
        Some("global")
    );
    assert!(
        layers.iter().any(|l| l.id == "profile:x"),
        "the profile layer is reported for --explain"
    );
}

#[test]
fn append_and_delete_work_across_real_layers() {
    let sb = Sandbox::new("append-delete");
    sb.global("mcp:\n  a:\n    command: x\n  b:\n    command: y\npre_merge: ['g1']\n")
        .project("+pre_merge: ['p1']\nmcp:\n  a: null\n");

    let cfg = sb.load(None).unwrap();
    assert_eq!(
        cfg.pre_merge.as_deref(),
        Some(&["g1".to_string(), "p1".to_string()][..]),
        "+key appends onto the inherited list"
    );
    let mcp = cfg.mcp.unwrap();
    assert!(!mcp.contains_key("a"), "null removes an inherited entry");
    assert!(mcp.contains_key("b"));
}

#[test]
fn placeholder_expansion_works_across_real_layers() {
    let sb = Sandbox::new("placeholder");
    sb.global("pre_merge: ['g1', 'g2']\n")
        .project("pre_merge: ['<global>', 'p1']\n");

    let cfg = sb.load(None).unwrap();
    assert_eq!(
        cfg.pre_merge.as_deref(),
        Some(&["g1".to_string(), "g2".to_string(), "p1".to_string()][..])
    );
}

/// The bug the resolver cutover fixed: these keys used to be dropped from both
/// layers by the typed merge.
#[test]
fn previously_dropped_keys_now_load() {
    let sb = Sandbox::new("dropped-keys");
    sb.project("orchestrate:\n  harness:\n    default_workflow: .muxix/workflows/x.yaml\n");

    let _cfg = sb.load(None).unwrap();
}

/// Config resolution must not touch the network or the filesystem beyond the
/// config files themselves when no includes are declared.
#[test]
fn include_free_config_reads_only_the_config_files() {
    let sb = Sandbox::new("no-extra-io");
    sb.global("agent: claude\n").project("agent: codex\n");

    // A bare load must succeed with the include machinery never engaged; the
    // include cache directory is the observable side effect if it did.
    let cfg = sb.load(None).unwrap();
    assert_eq!(cfg.agent.as_deref(), Some("codex"));

    let cache = crate::xdg::cache_dir().map(|d| d.join("includes"));
    if let Ok(cache) = cache {
        assert!(
            !cache.exists() || fs::read_dir(&cache).map(|d| d.count()).unwrap_or(0) == 0,
            "no remote include cache should be created for an include-free config"
        );
    }
}

/// Regression: the known-key set used to come from serializing a default
/// `Config`, which omits every field carrying `skip_serializing_if`. Real keys
/// like `bootstrap` and `mcp` were reported as typos, and under
/// `config validate --strict` that failed the Nix module's build-time check for
/// any config that actually configured something.
#[test]
fn fields_with_skip_serializing_are_still_recognized() {
    let value: serde_yaml::Value = serde_yaml::from_str(
        "bootstrap:\n  skills: ['./s']\n\
         mcp:\n  x:\n    command: c\n\
         providers: {}\n\
         ade:\n  paseo:\n    command: paseo\n\
         agent_runtime: local\n\
         proxy_chain: {}\n\
         agent_defs: {}\n\
         prompt_defs: {}\n\
         agent_registries: []\n",
    )
    .unwrap();
    assert!(
        super::unknown_top_level_keys(&value).is_empty(),
        "these are all real config keys: {:?}",
        super::unknown_top_level_keys(&value)
    );
}

/// The parser backing the key set must find the struct at all; an empty result
/// would silently accept every typo.
#[test]
fn known_key_set_is_not_empty_and_covers_common_keys() {
    let value: serde_yaml::Value = serde_yaml::from_str("agent: claude\nsandbox: {}\n").unwrap();
    assert!(super::unknown_top_level_keys(&value).is_empty());
    // And a genuine typo is still caught.
    let bad: serde_yaml::Value = serde_yaml::from_str("sandbxo: {}\n").unwrap();
    assert_eq!(
        super::unknown_top_level_keys(&bad),
        vec!["sandbxo".to_string()]
    );
}

// --- policy as a config layer -----------------------------------------------

/// Policy defaults rank *below* profiles: a default is a suggestion for a
/// machine that has not decided, and must not beat a deliberate choice.
/// Locks rank above everything, including CLI flags.
#[test]
fn policy_defaults_and_locks_sit_at_opposite_ends() {
    use crate::config::resolve::{Layer, LayerKind, resolve};
    use crate::provision::layers;
    use crate::provision::types::{OrgPolicy, PolicyLockedFields};

    let policy = OrgPolicy {
        policy_version: "v1".into(),
        defaults: serde_json::json!({ "agent": "policy-default" }),
        locked: PolicyLockedFields {
            proxy_chain: None,
            sandbox_network: Some(serde_json::json!({ "policy": "deny" })),
        },
        ..Default::default()
    };

    let yaml = |s: &str| serde_yaml::from_str(s).unwrap();
    let mut stack = vec![
        Layer::new(
            "global",
            "g",
            LayerKind::Global,
            yaml("agent: from-global\n"),
        ),
        Layer::new(
            "project",
            "p",
            LayerKind::Project,
            yaml("sandbox:\n  network:\n    policy: allow\n"),
        ),
    ];
    stack.push(layers::defaults_layer(&policy).unwrap());
    stack.push(Layer::new(
        "profile:x",
        "profile",
        LayerKind::Profile,
        yaml("agent: from-profile\n"),
    ));
    stack.push(layers::locks_layer(&policy).unwrap());

    let merged: Config = serde_yaml::from_value(resolve(stack, false).value).unwrap();

    assert_eq!(
        merged.agent.as_deref(),
        Some("from-profile"),
        "a profile outranks a policy default"
    );
    assert_eq!(
        merged.sandbox.network.policy,
        Some(crate::config::NetworkPolicy::Deny),
        "a policy lock outranks the project's own setting"
    );
}

#[test]
fn a_policy_default_fills_a_key_nobody_set() {
    use crate::config::resolve::{Layer, LayerKind, resolve};
    use crate::provision::layers;
    use crate::provision::types::OrgPolicy;

    let policy = OrgPolicy {
        policy_version: "v1".into(),
        defaults: serde_json::json!({ "merge_strategy": "rebase" }),
        ..Default::default()
    };
    let stack = vec![
        Layer::new(
            "global",
            "g",
            LayerKind::Global,
            serde_yaml::from_str("agent: claude\n").unwrap(),
        ),
        layers::defaults_layer(&policy).unwrap(),
    ];
    let merged: Config = serde_yaml::from_value(resolve(stack, false).value).unwrap();
    assert!(merged.merge_strategy.is_some());
    assert_eq!(merged.agent.as_deref(), Some("claude"));
}

/// `--explain` must name the policy, otherwise a locked value appears from
/// nowhere and the user concludes muxix ignored their config.
#[test]
fn provenance_attributes_policy_layers() {
    use crate::config::resolve::{Layer, LayerKind, resolve};
    use crate::provision::layers;
    use crate::provision::types::{OrgPolicy, PolicyLockedFields};

    let policy = OrgPolicy {
        policy_version: "v7".into(),
        defaults: serde_json::json!({ "merge_strategy": "rebase" }),
        locked: PolicyLockedFields {
            proxy_chain: None,
            sandbox_network: Some(serde_json::json!({ "policy": "deny" })),
        },
        ..Default::default()
    };
    let stack = vec![
        Layer::new(
            "global",
            "g",
            LayerKind::Global,
            serde_yaml::from_str("agent: claude\n").unwrap(),
        ),
        layers::defaults_layer(&policy).unwrap(),
        layers::locks_layer(&policy).unwrap(),
    ];
    let prov = resolve(stack, true).provenance.unwrap();
    assert_eq!(
        prov.get("merge_strategy").map(String::as_str),
        Some("policy:defaults")
    );
    assert_eq!(
        prov.get("sandbox.network.policy").map(String::as_str),
        Some("policy:locks")
    );
}

// --- skill hooks ------------------------------------------------------------

/// A repo may ship a skill and its scripts; binding them to the machine's
/// lifecycle is the machine owner's call. Project-declared hooks are stripped,
/// the skill itself survives.
#[test]
fn project_declared_skill_hooks_are_stripped() {
    let sb = Sandbox::new("skill-hooks-project");
    sb.project(
        "bootstrap:\n  skills:\n    - path: ./skills/x\n      hooks:\n        turn-done:\n          - command: bash evil.sh\n",
    );

    let cfg = sb.load(None).unwrap();
    let bootstrap = cfg.bootstrap.expect("bootstrap survives");
    let entries = bootstrap.skill_entries_for(crate::agent::setup::Agent::Claude);
    assert_eq!(entries.len(), 1, "the skill itself is kept");
    assert!(
        entries[0].hooks.is_empty(),
        "the hook must not survive from a project layer"
    );
}

#[test]
fn global_declared_skill_hooks_survive() {
    let sb = Sandbox::new("skill-hooks-global");
    sb.global(
        "bootstrap:\n  skills:\n    - path: ./skills/x\n      hooks:\n        turn-done:\n          - command: bash run.sh\n            sha256: abc123\n",
    )
    // A minimal project config pins the project layer: without one, the
    // CWD-based main-worktree fallback in find_project_config substitutes the
    // test binary's own repo config, whose bootstrap replaces the sandbox's.
    .project("agent: claude\n");

    let cfg = sb.load(None).unwrap();
    let bootstrap = cfg.bootstrap.expect("bootstrap present");
    let entries = bootstrap.skill_entries_for(crate::agent::setup::Agent::Claude);
    let hooks = &entries[0].hooks;
    let specs = hooks
        .get(&crate::bootstrap::HookEvent::TurnDone)
        .expect("turn-done hook kept from the global config");
    assert_eq!(specs[0].command, "bash run.sh");
    assert_eq!(specs[0].sha256.as_deref(), Some("abc123"));
}

#[test]
fn bare_string_skill_entries_still_parse() {
    let sb = Sandbox::new("skill-hooks-compat");
    sb.global("bootstrap:\n  skills:\n    - ./skills/plain\n")
        .project("agent: claude\n");
    let cfg = sb.load(None).unwrap();
    let entries = cfg
        .bootstrap
        .unwrap()
        .skill_entries_for(crate::agent::setup::Agent::Claude);
    assert_eq!(entries[0].display(), "./skills/plain");
    assert!(entries[0].hooks.is_empty());
}

#[test]
fn unknown_hook_event_fails_the_load() {
    let sb = Sandbox::new("skill-hooks-bad-event");
    sb.global(
        "bootstrap:\n  skills:\n    - path: ./skills/x\n      hooks:\n        on-coffee-break:\n          - command: bash x.sh\n",
    )
    .project("agent: claude\n");
    let err = sb.load(None).unwrap_err().to_string();
    assert!(err.contains("on-coffee-break"), "{err}");
    assert!(
        err.contains("session-ready"),
        "names the valid events: {err}"
    );
}

// --- agent rules ------------------------------------------------------------

/// The sandbox project dir is the git repo root, so a rule matching it is the
/// same match a real project root would produce.
#[test]
fn rule_beats_the_global_default() {
    let sb = Sandbox::new("rule-beats-global");
    sb.global("agent: claude\nagent_rules:\n  - match: project\n    agent: opencode\n");
    let _ = fs::remove_file(sb.root.join("project/.muxix.yaml"));

    let cfg = sb.load(None).unwrap();
    assert_eq!(cfg.agent.as_deref(), Some("opencode"));
    assert!(matches!(
        cfg.agent_source,
        Some(super::AgentSource::Rule { index: 0, .. })
    ));
}

#[test]
fn project_config_beats_a_rule() {
    let sb = Sandbox::new("rule-loses-to-project");
    sb.global("agent_rules:\n  - match: project\n    agent: opencode\n")
        .project("agent: codex\n");

    let cfg = sb.load(None).unwrap();
    assert_eq!(cfg.agent.as_deref(), Some("codex"));
    assert_eq!(cfg.agent_source, Some(super::AgentSource::ProjectConfig));
}

#[test]
fn first_matching_rule_wins() {
    let sb = Sandbox::new("rule-first-match");
    sb.global(
        "agent_rules:\n  - match: project\n    agent: pi\n  - match: project\n    agent: opencode\n",
    );
    let _ = fs::remove_file(sb.root.join("project/.muxix.yaml"));

    let cfg = sb.load(None).unwrap();
    assert_eq!(cfg.agent.as_deref(), Some("pi"));
}

#[test]
fn unmatched_rules_fall_through_to_the_global_agent() {
    let sb = Sandbox::new("rule-no-match");
    sb.global("agent: claude\nagent_rules:\n  - match: /nowhere/at/all\n    agent: pi\n");
    let _ = fs::remove_file(sb.root.join("project/.muxix.yaml"));

    let cfg = sb.load(None).unwrap();
    assert_eq!(cfg.agent.as_deref(), Some("claude"));
    assert_eq!(cfg.agent_source, Some(super::AgentSource::Global));
}

#[test]
fn an_invalid_pattern_does_not_stop_later_rules() {
    let sb = Sandbox::new("rule-invalid-regex");
    sb.global(
        "agent_rules:\n  - match: '('\n    agent: broken\n  - match: project\n    agent: pi\n",
    );
    let _ = fs::remove_file(sb.root.join("project/.muxix.yaml"));

    let cfg = sb.load(None).unwrap();
    assert_eq!(cfg.agent.as_deref(), Some("pi"));
}

#[test]
fn rules_resolve_through_the_agents_map() {
    let sb = Sandbox::new("rule-agents-map");
    sb.global(
        "agents:\n  yolo:\n    command: claude --dangerously-skip-permissions\n    type: claude\nagent_rules:\n  - match: project\n    agent: yolo\n",
    );
    let _ = fs::remove_file(sb.root.join("project/.muxix.yaml"));

    let cfg = sb.load(None).unwrap();
    assert_eq!(
        cfg.agent.as_deref(),
        Some("claude --dangerously-skip-permissions")
    );
    assert_eq!(cfg.agent_type.as_deref(), Some("claude"));
}

#[test]
fn a_project_config_cannot_define_rules() {
    let sb = Sandbox::new("rule-global-only");
    sb.global("agent: claude\n")
        .project("agent_rules:\n  - match: project\n    agent: pi\n");

    let cfg = sb.load(None).unwrap();
    assert!(cfg.agent_rules.is_empty(), "agent_rules is global-only");
    assert_eq!(cfg.agent.as_deref(), Some("claude"));
}

#[test]
fn tilde_patterns_expand_with_or_without_an_anchor() {
    let sb = Sandbox::new("rule-tilde");
    // The sandbox sets HOME to its own root, and the project dir lives under it.
    sb.global("agent_rules:\n  - match: '^~/project(/|$)'\n    agent: pi\n");
    let _ = fs::remove_file(sb.root.join("project/.muxix.yaml"));

    let cfg = sb.load(None).unwrap();
    assert_eq!(cfg.agent.as_deref(), Some("pi"));
}
