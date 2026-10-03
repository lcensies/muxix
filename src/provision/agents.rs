//! Configure local coding agents to route through the governed gateway.
//!
//! `muxix provision sync` calls [`apply_gateway`] with the org policy's
//! [`GatewayEndpoint`]. We write native provider config for each supported
//! agent so all of their LLM traffic goes through the organization's gateway --
//! the developer never wires `OPENAI_BASE_URL` / `ANTHROPIC_BASE_URL` by hand.
//!
//! Configs are *merged*: we own a single named provider block / a couple of env
//! keys and leave the rest of the user's config untouched. Secrets are never
//! written to disk — we reference the token by env var name (opencode's
//! `{env:VAR}` interpolation; Claude's `apiKeyHelper`).

use anyhow::{Context, Result};
use serde_json::{json, Map, Value};
use std::fs;
use std::path::Path;

use crate::provision::types::GatewayEndpoint;

/// Provider id we own in opencode.json when the server names none.
///
/// Vendor-neutral by design: the organization's own identity comes from the
/// gateway response, so muxix carries no one provider's branding.
const DEFAULT_PROVIDER_ID: &str = "provision-gateway";
/// Display name used when the server names none.
const DEFAULT_PROVIDER_NAME: &str = "Provision Gateway";
/// Default env var holding the user's gateway token when the policy omits one.
const DEFAULT_KEY_ENV: &str = "MUXIX_PROVISION_TOKEN";

/// Resolve the gateway root URL: the policy value, else the URL we provisioned
/// against (`fallback_url`). Trailing slashes are trimmed.
fn gateway_base(gw: &GatewayEndpoint, fallback_url: &str) -> String {
    let b = gw.base_url.trim().trim_end_matches('/');
    if b.is_empty() {
        fallback_url.trim().trim_end_matches('/').to_string()
    } else {
        b.to_string()
    }
}

/// Write agent provider configs for `gateway`. `fallback_url` is the URL we
/// provisioned against, used when the gateway advertises no `base_url`. Returns
/// a short summary line per agent configured (empty when nothing was written).
pub fn apply_gateway(gw: &GatewayEndpoint, fallback_url: &str) -> Result<Vec<String>> {
    let base = gateway_base(gw, fallback_url);
    if base.is_empty() {
        return Ok(vec![]);
    }
    let key_env = gw
        .api_key_env
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_KEY_ENV.to_string());

    let provider_id = gw
        .provider_id
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_PROVIDER_ID.to_string());
    let provider_name = gw
        .provider_name
        .clone()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_PROVIDER_NAME.to_string());

    let mut written = Vec::new();
    if let Some(msg) = write_opencode(&base, &key_env, &gw.models, &provider_id, &provider_name)? {
        written.push(msg);
    }
    if let Some(msg) = write_claude(&base, &key_env)? {
        written.push(msg);
    }
    Ok(written)
}

/// Load a JSON object from `path`, or start a fresh one. A corrupt/non-object
/// file is replaced rather than erroring — provisioning must not wedge on a
/// hand-edited config.
fn load_object(path: &Path, seed: Value) -> Value {
    if path.exists() {
        match fs::read_to_string(path) {
            Ok(s) => serde_json::from_str::<Value>(&s)
                .ok()
                .filter(Value::is_object)
                .unwrap_or(seed),
            Err(_) => seed,
        }
    } else {
        seed
    }
}

fn write_pretty(path: &Path, root: &Value) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create {}", parent.display()))?;
    }
    let mut s = serde_json::to_string_pretty(root)?;
    s.push('\n');
    fs::write(path, s).with_context(|| format!("write {}", path.display()))?;
    Ok(())
}

/// Write `~/.config/opencode/opencode.json` with a governed provider whose
/// `baseURL` is the gateway and whose models are the policy's allowed set.
/// Sets the provider as the default model so `opencode run` routes through it.
fn write_opencode(
    base: &str,
    key_env: &str,
    models: &[String],
    provider_id: &str,
    provider_name: &str,
) -> Result<Option<String>> {
    let Some(dir) = crate::agent::setup::opencode::opencode_config_dir() else {
        return Ok(None);
    };
    let path = dir.join("opencode.json");

    let mut root = load_object(
        &path,
        json!({ "$schema": "https://opencode.ai/config.json" }),
    );
    let obj = root.as_object_mut().expect("seed is an object");

    let models_obj: Map<String, Value> =
        models.iter().map(|m| (m.clone(), json!({}))).collect();

    let provider_block = json!({
        "npm": "@ai-sdk/openai-compatible",
        "name": provider_name,
        "options": {
            "baseURL": format!("{}/v1", base),
            "apiKey": format!("{{env:{}}}", key_env),
        },
        "models": models_obj,
    });

    obj.entry("provider")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .context("opencode.json: 'provider' is not an object")?
        .insert(provider_id.to_string(), provider_block);

    // Make the governed provider the default so agents route through it without
    // an explicit -m flag. Only set when we actually have a model to name and
    // the user hasn't pinned their own default.
    if !obj.contains_key("model") {
        if let Some(first) = models.first() {
            obj.insert("model".into(), json!(format!("{}/{}", provider_id, first)));
        }
    }

    write_pretty(&path, &root)?;
    Ok(Some(format!("opencode → {}", path.display())))
}

/// Write `~/.claude/settings.json`: point Claude Code at the gateway via
/// `env.ANTHROPIC_BASE_URL`, and supply the token through `apiKeyHelper` (which
/// reads the env var at runtime) so the secret never lands on disk.
fn write_claude(base: &str, key_env: &str) -> Result<Option<String>> {
    let Some(home) = home::home_dir() else {
        return Ok(None);
    };
    let path = home.join(".claude").join("settings.json");

    let mut root = load_object(&path, json!({}));
    let obj = root.as_object_mut().expect("seed is an object");

    obj.entry("env")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .context("settings.json: 'env' is not an object")?
        .insert("ANTHROPIC_BASE_URL".into(), json!(base));

    // apiKeyHelper runs in the shell Claude was launched from, where the token
    // env var is set — so we reference it by name instead of writing the value.
    obj.insert(
        "apiKeyHelper".into(),
        json!(format!("sh -c 'printf %s \"${{{}}}\"'", key_env)),
    );

    write_pretty(&path, &root)?;
    Ok(Some(format!("claude → {}", path.display())))
}
