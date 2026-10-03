//! Programmatic edits to muxix config files that keep the rest of the file
//! byte-identical — comments, key order, and formatting included.
//!
//! `muxix config agent` and `muxix bootstrap` are the writers. A serde
//! round-trip would be three lines and would also delete every comment in a
//! hand-written config, which is why this splices one block at a time.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};

/// Replace, insert, or remove one top-level block in a YAML document.
///
/// A block is the `key:` line plus every following line that belongs to it:
/// indented lines, blank lines, and comments. `new_block` is the full
/// replacement text (including its own `key:` line, no trailing newline);
/// `None` removes the block. A key that is absent is appended at the end.
pub fn splice_top_level(text: &str, key: &str, new_block: Option<&str>) -> String {
    splice_path(text, &[key], new_block)
}

/// Replace, insert, or remove a nested block addressed by a key path.
///
/// Each level is located by line scan inside the previous level's range at that
/// level's indent; missing levels are created on the way down. `new_block` is
/// rendered at column 0 and indented to the target depth here, so callers build
/// it without knowing where it lands.
///
/// ponytail: line scan, not a YAML CST. It handles block-style mappings, which
/// is what muxix configs are. If a config in flow style ever needs editing,
/// swap in a comment-preserving YAML crate rather than growing this.
pub fn splice_path(text: &str, path: &[&str], new_block: Option<&str>) -> String {
    assert!(!path.is_empty(), "splice_path needs at least one key");
    let lines: Vec<String> = text.lines().map(str::to_string).collect();
    let end = lines.len();
    let spliced = splice_in_range(lines, 0, end, path, 0, new_block);
    let mut joined = spliced.join("\n");
    if !joined.is_empty() {
        joined.push('\n');
    }
    joined
}

/// Splice `path` inside `lines[start..end]`, where that range is the body of a
/// mapping whose keys sit at `indent` spaces.
fn splice_in_range(
    mut lines: Vec<String>,
    start: usize,
    end: usize,
    path: &[&str],
    indent: usize,
    new_block: Option<&str>,
) -> Vec<String> {
    let pad = " ".repeat(indent);
    let key_line = format!("{pad}{}:", path[0]);

    let found = (start..end).find(|i| {
        let l = &lines[*i];
        *l == key_line || l.starts_with(&format!("{key_line} "))
    });

    let Some(key_at) = found else {
        // Missing key: nothing to remove, otherwise materialize the remaining
        // path as a nested block at the end of this range.
        let Some(block) = new_block else {
            return lines;
        };
        let rendered = nest(path, block, indent);
        let insert_at = trim_back(&lines, start, end);
        lines.splice(insert_at..insert_at, rendered.lines().map(str::to_string));
        return lines;
    };

    let body_end = block_end(&lines, key_at, end, indent);

    if path.len() > 1 {
        return splice_in_range(
            lines,
            key_at + 1,
            body_end,
            &path[1..],
            indent + 2,
            new_block,
        );
    }

    let replacement: Vec<String> = match new_block {
        Some(block) => indent_block(block, indent).lines().map(str::to_string).collect(),
        None => Vec::new(),
    };
    lines.splice(key_at..body_end, replacement);
    lines
}

/// Last line index of the block opened at `key_at`, exclusive. Trailing blank
/// lines belong to whatever comes next, not to the block.
fn block_end(lines: &[String], key_at: usize, end: usize, indent: usize) -> usize {
    let mut i = key_at + 1;
    while i < end {
        let line = &lines[i];
        let deeper = line.trim().is_empty()
            || line.chars().take_while(|c| *c == ' ').count() > indent
            || line[indent.min(line.len())..].starts_with('-');
        if !deeper {
            break;
        }
        i += 1;
    }
    trim_back(lines, key_at + 1, i)
}

/// Walk back over trailing blank lines in `lines[start..end]`.
fn trim_back(lines: &[String], start: usize, end: usize) -> usize {
    let mut i = end;
    while i > start && lines[i - 1].trim().is_empty() {
        i -= 1;
    }
    i
}

/// Wrap `block` in the parent keys of `path`, all indented from `indent`.
fn nest(path: &[&str], block: &str, indent: usize) -> String {
    let mut out = String::new();
    for (depth, key) in path[..path.len() - 1].iter().enumerate() {
        out.push_str(&" ".repeat(indent + depth * 2));
        out.push_str(key);
        out.push_str(":\n");
    }
    out.push_str(&indent_block(block, indent + (path.len() - 1) * 2));
    out
}

fn indent_block(block: &str, indent: usize) -> String {
    let pad = " ".repeat(indent);
    block
        .lines()
        .map(|l| {
            if l.trim().is_empty() {
                String::new()
            } else {
                format!("{pad}{l}")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Apply `splice_top_level` to a file, creating it if missing.
pub fn edit_file(path: &Path, key: &str, new_block: Option<&str>) -> Result<()> {
    edit_file_at(path, &[key], new_block)
}

/// Apply `splice_path` to a file, creating it if missing. Atomic, and the
/// result must still parse as YAML — a layout this scanner cannot handle fails
/// loudly instead of corrupting the config.
pub fn edit_file_at(path: &Path, key_path: &[&str], new_block: Option<&str>) -> Result<()> {
    let current = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("Failed to read {}", path.display())),
    };
    let updated = splice_path(&current, key_path, new_block);

    serde_yaml::from_str::<serde_yaml::Value>(&updated).with_context(|| {
        format!(
            "Editing {} would produce invalid YAML — edit the file by hand",
            path.display()
        )
    })?;

    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("yaml.tmp");
    fs::write(&tmp, updated)?;
    fs::rename(&tmp, path).with_context(|| format!("Failed to write {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "# global config\nagent: claude\n\n# panes are documented upstream\npanes:\n  - command: <agent>\n\nmode: window\n";

    const NESTED: &str = "# top\nbootstrap:\n  # keep this comment\n  plugins:\n    - old\n  agents:\n    pi:\n      add_plugins:\n        - a\n\nmode: window\n";

    #[test]
    fn replaces_scalar_key_and_keeps_comments() {
        let out = splice_top_level(SAMPLE, "agent", Some("agent: pi"));
        assert!(out.starts_with("# global config\nagent: pi\n"));
        assert!(out.contains("# panes are documented upstream"));
        assert!(out.contains("mode: window"));
    }

    #[test]
    fn replaces_block_key_without_eating_neighbours() {
        let out = splice_top_level(SAMPLE, "panes", Some("panes:\n  - command: zsh"));
        assert!(out.contains("panes:\n  - command: zsh\n"));
        assert!(out.contains("mode: window"));
        assert!(out.contains("agent: claude"));
        assert!(!out.contains("<agent>"));
    }

    #[test]
    fn appends_missing_key() {
        let out = splice_top_level(SAMPLE, "agent_rules", Some("agent_rules:\n- match: /x\n  agent: pi"));
        assert!(out.ends_with("agent_rules:\n- match: /x\n  agent: pi\n"));
        assert!(out.contains("mode: window"));
    }

    #[test]
    fn removes_block() {
        let out = splice_top_level(SAMPLE, "panes", None);
        assert!(!out.contains("panes:"));
        assert!(out.contains("# panes are documented upstream"));
        assert!(out.contains("mode: window"));
    }

    #[test]
    fn appending_to_empty_file_has_no_leading_newline() {
        let out = splice_top_level("", "agent", Some("agent: pi"));
        assert_eq!(out, "agent: pi\n");
    }

    #[test]
    fn round_trips_through_yaml() {
        let out = splice_top_level(SAMPLE, "agent_rules", Some("agent_rules:\n- match: /x\n  agent: pi"));
        let value: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(value["agent"].as_str(), Some("claude"));
        assert_eq!(value["agent_rules"][0]["agent"].as_str(), Some("pi"));
    }

    #[test]
    fn replaces_a_nested_list() {
        let out = splice_path(
            NESTED,
            &["bootstrap", "plugins"],
            Some("plugins:\n- old\n- new"),
        );
        let value: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(value["bootstrap"]["plugins"][1].as_str(), Some("new"));
        assert!(out.contains("# keep this comment"), "{out}");
        assert!(out.contains("mode: window"));
        assert_eq!(
            value["bootstrap"]["agents"]["pi"]["add_plugins"][0].as_str(),
            Some("a"),
            "sibling keys survive"
        );
    }

    #[test]
    fn replaces_a_deeply_nested_list() {
        let out = splice_path(
            NESTED,
            &["bootstrap", "agents", "pi", "add_plugins"],
            Some("add_plugins:\n- a\n- b"),
        );
        let value: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(
            value["bootstrap"]["agents"]["pi"]["add_plugins"][1].as_str(),
            Some("b")
        );
        assert_eq!(value["bootstrap"]["plugins"][0].as_str(), Some("old"));
        assert!(out.contains("mode: window"));
    }

    #[test]
    fn creates_missing_levels() {
        let out = splice_path(
            NESTED,
            &["bootstrap", "agents", "claude", "add_skills"],
            Some("add_skills:\n- ./skills/x"),
        );
        let value: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(
            value["bootstrap"]["agents"]["claude"]["add_skills"][0].as_str(),
            Some("./skills/x")
        );
        assert_eq!(
            value["bootstrap"]["agents"]["pi"]["add_plugins"][0].as_str(),
            Some("a"),
            "the existing agent is untouched"
        );
    }

    #[test]
    fn creates_the_whole_path_in_an_empty_file() {
        let out = splice_path(
            "",
            &["bootstrap", "agents", "pi", "add_plugins"],
            Some("add_plugins:\n- x"),
        );
        let value: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(
            value["bootstrap"]["agents"]["pi"]["add_plugins"][0].as_str(),
            Some("x")
        );
    }

    #[test]
    fn removes_a_nested_list() {
        let out = splice_path(NESTED, &["bootstrap", "plugins"], None);
        let value: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        assert!(value["bootstrap"].get("plugins").is_none());
        assert_eq!(
            value["bootstrap"]["agents"]["pi"]["add_plugins"][0].as_str(),
            Some("a")
        );
    }
}
