use anyhow::{bail, Result};
use serde::Deserialize;
use std::fs;
use std::os::unix::fs::PermissionsExt;

use crate::provision::profile::ProfileSnapshot;
use crate::provision::types::{GatewayEndpoint, OrgPolicy, ProvisionConfig};

/// Environment variables naming the provision server and bearer token.
pub const URL_ENV: &str = "WORKMUX_PROVISION_URL";
pub const TOKEN_ENV: &str = "WORKMUX_PROVISION_TOKEN";

/// Read `name`, treating an empty value as unset.
fn env_var(name: &str) -> Option<String> {
    let value = std::env::var(name).ok()?;
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

/// Resolve the provision server URL. Resolution order:
/// 1. `WORKMUX_PROVISION_URL` env var
/// 2. `provision.server_url` in config
///
/// The env var lets a dev container / CI runner point workmux at its server
/// without templating a config file -- the same story as
/// `WORKMUX_PROVISION_TOKEN` for the token. Returns `None` when neither source
/// is set.
pub fn resolve_server_url(config: Option<&ProvisionConfig>) -> Option<String> {
    if let Some(url) = env_var(URL_ENV) {
        return Some(url);
    }
    config
        .and_then(|c| c.server_url.as_deref())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Resolve the bearer token. Resolution order:
/// 1. `WORKMUX_PROVISION_TOKEN` env var
/// 2. `provision.token` in config
/// 3. `provision.token_path` file (must be mode 600)
///
/// Every source accepts `${env:VAR}` / `${file:/path}` placeholders, expanded
/// here rather than during config resolution so the token never appears in a
/// resolved config.
pub fn resolve_token(config: &ProvisionConfig) -> Result<String> {
    if let Some(token) = env_var(TOKEN_ENV) {
        return crate::config::secrets::expand(&token, TOKEN_ENV);
    }

    // An inline `token:` is the natural place for a `${env:...}` reference, so
    // an org can name its own variable rather than adopt workmux's.
    if let Some(ref token) = config.token {
        let token = crate::config::secrets::expand(token, "provision.token")?;
        if !token.trim().is_empty() {
            return Ok(token.trim().to_string());
        }
    }

    if let Some(ref path_str) = config.token_path {
        // A `token_path` may itself be a placeholder, letting a config point at
        // a secret without naming a fixed path.
        let path_str = if crate::config::secrets::has_placeholder(path_str) {
            crate::config::secrets::expand(path_str, "provision.token_path")?
        } else {
            path_str.clone()
        };

        let path = std::path::Path::new(&path_str);
        if !path.exists() {
            bail!("provision token file not found: {}", path_str);
        }
        let meta = fs::metadata(path)?;
        let mode = meta.permissions().mode();
        if mode & 0o177 != 0 {
            bail!(
                "provision token file {} is world/group readable (mode {:o}); chmod 600 it",
                path_str,
                mode & 0o777
            );
        }
        let token = fs::read_to_string(path)?.trim().to_string();
        if token.is_empty() {
            bail!("provision token file {} is empty", path_str);
        }
        return Ok(token);
    }

    bail!(
        "no provision token found -- set the {TOKEN_ENV} env var, or provision.token / \
         provision.token_path in config"
    )
}

#[derive(Debug, Deserialize)]
pub struct PushProfileResponse {
    pub accepted: bool,
    pub reason: Option<String>,
    pub profile_id: Option<String>,
    pub policy_needs_update: bool,
}

/// POST /api/provision/profile — push snapshot to server.
/// Returns whether the server says we need to re-fetch the policy.
pub fn push_profile(
    server_url: &str,
    token: &str,
    snapshot: &ProfileSnapshot,
) -> Result<PushProfileResponse> {
    let url = format!("{}/api/provision/profile", server_url.trim_end_matches('/'));
    let body = serde_json::json!({ "profile": snapshot });
    let resp = ureq::post(&url)
        .set("Authorization", &format!("Bearer {}", token))
        .set("Content-Type", "application/json")
        .send_json(&body);
    match resp {
        Ok(r) => Ok(r.into_json::<PushProfileResponse>()?),
        Err(ureq::Error::Status(code, r)) => {
            bail!(
                "provision server returned {}: {}",
                code,
                r.into_string().unwrap_or_default()
            )
        }
        Err(e) => bail!("provision request failed: {}", e),
    }
}

#[derive(Debug, Deserialize)]
pub struct FetchPolicyResponse {
    pub policy: OrgPolicy,
    pub version: String,
    pub checksum: String,
    pub ttl_seconds: u64,
    /// Governed gateway endpoint (response sibling, not part of the policy) used
    /// to configure local coding agents. Absent on servers that don't set it.
    #[serde(default)]
    pub gateway: Option<GatewayEndpoint>,
}

/// GET /api/provision/policy — fetch the org policy for this tenant.
pub fn fetch_policy(server_url: &str, token: &str) -> Result<FetchPolicyResponse> {
    let url = format!("{}/api/provision/policy", server_url.trim_end_matches('/'));
    let resp = ureq::get(&url)
        .set("Authorization", &format!("Bearer {}", token))
        .call();
    match resp {
        Ok(r) => Ok(r.into_json::<FetchPolicyResponse>()?),
        Err(ureq::Error::Status(404, _)) => {
            bail!("no policy configured on the provision server — ask your org admin to set one")
        }
        Err(ureq::Error::Status(code, r)) => {
            bail!(
                "provision server returned {}: {}",
                code,
                r.into_string().unwrap_or_default()
            )
        }
        Err(e) => bail!("provision request failed: {}", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Env vars are process-global; these tests serialize behind one lock.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct EnvGuard {
        _lock: std::sync::MutexGuard<'static, ()>,
        saved: Vec<(String, Option<String>)>,
    }

    impl EnvGuard {
        fn new(vars: &[(&str, Option<&str>)]) -> Self {
            let lock = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let mut saved = Vec::new();
            for (name, value) in vars {
                saved.push(((*name).to_string(), std::env::var(name).ok()));
                unsafe {
                    match value {
                        Some(v) => std::env::set_var(name, v),
                        None => std::env::remove_var(name),
                    }
                }
            }
            Self { _lock: lock, saved }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (name, value) in &self.saved {
                unsafe {
                    match value {
                        Some(v) => std::env::set_var(name, v),
                        None => std::env::remove_var(name),
                    }
                }
            }
        }
    }

    fn clear_all() -> Vec<(&'static str, Option<&'static str>)> {
        vec![
            (URL_ENV, None),
            (TOKEN_ENV, None),
        ]
    }

    #[test]
    fn neutral_url_env_is_used() {
        let mut vars = clear_all();
        vars[0] = (URL_ENV, Some("https://p.example.com"));
        let _g = EnvGuard::new(&vars);
        assert_eq!(
            resolve_server_url(None).as_deref(),
            Some("https://p.example.com")
        );
    }



    #[test]
    fn config_url_used_when_no_env_is_set() {
        let _g = EnvGuard::new(&clear_all());
        let cfg = ProvisionConfig {
            server_url: Some("https://cfg.example.com".into()),
            ..Default::default()
        };
        assert_eq!(
            resolve_server_url(Some(&cfg)).as_deref(),
            Some("https://cfg.example.com")
        );
    }

    #[test]
    fn no_url_anywhere_is_none() {
        let _g = EnvGuard::new(&clear_all());
        assert!(resolve_server_url(None).is_none());
    }

    #[test]
    fn neutral_token_env_is_used() {
        let mut vars = clear_all();
        vars[1] = (TOKEN_ENV, Some("tok-new"));
        let _g = EnvGuard::new(&vars);
        assert_eq!(
            resolve_token(&ProvisionConfig::default()).unwrap(),
            "tok-new"
        );
    }



    /// The point of the inline `token:` field: name your own env var rather
    /// than adopt workmux's.
    #[test]
    fn config_token_expands_an_env_placeholder() {
        let mut vars = clear_all();
        vars.push(("MY_ORG_TOKEN", Some("org-secret")));
        let _g = EnvGuard::new(&vars);
        let cfg = ProvisionConfig {
            token: Some("${env:MY_ORG_TOKEN}".into()),
            ..Default::default()
        };
        assert_eq!(resolve_token(&cfg).unwrap(), "org-secret");
    }

    #[test]
    fn token_env_beats_config_token() {
        let mut vars = clear_all();
        vars[1] = (TOKEN_ENV, Some("from-env"));
        let _g = EnvGuard::new(&vars);
        let cfg = ProvisionConfig {
            token: Some("from-config".into()),
            ..Default::default()
        };
        assert_eq!(resolve_token(&cfg).unwrap(), "from-env");
    }

    #[test]
    fn missing_token_names_the_neutral_env_var() {
        let _g = EnvGuard::new(&clear_all());
        let err = resolve_token(&ProvisionConfig::default())
            .unwrap_err()
            .to_string();
        assert!(err.contains(TOKEN_ENV), "{err}");
    }
}
