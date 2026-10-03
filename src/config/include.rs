//! `include:` expansion — turning a config file's include list into layers.
//!
//! An included config is merged *before* the file that pulled it in, so the
//! including file always wins on conflict. Includes are resolved depth-first:
//! a file's own includes are expanded before its own keys are contributed.
//!
//! # Cost
//!
//! Every entry point here is gated on the `include:` key actually being
//! present. A config that does not use includes performs no extra filesystem
//! or network work, which matters because config loading runs on every
//! `workmux` invocation and inside the sidebar daemon's per-project loops.

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use serde_yaml::Value;
use std::path::{Path, PathBuf};

use super::resolve::{Layer, LayerKind};

/// Maximum include nesting depth. Deep chains are far more likely to be a
/// mistake than an intent, and the cap keeps a pathological config from
/// exhausting the stack before cycle detection notices.
pub const MAX_INCLUDE_DEPTH: usize = 16;

/// How long a fetched remote include stays fresh before it is re-fetched.
const REMOTE_CACHE_TTL_SECS: u64 = 24 * 3600;

/// One entry of a config's `include:` list.
///
/// Accepts a bare string (`include: [./base.yaml]`) or a table
/// (`include: [{ path: ./base.yaml, optional: true }]`).
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum IncludeEntry {
    Path(String),
    Detailed {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        url: Option<String>,
        /// A missing source is skipped instead of failing the load.
        #[serde(default)]
        optional: bool,
    },
}

impl IncludeEntry {
    /// The raw, unresolved source string, and whether it may be missing.
    fn parts(&self) -> Result<(&str, bool)> {
        match self {
            IncludeEntry::Path(s) => Ok((s.as_str(), false)),
            IncludeEntry::Detailed {
                path,
                url,
                optional,
            } => match (path.as_deref(), url.as_deref()) {
                (Some(p), None) => Ok((p, *optional)),
                (None, Some(u)) => Ok((u, *optional)),
                (Some(_), Some(_)) => {
                    bail!("include entry sets both `path` and `url`; use one or the other")
                }
                (None, None) => bail!("include entry sets neither `path` nor `url`"),
            },
        }
    }
}

/// Where an include's content came from, used for cycle detection and for the
/// layer's `source` in `--explain` output.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Source {
    File(PathBuf),
    Url(String),
}

impl Source {
    fn display(&self) -> String {
        match self {
            Source::File(p) => p.display().to_string(),
            Source::Url(u) => u.clone(),
        }
    }
}

/// Read a config file's `include:` list, if it has one.
///
/// Returns `None` when the key is absent — the caller uses that to skip include
/// handling entirely.
pub fn include_list(value: &Value) -> Result<Option<Vec<IncludeEntry>>> {
    let Some(raw) = value.get("include") else {
        return Ok(None);
    };
    if raw.is_null() {
        return Ok(None);
    }
    let entries: Vec<IncludeEntry> = serde_yaml::from_value(raw.clone())
        .context("`include:` must be a list of paths, URLs, or {path|url, optional} entries")?;
    Ok(Some(entries))
}

/// Remove the `include:` key from a value.
///
/// The key is directive, not configuration: it must not survive into the
/// merged config, where it would be an unknown field.
pub fn strip_include_key(value: &mut Value) {
    if let Value::Mapping(map) = value {
        map.remove(Value::String("include".to_string()));
    }
}

/// Expand `value`'s includes into layers, depth-first.
///
/// `base_dir` resolves relative paths; `origin` is the file being expanded and
/// seeds the cycle-detection chain. `trusted` is inherited by every produced
/// layer — an include pulled in by the global config may set global-only keys,
/// one pulled in by a project config may not.
///
/// The returned layers are ordered lowest precedence first and do NOT include
/// `value` itself; the caller appends that after.
pub fn expand(
    value: &Value,
    base_dir: &Path,
    origin: &Path,
    trusted: bool,
    id_prefix: &str,
) -> Result<Vec<Layer>> {
    let Some(entries) = include_list(value)? else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    let mut chain = vec![Source::File(origin.to_path_buf())];
    expand_entries(
        &entries,
        base_dir,
        trusted,
        id_prefix,
        &mut chain,
        &mut out,
    )?;
    Ok(out)
}

fn expand_entries(
    entries: &[IncludeEntry],
    base_dir: &Path,
    trusted: bool,
    id_prefix: &str,
    chain: &mut Vec<Source>,
    out: &mut Vec<Layer>,
) -> Result<()> {
    if chain.len() > MAX_INCLUDE_DEPTH {
        bail!(
            "include chain exceeds the maximum depth of {}: {}",
            MAX_INCLUDE_DEPTH,
            chain
                .iter()
                .map(Source::display)
                .collect::<Vec<_>>()
                .join(" -> ")
        );
    }

    for entry in entries {
        let (raw, optional) = entry.parts()?;
        let source = resolve_source(raw, base_dir)?;

        if chain.contains(&source) {
            let mut cycle: Vec<String> = chain.iter().map(Source::display).collect();
            cycle.push(source.display());
            bail!("include cycle detected: {}", cycle.join(" -> "));
        }

        let Some(contents) = read_source(&source, optional)? else {
            continue;
        };

        let mut child: Value = serde_yaml::from_str(&contents)
            .map_err(|e| anyhow::anyhow!("Failed to parse include {}: {}", source.display(), e))?;
        if child.is_null() {
            continue;
        }

        // Depth-first: the child's own includes rank below the child itself.
        if let Some(nested) = include_list(&child)? {
            let child_dir = match &source {
                Source::File(p) => p.parent().map(Path::to_path_buf).unwrap_or_default(),
                // A remote include's relative paths have no meaningful local
                // base; only absolute paths and URLs make sense beneath it.
                Source::Url(_) => base_dir.to_path_buf(),
            };
            chain.push(source.clone());
            expand_entries(&nested, &child_dir, trusted, id_prefix, chain, out)?;
            chain.pop();
        }
        strip_include_key(&mut child);

        let display = source.display();
        out.push(
            Layer::new(
                format!("{id_prefix}include:{display}"),
                display,
                LayerKind::Include,
                child,
            )
            .trusted(trusted),
        );
    }
    Ok(())
}

/// Turn a raw include string into a concrete source.
fn resolve_source(raw: &str, base_dir: &Path) -> Result<Source> {
    let raw = raw.trim();
    if raw.starts_with("http://") {
        bail!(
            "include {raw} uses plain HTTP; only https:// remote includes are permitted \
             (an attacker on the network could otherwise rewrite your config)"
        );
    }
    if raw.starts_with("https://") {
        return Ok(Source::Url(raw.to_string()));
    }
    if raw.starts_with('~') {
        return Ok(Source::File(crate::util::expand_tilde(raw)));
    }
    let path = Path::new(raw);
    if path.is_absolute() {
        return Ok(Source::File(path.to_path_buf()));
    }
    Ok(Source::File(base_dir.join(path)))
}

/// Fetch a source's contents. Returns `None` when it is missing and optional.
fn read_source(source: &Source, optional: bool) -> Result<Option<String>> {
    match source {
        Source::File(path) => {
            if !path.exists() {
                if optional {
                    return Ok(None);
                }
                bail!("include not found: {}", path.display());
            }
            Ok(Some(std::fs::read_to_string(path).with_context(|| {
                format!("failed to read include {}", path.display())
            })?))
        }
        Source::Url(url) => fetch_remote(url, optional),
    }
}

/// Path of the on-disk cache entry for a remote include.
fn remote_cache_path(url: &str) -> Result<PathBuf> {
    // A hash keeps the filename filesystem-safe and bounded. This is a cache
    // key, not a security boundary — the URL is the trust decision, made in
    // `resolve_source`, and the cache lives in the user's own cache dir.
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut hasher);
    Ok(crate::xdg::cache_dir()?
        .join("includes")
        .join(format!("{:016x}.yaml", hasher.finish())))
}

/// Fetch a remote include, using the on-disk cache when it is fresh and falling
/// back to a stale copy when the network is unavailable.
fn fetch_remote(url: &str, optional: bool) -> Result<Option<String>> {
    let cache_path = remote_cache_path(url)?;

    if let Ok(meta) = std::fs::metadata(&cache_path)
        && let Ok(modified) = meta.modified()
        && let Ok(age) = modified.elapsed()
        && age.as_secs() < REMOTE_CACHE_TTL_SECS
        && let Ok(cached) = std::fs::read_to_string(&cache_path)
    {
        return Ok(Some(cached));
    }

    match ureq::get(url).call() {
        Ok(resp) => {
            let body = resp
                .into_string()
                .with_context(|| format!("include {url} returned a body that is not text"))?;
            if let Some(parent) = cache_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&cache_path, &body);
            Ok(Some(body))
        }
        Err(e) => {
            // A warm cache beats failing the whole config load when the network
            // is down or the server is briefly unavailable.
            if let Ok(cached) = std::fs::read_to_string(&cache_path) {
                eprintln!("warning: include {url} could not be fetched ({e}); using cached copy");
                return Ok(Some(cached));
            }
            if optional {
                return Ok(None);
            }
            bail!("failed to fetch include {url}: {e}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, body).unwrap();
        path
    }

    fn tmpdir() -> PathBuf {
        let base = std::env::temp_dir().join(format!(
            "workmux-include-test-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = fs::remove_dir_all(&base);
        fs::create_dir_all(&base).unwrap();
        base
    }

    fn expand_file(path: &Path, trusted: bool) -> Result<Vec<Layer>> {
        let value: Value = serde_yaml::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        expand(&value, path.parent().unwrap(), path, trusted, "")
    }

    #[test]
    fn no_include_key_produces_no_layers_and_no_io() {
        let value: Value = serde_yaml::from_str("agent: claude").unwrap();
        // A nonexistent base dir proves nothing was read.
        let layers = expand(
            &value,
            Path::new("/nonexistent/base"),
            Path::new("/nonexistent/base/config.yaml"),
            true,
            "",
        )
        .unwrap();
        assert!(layers.is_empty());
        assert!(include_list(&value).unwrap().is_none());
    }

    #[test]
    fn relative_include_is_resolved_against_the_including_file() {
        let dir = tmpdir();
        write(&dir, "base.yaml", "agent: codex\nmerge_strategy: rebase\n");
        let main = write(&dir, "config.yaml", "include: [./base.yaml]\nagent: claude\n");

        let layers = expand_file(&main, true).unwrap();
        assert_eq!(layers.len(), 1);
        assert_eq!(layers[0].value.get("agent").unwrap().as_str(), Some("codex"));
    }

    #[test]
    fn include_order_is_declaration_order() {
        let dir = tmpdir();
        write(&dir, "a.yaml", "agent: one\n");
        write(&dir, "b.yaml", "agent: two\n");
        let main = write(&dir, "config.yaml", "include: [./a.yaml, ./b.yaml]\n");

        let layers = expand_file(&main, true).unwrap();
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[0].value.get("agent").unwrap().as_str(), Some("one"));
        assert_eq!(layers[1].value.get("agent").unwrap().as_str(), Some("two"));
    }

    #[test]
    fn absolute_include_path() {
        let dir = tmpdir();
        let base = write(&dir, "base.yaml", "agent: codex\n");
        let main = write(
            &dir,
            "config.yaml",
            &format!("include: ['{}']\n", base.display()),
        );

        let layers = expand_file(&main, true).unwrap();
        assert_eq!(layers.len(), 1);
    }

    #[test]
    fn nested_include_is_depth_first() {
        let dir = tmpdir();
        write(&dir, "deep.yaml", "agent: deep\n");
        write(&dir, "mid.yaml", "include: [./deep.yaml]\nagent: mid\n");
        let main = write(&dir, "config.yaml", "include: [./mid.yaml]\nagent: main\n");

        let layers = expand_file(&main, true).unwrap();
        // deep ranks below mid, and both below the including file.
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[0].value.get("agent").unwrap().as_str(), Some("deep"));
        assert_eq!(layers[1].value.get("agent").unwrap().as_str(), Some("mid"));
    }

    #[test]
    fn nested_include_resolves_against_its_own_directory() {
        let dir = tmpdir();
        write(&dir, "sub/deep.yaml", "agent: deep\n");
        write(&dir, "sub/mid.yaml", "include: [./deep.yaml]\n");
        let main = write(&dir, "config.yaml", "include: [./sub/mid.yaml]\n");

        let layers = expand_file(&main, true).unwrap();
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[0].value.get("agent").unwrap().as_str(), Some("deep"));
    }

    #[test]
    fn include_key_is_stripped_from_produced_layers() {
        let dir = tmpdir();
        write(&dir, "deep.yaml", "agent: deep\n");
        write(&dir, "mid.yaml", "include: [./deep.yaml]\nagent: mid\n");
        let main = write(&dir, "config.yaml", "include: [./mid.yaml]\n");

        let layers = expand_file(&main, true).unwrap();
        for layer in &layers {
            assert!(
                layer.value.get("include").is_none(),
                "include: must not survive into a merged layer"
            );
        }
    }

    #[test]
    fn cycle_is_rejected_with_the_full_path() {
        let dir = tmpdir();
        write(&dir, "a.yaml", "include: [./b.yaml]\n");
        write(&dir, "b.yaml", "include: [./a.yaml]\n");
        let main = write(&dir, "a.yaml", "include: [./b.yaml]\n");

        let err = expand_file(&main, true).unwrap_err().to_string();
        assert!(err.contains("include cycle detected"), "{err}");
        assert!(err.contains("a.yaml"), "{err}");
        assert!(err.contains("b.yaml"), "{err}");
    }

    #[test]
    fn self_include_is_a_cycle() {
        let dir = tmpdir();
        let main = write(&dir, "config.yaml", "include: [./config.yaml]\n");
        let err = expand_file(&main, true).unwrap_err().to_string();
        assert!(err.contains("include cycle detected"), "{err}");
    }

    #[test]
    fn depth_cap_is_enforced() {
        let dir = tmpdir();
        // A chain longer than the cap, each file distinct so it is not a cycle.
        let depth = MAX_INCLUDE_DEPTH + 4;
        for i in 0..depth {
            let body = if i + 1 < depth {
                format!("include: [./f{}.yaml]\n", i + 1)
            } else {
                "agent: last\n".to_string()
            };
            write(&dir, &format!("f{i}.yaml"), &body);
        }
        let main = dir.join("f0.yaml");
        let err = expand_file(&main, true).unwrap_err().to_string();
        assert!(err.contains("maximum depth"), "{err}");
    }

    #[test]
    fn missing_include_is_an_error_naming_the_path() {
        let dir = tmpdir();
        let main = write(&dir, "config.yaml", "include: [./absent.yaml]\n");
        let err = expand_file(&main, true).unwrap_err().to_string();
        assert!(err.contains("include not found"), "{err}");
        assert!(err.contains("absent.yaml"), "{err}");
    }

    #[test]
    fn optional_missing_include_is_skipped() {
        let dir = tmpdir();
        let main = write(
            &dir,
            "config.yaml",
            "include:\n  - path: ./absent.yaml\n    optional: true\n",
        );
        let layers = expand_file(&main, true).unwrap();
        assert!(layers.is_empty());
    }

    #[test]
    fn plain_http_include_is_rejected() {
        let dir = tmpdir();
        let main = write(&dir, "config.yaml", "include: ['http://example.com/c.yaml']\n");
        let err = expand_file(&main, true).unwrap_err().to_string();
        assert!(err.contains("plain HTTP"), "{err}");
    }

    #[test]
    fn empty_include_file_contributes_no_layer() {
        let dir = tmpdir();
        write(&dir, "empty.yaml", "\n");
        let main = write(&dir, "config.yaml", "include: [./empty.yaml]\n");
        let layers = expand_file(&main, true).unwrap();
        assert!(layers.is_empty());
    }

    #[test]
    fn include_entry_with_both_path_and_url_is_rejected() {
        let entry = IncludeEntry::Detailed {
            path: Some("a".into()),
            url: Some("https://b".into()),
            optional: false,
        };
        assert!(entry.parts().is_err());
    }

    #[test]
    fn trust_is_inherited_by_produced_layers() {
        let dir = tmpdir();
        write(&dir, "base.yaml", "agent: codex\n");
        let main = write(&dir, "config.yaml", "include: [./base.yaml]\n");

        assert!(expand_file(&main, true).unwrap()[0].trusted);
        assert!(!expand_file(&main, false).unwrap()[0].trusted);
    }

    #[test]
    fn malformed_include_list_is_reported() {
        let value: Value = serde_yaml::from_str("include: 5").unwrap();
        assert!(include_list(&value).is_err());
    }
}
