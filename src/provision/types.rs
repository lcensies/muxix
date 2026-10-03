use serde::{Deserialize, Serialize};

/// Provision server configuration. Global-only (project config cannot override).
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct ProvisionConfig {
    /// Provision server URL (e.g. "https://provision.corp.example.com").
    /// Env var: MUXIX_PROVISION_URL (takes precedence over this field).
    pub server_url: Option<String>,

    /// Bearer token, or a `${env:VAR}` / `${file:/path}` reference to one.
    ///
    /// Placeholders are expanded at use, never during config resolution, so a
    /// resolved config never holds the secret. Prefer a placeholder over a
    /// literal -- this field lives in a config file.
    /// Env var: MUXIX_PROVISION_TOKEN (takes precedence over this field).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,

    /// Path to a file containing the bearer token (must be mode 600).
    /// Env var: MUXIX_PROVISION_TOKEN (takes precedence over this field).
    pub token_path: Option<String>,

    /// Where the policy comes from: `http` (default), `file`, or `exec`.
    ///
    /// Pluggable so an organization can be governed without standing up a
    /// muxix-shaped HTTP server: `file` reads a policy dropped on disk,
    /// `exec` runs a helper that already knows how to authenticate.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,

    /// Policy document path, for `backend: file`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,

    /// Command producing a policy document on stdout, for `backend: exec`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,

    /// How long the `exec` backend's command may run. Default: 30s.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,

    /// Request paths for `backend: http`, so a server need not adopt any
    /// particular route layout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoints: Option<ProvisionEndpoints>,

    /// Run provision sync automatically during `muxix setup`. Default: false.
    pub sync_on_setup: Option<bool>,

    /// How long fetched policy stays fresh (seconds). Default: 86400 (24h).
    pub policy_ttl_secs: Option<u64>,

    /// How long to keep using an expired policy before hard-failing (seconds).
    /// Default: 259200 (72h).
    pub grace_period_secs: Option<u64>,

    /// Skip TLS verification. Never use in production.
    pub insecure_skip_tls: Option<bool>,
}

/// Configurable request paths for the HTTP provisioning backend.
///
/// `profile` distinguishes "not configured" (use the default path) from an
/// explicit `profile: null` (do not push a machine profile at all). A plain
/// `Option<String>` cannot express that difference, so the deserializer records
/// presence separately.
#[derive(Debug, Serialize, Default, Clone)]
pub struct ProvisionEndpoints {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub policy: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,

    /// Whether `profile` appeared in the config at all. Derived, never written
    /// by a user.
    #[serde(skip)]
    pub profile_set: bool,
}

impl<'de> Deserialize<'de> for ProvisionEndpoints {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        // Deserialized as a raw mapping rather than a struct: serde collapses
        // an explicit `profile: null` to `None`, indistinguishable from the key
        // being absent, and that distinction is this type's whole purpose.
        let map = serde_yaml::Mapping::deserialize(d)?;
        let get = |k: &str| map.get(serde_yaml::Value::String(k.to_string()));

        let as_str = |v: Option<&serde_yaml::Value>| {
            v.and_then(|v| v.as_str()).map(str::to_owned)
        };

        Ok(ProvisionEndpoints {
            policy: as_str(get("policy")),
            profile_set: get("profile").is_some(),
            profile: as_str(get("profile")),
        })
    }
}

/// One allowed egress endpoint (model API provider).
/// `api_key_env` is the env var *name* to look up — never the key itself.
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct ProviderConfig {
    pub name: String,
    pub base_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
    #[serde(default)]
    pub is_default: bool,
}

/// Locked field overrides — values set here cannot be changed by user config.
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct PolicyLockedFields {
    /// Locks the entire proxy_chain config to this value.
    pub proxy_chain: Option<serde_json::Value>,
    /// Locks sandbox.network to this value.
    pub sandbox_network: Option<serde_json::Value>,
}

/// Org policy fetched from the provision server and cached locally.
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct OrgPolicy {
    pub schema_version: u32,
    pub policy_version: String,
    /// Content checksum. Sent alongside the policy (not inside it) and stamped
    /// here after fetch, so it defaults when deserializing the server payload.
    #[serde(default)]
    pub checksum: String,
    /// Unix timestamp (seconds) when the policy was issued. Stamped client-side.
    #[serde(default)]
    pub issued_at: u64,
    /// Unix timestamp (seconds) when the policy expires. Stamped client-side.
    #[serde(default)]
    pub expires_at: u64,
    #[serde(default)]
    pub locked: PolicyLockedFields,
    /// Default values to apply when user config is absent.
    #[serde(default)]
    pub defaults: serde_json::Value,
    /// MCP command patterns that must not appear in any MCP server command.
    #[serde(default)]
    pub forbidden_mcp_commands: Vec<String>,
    /// When non-empty, only these agent kinds are permitted.
    #[serde(default)]
    pub allowed_agent_kinds: Vec<String>,
    /// URL to fetch the team profile YAML from.
    pub team_profile_url: Option<String>,
    /// "warn" (default) or "error" — severity of policy violations.
    #[serde(default = "default_violation_severity")]
    pub violation_severity: String,
    /// How long until this policy is considered stale (seconds).
    #[serde(default = "default_ttl")]
    pub ttl_seconds: u64,

    // Egress control
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_providers: Vec<ProviderConfig>,
    #[serde(default)]
    pub deny_external_providers: bool,

    // Skill control
    #[serde(default)]
    pub deny_external_skills: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_skills: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_mcp_servers: Vec<String>,
}

/// The single governed endpoint provisioned agents route through. Mirrors
/// core's contracts.GatewayEndpoint. Returned as a sibling of the policy in the
/// fetch-policy response (not inside OrgPolicy, so it never affects the policy
/// checksum). `base_url` is the gateway root (no /v1); when empty, the client
/// falls back to the URL it provisioned against. `api_key_env` names the env var
/// holding the user's gateway token.
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct GatewayEndpoint {
    #[serde(default)]
    pub base_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<String>,
    /// Provider id written into the agent's own config. Supplied by the server
    /// so each organization names its own gateway; muxix falls back to a
    /// neutral default rather than carrying any vendor's branding.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_id: Option<String>,
    /// Human-readable provider name shown in the agent's UI.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_name: Option<String>,
}

fn default_violation_severity() -> String {
    "warn".to_string()
}

fn default_ttl() -> u64 {
    86400
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyViolation {
    pub field: String,
    pub message: String,
    pub severity: ViolationSeverity,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ViolationSeverity {
    Warn,
    Error,
}

pub struct PolicyMergeResult {
    pub config: crate::config::Config,
    pub violations: Vec<PolicyViolation>,
}

#[derive(Debug, Serialize)]
pub struct AuditEvent {
    pub timestamp: String,
    pub event_type: String,
    pub server_url: Option<String>,
    pub policy_version: Option<String>,
    pub policy_checksum: Option<String>,
    pub violations: usize,
    /// Which backend produced the policy: `http`, `file`, or `exec`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backend: Option<String>,
    /// How many of the user's own settings a policy lock overrode.
    #[serde(default)]
    pub lock_overrides: usize,
    pub result: String,
}
