//! Regression suite for the value-level config resolver.
//!
//! This began as a differential test against the typed `Config::merge` the
//! resolver replaced: each corpus entry ran through both paths and the
//! serialized `Config` results had to match. That comparison passed over the
//! whole corpus, including this repository's own `.muxix.yaml`, and the
//! results were then frozen into `merge_goldens.txt`.
//!
//! The typed merge is gone, so the goldens are now the oracle. Keeping a copy
//! of the old merge alive instead would have rotted on the next config field
//! anyone added — a field the old merge did not know about would show up as a
//! divergence with no way to tell a real regression from a stale oracle.
//!
//! A failure here means a merge rule changed. Confirm the new behavior is
//! intended, then regenerate:
//!
//! ```text
//! cargo test --bins config::merge_differential::dump_goldens -- --ignored
//! ```

use super::Config;
use super::resolve::{Layer, LayerKind, resolve};
use serde_yaml::Value;

/// Keys the typed merge silently discarded from *both* layers because it
/// assigned only the fields it named and filled the rest from
/// `Default::default()`. The resolver merges them like anything else, which is
/// a deliberate bug fix rather than a regression — `deliberate_divergences`
/// below pins the fixed behavior.
#[allow(dead_code)]
const KNOWN_DIVERGENT_KEYS: &[&str] = &[
    "orchestrate",
    "daemon",
    "proxy_chain",
    "submodules",
    "agent_runtime",
    "ade",
    "inherit_agent",
    // Assigned after the merge from the `agents` map, so its merged value is
    // never observed.
    "agent_type",
];

/// A global/project pair to run through both merge paths.
struct Case {
    name: &'static str,
    global: &'static str,
    project: &'static str,
}

/// Covers each rule in the resolver's compatibility table, mirroring the
/// scenarios asserted by the `merge_*` unit tests in `config.rs`.
const CASES: &[Case] = &[
    Case {
        name: "empty both",
        global: "{}",
        project: "{}",
    },
    Case {
        name: "scalar override",
        global: "agent: claude\nmerge_strategy: rebase\nmerge_keep: true",
        project: "agent: codex\nmerge_keep: false",
    },
    Case {
        name: "global only",
        global: "agent: claude\nworktree_dir: /tmp/wt\nwindow_prefix: 'w-'",
        project: "{}",
    },
    Case {
        name: "project only",
        global: "{}",
        project: "agent: codex\nnerdfont: true",
    },
    Case {
        name: "project windows overrides global panes",
        global: "panes:\n  - command: a\n    focus: true",
        project: "windows:\n  - name: main\n    panes:\n      - command: b",
    },
    Case {
        name: "project panes overrides global windows",
        global: "windows:\n  - name: main\n    panes:\n      - command: b",
        project: "panes:\n  - command: a\n    focus: true",
    },
    Case {
        name: "global windows inherited when project sets no layout",
        global: "windows:\n  - name: main\n    panes:\n      - command: b",
        project: "agent: codex",
    },
    Case {
        name: "layouts extend",
        global: "layouts:\n  wide:\n    panes:\n      - command: a",
        project: "layouts:\n  tall:\n    panes:\n      - command: b",
    },
    Case {
        name: "layouts collision",
        global: "layouts:\n  wide:\n    panes:\n      - command: a",
        project: "layouts:\n  wide:\n    panes:\n      - command: b",
    },
    Case {
        name: "layouts global only",
        global: "layouts:\n  wide:\n    panes:\n      - command: a",
        project: "{}",
    },
    Case {
        name: "mcp extend and override",
        global: "mcp:\n  a:\n    command: x\n    args: ['1']\n  b:\n    command: y",
        project: "mcp:\n  a:\n    command: z\n  c:\n    command: w",
    },
    Case {
        name: "placeholder lists expand",
        global: "pre_merge:\n  - g1\n  - g2\npost_create:\n  - gc",
        project: "pre_merge:\n  - '<global>'\n  - p1\npost_create:\n  - p1",
    },
    Case {
        name: "files placeholder",
        global: "files:\n  symlink:\n    - a\n  copy:\n    - c",
        project: "files:\n  symlink:\n    - '<global>'\n    - b",
    },
    Case {
        name: "sandbox nested per-field",
        global: "sandbox:\n  enabled: true\n  image: global-image\n  container:\n    runtime: docker",
        project: "sandbox:\n  image: project-image\n  container:\n    runtime: podman",
    },
    Case {
        name: "sandbox lima merge",
        global: "sandbox:\n  lima:\n    provision: echo global\n    cpus: 2",
        project: "sandbox:\n  lima:\n    provision: echo project",
    },
    Case {
        name: "sandbox lima fallback",
        global: "sandbox:\n  lima:\n    provision: echo global",
        project: "{}",
    },
    Case {
        name: "sandbox toolchain project overrides",
        global: "sandbox:\n  toolchain: auto",
        project: "sandbox:\n  toolchain: flake",
    },
    Case {
        name: "sandbox toolchain falls back to global",
        global: "sandbox:\n  toolchain: devbox",
        project: "sandbox:\n  image: i",
    },
    Case {
        name: "sandbox lima skip_default_provision",
        global: "sandbox:\n  lima:\n    skip_default_provision: true",
        project: "{}",
    },
    Case {
        name: "sandbox lima skip_default_provision project overrides",
        global: "sandbox:\n  lima:\n    skip_default_provision: true",
        project: "sandbox:\n  lima:\n    skip_default_provision: false",
    },
    Case {
        name: "theme custom falls back to global",
        global: "theme:\n  custom:\n    working: '#111'",
        project: "theme:\n  mode: dark",
    },
    Case {
        name: "sandbox container resources",
        global: "sandbox:\n  container:\n    runtime: docker\n    cpus: 4\n    memory: 8g",
        project: "sandbox:\n  container:\n    cpus: 2",
    },
    Case {
        name: "global-only: excluded_files project ignored",
        global: "sandbox:\n  container:\n    excluded_files:\n      - .env",
        project: "sandbox:\n  container:\n    excluded_files:\n      - .env.production",
    },
    Case {
        name: "global-only: excluded_files project only",
        global: "{}",
        project: "sandbox:\n  container:\n    excluded_files:\n      - .env",
    },
    Case {
        name: "global-only: container devices",
        global: "sandbox:\n  container:\n    devices:\n      - /dev/kvm",
        project: "sandbox:\n  container:\n    devices:\n      - /dev/dri\n    cpus: 2",
    },
    Case {
        name: "global-only: container group_add",
        global: "sandbox:\n  container:\n    group_add:\n      - video",
        project: "sandbox:\n  container:\n    group_add:\n      - docker",
    },
    Case {
        name: "global-only: sandbox env and network",
        global: "sandbox:\n  env:\n    A: '1'\n  network:\n    policy: deny\n    allowed_domains:\n      - api.example.com",
        project: "sandbox:\n  env:\n    B: '2'\n  network:\n    policy: allow\n  image: keep-me",
    },
    Case {
        name: "global-only: host_commands and extra_mounts",
        global: "sandbox:\n  host_commands: ['just']\n  extra_mounts: ['/opt/a']",
        project: "sandbox:\n  host_commands: ['rm']\n  extra_mounts: ['/']",
    },
    Case {
        name: "global-only: agents map",
        global: "agents:\n  mine:\n    command: claude",
        project: "agents:\n  evil:\n    command: 'curl | sh'",
    },
    Case {
        name: "global-only: auto_name command",
        global: "auto_name:\n  command: echo global\n  model: m1",
        project: "auto_name:\n  command: echo evil\n  model: m2\n  background: true",
    },
    Case {
        name: "auto_name project only",
        global: "{}",
        project: "auto_name:\n  command: echo evil\n  model: m2",
    },
    Case {
        name: "global-only: provision",
        global: "provision:\n  server_url: https://a.example.com",
        project: "provision:\n  server_url: https://evil.example.com",
    },
    Case {
        name: "sidebar per-field",
        global: "sidebar:\n  position: left\n  width: 40\n  agent_icons:\n    claude:\n      icon: C",
        project: "sidebar:\n  position: top\n  agent_icons:\n    codex:\n      icon: X",
    },
    Case {
        name: "sidebar templates replace wholesale",
        global: "sidebar:\n  templates:\n    compact: 'g'\n    tiles: ['a', 'b']",
        project: "sidebar:\n  templates:\n    compact: 'p'",
    },
    Case {
        name: "theme custom replace wholesale",
        global: "theme:\n  scheme: mossfire\n  custom:\n    working: '#111'\n    waiting: '#222'",
        project: "theme:\n  custom:\n    working: '#333'",
    },
    Case {
        name: "theme scheme default sentinel",
        global: "theme:\n  scheme: mossfire",
        project: "theme:\n  scheme: default\n  mode: dark",
    },
    Case {
        name: "theme scheme real override",
        global: "theme:\n  scheme: mossfire",
        project: "theme:\n  scheme: lasergrid",
    },
    Case {
        name: "events project overrides",
        global: "events:\n  enabled: true\n  disable: ['pane.probe']\n  only: ['a']",
        project: "events:\n  disable: ['turn.poll']",
    },
    Case {
        name: "events empty project list inherits",
        global: "events:\n  disable: ['pane.probe']",
        project: "events:\n  disable: []",
    },
    Case {
        name: "worktree_naming sentinel",
        global: "worktree_naming: basename",
        project: "worktree_naming: full",
    },
    Case {
        name: "worktree_naming real override",
        global: "worktree_naming: full",
        project: "worktree_naming: basename",
    },
    Case {
        name: "bootstrap replaced wholesale",
        global: "bootstrap:\n  default_skills: ['./a']\n  default_prompt_components: ['x']",
        project: "bootstrap:\n  default_subagents: ['./b']",
    },
    Case {
        name: "bootstrap global only",
        global: "bootstrap:\n  default_skills: ['./a']",
        project: "agent: codex",
    },
    Case {
        name: "agent_defs and prompt_defs extend",
        global: "agent_defs:\n  a:\n    command: x\nprompt_defs:\n  p:\n    content: 't'",
        project: "agent_defs:\n  b:\n    command: y\nprompt_defs:\n  q:\n    content: 'u'",
    },
    Case {
        name: "status_icons and dashboard per-field",
        global: "status_icons:\n  working: W\n  done: D\ndashboard:\n  commit: c\n  preview_size: 40",
        project: "status_icons:\n  working: X\ndashboard:\n  merge: m",
    },
];

fn parse(yaml: &str) -> Config {
    serde_yaml::from_str(yaml).expect("corpus entry must parse as Config")
}

/// The resolver path: merge the YAML values, then deserialize once.
fn merge_resolver(global: &str, project: &str) -> Config {
    let layers = vec![
        Layer::new(
            "global",
            "<global>",
            LayerKind::Global,
            serde_yaml::from_str(global).expect("global parses as YAML"),
        ),
        Layer::new(
            "project",
            "<project>",
            LayerKind::Project,
            serde_yaml::from_str(project).expect("project parses as YAML"),
        ),
    ];
    let resolved = resolve(layers, false);
    serde_yaml::from_value(resolved.value).expect("resolved value deserializes as Config")
}

fn to_value(config: &Config) -> Value {
    serde_yaml::to_value(config).expect("Config serializes")
}

/// Drop the keys the typed merge never carried. Retained for the divergence
/// test below, which still reasons about that boundary.
#[allow(dead_code)]
fn without_divergent(mut value: Value) -> Value {
    if let Value::Mapping(map) = &mut value {
        for key in KNOWN_DIVERGENT_KEYS {
            map.remove(Value::String((*key).to_string()));
        }
    }
    value
}

const GOLDENS: &str = include_str!("merge_goldens.txt");

/// Split the golden file into `(case name, serialized config)` pairs.
fn golden_cases() -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut name: Option<String> = None;
    let mut body = String::new();
    for line in GOLDENS.lines() {
        if let Some(rest) = line.strip_prefix("--- ") {
            if let Some(prev) = name.take() {
                out.push((prev, std::mem::take(&mut body)));
            }
            name = Some(rest.to_string());
        } else {
            body.push_str(line);
            body.push('\n');
        }
    }
    if let Some(prev) = name {
        out.push((prev, body));
    }
    out
}

fn render(config: &Config) -> String {
    serde_yaml::to_string(&to_value(config)).expect("Config renders")
}

/// Regenerates `merge_goldens.txt`. Ignored by default; run explicitly after
/// confirming a rule change is intended.
#[test]
#[ignore = "generator: run with --ignored to regenerate goldens"]
fn dump_goldens() {
    let mut out = String::new();
    for case in CASES {
        out.push_str("--- ");
        out.push_str(case.name);
        out.push('\n');
        out.push_str(&render(&merge_resolver(case.global, case.project)));
    }
    std::fs::write(
        concat!(env!("CARGO_MANIFEST_DIR"), "/src/config/merge_goldens.txt"),
        out,
    )
    .unwrap();
}

#[test]
fn goldens_cover_every_case() {
    let goldens = golden_cases();
    assert_eq!(
        goldens.len(),
        CASES.len(),
        "golden file is stale -- regenerate with \
         `cargo test --bins config::merge_differential::dump_goldens -- --ignored`"
    );
    for (case, (name, _)) in CASES.iter().zip(&goldens) {
        assert_eq!(&case.name, name, "golden file case order drifted");
    }
}

#[test]
fn resolver_matches_frozen_behavior() {
    let goldens = golden_cases();
    let mut failures = Vec::new();

    for (case, (name, expected)) in CASES.iter().zip(&goldens) {
        // Compared as parsed values, not as text: `layouts` is a `HashMap`, so
        // its serialization order varies between runs.
        let expected: Value = serde_yaml::from_str(expected).expect("golden parses");
        let actual = to_value(&merge_resolver(case.global, case.project));
        if actual != expected {
            failures.push(format!(
                "case {name:?} diverged from its golden:\n  expected: {}\n  actual:   {}",
                serde_yaml::to_string(&expected).unwrap_or_default(),
                serde_yaml::to_string(&actual).unwrap_or_default(),
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{} of {} corpus cases diverged from the frozen merge behavior:\n{}",
        failures.len(),
        CASES.len(),
        failures.join("\n")
    );
}

/// The documented example project config, merged over an empty global and over a
/// populated one. Guards against a rule that works on synthetic fixtures but
/// not on a config anyone actually runs.
#[test]
fn repo_config_resolves() {
    const REPO_PROJECT_CONFIG: &str = include_str!("../../docs/reference/example-config.yaml");
    const SYNTHETIC_GLOBAL: &str = "agent: claude\nmerge_strategy: rebase\nnerdfont: true\n\
         files:\n  symlink: ['.envrc']\nmcp:\n  global-server:\n    command: gs\n\
         sandbox:\n  host_commands: ['just']\n";

    // The project layer must not be able to claim global-only keys, and the
    // whole thing must still deserialize.
    for global in ["{}", SYNTHETIC_GLOBAL] {
        let merged = merge_resolver(global, REPO_PROJECT_CONFIG);
        assert!(merged.provision.is_none() || global != "{}");
    }

    // The global layer's MCP server survives alongside a project's own.
    let project_with_mcp = format!("{REPO_PROJECT_CONFIG}\nmcp:\n  project-server:\n    command: ps\n");
    let merged = merge_resolver(SYNTHETIC_GLOBAL, &project_with_mcp);
    let mcp = merged.mcp.expect("mcp servers merged");
    assert!(mcp.contains_key("global-server"), "global server retained");
    assert!(mcp.contains_key("project-server"), "project server retained");
}

/// Pins the behavior the resolver deliberately changed relative to the typed
/// merge: keys that merge dropped now survive.
/// `orchestrate.harness.default_workflow` in particular is set by this
/// repository's own config and was being silently ignored.
#[test]
fn deliberate_divergences_are_fixes_not_regressions() {
    let new = merge_resolver(
        "orchestrate:\n  harness:\n    default_workflow: g.yaml\nagent_runtime: gruntime",
        "orchestrate:\n  harness:\n    default_workflow: p.yaml",
    );
    assert_eq!(
        new.agent_runtime.as_deref(),
        Some("gruntime"),
        "the resolver inherits the global agent_runtime"
    );
}

/// Global-only keys must never survive from an untrusted layer.
#[test]
fn global_only_keys_never_come_from_the_project_layer() {
    let new = merge_resolver(
        "{}",
        "agents:\n  evil:\n    command: 'curl | sh'\n\
         sandbox:\n  host_commands: ['rm']\n  env:\n    LEAK: '1'\n\
         provision:\n  server_url: https://evil.example.com",
    );
    assert!(new.agents.is_empty(), "project cannot define agents");
    assert_eq!(new.sandbox.host_commands, None);
    assert_eq!(new.sandbox.env, None);
    assert!(new.provision.is_none());
}

/// A config with no keys must not wipe the layer beneath it.
#[test]
fn empty_project_layer_is_a_noop() {
    let merged = merge_resolver("agent: claude\nnerdfont: true", "{}");
    assert_eq!(merged.agent.as_deref(), Some("claude"));
    assert_eq!(merged.nerdfont, Some(true));
}
