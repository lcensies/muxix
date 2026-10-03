//! Hot-reloadable spec source.
//!
//! Wraps an embedded baseline spec and an optional on-disk override. The file
//! is re-read when its modification time changes; a parse error keeps the
//! last-good tree and exposes the error for display, so a typo while
//! prototyping never blanks the screen.

use std::path::PathBuf;
use std::time::SystemTime;

use super::node::Node;
use super::parse::parse_yaml;

pub struct SpecSource {
    embedded: &'static str,
    path: Option<PathBuf>,
    node: Node,
    error: Option<String>,
    last_mtime: Option<SystemTime>,
    loaded_from_file: bool,
}

impl SpecSource {
    /// Build a source from an embedded baseline (which must parse) and an
    /// optional override path. The override is loaded immediately if present.
    pub fn new(embedded: &'static str, path: Option<PathBuf>) -> Self {
        let node = parse_yaml(embedded)
            .unwrap_or_else(|e| panic!("embedded DSL spec failed to parse: {e}"));
        let mut source = Self {
            embedded,
            path,
            node,
            error: None,
            last_mtime: None,
            loaded_from_file: false,
        };
        source.reload_if_changed();
        source
    }

    /// The current (last-good) node tree.
    pub fn node(&self) -> &Node {
        &self.node
    }

    /// The most recent parse error, if the override currently fails to parse.
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    /// Re-read the override file when its mtime changed. Returns `true` when the
    /// rendered tree changed (caller should redraw).
    pub fn reload_if_changed(&mut self) -> bool {
        let Some(path) = self.path.clone() else {
            return false;
        };

        let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();

        match mtime {
            // File missing: fall back to embedded if we were on a file copy.
            None => {
                self.last_mtime = None;
                if self.loaded_from_file {
                    self.loaded_from_file = false;
                    self.error = None;
                    if let Ok(node) = parse_yaml(self.embedded) {
                        self.node = node;
                        return true;
                    }
                }
                false
            }
            Some(mtime) => {
                if self.last_mtime == Some(mtime) {
                    return false;
                }
                self.last_mtime = Some(mtime);
                match std::fs::read_to_string(&path) {
                    Ok(text) => match parse_yaml(&text) {
                        Ok(node) => {
                            self.node = node;
                            self.error = None;
                            self.loaded_from_file = true;
                            true
                        }
                        Err(e) => {
                            // Keep last-good tree; surface the error.
                            self.error = Some(e);
                            false
                        }
                    },
                    Err(e) => {
                        self.error = Some(e.to_string());
                        false
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const EMBEDDED: &str = "type: text\ncontent: embedded\n";

    #[test]
    fn embedded_only_when_no_path() {
        let s = SpecSource::new(EMBEDDED, None);
        assert!(matches!(s.node(), Node::Text { .. }));
        assert!(s.error().is_none());
    }

    #[test]
    fn loads_override_and_reloads_on_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("view.yaml");
        std::fs::write(&path, "type: text\ncontent: v1\n").unwrap();

        let mut s = SpecSource::new(EMBEDDED, Some(path.clone()));
        match s.node() {
            Node::Text { content, .. } => assert_eq!(content, "v1"),
            other => panic!("{other:?}"),
        }

        // No change -> no reload.
        assert!(!s.reload_if_changed());

        // Change content (and bump mtime explicitly to avoid same-second races).
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .open(&path)
            .unwrap();
        f.write_all(b"type: text\ncontent: v2\n").unwrap();
        f.sync_all().unwrap();
        filetime::set_file_mtime(&path, filetime::FileTime::from_unix_time(1_000_000, 0)).unwrap();

        assert!(s.reload_if_changed());
        match s.node() {
            Node::Text { content, .. } => assert_eq!(content, "v2"),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn parse_error_keeps_last_good() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("view.yaml");
        std::fs::write(&path, "type: text\ncontent: good\n").unwrap();
        let mut s = SpecSource::new(EMBEDDED, Some(path.clone()));

        std::fs::write(&path, "type: bogus\n").unwrap();
        filetime::set_file_mtime(&path, filetime::FileTime::from_unix_time(2_000_000, 0)).unwrap();
        let changed = s.reload_if_changed();
        assert!(!changed, "broken spec must not change the tree");
        assert!(s.error().is_some(), "error should be surfaced");
        match s.node() {
            Node::Text { content, .. } => assert_eq!(content, "good"),
            other => panic!("{other:?}"),
        }
    }
}
