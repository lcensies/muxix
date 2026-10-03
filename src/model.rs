//! Unified, provider-centric model registry.
//!
//! Declared once per project, this maps each **provider** (the source that
//! serves a model) to the models it offers. A single logical model such as
//! `sonnet` can therefore appear under several providers with different
//! provider-specific ids and limits — e.g. Anthropic direct and AWS Bedrock.
//! The rest of muxix resolves context/compaction limits by provider id or
//! logical name instead of hard-coding them per agent.
//!
//! ```yaml
//! providers:
//!   anthropic:
//!     limit: 200000                       # provider default context window
//!     models:
//!       - name: sonnet
//!         id: claude-sonnet-4-6
//!       - name: sonnet-1m
//!         id: "claude-sonnet-4-6[1m]"
//!         limit: 1000000                  # per-model override
//!       - name: opus
//!         id: claude-opus-4-8
//!   bedrock:
//!     limit: 200000
//!     models:
//!       - name: sonnet                    # same logical model, another source
//!         id: us.anthropic.claude-sonnet-4-6-v1:0
//! ```

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Default fraction of the context window at which compaction should trigger
/// when neither the model nor its provider specify a `compaction_limit`.
pub const DEFAULT_COMPACTION_RATIO: f64 = 0.85;

/// Top-level registry: provider key (source) -> its configuration.
pub type ProviderRegistry = BTreeMap<String, ProviderConfig>;

/// One provider (source) and the models it serves.
///
/// The optional connection fields (`base_url`, `api_key_env`, `npm`, `api`,
/// `options`) describe how to *reach* the provider; when any of the first
/// three is set, `muxix setup` materializes the provider into each
/// supported agent's native config (see [`has_connection`]). Secrets are
/// referenced by environment variable NAME only — muxix never reads or
/// writes their values.
///
/// [`has_connection`]: ProviderConfig::has_connection
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ProviderConfig {
    /// Default context window (tokens) for this provider's models, used when a
    /// model does not set its own `limit`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,

    /// Default compaction threshold (tokens) for this provider's models, used
    /// when a model does not set its own `compaction_limit`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compaction_limit: Option<u64>,

    /// Endpoint base URL. May embed `{env:VAR}` references, which are passed
    /// through verbatim to agents that support them (OpenCode) and cause the
    /// provider to be skipped on agents that do not (Codex).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,

    /// Name of the environment variable holding the API key. Only the name is
    /// ever stored or rendered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,

    /// Adapter package for agents that load providers via npm (OpenCode).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub npm: Option<String>,

    /// Wire protocol spoken at `base_url`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api: Option<ProviderApi>,

    /// Extra options merged verbatim into agent provider config (e.g.
    /// OpenCode `options.*`).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub options: BTreeMap<String, serde_json::Value>,

    /// The models this provider serves.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<ProviderModel>,
}

impl ProviderConfig {
    /// Whether this provider declares connection details and should be synced
    /// into agent configs. Registry-only providers (models/limits) return
    /// false and are never written anywhere.
    pub fn has_connection(&self) -> bool {
        self.base_url.is_some() || self.api_key_env.is_some() || self.npm.is_some()
    }

    /// Effective wire protocol (defaults to OpenAI-compatible).
    pub fn api(&self) -> ProviderApi {
        self.api.unwrap_or_default()
    }
}

/// Wire protocol a provider endpoint speaks.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProviderApi {
    #[default]
    Openai,
    Anthropic,
}

/// A model as served by a specific provider.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProviderModel {
    /// Logical name/alias, e.g. `sonnet`. May repeat across providers.
    pub name: String,

    /// Provider-specific model id passed to that provider.
    pub id: String,

    /// Optional routing tier hint, e.g. `low` / `medium` / `high`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<String>,

    /// Context window (tokens). Falls back to the provider default when unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u64>,

    /// Token count at which to compact. Optional; callers can derive one from
    /// the effective limit via [`ResolvedModel::effective_compaction_limit`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compaction_limit: Option<u64>,
}

/// A model resolved against its provider, carrying enough context to compute
/// effective limits (model value first, then provider default).
#[derive(Debug, Clone, Copy)]
pub struct ResolvedModel<'a> {
    pub provider: &'a str,
    pub provider_config: &'a ProviderConfig,
    pub model: &'a ProviderModel,
}

impl ResolvedModel<'_> {
    /// Effective context window: the model's `limit`, else the provider default.
    pub fn context_limit(&self) -> Option<u64> {
        self.model.limit.or(self.provider_config.limit)
    }

    /// Compaction threshold in tokens: explicit model value, else explicit
    /// provider value, else `context_limit * ratio`, else `None`.
    pub fn effective_compaction_limit(&self, ratio: f64) -> Option<u64> {
        if let Some(explicit) = self.model.compaction_limit {
            return Some(explicit);
        }
        if let Some(explicit) = self.provider_config.compaction_limit {
            return Some(explicit);
        }
        self.context_limit()
            .map(|limit| (limit as f64 * ratio).round() as u64)
    }
}

/// Resolve every model binding matching `key`, matching (case-insensitively)
/// either a provider-specific id or a logical model name. A logical name can
/// match multiple providers; a provider id is unique in practice.
pub fn resolve_all<'a>(registry: &'a ProviderRegistry, key: &str) -> Vec<ResolvedModel<'a>> {
    let k = key.trim().to_lowercase();
    let mut out = Vec::new();
    for (provider, cfg) in registry {
        for model in &cfg.models {
            if model.id.to_lowercase() == k || model.name.to_lowercase() == k {
                out.push(ResolvedModel {
                    provider,
                    provider_config: cfg,
                    model,
                });
            }
        }
    }
    out
}

/// Resolve a single model binding, preferring an exact provider-id match over a
/// logical-name match (id is the more specific key).
pub fn resolve<'a>(registry: &'a ProviderRegistry, key: &str) -> Option<ResolvedModel<'a>> {
    let k = key.trim().to_lowercase();
    let matches = resolve_all(registry, key);
    matches
        .iter()
        .find(|r| r.model.id.to_lowercase() == k)
        .or_else(|| matches.first())
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn registry() -> ProviderRegistry {
        let mut reg = ProviderRegistry::new();
        reg.insert(
            "anthropic".into(),
            ProviderConfig {
                limit: Some(200_000),
                compaction_limit: None,
                models: vec![
                    ProviderModel {
                        name: "sonnet".into(),
                        id: "claude-sonnet-4-6".into(),
                        tier: Some("medium".into()),
                        limit: None,
                        compaction_limit: None,
                    },
                    ProviderModel {
                        name: "sonnet-1m".into(),
                        id: "claude-sonnet-4-6[1m]".into(),
                        tier: None,
                        limit: Some(1_000_000),
                        compaction_limit: None,
                    },
                ],
                ..Default::default()
            },
        );
        reg.insert(
            "bedrock".into(),
            ProviderConfig {
                limit: Some(200_000),
                compaction_limit: None,
                models: vec![ProviderModel {
                    name: "sonnet".into(),
                    id: "us.anthropic.claude-sonnet-4-6-v1:0".into(),
                    tier: None,
                    limit: None,
                    compaction_limit: None,
                }],
                ..Default::default()
            },
        );
        reg
    }

    #[test]
    fn logical_name_matches_across_providers() {
        let reg = registry();
        let all = resolve_all(&reg, "sonnet");
        let providers: Vec<_> = all.iter().map(|r| r.provider).collect();
        assert_eq!(providers, vec!["anthropic", "bedrock"]);
    }

    #[test]
    fn provider_id_resolves_uniquely() {
        let reg = registry();
        let r = resolve(&reg, "us.anthropic.claude-sonnet-4-6-v1:0").unwrap();
        assert_eq!(r.provider, "bedrock");
        assert_eq!(r.model.name, "sonnet");
    }

    #[test]
    fn model_limit_falls_back_to_provider_default() {
        let reg = registry();
        let r = resolve(&reg, "claude-sonnet-4-6").unwrap();
        assert_eq!(r.context_limit(), Some(200_000));
    }

    #[test]
    fn per_model_limit_overrides_provider_default() {
        let reg = registry();
        let r = resolve(&reg, "claude-sonnet-4-6[1m]").unwrap();
        assert_eq!(r.context_limit(), Some(1_000_000));
    }

    #[test]
    fn compaction_limit_defaults_from_ratio() {
        let reg = registry();
        let r = resolve(&reg, "sonnet").unwrap();
        assert_eq!(r.effective_compaction_limit(0.85), Some(170_000));
    }

    #[test]
    fn explicit_model_compaction_limit_wins() {
        let mut reg = registry();
        reg.get_mut("anthropic").unwrap().models[0].compaction_limit = Some(150_000);
        let r = resolve(&reg, "claude-sonnet-4-6").unwrap();
        assert_eq!(r.effective_compaction_limit(0.85), Some(150_000));
    }

    #[test]
    fn provider_compaction_default_applies() {
        let mut reg = registry();
        reg.get_mut("anthropic").unwrap().compaction_limit = Some(160_000);
        let r = resolve(&reg, "claude-sonnet-4-6").unwrap();
        assert_eq!(r.effective_compaction_limit(0.85), Some(160_000));
    }

    #[test]
    fn unknown_key_resolves_to_none() {
        let reg = registry();
        assert!(resolve(&reg, "haiku").is_none());
        assert!(resolve_all(&reg, "haiku").is_empty());
    }

    #[test]
    fn connection_fields_parse() {
        let cfg: ProviderConfig = serde_yaml::from_str(
            "base_url: \"{env:LITELLM_BASE_URL}\"\napi_key_env: LITELLM_API_KEY\nnpm: \"@ai-sdk/openai-compatible\"\napi: anthropic\noptions:\n  timeout: 30\n",
        )
        .unwrap();
        assert!(cfg.has_connection());
        assert_eq!(cfg.api(), ProviderApi::Anthropic);
        assert_eq!(cfg.api_key_env.as_deref(), Some("LITELLM_API_KEY"));
        assert_eq!(cfg.options["timeout"], serde_json::json!(30));
    }

    #[test]
    fn registry_only_provider_has_no_connection() {
        let cfg: ProviderConfig = serde_yaml::from_str("limit: 200000").unwrap();
        assert!(!cfg.has_connection());
        assert_eq!(cfg.api(), ProviderApi::Openai);
    }

    #[test]
    fn unknown_provider_field_rejected() {
        let err = serde_yaml::from_str::<ProviderConfig>("api_key_evn: OOPS").unwrap_err();
        assert!(err.to_string().contains("api_key_evn"));
    }
}
