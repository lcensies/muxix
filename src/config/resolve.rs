//! Value-level config layer resolution.
//!
//! Workmux composes its effective config from several layers — includes, the
//! global file, the project file, a provisioning policy, selected profiles, and
//! CLI overrides. This module merges them as `serde_yaml::Value` trees and
//! hands the single merged value to serde once, rather than merging typed
//! `Config` structs field by field.
//!
//! Merging at the value level is what makes three things possible that a typed
//! merge cannot do: per-key provenance (`workmux config resolve --explain`),
//! distinguishing "absent" from "explicitly null", and folding an arbitrary
//! number of layers without reinterpreting each rule for each new layer.
//!
//! # Rule table
//!
//! The default rules (D2 in the change design) are:
//!
//! | Shape in the overriding layer | Result                                  |
//! |-------------------------------|-----------------------------------------|
//! | scalar / enum                 | replaces the base value                 |
//! | mapping                       | deep-merges into a base mapping         |
//! | sequence                      | replaces the base sequence              |
//! | `+key:` sequence              | concatenates onto the base sequence     |
//! | `key: null`                   | removes the inherited key entirely      |
//! | any shape over a differing shape | replaces                             |
//!
//! # Compatibility rules
//!
//! The rules above are not sufficient on their own: the typed merge they
//! replace (`Config::merge`) had accumulated per-key behavior that real configs
//! depend on. Each of those is encoded in [`rule_for`] as an explicit exception
//! keyed by dotted path, so the merge itself stays rule-free and the exceptions
//! are enumerable, testable, and documentable.
//!
//! | Path(s)                                              | Rule            | Why it is not the default |
//! |------------------------------------------------------|-----------------|---------------------------|
//! | `bootstrap`, `providers`                             | `Replace`       | The typed merge used `project.or(global)` on these whole structs. Deep-merging them would silently blend a project's agent bootstrap into the user's global one. |
//! | `theme.custom`, `sidebar.templates`                  | `Replace`       | Same: `Option<Struct>` fields the typed merge swapped wholesale. A project that overrides one theme color would otherwise inherit the rest of the global palette. |
//! | `mcp`, `layouts`, `agent_defs`, `prompt_defs`, `sidebar.agent_icons` | `ShallowExtend` | The typed merge used `map.extend(other)`: the overriding layer replaces a whole entry rather than merging into it. `mcp.foo` set in both layers takes the override's definition wholesale, not a field-wise blend. |
//! | `agent_registries`                                   | `Append`        | Global registries are searched before project-local ones, so the lists concatenate rather than replace. |
//! | `post_create`, `pre_merge`, `pre_remove`, `files.copy`, `files.symlink` | `PlaceholderList` | These support the `<global>` placeholder, which expands to the inherited list at that position. |
//! | `events.only`, `events.disable`                      | `NonEmptyReplace` | The typed merge took the override's list only when non-empty; an explicit `[]` inherits rather than clears. |
//! | `worktree_naming`                                    | `DefaultSentinel("full")` | The typed merge compared against `WorktreeNaming::default()`, so writing the default value explicitly inherits instead of overriding. |
//! | `theme.scheme`                                        | `DefaultSentinel("default")` | Same, against `ThemeScheme::Default`. |
//!
//! # Global-only keys
//!
//! Several keys are security-sensitive: a repository's `.workmux.yaml` must not
//! be able to set them, because a malicious repo (or an agent that can write to
//! one) would otherwise gain host command execution, secret passthrough, or a
//! weakened sandbox. These are listed in [`GLOBAL_ONLY_PATHS`] and stripped from
//! every non-global layer by [`strip_global_only`] *before* merging, which is
//! why the merge itself needs no notion of which layer it is looking at.
//!
//! # Deliberate divergences from the typed merge
//!
//! The typed merge assigned only the fields it named and filled the rest from
//! `Default::default()`, so these keys were silently discarded from **both**
//! layers: `orchestrate`, `daemon`, `proxy_chain`, `submodules`, `agent_runtime`,
//! `ade`, and `inherit_agent`. A config setting `orchestrate.harness.default_workflow`
//! had no effect. This resolver merges them like any other key, which fixes the
//! bug; the differential test records it as an intentional divergence rather
//! than reproducing it.

use serde_yaml::{Mapping, Value};
use std::collections::BTreeMap;

/// Where a layer came from, for diagnostics and `--explain` output.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayerKind {
    /// A file or URL pulled in by `include:`.
    Include,
    /// The global config file.
    Global,
    /// The project `.workmux.yaml`.
    Project,
    /// Defaults supplied by a cached provisioning policy.
    PolicyDefaults,
    /// A selected named profile.
    Profile,
    /// Values locked by a provisioning policy. Applied above everything.
    PolicyLocks,
    /// Overrides from CLI flags.
    Cli,
}

impl LayerKind {
    /// Whether a layer of this kind may set global-only keys.
    ///
    /// Includes inherit the trust of the file that pulled them in, so this is
    /// decided per-layer by the caller rather than by kind alone; see
    /// [`Layer::trusted`].
    pub fn is_global_by_default(&self) -> bool {
        matches!(
            self,
            LayerKind::Global | LayerKind::PolicyDefaults | LayerKind::PolicyLocks
        )
    }
}

/// One config layer awaiting merge.
#[derive(Clone, Debug)]
pub struct Layer {
    /// Stable identifier used in provenance output, e.g. `global`, `project`,
    /// `profile:corp`.
    pub id: String,
    /// Human-readable origin shown by `--explain`: a file path, a URL, or
    /// `<cli>`.
    pub source: String,
    pub kind: LayerKind,
    /// Whether this layer may set global-only keys. Includes pulled in by the
    /// global config are trusted; the project file and anything it includes are
    /// not.
    pub trusted: bool,
    pub value: Value,
}

impl Layer {
    pub fn new(id: impl Into<String>, source: impl Into<String>, kind: LayerKind, value: Value) -> Self {
        let trusted = kind.is_global_by_default();
        Self {
            id: id.into(),
            source: source.into(),
            kind,
            trusted,
            value,
        }
    }

    pub fn trusted(mut self, trusted: bool) -> Self {
        self.trusted = trusted;
        self
    }
}

/// Per-key attribution: dotted key path -> id of the layer that set the final
/// value. Only populated when resolution runs with tracking on, so the common
/// path allocates nothing.
pub type Provenance = BTreeMap<String, String>;

/// A warning raised while normalizing a layer, e.g. a project config trying to
/// set a global-only key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayerWarning {
    pub layer_id: String,
    pub source: String,
    pub path: String,
    pub message: String,
}

/// Result of merging every layer.
#[derive(Debug, Default)]
pub struct Resolved {
    pub value: Value,
    /// `None` unless resolution ran with tracking on.
    pub provenance: Option<Provenance>,
    pub warnings: Vec<LayerWarning>,
}

/// Per-path merge behavior. Anything not named in [`rule_for`] uses [`Rule::Default`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    /// Mappings deep-merge, sequences and scalars replace.
    Default,
    /// The overriding value replaces the base wholesale, even for mappings.
    Replace,
    /// Mapping whose top-level entries replace base entries wholesale, without
    /// descending into them.
    ShallowExtend,
    /// Sequence concatenated after the base sequence.
    Append,
    /// Sequence in which the literal `<global>` element expands to the base
    /// sequence at that position.
    PlaceholderList,
    /// Sequence that overrides only when it is non-empty.
    NonEmptyReplace,
    /// Scalar that overrides only when it differs from this default.
    DefaultSentinel(&'static str),
}

/// Keys a non-global layer may not set. Each is security-sensitive: allowing a
/// repository's `.workmux.yaml` to set it would grant the repo host execution,
/// secret exposure, or a weakened sandbox.
pub const GLOBAL_ONLY_PATHS: &[&str] = &[
    // Executes an arbitrary command to name worktrees.
    "auto_name.command",
    // Agent definitions are commands workmux will run.
    "agents",
    // Same: a rule selects a command to execute, keyed by path.
    "agent_rules",
    // Points at a provisioning server and a token file.
    "provision",
    // Sandbox escapes and secret exposure.
    "sandbox.env",
    "sandbox.env_passthrough",
    "sandbox.rpc_host",
    "sandbox.host_commands",
    "sandbox.extra_mounts",
    "sandbox.agent_config_dir",
    "sandbox.network",
    "sandbox.dangerously_allow_unsandboxed_host_exec",
    "sandbox.microsandbox",
    "sandbox.checkpoint",
    "sandbox.container.devices",
    "sandbox.container.group_add",
    "sandbox.container.excluded_files",
];

/// Compatibility rule for a dotted key path. See the module docs for why each
/// exception exists.
pub fn rule_for(path: &str) -> Rule {
    match path {
        "bootstrap" | "providers" | "theme.custom" | "sidebar.templates" => Rule::Replace,
        "mcp" | "layouts" | "agent_defs" | "prompt_defs" | "sidebar.agent_icons" => {
            Rule::ShallowExtend
        }
        "agent_registries" => Rule::Append,
        "post_create" | "pre_merge" | "pre_remove" | "files.copy" | "files.symlink" => {
            Rule::PlaceholderList
        }
        "events.only" | "events.disable" => Rule::NonEmptyReplace,
        "worktree_naming" => Rule::DefaultSentinel("full"),
        "theme.scheme" => Rule::DefaultSentinel("default"),
        _ => Rule::Default,
    }
}

/// The `<global>` placeholder expanded by [`Rule::PlaceholderList`].
const PLACEHOLDER: &str = "<global>";

fn join_path(prefix: &str, key: &str) -> String {
    if prefix.is_empty() {
        key.to_string()
    } else {
        format!("{prefix}.{key}")
    }
}

/// Record `path` (and, for a mapping, every path beneath it) as set by `layer_id`.
fn record(prov: &mut Option<Provenance>, path: &str, layer_id: &str, value: &Value) {
    let Some(map) = prov.as_mut() else { return };
    map.insert(path.to_string(), layer_id.to_string());
    if let Value::Mapping(m) = value {
        for (k, v) in m {
            if let Some(key) = k.as_str() {
                record(prov, &join_path(path, key), layer_id, v);
            }
        }
    }
}

/// Drop every provenance entry at or beneath `path`, used when a key is deleted.
fn forget(prov: &mut Option<Provenance>, path: &str) {
    let Some(map) = prov.as_mut() else { return };
    let prefix = format!("{path}.");
    map.retain(|k, _| k != path && !k.starts_with(&prefix));
}

/// Merge `over` into `base` in place, applying the rule table at each path.
///
/// `path` is the dotted path of `base` within the config (empty at the root).
/// `layer_id` attributes any value written here; `prov` is `None` when tracking
/// is off, in which case no attribution map is allocated.
pub fn merge_values(
    base: &mut Value,
    over: Value,
    path: &str,
    layer_id: &str,
    prov: &mut Option<Provenance>,
) {
    match rule_for(path) {
        Rule::Replace => {
            // `bootstrap` replaces wholesale, but hooks are global-only
            // (untrusted layers get theirs stripped before merging), so the
            // replace must not wipe the trusted values — otherwise any project
            // declaring `bootstrap` silently disables every global hook. That
            // covers both `bootstrap.hooks` and hook-carrying entries of
            // `bootstrap.default_skills` (a skill and its hooks are declared
            // as one unit and must survive as one).
            let mut over = over;
            if path == "bootstrap"
                && let (Value::Mapping(base_map), Value::Mapping(over_map)) = (&*base, &mut over)
            {
                let key = Value::String("hooks".to_string());
                if !over_map.contains_key(&key)
                    && let Some(hooks) = base_map.get(&key)
                {
                    over_map.insert(key, hooks.clone());
                }
                graft_hook_skills(base_map, over_map);
            }
            forget(prov, path);
            record(prov, path, layer_id, &over);
            *base = over;
            return;
        }
        Rule::ShallowExtend => {
            if let (Value::Mapping(base_map), Value::Mapping(over_map)) = (&mut *base, &over) {
                for (k, v) in over_map.clone() {
                    let key_path = k
                        .as_str()
                        .map(|s| join_path(path, s))
                        .unwrap_or_else(|| path.to_string());
                    if v.is_null() {
                        base_map.remove(&k);
                        forget(prov, &key_path);
                        continue;
                    }
                    forget(prov, &key_path);
                    record(prov, &key_path, layer_id, &v);
                    base_map.insert(k, v);
                }
                return;
            }
        }
        Rule::Append => {
            if let (Value::Sequence(base_seq), Value::Sequence(over_seq)) = (&mut *base, &over) {
                base_seq.extend(over_seq.clone());
                record(prov, path, layer_id, &over);
                return;
            }
        }
        Rule::PlaceholderList => {
            if let (Value::Sequence(base_seq), Value::Sequence(over_seq)) = (&*base, &over) {
                let expanded = expand_placeholder(base_seq, over_seq);
                record(prov, path, layer_id, &over);
                *base = Value::Sequence(expanded);
                return;
            }
        }
        Rule::NonEmptyReplace => {
            if let Value::Sequence(over_seq) = &over
                && over_seq.is_empty()
            {
                return;
            }
        }
        Rule::DefaultSentinel(default) => {
            if over.as_str() == Some(default) {
                return;
            }
        }
        Rule::Default => {}
    }

    match (base, over) {
        (Value::Mapping(base_map), Value::Mapping(over_map)) => {
            merge_mapping(base_map, over_map, path, layer_id, prov);
        }
        (base_slot, over) => {
            forget(prov, path);
            record(prov, path, layer_id, &over);
            *base_slot = over;
        }
    }
}

fn merge_mapping(
    base_map: &mut Mapping,
    over_map: Mapping,
    path: &str,
    layer_id: &str,
    prov: &mut Option<Provenance>,
) {
    for (key, over_value) in over_map {
        let Some(key_str) = key.as_str().map(str::to_owned) else {
            // Non-string keys have no dotted path; replace positionally.
            base_map.insert(key, over_value);
            continue;
        };

        // `+key:` concatenates onto the inherited list instead of replacing it.
        if let Some(target) = key_str.strip_prefix('+') {
            let target_key = Value::String(target.to_string());
            let key_path = join_path(path, target);
            let mut merged = match base_map.get(&target_key) {
                Some(Value::Sequence(existing)) => existing.clone(),
                _ => Vec::new(),
            };
            if let Value::Sequence(add) = over_value {
                merged.extend(add);
            }
            record(prov, &key_path, layer_id, &Value::Null);
            base_map.insert(target_key, Value::Sequence(merged));
            continue;
        }

        let key_path = join_path(path, &key_str);

        // An explicit null removes the inherited key.
        if over_value.is_null() {
            base_map.remove(&key);
            forget(prov, &key_path);
            continue;
        }

        match base_map.get_mut(&key) {
            Some(base_value) => {
                merge_values(base_value, over_value, &key_path, layer_id, prov);
            }
            None => {
                record(prov, &key_path, layer_id, &over_value);
                base_map.insert(key, over_value);
            }
        }
    }
}

/// Expand `<global>` elements of `over` to the whole of `base`. With no
/// placeholder present the override replaces outright.
fn expand_placeholder(base: &[Value], over: &[Value]) -> Vec<Value> {
    if !over.iter().any(|v| v.as_str() == Some(PLACEHOLDER)) {
        return over.to_vec();
    }
    let mut out = Vec::with_capacity(over.len() + base.len());
    for item in over {
        if item.as_str() == Some(PLACEHOLDER) {
            out.extend(base.iter().cloned());
        } else {
            out.push(item.clone());
        }
    }
    out
}

/// Carry `default_skills` entries that declare hooks across a bootstrap
/// replace. Such entries can only come from trusted layers (untrusted ones
/// have hooks stripped by [`strip_skill_hooks`] before merging), and a skill
/// with its hooks is one unit — dropping it would silently disable the hook.
/// An entry in `over` with the same `path`/`url` is superseded by the trusted
/// one.
fn graft_hook_skills(base_map: &Mapping, over_map: &mut Mapping) {
    let skills_key = Value::String("default_skills".to_string());
    let hooks_key = Value::String("hooks".to_string());

    fn source_of(entry: &Value) -> Option<&Value> {
        let map = entry.as_mapping()?;
        map.get(Value::String("path".to_string()))
            .or_else(|| map.get(Value::String("url".to_string())))
    }

    let Some(Value::Sequence(base_skills)) = base_map.get(&skills_key) else {
        return;
    };
    let hook_skills: Vec<Value> = base_skills
        .iter()
        .filter(|e| {
            e.as_mapping()
                .and_then(|m| m.get(&hooks_key))
                .is_some_and(|h| h.as_mapping().is_some_and(|m| !m.is_empty()))
        })
        .cloned()
        .collect();
    if hook_skills.is_empty() {
        return;
    }

    let slot = over_map
        .entry(skills_key)
        .or_insert_with(|| Value::Sequence(Vec::new()));
    if !slot.is_sequence() {
        return;
    }
    let entries = slot.as_sequence_mut().expect("just checked");
    for skill in hook_skills {
        let src = source_of(&skill);
        match entries
            .iter_mut()
            .find(|e| src.is_some() && source_of(e) == src)
        {
            Some(existing) => *existing = skill,
            None => entries.push(skill),
        }
    }
}

/// Strip `hooks` from every skill entry in an untrusted layer.
///
/// A shipped SKILL.md is text an agent may act on; a hook is a command the
/// harness runs unattended on every turn. A cloned repository may ship the
/// skill and its scripts, but binding them to the machine's lifecycle is the
/// machine owner's call, made in the global config.
fn strip_skill_hooks(layer: &mut Layer, warnings: &mut Vec<LayerWarning>) {
    let Some(bootstrap) = layer.value.get_mut("bootstrap") else {
        return;
    };

    // Global (skill-less) hooks are the same hazard in a different position.
    if let Value::Mapping(map) = &mut *bootstrap
        && map.remove(Value::String("hooks".to_string())).is_some()
    {
        warnings.push(LayerWarning {
            layer_id: String::new(),
            source: String::new(),
            path: "bootstrap.hooks".to_string(),
            message: "bootstrap.hooks is ignored outside the global config -- a hook is a \
                      command run unattended on every turn, so enabling it is the machine \
                      owner's decision"
                .to_string(),
        });
    }

    let mut strip_list = |list: &mut Value, path: &str| {
        let Value::Sequence(entries) = list else { return };
        for entry in entries {
            let Value::Mapping(map) = entry else { continue };
            let hooks_key = Value::String("hooks".to_string());
            if map.remove(&hooks_key).is_some() {
                let name = map
                    .get(Value::String("path".to_string()))
                    .or_else(|| map.get(Value::String("url".to_string())))
                    .and_then(|v| v.as_str())
                    .unwrap_or("<skill>");
                warnings.push(LayerWarning {
                    layer_id: layer_id_placeholder(),
                    source: String::new(),
                    path: format!("{path} ({name})"),
                    message: format!(
                        "skill hooks on {name} are ignored outside the global config -- \
                         a hook is a command run unattended on every turn, so enabling it \
                         is the machine owner's decision"
                    ),
                });
            }
        }
    };

    // Placeholder identity filled in by the caller below; the closure cannot
    // borrow `layer` while `bootstrap` is borrowed from it.
    fn layer_id_placeholder() -> String {
        String::new()
    }

    if let Some(list) = bootstrap.get_mut("default_skills") {
        strip_list(list, "bootstrap.default_skills");
    }

    if let Some(Value::Mapping(agents)) = bootstrap.get_mut("agents") {
        for (_, overrides) in agents.iter_mut() {
            if let Some(list) = overrides.get_mut("additional_skills") {
                strip_list(list, "bootstrap.agents.*.additional_skills");
            }
        }
    }
}

/// Remove global-only keys from an untrusted layer, returning a warning for
/// each one removed.
///
/// Running this before the merge is what keeps [`merge_values`] free of any
/// notion of layer trust.
pub fn strip_global_only(layer: &mut Layer) -> Vec<LayerWarning> {
    if layer.trusted {
        return Vec::new();
    }
    let mut warnings = Vec::new();

    let before = warnings.len();
    strip_skill_hooks(layer, &mut warnings);
    for w in &mut warnings[before..] {
        w.layer_id = layer.id.clone();
        w.source = layer.source.clone();
    }

    for path in GLOBAL_ONLY_PATHS {
        if remove_path(&mut layer.value, path) {
            warnings.push(LayerWarning {
                layer_id: layer.id.clone(),
                source: layer.source.clone(),
                path: (*path).to_string(),
                message: format!(
                    "{path} is ignored outside the global config -- move it to \
                     your global config (~/.config/workmux/config.yaml)"
                ),
            });
        }
    }
    warnings
}

/// Remove a dotted path from a value tree. Returns whether anything was removed.
fn remove_path(value: &mut Value, path: &str) -> bool {
    let Some((head, rest)) = split_path(path) else {
        return false;
    };
    let Value::Mapping(map) = value else {
        return false;
    };
    let key = Value::String(head.to_string());
    match rest {
        None => map.remove(&key).is_some(),
        Some(rest) => match map.get_mut(&key) {
            Some(child) => remove_path(child, rest),
            None => false,
        },
    }
}

fn split_path(path: &str) -> Option<(&str, Option<&str>)> {
    if path.is_empty() {
        return None;
    }
    match path.split_once('.') {
        Some((head, rest)) => Some((head, Some(rest))),
        None => Some((path, None)),
    }
}

/// `panes:` and `windows:` are mutually exclusive layout choices. Whichever a
/// layer specifies wins entirely, clearing the other from everything inherited
/// so far.
///
/// Applied after each layer merges rather than inside the merge, so the rule
/// generalizes to any number of layers instead of only global-vs-project.
fn normalize_layout_exclusivity(base: &mut Value, over: &Value, prov: &mut Option<Provenance>) {
    let Value::Mapping(over_map) = over else {
        return;
    };
    let sets = |k: &str| {
        over_map
            .get(Value::String(k.to_string()))
            .is_some_and(|v| !v.is_null())
    };
    let Value::Mapping(base_map) = base else {
        return;
    };
    if sets("windows") {
        base_map.remove(Value::String("panes".to_string()));
        forget(prov, "panes");
    } else if sets("panes") {
        base_map.remove(Value::String("windows".to_string()));
        forget(prov, "windows");
    }
}

/// Merge every layer in order, lowest precedence first.
///
/// Set `track` only when attribution is needed (`config resolve --explain`);
/// leaving it off keeps the hot config-load path allocation-free.
pub fn resolve(layers: Vec<Layer>, track: bool) -> Resolved {
    let mut value = Value::Mapping(Mapping::new());
    let mut provenance = if track { Some(Provenance::new()) } else { None };
    let mut warnings = Vec::new();

    for mut layer in layers {
        warnings.extend(strip_global_only(&mut layer));
        normalize_layout_exclusivity(&mut value, &layer.value, &mut provenance);
        merge_values(&mut value, layer.value, "", &layer.id, &mut provenance);
    }

    Resolved {
        value,
        provenance,
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn yaml(s: &str) -> Value {
        serde_yaml::from_str(s).unwrap()
    }

    fn merge2(base: &str, over: &str) -> Value {
        let mut b = yaml(base);
        merge_values(&mut b, yaml(over), "", "over", &mut None);
        b
    }

    // --- default rules -----------------------------------------------------

    #[test]
    fn scalar_replaces() {
        assert_eq!(merge2("agent: claude", "agent: codex"), yaml("agent: codex"));
    }

    #[test]
    fn mapping_deep_merges() {
        let got = merge2(
            "sandbox:\n  enabled: true\n  image: a",
            "sandbox:\n  image: b",
        );
        assert_eq!(got, yaml("sandbox:\n  enabled: true\n  image: b"));
    }

    #[test]
    fn nested_mapping_deep_merges() {
        let got = merge2(
            "orchestrate:\n  harness:\n    default_workflow: g.yaml\n    first_prompt: keep",
            "orchestrate:\n  harness:\n    default_workflow: p.yaml",
        );
        assert_eq!(
            got,
            yaml("orchestrate:\n  harness:\n    default_workflow: p.yaml\n    first_prompt: keep")
        );
    }

    #[test]
    fn sequence_replaces_by_default() {
        let got = merge2("windows: [a, b]", "windows: [c]");
        assert_eq!(got, yaml("windows: [c]"));
    }

    #[test]
    fn differing_shapes_replace() {
        let got = merge2("agent: {a: 1}", "agent: claude");
        assert_eq!(got, yaml("agent: claude"));
    }

    #[test]
    fn absent_key_is_taken_from_override() {
        let got = merge2("agent: claude", "merge_strategy: rebase");
        assert_eq!(got, yaml("agent: claude\nmerge_strategy: rebase"));
    }

    // --- append and delete -------------------------------------------------

    #[test]
    fn plus_key_appends_to_inherited_list() {
        let got = merge2("windows: [a, b]", "+windows: [c]");
        assert_eq!(got, yaml("windows: [a, b, c]"));
    }

    #[test]
    fn plus_key_on_absent_base_creates_list() {
        let got = merge2("agent: claude", "+windows: [c]");
        assert_eq!(got, yaml("agent: claude\nwindows: [c]"));
    }

    #[test]
    fn null_deletes_inherited_key() {
        let got = merge2("mcp:\n  a:\n    command: x\nagent: claude", "agent: null");
        assert_eq!(got, yaml("mcp:\n  a:\n    command: x"));
    }

    #[test]
    fn null_deletes_nested_key() {
        let got = merge2(
            "sandbox:\n  enabled: true\n  image: a",
            "sandbox:\n  image: null",
        );
        assert_eq!(got, yaml("sandbox:\n  enabled: true"));
    }

    #[test]
    fn null_on_absent_key_is_a_noop() {
        let got = merge2("agent: claude", "windows: null");
        assert_eq!(got, yaml("agent: claude"));
    }

    // --- compatibility rules ----------------------------------------------

    #[test]
    fn bootstrap_replaces_wholesale() {
        // Deep-merging would leave `default_skills` behind from the base.
        let got = merge2(
            "bootstrap:\n  default_skills: [a]\n  default_subagents: [s]",
            "bootstrap:\n  default_plugins: [p]",
        );
        assert_eq!(got, yaml("bootstrap:\n  default_plugins: [p]"));
    }

    #[test]
    fn bootstrap_replace_keeps_hook_carrying_skills() {
        // A skill entry with hooks is a trusted, global-only unit; a project
        // bootstrap must not drop it (and a same-source project entry without
        // hooks is superseded, not duplicated).
        let got = merge2(
            "bootstrap:\n  default_skills:\n    - path: /s/auto-git\n      hooks:\n        turn-done:\n          - command: x\n    - /s/plain",
            "bootstrap:\n  default_skills: [/p/own]",
        );
        assert_eq!(
            got,
            yaml(
                "bootstrap:\n  default_skills:\n    - /p/own\n    - path: /s/auto-git\n      hooks:\n        turn-done:\n          - command: x"
            )
        );

        let superseded = merge2(
            "bootstrap:\n  default_skills:\n    - path: /s/auto-git\n      hooks:\n        turn-done:\n          - command: x",
            "bootstrap:\n  default_skills:\n    - path: /s/auto-git",
        );
        assert_eq!(
            superseded,
            yaml(
                "bootstrap:\n  default_skills:\n    - path: /s/auto-git\n      hooks:\n        turn-done:\n          - command: x"
            )
        );
    }

    #[test]
    fn bootstrap_replace_keeps_global_hooks() {
        // hooks is global-only (stripped from untrusted layers before merging),
        // so a project bootstrap must not wipe it.
        let got = merge2(
            "bootstrap:\n  hooks:\n    turn-done:\n      - command: x\n  default_skills: [a]",
            "bootstrap:\n  default_plugins: [p]",
        );
        assert_eq!(
            got,
            yaml("bootstrap:\n  default_plugins: [p]\n  hooks:\n    turn-done:\n      - command: x")
        );
    }

    #[test]
    fn providers_replace_wholesale() {
        let got = merge2("providers:\n  a:\n    x: 1", "providers:\n  b:\n    y: 2");
        assert_eq!(got, yaml("providers:\n  b:\n    y: 2"));
    }

    #[test]
    fn mcp_shallow_extends() {
        let got = merge2(
            "mcp:\n  a:\n    command: x\n    args: [1]\n  b:\n    command: y",
            "mcp:\n  a:\n    command: z",
        );
        // `a` is replaced wholesale (args gone), `b` is untouched.
        assert_eq!(
            got,
            yaml("mcp:\n  a:\n    command: z\n  b:\n    command: y")
        );
    }

    #[test]
    fn mcp_null_removes_one_server() {
        let got = merge2(
            "mcp:\n  a:\n    command: x\n  b:\n    command: y",
            "mcp:\n  a: null",
        );
        assert_eq!(got, yaml("mcp:\n  b:\n    command: y"));
    }

    #[test]
    fn layouts_shallow_extend() {
        let got = merge2("layouts:\n  g:\n    panes: [1]", "layouts:\n  p:\n    panes: [2]");
        assert_eq!(
            got,
            yaml("layouts:\n  g:\n    panes: [1]\n  p:\n    panes: [2]")
        );
    }

    #[test]
    fn agent_registries_append() {
        let got = merge2("agent_registries: [g]", "agent_registries: [p]");
        assert_eq!(got, yaml("agent_registries: [g, p]"));
    }

    #[test]
    fn placeholder_list_expands_global() {
        let got = merge2("pre_merge: [a, b]", "pre_merge: ['<global>', c]");
        assert_eq!(got, yaml("pre_merge: [a, b, c]"));
    }

    #[test]
    fn placeholder_list_without_placeholder_replaces() {
        let got = merge2("pre_merge: [a, b]", "pre_merge: [c]");
        assert_eq!(got, yaml("pre_merge: [c]"));
    }

    #[test]
    fn placeholder_expands_in_files_symlink() {
        let got = merge2(
            "files:\n  symlink: [a]",
            "files:\n  symlink: [b, '<global>']",
        );
        assert_eq!(got, yaml("files:\n  symlink: [b, a]"));
    }

    #[test]
    fn events_empty_list_inherits() {
        let got = merge2("events:\n  disable: [a]", "events:\n  disable: []");
        assert_eq!(got, yaml("events:\n  disable: [a]"));
    }

    #[test]
    fn events_non_empty_list_replaces() {
        let got = merge2("events:\n  disable: [a]", "events:\n  disable: [b]");
        assert_eq!(got, yaml("events:\n  disable: [b]"));
    }

    #[test]
    fn default_sentinel_inherits() {
        let got = merge2("worktree_naming: basename", "worktree_naming: full");
        assert_eq!(got, yaml("worktree_naming: basename"));
    }

    #[test]
    fn non_default_sentinel_overrides() {
        let got = merge2("worktree_naming: full", "worktree_naming: basename");
        assert_eq!(got, yaml("worktree_naming: basename"));
    }

    #[test]
    fn theme_scheme_sentinel() {
        let got = merge2("theme:\n  scheme: mossfire", "theme:\n  scheme: default");
        assert_eq!(got, yaml("theme:\n  scheme: mossfire"));
    }

    // --- global-only stripping --------------------------------------------

    #[test]
    fn untrusted_layer_loses_global_only_keys() {
        let mut layer = Layer::new(
            "project",
            ".workmux.yaml",
            LayerKind::Project,
            yaml("agent: claude\nagents:\n  x:\n    command: sh\nsandbox:\n  env:\n    A: 1\n  image: keep"),
        );
        let warnings = strip_global_only(&mut layer);
        assert_eq!(layer.value, yaml("agent: claude\nsandbox:\n  image: keep"));
        let paths: Vec<_> = warnings.iter().map(|w| w.path.as_str()).collect();
        assert!(paths.contains(&"agents"));
        assert!(paths.contains(&"sandbox.env"));
    }

    #[test]
    fn trusted_layer_keeps_global_only_keys() {
        let mut layer = Layer::new(
            "global",
            "config.yaml",
            LayerKind::Global,
            yaml("agents:\n  x:\n    command: sh"),
        );
        let warnings = strip_global_only(&mut layer);
        assert!(warnings.is_empty());
        assert_eq!(layer.value, yaml("agents:\n  x:\n    command: sh"));
    }

    #[test]
    fn stripping_a_leaf_leaves_siblings() {
        let mut layer = Layer::new(
            "project",
            ".workmux.yaml",
            LayerKind::Project,
            yaml("sandbox:\n  container:\n    devices: [/dev/kvm]\n    cpus: 2"),
        );
        strip_global_only(&mut layer);
        assert_eq!(layer.value, yaml("sandbox:\n  container:\n    cpus: 2"));
    }

    // --- layout exclusivity ------------------------------------------------

    #[test]
    fn project_windows_clears_inherited_panes() {
        let layers = vec![
            Layer::new("global", "g", LayerKind::Global, yaml("panes: [{command: a}]")),
            Layer::new(
                "project",
                "p",
                LayerKind::Project,
                yaml("windows: [{name: w}]"),
            ),
        ];
        let got = resolve(layers, false).value;
        assert_eq!(got, yaml("windows: [{name: w}]"));
    }

    #[test]
    fn project_panes_clears_inherited_windows() {
        let layers = vec![
            Layer::new("global", "g", LayerKind::Global, yaml("windows: [{name: w}]")),
            Layer::new(
                "project",
                "p",
                LayerKind::Project,
                yaml("panes: [{command: a}]"),
            ),
        ];
        let got = resolve(layers, false).value;
        assert_eq!(got, yaml("panes: [{command: a}]"));
    }

    #[test]
    fn layout_untouched_when_override_sets_neither() {
        let layers = vec![
            Layer::new("global", "g", LayerKind::Global, yaml("panes: [{command: a}]")),
            Layer::new("project", "p", LayerKind::Project, yaml("agent: claude")),
        ];
        let got = resolve(layers, false).value;
        assert_eq!(got, yaml("panes: [{command: a}]\nagent: claude"));
    }

    // --- provenance --------------------------------------------------------

    #[test]
    fn tracking_off_allocates_no_map() {
        let layers = vec![Layer::new(
            "global",
            "g",
            LayerKind::Global,
            yaml("agent: claude"),
        )];
        assert!(resolve(layers, false).provenance.is_none());
    }

    #[test]
    fn provenance_attributes_final_writer() {
        let layers = vec![
            Layer::new(
                "global",
                "g",
                LayerKind::Global,
                yaml("agent: claude\nmerge_strategy: rebase"),
            ),
            Layer::new("project", "p", LayerKind::Project, yaml("agent: codex")),
        ];
        let prov = resolve(layers, true).provenance.unwrap();
        assert_eq!(prov.get("agent").map(String::as_str), Some("project"));
        assert_eq!(
            prov.get("merge_strategy").map(String::as_str),
            Some("global")
        );
    }

    #[test]
    fn provenance_descends_into_mappings() {
        let layers = vec![
            Layer::new(
                "global",
                "g",
                LayerKind::Global,
                yaml("sandbox:\n  image: a\n  enabled: true"),
            ),
            Layer::new(
                "project",
                "p",
                LayerKind::Project,
                yaml("sandbox:\n  image: b"),
            ),
        ];
        let prov = resolve(layers, true).provenance.unwrap();
        assert_eq!(prov.get("sandbox.image").map(String::as_str), Some("project"));
        assert_eq!(
            prov.get("sandbox.enabled").map(String::as_str),
            Some("global")
        );
    }

    #[test]
    fn provenance_forgets_deleted_keys() {
        let layers = vec![
            Layer::new(
                "global",
                "g",
                LayerKind::Global,
                yaml("sandbox:\n  image: a\n  enabled: true"),
            ),
            Layer::new(
                "project",
                "p",
                LayerKind::Project,
                yaml("sandbox:\n  image: null"),
            ),
        ];
        let prov = resolve(layers, true).provenance.unwrap();
        assert!(!prov.contains_key("sandbox.image"));
    }

    // --- ordering ----------------------------------------------------------

    #[test]
    fn later_layers_win() {
        let layers = vec![
            Layer::new("a", "a", LayerKind::Include, yaml("agent: one")),
            Layer::new("b", "b", LayerKind::Include, yaml("agent: two")),
            Layer::new("c", "c", LayerKind::Global, yaml("agent: three")),
        ];
        assert_eq!(resolve(layers, false).value, yaml("agent: three"));
    }


    #[test]
    fn empty_layer_list_resolves_to_empty_mapping() {
        assert_eq!(resolve(vec![], false).value, Value::Mapping(Mapping::new()));
    }
}
