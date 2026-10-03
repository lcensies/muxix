//! Turning a cached org policy into config layers.
//!
//! A policy participates in ordinary config resolution rather than being a
//! post-hoc patch, which is what makes `muxix config resolve --explain` able
//! to say "this value came from the org policy" at all.
//!
//! The two halves sit at opposite ends of the layer stack, and that is
//! deliberate:
//!
//! * **defaults** rank *below* profiles and CLI flags. A policy default is a
//!   suggestion for a machine that has not decided; it must not silently beat a
//!   deliberate project or profile choice.
//! * **locks** rank *above* everything, including CLI flags. A lock is the one
//!   thing an organization can actually rely on, so it wins and reports every
//!   value it overrode.
//!
//! Splitting them is the only ordering under which both words mean what they
//! say.

use serde_yaml::Value;

use crate::config::resolve::{Layer, LayerKind};
use crate::provision::types::OrgPolicy;

/// Convert a `serde_json::Value` to the YAML value the resolver merges.
///
/// Policies travel as JSON but configs resolve as YAML; YAML is a superset, so
/// this is lossless.
fn json_to_yaml(v: &serde_json::Value) -> Option<Value> {
    serde_json::from_value::<Value>(v.clone()).ok()
}

/// The layer carrying a policy's `defaults`, or `None` when it declares none.
pub fn defaults_layer(policy: &OrgPolicy) -> Option<Layer> {
    let value = json_to_yaml(&policy.defaults)?;
    if !matches!(value, Value::Mapping(ref m) if !m.is_empty()) {
        return None;
    }
    Some(Layer::new(
        "policy:defaults",
        format!("org policy {} (defaults)", policy.policy_version),
        LayerKind::PolicyDefaults,
        value,
    ))
}

/// The layer carrying a policy's `locked` values, or `None` when it locks
/// nothing.
///
/// Locks are trusted by construction: they come from the organization, not from
/// a repository, so global-only keys are theirs to set.
pub fn locks_layer(policy: &OrgPolicy) -> Option<Layer> {
    let mut map = serde_yaml::Mapping::new();

    if let Some(v) = policy.locked.proxy_chain.as_ref().and_then(json_to_yaml) {
        map.insert(Value::String("proxy_chain".into()), v);
    }
    if let Some(v) = policy
        .locked
        .sandbox_network
        .as_ref()
        .and_then(json_to_yaml)
    {
        // `sandbox.network` is nested; build the path so it deep-merges rather
        // than replacing the whole sandbox block.
        let mut sandbox = serde_yaml::Mapping::new();
        sandbox.insert(Value::String("network".into()), v);
        map.insert(Value::String("sandbox".into()), Value::Mapping(sandbox));
    }

    if map.is_empty() {
        return None;
    }
    Some(Layer::new(
        "policy:locks",
        format!("org policy {} (locked)", policy.policy_version),
        LayerKind::PolicyLocks,
        Value::Mapping(map),
    ))
}

/// Dotted paths a locks layer sets, for override reporting.
pub fn locked_paths(layer: &Layer) -> Vec<String> {
    fn walk(value: &Value, prefix: &str, out: &mut Vec<String>) {
        let Value::Mapping(map) = value else {
            if !prefix.is_empty() {
                out.push(prefix.to_string());
            }
            return;
        };
        for (k, v) in map {
            let Some(key) = k.as_str() else { continue };
            let path = if prefix.is_empty() {
                key.to_string()
            } else {
                format!("{prefix}.{key}")
            };
            match v {
                Value::Mapping(_) => walk(v, &path, out),
                _ => out.push(path),
            }
        }
    }
    let mut out = Vec::new();
    walk(&layer.value, "", &mut out);
    out
}

/// Read a dotted path out of a value tree.
fn get_path<'a>(value: &'a Value, path: &str) -> Option<&'a Value> {
    let mut cur = value;
    for segment in path.split('.') {
        cur = cur.get(segment)?;
    }
    Some(cur)
}

/// Which locked paths a lower layer had already set, and to what.
///
/// Reported so a user whose explicit setting was overridden learns why, rather
/// than concluding muxix ignored their config.
pub fn overrides(base: &Value, locks: &Layer) -> Vec<String> {
    locked_paths(locks)
        .into_iter()
        .filter_map(|path| {
            let existing = get_path(base, &path)?;
            let locked = get_path(&locks.value, &path)?;
            if existing == locked {
                return None;
            }
            Some(path)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provision::types::PolicyLockedFields;

    fn policy_with_defaults(json: serde_json::Value) -> OrgPolicy {
        OrgPolicy {
            policy_version: "v1".into(),
            defaults: json,
            ..Default::default()
        }
    }

    #[test]
    fn no_defaults_produces_no_layer() {
        assert!(defaults_layer(&OrgPolicy::default()).is_none());
        assert!(defaults_layer(&policy_with_defaults(serde_json::json!({}))).is_none());
    }

    #[test]
    fn defaults_become_a_layer() {
        let policy = policy_with_defaults(serde_json::json!({ "agent": "codex" }));
        let layer = defaults_layer(&policy).unwrap();
        assert_eq!(layer.id, "policy:defaults");
        assert_eq!(layer.value.get("agent").unwrap().as_str(), Some("codex"));
    }

    #[test]
    fn defaults_layer_is_named_after_the_policy_version() {
        let layer = defaults_layer(&policy_with_defaults(
            serde_json::json!({ "agent": "codex" }),
        ))
        .unwrap();
        assert!(layer.source.contains("v1"), "{}", layer.source);
    }

    #[test]
    fn no_locks_produces_no_layer() {
        assert!(locks_layer(&OrgPolicy::default()).is_none());
    }

    #[test]
    fn locked_proxy_chain_becomes_a_layer() {
        let policy = OrgPolicy {
            policy_version: "v2".into(),
            locked: PolicyLockedFields {
                proxy_chain: Some(serde_json::json!({ "enabled": true })),
                sandbox_network: None,
            },
            ..Default::default()
        };
        let layer = locks_layer(&policy).unwrap();
        assert!(layer.value.get("proxy_chain").is_some());
        assert!(layer.trusted, "policy locks may set global-only keys");
    }

    #[test]
    fn locked_sandbox_network_nests_under_sandbox() {
        let policy = OrgPolicy {
            locked: PolicyLockedFields {
                proxy_chain: None,
                sandbox_network: Some(serde_json::json!({ "policy": "deny" })),
            },
            ..Default::default()
        };
        let layer = locks_layer(&policy).unwrap();
        let net = layer.value.get("sandbox").unwrap().get("network").unwrap();
        assert_eq!(net.get("policy").unwrap().as_str(), Some("deny"));
    }

    #[test]
    fn locked_paths_are_dotted_leaves() {
        let policy = OrgPolicy {
            locked: PolicyLockedFields {
                proxy_chain: Some(serde_json::json!({ "enabled": true })),
                sandbox_network: Some(serde_json::json!({ "policy": "deny" })),
            },
            ..Default::default()
        };
        let mut paths = locked_paths(&locks_layer(&policy).unwrap());
        paths.sort();
        assert_eq!(
            paths,
            vec![
                "proxy_chain.enabled".to_string(),
                "sandbox.network.policy".to_string()
            ]
        );
    }

    #[test]
    fn overrides_reports_a_conflicting_value() {
        let policy = OrgPolicy {
            locked: PolicyLockedFields {
                proxy_chain: None,
                sandbox_network: Some(serde_json::json!({ "policy": "deny" })),
            },
            ..Default::default()
        };
        let locks = locks_layer(&policy).unwrap();
        let base: Value =
            serde_yaml::from_str("sandbox:\n  network:\n    policy: allow\n").unwrap();
        assert_eq!(overrides(&base, &locks), vec!["sandbox.network.policy"]);
    }

    #[test]
    fn overrides_is_silent_when_the_user_agrees() {
        let policy = OrgPolicy {
            locked: PolicyLockedFields {
                proxy_chain: None,
                sandbox_network: Some(serde_json::json!({ "policy": "deny" })),
            },
            ..Default::default()
        };
        let locks = locks_layer(&policy).unwrap();
        let base: Value = serde_yaml::from_str("sandbox:\n  network:\n    policy: deny\n").unwrap();
        assert!(overrides(&base, &locks).is_empty());
    }

    #[test]
    fn overrides_is_silent_when_the_user_set_nothing() {
        let policy = OrgPolicy {
            locked: PolicyLockedFields {
                proxy_chain: None,
                sandbox_network: Some(serde_json::json!({ "policy": "deny" })),
            },
            ..Default::default()
        };
        let locks = locks_layer(&policy).unwrap();
        let base: Value = serde_yaml::from_str("agent: claude\n").unwrap();
        assert!(overrides(&base, &locks).is_empty());
    }
}
