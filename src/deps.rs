//! Dependencies declared next to harness entities (`requires:` on skill and
//! MCP entries): npm packages muxix installs into a user prefix, and
//! executables it only asserts are on PATH.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const DEFAULT_NPM_PREFIX: &str = "~/.local";

/// What an entity needs to work. Empty by default so the field can be
/// `#[serde(default)]` on every entity type.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Requires {
    /// npm specs (`name` or `name@version`); installed by muxix.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub npm: Vec<String>,
    /// Executable names; asserted on PATH, never installed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bin: Vec<String>,
}

impl Requires {
    pub fn is_empty(&self) -> bool {
        self.npm.is_empty() && self.bin.is_empty()
    }
}

/// A parsed npm spec. `@scope/name@1.2.3` → name `@scope/name`, version `1.2.3`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NpmSpec {
    pub name: String,
    pub version: Option<String>,
}

impl NpmSpec {
    pub fn parse(spec: &str) -> Result<Self> {
        let spec = spec.trim();
        if spec.is_empty() {
            bail!("empty npm spec");
        }
        // The leading `@` of a scope is not a version separator.
        let body = spec.strip_prefix('@').unwrap_or(spec);
        let (name, version) = match body.rsplit_once('@') {
            Some((n, v)) if !n.is_empty() && !v.is_empty() => (n, Some(v.to_string())),
            _ => (body, None),
        };
        let name = if spec.starts_with('@') { format!("@{name}") } else { name.to_string() };
        if name.contains(char::is_whitespace) || name.contains('/') && !name.starts_with('@') {
            bail!("invalid npm spec: {spec}");
        }
        Ok(Self { name, version })
    }

    pub fn is_pinned(&self) -> bool {
        self.version.is_some()
    }
}

impl std::fmt::Display for NpmSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.version {
            Some(v) => write!(f, "{}@{v}", self.name),
            None => f.write_str(&self.name),
        }
    }
}

/// The npm global prefix: `bootstrap.npm_prefix` or [`DEFAULT_NPM_PREFIX`],
/// `~` expanded against the real home. No home → error, never cwd.
pub fn npm_prefix(configured: Option<&str>) -> Result<PathBuf> {
    let raw = configured.unwrap_or(DEFAULT_NPM_PREFIX);
    if raw.starts_with('~') && home::home_dir().is_none() {
        bail!("npm prefix {raw}: cannot resolve `~`, home directory unknown");
    }
    let path = crate::util::expand_tilde(raw);
    if !path.is_absolute() {
        bail!("npm prefix must be absolute: {raw}");
    }
    Ok(path)
}

/// Create the prefix if absent; fail unless it is a writable directory.
pub fn ensure_prefix(prefix: &Path) -> Result<()> {
    if !prefix.exists() {
        std::fs::create_dir_all(prefix)
            .with_context(|| format!("npm prefix {}: cannot create", prefix.display()))?;
    }
    if !prefix.is_dir() {
        bail!("npm prefix {} is not a directory", prefix.display());
    }
    if std::fs::metadata(prefix)?.permissions().readonly() {
        bail!("npm prefix {} is not a writable directory", prefix.display());
    }
    Ok(())
}

/// Where `npm install -g --prefix <prefix> <name>` puts the package.
pub fn package_dir(prefix: &Path, name: &str) -> PathBuf {
    prefix.join("lib/node_modules").join(name)
}

/// Inverse of [`package_dir`]: `(prefix, name)` from a stored target path.
pub fn split_package_dir(target: &str) -> Option<(PathBuf, String)> {
    let (prefix, name) = target.split_once("/lib/node_modules/")?;
    Some((PathBuf::from(prefix), name.to_string()))
}

/// Installed version under the prefix, from the package's own package.json.
pub fn installed_version(prefix: &Path, name: &str) -> Option<String> {
    let text = std::fs::read_to_string(package_dir(prefix, name).join("package.json")).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    json.get("version")?.as_str().map(str::to_owned)
}

/// Every package name installed under the prefix (scoped ones as `@scope/name`).
pub fn installed_packages(prefix: &Path) -> Vec<String> {
    let root = prefix.join("lib/node_modules");
    let Ok(entries) = std::fs::read_dir(&root) else { return Vec::new() };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.') {
            continue;
        }
        if name.starts_with('@') {
            if let Ok(scoped) = std::fs::read_dir(entry.path()) {
                for inner in scoped.flatten() {
                    out.push(format!("{name}/{}", inner.file_name().to_string_lossy()));
                }
            }
        } else {
            out.push(name);
        }
    }
    out.sort();
    out
}

/// Whether `<prefix>/bin` is on PATH, so installed bins are reachable.
pub fn prefix_bin_on_path(prefix: &Path) -> bool {
    prefix_bin_in(std::env::var_os("PATH").as_deref(), prefix)
}

fn prefix_bin_in(path: Option<&std::ffi::OsStr>, prefix: &Path) -> bool {
    let bin = prefix.join("bin");
    path.map(|p| std::env::split_paths(p).any(|d| d == bin)).unwrap_or(false)
}

/// First PATH entry holding an executable `name`.
pub fn which(name: &str) -> Option<PathBuf> {
    which_in(std::env::var_os("PATH").as_deref(), name)
}

fn which_in(path: Option<&std::ffi::OsStr>, name: &str) -> Option<PathBuf> {
    if name.is_empty() || name.contains('/') {
        return None;
    }
    std::env::split_paths(path?)
        .map(|d| d.join(name))
        .find(|p| is_executable(p))
}

#[cfg(unix)]
fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.is_file() && p.metadata().map(|m| m.permissions().mode() & 0o111 != 0).unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(p: &Path) -> bool {
    p.is_file()
}

fn npm(prefix: &Path, args: &[&str]) -> Result<()> {
    let status = std::process::Command::new("npm")
        .arg(args[0])
        .args(["-g", "--prefix"])
        .arg(prefix)
        .args(&args[1..])
        .status()
        .context("failed to run npm")?;
    if !status.success() {
        bail!("npm {} exited with {status}", args.join(" "));
    }
    Ok(())
}

pub fn npm_install(prefix: &Path, spec: &NpmSpec) -> Result<()> {
    npm(prefix, &["install", &spec.to_string()])
}

pub fn npm_uninstall(prefix: &Path, name: &str) -> Result<()> {
    npm(prefix, &["uninstall", name])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_parsing() {
        let s = NpmSpec::parse("@fission-ai/openspec@1.6.0").unwrap();
        assert_eq!((s.name.as_str(), s.version.as_deref()), ("@fission-ai/openspec", Some("1.6.0")));
        let s = NpmSpec::parse("@scope/pkg").unwrap();
        assert_eq!((s.name.as_str(), s.version.as_deref()), ("@scope/pkg", None));
        let s = NpmSpec::parse("socraticode@0.4.1").unwrap();
        assert_eq!((s.name.as_str(), s.version.as_deref()), ("socraticode", Some("0.4.1")));
        let s = NpmSpec::parse("socraticode").unwrap();
        assert!(!s.is_pinned());
        assert_eq!(s.to_string(), "socraticode");
        assert!(NpmSpec::parse("").is_err());
        assert!(NpmSpec::parse("a b").is_err());
    }

    #[test]
    fn prefix_probe_and_listing() {
        let tmp = tempfile::tempdir().unwrap();
        let prefix = tmp.path();
        assert!(installed_version(prefix, "foo").is_none());
        for (name, ver) in [("foo", "1.2.3"), ("@s/bar", "0.1.0")] {
            let dir = package_dir(prefix, name);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("package.json"), format!(r#"{{"version":"{ver}"}}"#)).unwrap();
        }
        assert_eq!(installed_version(prefix, "foo").as_deref(), Some("1.2.3"));
        assert_eq!(installed_version(prefix, "@s/bar").as_deref(), Some("0.1.0"));
        assert_eq!(installed_packages(prefix), vec!["@s/bar".to_string(), "foo".to_string()]);
        let target = package_dir(prefix, "@s/bar").to_string_lossy().into_owned();
        assert_eq!(split_package_dir(&target), Some((prefix.to_path_buf(), "@s/bar".to_string())));
    }

    #[test]
    fn prefix_resolution_and_which() {
        assert!(npm_prefix(Some("relative/x")).is_err());
        let tmp = tempfile::tempdir().unwrap();
        let p = npm_prefix(Some(tmp.path().join("np").to_str().unwrap())).unwrap();
        ensure_prefix(&p).unwrap();
        assert!(p.is_dir());
        let file = tmp.path().join("afile");
        std::fs::write(&file, "").unwrap();
        assert!(ensure_prefix(&file).is_err());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let bin = tmp.path().join("bin");
            std::fs::create_dir_all(&bin).unwrap();
            let exe = bin.join("wm-test-bin");
            std::fs::write(&exe, "#!/bin/sh\n").unwrap();
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
            let path = std::env::join_paths([bin.clone(), tmp.path().join("bin")]).unwrap();
            assert_eq!(which_in(Some(&path), "wm-test-bin"), Some(exe));
            assert!(which_in(Some(&path), "definitely-not-a-binary-xyz").is_none());
            assert!(which_in(None, "wm-test-bin").is_none());
            assert!(prefix_bin_in(Some(&path), tmp.path()));
            assert!(!prefix_bin_in(Some(&path), &p));
        }
    }
}
