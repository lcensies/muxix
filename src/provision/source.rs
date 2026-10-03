//! Where a provisioning policy comes from.
//!
//! Muxix does not assume any particular organization's server. A policy can
//! arrive over HTTPS, be dropped on disk by a configuration-management system,
//! or be produced by a helper the organization already trusts to authenticate.
//! Each shape is a [`PolicySource`]; everything downstream — caching, layering,
//! auditing — is written once against the trait.
//!
//! The `exec` backend is what makes this genuinely portable: no fixed HTTP
//! contract survives contact with every organization's auth, so the escape
//! hatch is "run our tool, read its stdout".

use anyhow::{Context as _, Result, bail};
use std::io::Read as _;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::provision::client::FetchPolicyResponse;
use crate::provision::profile::ProfileSnapshot;
use crate::provision::types::ProvisionConfig;

/// Highest policy `schema_version` this build understands.
pub const SUPPORTED_SCHEMA_VERSION: u32 = 1;

/// Default timeout for the `exec` backend, in seconds.
const DEFAULT_EXEC_TIMEOUT_SECS: u64 = 30;

/// How the policy is obtained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// Fetch over HTTPS from a provision server. The default.
    Http,
    /// Read a policy document from a local path. For air-gapped machines.
    File,
    /// Run an organization-supplied command and parse its stdout.
    Exec,
}

impl Backend {
    pub fn as_str(self) -> &'static str {
        match self {
            Backend::Http => "http",
            Backend::File => "file",
            Backend::Exec => "exec",
        }
    }

    /// Parse a configured backend name.
    pub fn parse(name: &str) -> Result<Backend> {
        match name.trim().to_ascii_lowercase().as_str() {
            "http" | "https" => Ok(Backend::Http),
            "file" => Ok(Backend::File),
            "exec" => Ok(Backend::Exec),
            other => bail!(
                "unsupported provision backend `{other}`; supported backends: http, file, exec"
            ),
        }
    }

    /// The backend a config selects, defaulting to `http`.
    pub fn from_config(config: &ProvisionConfig) -> Result<Backend> {
        match config.backend.as_deref() {
            Some(name) => Backend::parse(name),
            None => Ok(Backend::Http),
        }
    }
}

/// Obtain a policy, and optionally report a machine profile.
pub trait PolicySource {
    /// Fetch the current policy.
    fn fetch_policy(&self) -> Result<FetchPolicyResponse>;

    /// Report this machine's profile, when the backend supports it.
    ///
    /// A backend with nowhere to send a profile returns `Ok(None)` rather than
    /// erroring: pushing a snapshot is an optional courtesy to the server, not
    /// a precondition for being governed.
    fn push_profile(
        &self,
        _snapshot: &ProfileSnapshot,
    ) -> Result<Option<crate::provision::client::PushProfileResponse>> {
        Ok(None)
    }

    /// Human-readable description of where the policy came from, for logs and
    /// the audit trail.
    fn describe(&self) -> String;
}

/// Reject a policy this build cannot interpret, and tolerate unknown fields
/// within a version it can.
///
/// Unknown fields are already ignored by serde; the version gate is what stops
/// a newer server's semantics being silently half-applied.
pub fn check_schema_version(resp: &FetchPolicyResponse) -> Result<()> {
    let got = resp.policy.schema_version;
    // A server that omits the field entirely predates versioning; treat it as
    // version 1 rather than failing every legacy deployment.
    let got = if got == 0 { 1 } else { got };
    if got > SUPPORTED_SCHEMA_VERSION {
        bail!(
            "policy schema version {got} is newer than this muxix supports \
             (max {SUPPORTED_SCHEMA_VERSION}); upgrade muxix to use this policy"
        );
    }
    Ok(())
}

// ── http ────────────────────────────────────────────────────────────────

/// Fetch over HTTPS from a provision server.
///
/// Endpoint paths are configurable so an organization's server need not adopt
/// any particular route layout.
pub struct HttpSource {
    pub server_url: String,
    pub token: String,
    pub policy_path: String,
    pub profile_path: Option<String>,
}

/// Default request paths, used when the config names none.
pub const DEFAULT_POLICY_PATH: &str = "/api/provision/policy";
pub const DEFAULT_PROFILE_PATH: &str = "/api/provision/profile";

impl HttpSource {
    pub fn new(server_url: String, token: String, config: &ProvisionConfig) -> Self {
        let endpoints = config.endpoints.clone().unwrap_or_default();
        Self {
            server_url,
            token,
            policy_path: endpoints
                .policy
                .clone()
                .unwrap_or_else(|| DEFAULT_POLICY_PATH.to_string()),
            // An explicit `profile: null` disables the snapshot push; an absent
            // key keeps the default.
            profile_path: if endpoints.profile_set {
                endpoints.profile.clone()
            } else {
                Some(DEFAULT_PROFILE_PATH.to_string())
            },
        }
    }

    fn url(&self, path: &str) -> String {
        format!(
            "{}/{}",
            self.server_url.trim_end_matches('/'),
            path.trim_start_matches('/')
        )
    }
}

impl PolicySource for HttpSource {
    fn fetch_policy(&self) -> Result<FetchPolicyResponse> {
        let url = self.url(&self.policy_path);
        match ureq::get(&url)
            .set("Authorization", &format!("Bearer {}", self.token))
            .call()
        {
            Ok(r) => Ok(r.into_json::<FetchPolicyResponse>()?),
            Err(ureq::Error::Status(404, _)) => bail!(
                "no policy configured at {url} — ask your org admin to set one"
            ),
            Err(ureq::Error::Status(code, r)) => bail!(
                "provision server returned {}: {}",
                code,
                r.into_string().unwrap_or_default()
            ),
            Err(e) => bail!("provision request failed: {}", e),
        }
    }

    fn push_profile(
        &self,
        snapshot: &ProfileSnapshot,
    ) -> Result<Option<crate::provision::client::PushProfileResponse>> {
        let Some(ref path) = self.profile_path else {
            return Ok(None);
        };
        let url = self.url(path);
        let body = serde_json::json!({ "profile": snapshot });
        match ureq::post(&url)
            .set("Authorization", &format!("Bearer {}", self.token))
            .set("Content-Type", "application/json")
            .send_json(&body)
        {
            Ok(r) => Ok(Some(r.into_json()?)),
            Err(ureq::Error::Status(code, r)) => bail!(
                "provision server returned {}: {}",
                code,
                r.into_string().unwrap_or_default()
            ),
            Err(e) => bail!("provision request failed: {}", e),
        }
    }

    fn describe(&self) -> String {
        self.url(&self.policy_path)
    }
}

// ── file ────────────────────────────────────────────────────────────────

/// Read a policy document from a local path. Makes no network request, so an
/// air-gapped machine can still be governed by whatever put the file there.
pub struct FileSource {
    pub path: std::path::PathBuf,
}

impl PolicySource for FileSource {
    fn fetch_policy(&self) -> Result<FetchPolicyResponse> {
        if !self.path.exists() {
            bail!("provision policy file not found: {}", self.path.display());
        }
        let body = std::fs::read_to_string(&self.path)
            .with_context(|| format!("failed to read {}", self.path.display()))?;
        parse_policy_document(&body)
            .with_context(|| format!("in policy file {}", self.path.display()))
    }

    fn describe(&self) -> String {
        self.path.display().to_string()
    }
}

// ── exec ────────────────────────────────────────────────────────────────

/// Run an organization-supplied command and read the policy from its stdout.
pub struct ExecSource {
    pub command: String,
    pub args: Vec<String>,
    pub timeout_secs: u64,
}

impl ExecSource {
    pub fn new(config: &ProvisionConfig) -> Result<Self> {
        let raw = config
            .command
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .context("provision.backend is `exec` but provision.command is not set")?;

        // Split on whitespace so `command: "op read op://vault/policy"` works
        // without a separate args key. Quoting is deliberately not supported:
        // anything that needs it should be a script.
        let mut parts = raw.split_whitespace().map(str::to_owned);
        let command = parts.next().context("provision.command is empty")?;
        Ok(Self {
            command,
            args: parts.collect(),
            timeout_secs: config.timeout_secs.unwrap_or(DEFAULT_EXEC_TIMEOUT_SECS),
        })
    }
}

impl PolicySource for ExecSource {
    fn fetch_policy(&self) -> Result<FetchPolicyResponse> {
        let mut child = Command::new(&self.command)
            .args(&self.args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("failed to run provision command `{}`", self.command))?;

        // Poll rather than block: a helper that hangs must not wedge every
        // muxix invocation behind it.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(self.timeout_secs);
        let status = loop {
            match child.try_wait()? {
                Some(status) => break status,
                None if std::time::Instant::now() >= deadline => {
                    let _ = child.kill();
                    let _ = child.wait();
                    bail!(
                        "provision command `{}` did not finish within {}s",
                        self.command,
                        self.timeout_secs
                    );
                }
                None => std::thread::sleep(std::time::Duration::from_millis(50)),
            }
        };

        let mut stdout = String::new();
        if let Some(mut out) = child.stdout.take() {
            let _ = out.read_to_string(&mut stdout);
        }
        let mut stderr = String::new();
        if let Some(mut err) = child.stderr.take() {
            let _ = err.read_to_string(&mut stderr);
        }

        if !status.success() {
            bail!(
                "provision command `{}` failed ({}): {}",
                self.command,
                status,
                stderr.trim()
            );
        }

        parse_policy_document(&stdout)
            .with_context(|| format!("in output of provision command `{}`", self.command))
    }

    fn describe(&self) -> String {
        format!("exec:{}", self.command)
    }
}

// ── shared ──────────────────────────────────────────────────────────────

/// Parse a policy document.
///
/// Accepts either the full fetch response (`{policy, version, ...}`) or a bare
/// policy object, so a hand-written file does not have to mimic an HTTP
/// envelope it never travelled in.
pub fn parse_policy_document(body: &str) -> Result<FetchPolicyResponse> {
    let body = body.trim();
    if body.is_empty() {
        bail!("policy document is empty");
    }

    // YAML is a superset of JSON, so one parser reads both forms.
    let value: serde_yaml::Value =
        serde_yaml::from_str(body).context("policy document is not valid JSON or YAML")?;

    if value.get("policy").is_some() {
        return serde_yaml::from_value(value).context("policy document has an unexpected shape");
    }

    let policy: crate::provision::types::OrgPolicy =
        serde_yaml::from_value(value).context("policy document has an unexpected shape")?;
    Ok(FetchPolicyResponse {
        version: policy.policy_version.clone(),
        checksum: policy.checksum.clone(),
        ttl_seconds: policy.ttl_seconds,
        gateway: None,
        policy,
    })
}

/// Build the source a config selects.
pub fn source_for(config: &ProvisionConfig) -> Result<Box<dyn PolicySource>> {
    match Backend::from_config(config)? {
        Backend::Http => {
            let server_url = crate::provision::client::resolve_server_url(Some(config))
                .context(
                    "no provision server configured -- set the MUXIX_PROVISION_URL env var, \
                     or provision.server_url in your global config",
                )?;
            let token = crate::provision::client::resolve_token(config)?;
            Ok(Box::new(HttpSource::new(server_url, token, config)))
        }
        Backend::File => {
            let path = config
                .path
                .as_deref()
                .context("provision.backend is `file` but provision.path is not set")?;
            Ok(Box::new(FileSource {
                path: crate::util::expand_tilde(path),
            }))
        }
        Backend::Exec => Ok(Box::new(ExecSource::new(config)?)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(yaml: &str) -> ProvisionConfig {
        serde_yaml::from_str(yaml).unwrap()
    }

    // --- backend selection -------------------------------------------------

    #[test]
    fn http_is_the_default_backend() {
        assert_eq!(Backend::from_config(&cfg("{}")).unwrap(), Backend::Http);
    }

    #[test]
    fn backends_parse_by_name() {
        assert_eq!(Backend::parse("http").unwrap(), Backend::Http);
        assert_eq!(Backend::parse("File").unwrap(), Backend::File);
        assert_eq!(Backend::parse(" exec ").unwrap(), Backend::Exec);
    }

    #[test]
    fn unknown_backend_lists_the_supported_ones() {
        let err = Backend::parse("carrier-pigeon").unwrap_err().to_string();
        assert!(err.contains("carrier-pigeon"), "{err}");
        assert!(err.contains("http, file, exec"), "{err}");
    }

    // --- endpoints ---------------------------------------------------------

    #[test]
    fn default_endpoint_paths_are_used_when_unset() {
        let s = HttpSource::new("https://x.example.com".into(), "t".into(), &cfg("{}"));
        assert_eq!(s.policy_path, DEFAULT_POLICY_PATH);
        assert_eq!(s.profile_path.as_deref(), Some(DEFAULT_PROFILE_PATH));
    }

    #[test]
    fn custom_endpoint_paths_are_honored() {
        let s = HttpSource::new(
            "https://x.example.com".into(),
            "t".into(),
            &cfg("endpoints:\n  policy: /v2/harness-policy\n"),
        );
        assert_eq!(s.policy_path, "/v2/harness-policy");
        assert_eq!(
            s.describe(),
            "https://x.example.com/v2/harness-policy",
            "url joining must not double or drop the slash"
        );
    }

    #[test]
    fn url_join_tolerates_trailing_and_leading_slashes() {
        let s = HttpSource::new(
            "https://x.example.com/".into(),
            "t".into(),
            &cfg("endpoints:\n  policy: v2/policy\n"),
        );
        assert_eq!(s.describe(), "https://x.example.com/v2/policy");
    }

    #[test]
    fn explicit_null_profile_endpoint_disables_the_push() {
        let s = HttpSource::new(
            "https://x.example.com".into(),
            "t".into(),
            &cfg("endpoints:\n  policy: /p\n  profile: null\n"),
        );
        assert!(
            s.profile_path.is_none(),
            "an explicit null must disable the snapshot push"
        );
    }

    // --- schema version ----------------------------------------------------

    fn resp_with_version(v: u32) -> FetchPolicyResponse {
        let mut policy = crate::provision::types::OrgPolicy::default();
        policy.schema_version = v;
        FetchPolicyResponse {
            policy,
            version: "1".into(),
            checksum: String::new(),
            ttl_seconds: 60,
            gateway: None,
        }
    }

    #[test]
    fn supported_schema_version_is_accepted() {
        assert!(check_schema_version(&resp_with_version(SUPPORTED_SCHEMA_VERSION)).is_ok());
    }

    #[test]
    fn missing_schema_version_is_treated_as_v1() {
        assert!(check_schema_version(&resp_with_version(0)).is_ok());
    }

    #[test]
    fn newer_schema_version_is_rejected_naming_both() {
        let err = check_schema_version(&resp_with_version(SUPPORTED_SCHEMA_VERSION + 1))
            .unwrap_err()
            .to_string();
        assert!(err.contains(&(SUPPORTED_SCHEMA_VERSION + 1).to_string()), "{err}");
        assert!(err.contains(&SUPPORTED_SCHEMA_VERSION.to_string()), "{err}");
    }

    // --- document parsing --------------------------------------------------

    #[test]
    fn parses_a_full_fetch_envelope() {
        let doc = r#"{"policy":{"schema_version":1,"policy_version":"v9"},
                      "version":"v9","checksum":"abc","ttl_seconds":120}"#;
        let got = parse_policy_document(doc).unwrap();
        assert_eq!(got.version, "v9");
        assert_eq!(got.ttl_seconds, 120);
    }

    /// A hand-written file should not have to mimic an HTTP envelope.
    #[test]
    fn parses_a_bare_policy_object() {
        let doc = r#"{"schema_version":1,"policy_version":"v3","ttl_seconds":99}"#;
        let got = parse_policy_document(doc).unwrap();
        assert_eq!(got.policy.policy_version, "v3");
        assert_eq!(got.version, "v3");
        assert_eq!(got.ttl_seconds, 99);
    }

    #[test]
    fn parses_yaml_as_well_as_json() {
        let got = parse_policy_document("schema_version: 1\npolicy_version: yamlv\n").unwrap();
        assert_eq!(got.policy.policy_version, "yamlv");
    }

    #[test]
    fn unknown_fields_are_tolerated() {
        let doc = r#"{"schema_version":1,"policy_version":"v1","a_field_from_the_future":42}"#;
        assert!(parse_policy_document(doc).is_ok());
    }

    #[test]
    fn empty_document_is_rejected() {
        assert!(parse_policy_document("   ").is_err());
    }

    #[test]
    fn malformed_document_is_rejected() {
        assert!(parse_policy_document("{ this is not: valid: yaml: at all").is_err());
    }

    // --- file backend ------------------------------------------------------

    fn tmpfile(name: &str, body: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "muxix-provision-src-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn file_backend_reads_a_policy() {
        let path = tmpfile(
            "policy.json",
            r#"{"schema_version":1,"policy_version":"from-file"}"#,
        );
        let src = FileSource { path };
        assert_eq!(src.fetch_policy().unwrap().policy.policy_version, "from-file");
    }

    #[test]
    fn file_backend_reports_a_missing_file() {
        let src = FileSource {
            path: "/nonexistent/muxix/policy.json".into(),
        };
        let err = src.fetch_policy().unwrap_err().to_string();
        assert!(err.contains("not found"), "{err}");
    }

    #[test]
    fn file_backend_pushes_no_profile() {
        let src = FileSource {
            path: "/whatever".into(),
        };
        let snap = ProfileSnapshot::default();
        assert!(src.push_profile(&snap).unwrap().is_none());
    }

    #[test]
    fn file_backend_requires_a_path() {
        let err = match source_for(&cfg("backend: file\n")) {
            Ok(_) => panic!("a file backend without a path must not build"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("provision.path"), "{err}");
    }

    // --- exec backend ------------------------------------------------------

    #[test]
    fn exec_backend_reads_stdout() {
        let src = ExecSource {
            command: "printf".into(),
            args: vec![r#"{"schema_version":1,"policy_version":"from-exec"}"#.into()],
            timeout_secs: 10,
        };
        assert_eq!(src.fetch_policy().unwrap().policy.policy_version, "from-exec");
    }

    #[test]
    fn exec_backend_surfaces_stderr_on_failure() {
        let src = ExecSource {
            command: "sh".into(),
            args: vec!["-c".into(), "echo boom >&2; exit 3".into()],
            timeout_secs: 10,
        };
        let err = src.fetch_policy().unwrap_err().to_string();
        assert!(err.contains("boom"), "stderr must be surfaced: {err}");
    }

    #[test]
    fn exec_backend_times_out() {
        let src = ExecSource {
            command: "sleep".into(),
            args: vec!["30".into()],
            timeout_secs: 1,
        };
        let err = src.fetch_policy().unwrap_err().to_string();
        assert!(err.contains("did not finish within"), "{err}");
    }

    #[test]
    fn exec_backend_reports_a_missing_command() {
        let src = ExecSource {
            command: "muxix-no-such-helper".into(),
            args: vec![],
            timeout_secs: 5,
        };
        assert!(src.fetch_policy().is_err());
    }

    #[test]
    fn exec_backend_requires_a_command() {
        let err = match source_for(&cfg("backend: exec\n")) {
            Ok(_) => panic!("an exec backend without a command must not build"),
            Err(e) => e.to_string(),
        };
        assert!(err.contains("provision.command"), "{err}");
    }

    #[test]
    fn exec_command_splits_into_args() {
        let src = ExecSource::new(&cfg("backend: exec\ncommand: printf hello\n")).unwrap();
        assert_eq!(src.command, "printf");
        assert_eq!(src.args, vec!["hello".to_string()]);
    }
}
