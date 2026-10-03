//! Agent-specific conversation forking for resuming sessions across worktrees.

use anyhow::{Context, Result};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Information about a conversation session
#[derive(Debug, Clone)]
pub struct SessionInfo {
    /// Session UUID (stem of the .jsonl file)
    pub id: String,
    /// Full path to the .jsonl file
    pub path: PathBuf,
    /// Last modification time
    pub timestamp: SystemTime,
}

/// Trait for agent-specific conversation forking
pub trait ConversationForker: Send + Sync {
    /// Find the most recent conversation for a worktree path
    fn find_latest_conversation(&self, worktree_path: &Path) -> Result<Option<SessionInfo>>;

    /// Find a specific conversation by session ID (or prefix)
    fn find_conversation(
        &self,
        worktree_path: &Path,
        session_id: &str,
    ) -> Result<Option<SessionInfo>>;

    /// Copy a conversation's files to the target worktree's project directory.
    /// Returns the session UUID for resume args.
    fn fork_conversation(&self, session: &SessionInfo, target_worktree: &Path) -> Result<String>;

    /// CLI args to resume a specific session (e.g., ["--resume", uuid])
    fn resume_args(&self, session_id: &str) -> Vec<String>;
}

/// Claude Code conversation forker
pub struct ClaudeForker {
    config_dir: PathBuf,
}

impl ClaudeForker {
    pub fn new() -> Self {
        let config_dir = std::env::var("CLAUDE_CONFIG_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                home::home_dir()
                    .expect("could not determine home directory")
                    .join(".claude")
            });
        Self { config_dir }
    }

    /// Encode a path the same way Claude Code does for project directories.
    /// Non-alphanumeric characters (except `-`) become `-`.
    fn encode_path(path: &Path) -> String {
        path.to_string_lossy()
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' {
                    c
                } else {
                    '-'
                }
            })
            .collect()
    }

    fn projects_dir(&self) -> PathBuf {
        self.config_dir.join("projects")
    }

    fn project_dir_for(&self, worktree_path: &Path) -> PathBuf {
        self.projects_dir().join(Self::encode_path(worktree_path))
    }

    /// List all .jsonl sessions in a project dir, sorted by mtime descending
    fn list_sessions(&self, project_dir: &Path) -> Result<Vec<SessionInfo>> {
        if !project_dir.exists() {
            return Ok(Vec::new());
        }

        let mut sessions = Vec::new();
        for entry in fs::read_dir(project_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("jsonl")
                && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            {
                let metadata = fs::metadata(&path)?;
                sessions.push(SessionInfo {
                    id: stem.to_string(),
                    path: path.clone(),
                    timestamp: metadata.modified()?,
                });
            }
        }

        sessions.sort_by_key(|s| std::cmp::Reverse(s.timestamp));
        Ok(sessions)
    }
}

impl ConversationForker for ClaudeForker {
    fn find_latest_conversation(&self, worktree_path: &Path) -> Result<Option<SessionInfo>> {
        let project_dir = self.project_dir_for(worktree_path);
        let sessions = self.list_sessions(&project_dir)?;
        Ok(sessions.into_iter().next())
    }

    fn find_conversation(
        &self,
        worktree_path: &Path,
        session_id: &str,
    ) -> Result<Option<SessionInfo>> {
        let project_dir = self.project_dir_for(worktree_path);
        let sessions = self.list_sessions(&project_dir)?;
        // Match by exact ID or prefix
        Ok(sessions
            .into_iter()
            .find(|s| s.id == session_id || s.id.starts_with(session_id)))
    }

    fn fork_conversation(&self, session: &SessionInfo, target_worktree: &Path) -> Result<String> {
        let target_dir = self.project_dir_for(target_worktree);
        fs::create_dir_all(&target_dir).context("Failed to create target project directory")?;

        // Copy the .jsonl file
        let target_jsonl = target_dir.join(format!("{}.jsonl", session.id));
        fs::copy(&session.path, &target_jsonl).context("Failed to copy conversation file")?;

        // Copy the session subdirectory if it exists (tool results, subagent data)
        let source_dir = session.path.parent().unwrap();
        let session_subdir = source_dir.join(&session.id);
        if session_subdir.is_dir() {
            let target_subdir = target_dir.join(&session.id);
            crate::workflow::file_ops::copy_dir_recursive(&session_subdir, &target_subdir)
                .context("Failed to copy session data directory")?;
        }

        Ok(session.id.clone())
    }

    fn resume_args(&self, session_id: &str) -> Vec<String> {
        vec!["--resume".to_string(), session_id.to_string()]
    }
}

/// pi-style session forker (pi and its fork omp share the layout):
/// `<sessions_dir>/-<cwd with '/' → '-'>--/<timestamp>_<uuid>.jsonl`,
/// resume by id via `--session <id>` (partial UUIDs accepted by the CLI).
pub struct PiStyleForker {
    sessions_dir: PathBuf,
}

impl PiStyleForker {
    pub fn pi() -> Self {
        let agent_dir = std::env::var("PI_AGENT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                home::home_dir()
                    .expect("could not determine home directory")
                    .join(".pi/agent")
            });
        Self {
            sessions_dir: agent_dir.join("sessions"),
        }
    }

    pub fn omp() -> Self {
        Self {
            sessions_dir: home::home_dir()
                .expect("could not determine home directory")
                .join(".omp/agent/sessions"),
        }
    }

    /// `/home/user/repo` → `--home-user-repo--`
    fn encode_path(path: &Path) -> String {
        format!("-{}--", path.to_string_lossy().replace('/', "-"))
    }

    fn project_dir_for(&self, worktree_path: &Path) -> PathBuf {
        self.sessions_dir.join(Self::encode_path(worktree_path))
    }

    /// Session id is the UUID after the timestamp: `<ts>_<uuid>.jsonl`.
    fn session_id_of(stem: &str) -> String {
        stem.split_once('_')
            .map(|(_, id)| id.to_string())
            .unwrap_or_else(|| stem.to_string())
    }

    fn list_sessions(&self, project_dir: &Path) -> Result<Vec<SessionInfo>> {
        if !project_dir.exists() {
            return Ok(Vec::new());
        }
        let mut sessions = Vec::new();
        for entry in fs::read_dir(project_dir)? {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) == Some("jsonl")
                && let Some(stem) = path.file_stem().and_then(|s| s.to_str())
            {
                sessions.push(SessionInfo {
                    id: Self::session_id_of(stem),
                    timestamp: fs::metadata(&path)?.modified()?,
                    path,
                });
            }
        }
        sessions.sort_by_key(|s| std::cmp::Reverse(s.timestamp));
        Ok(sessions)
    }
}

impl ConversationForker for PiStyleForker {
    fn find_latest_conversation(&self, worktree_path: &Path) -> Result<Option<SessionInfo>> {
        Ok(self
            .list_sessions(&self.project_dir_for(worktree_path))?
            .into_iter()
            .next())
    }

    fn find_conversation(
        &self,
        worktree_path: &Path,
        session_id: &str,
    ) -> Result<Option<SessionInfo>> {
        Ok(self
            .list_sessions(&self.project_dir_for(worktree_path))?
            .into_iter()
            .find(|s| s.id == session_id || s.id.starts_with(session_id)))
    }

    fn fork_conversation(&self, session: &SessionInfo, target_worktree: &Path) -> Result<String> {
        let target_dir = self.project_dir_for(target_worktree);
        fs::create_dir_all(&target_dir).context("Failed to create target session directory")?;
        let file_name = session
            .path
            .file_name()
            .context("Session file has no name")?;
        fs::copy(&session.path, target_dir.join(file_name))
            .context("Failed to copy session file")?;
        Ok(session.id.clone())
    }

    fn resume_args(&self, session_id: &str) -> Vec<String> {
        vec!["--session".to_string(), session_id.to_string()]
    }
}

/// Codex CLI forker. Sessions are date-partitioned rollout files
/// (`~/.codex/sessions/YYYY/MM/DD/rollout-<ts>-<uuid>.jsonl`) whose first
/// JSONL line records the session's `cwd` — that is how they map to worktrees.
pub struct CodexForker {
    sessions_dir: PathBuf,
}

impl CodexForker {
    pub fn new() -> Self {
        let codex_home = std::env::var("CODEX_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                home::home_dir()
                    .expect("could not determine home directory")
                    .join(".codex")
            });
        Self {
            sessions_dir: codex_home.join("sessions"),
        }
    }

    /// Read the SessionMeta first line and pull out (cwd, session id), leniently.
    fn read_meta(path: &Path) -> Option<(PathBuf, String)> {
        use std::io::BufRead;
        let file = fs::File::open(path).ok()?;
        let mut first_line = String::new();
        std::io::BufReader::new(file)
            .read_line(&mut first_line)
            .ok()?;
        let v: serde_json::Value = serde_json::from_str(first_line.trim()).ok()?;
        let meta = v.get("payload").unwrap_or(&v);
        let cwd = meta.get("cwd").and_then(|c| c.as_str())?;
        let id = ["id", "session_id"]
            .iter()
            .find_map(|k| meta.get(*k).and_then(|s| s.as_str()))?;
        Some((PathBuf::from(cwd), id.to_string()))
    }

    /// All rollout files whose recorded cwd matches the worktree, newest first.
    fn sessions_for(&self, worktree_path: &Path) -> Result<Vec<SessionInfo>> {
        let canon_wt = crate::util::canon_or_self(worktree_path);
        let mut sessions = Vec::new();
        // Fixed YYYY/MM/DD depth — walk it directly instead of pulling a crate.
        for year in read_dirs(&self.sessions_dir) {
            for month in read_dirs(&year) {
                for day in read_dirs(&month) {
                    let Ok(entries) = fs::read_dir(&day) else {
                        continue;
                    };
                    for entry in entries.flatten() {
                        let path = entry.path();
                        let is_rollout = path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| n.starts_with("rollout-") && n.ends_with(".jsonl"));
                        if !is_rollout {
                            continue;
                        }
                        let Some((cwd, id)) = Self::read_meta(&path) else {
                            continue;
                        };
                        if crate::util::canon_or_self(&cwd) != canon_wt {
                            continue;
                        }
                        let Ok(timestamp) = fs::metadata(&path).and_then(|m| m.modified()) else {
                            continue;
                        };
                        sessions.push(SessionInfo {
                            id,
                            path,
                            timestamp,
                        });
                    }
                }
            }
        }
        sessions.sort_by_key(|s| std::cmp::Reverse(s.timestamp));
        Ok(sessions)
    }
}

fn read_dirs(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir())
                .collect()
        })
        .unwrap_or_default()
}

impl ConversationForker for CodexForker {
    fn find_latest_conversation(&self, worktree_path: &Path) -> Result<Option<SessionInfo>> {
        Ok(self.sessions_for(worktree_path)?.into_iter().next())
    }

    fn find_conversation(
        &self,
        worktree_path: &Path,
        session_id: &str,
    ) -> Result<Option<SessionInfo>> {
        Ok(self
            .sessions_for(worktree_path)?
            .into_iter()
            .find(|s| s.id == session_id || s.id.starts_with(session_id)))
    }

    fn fork_conversation(&self, _session: &SessionInfo, _target: &Path) -> Result<String> {
        // Codex sessions are not directory-partitioned; a copied rollout would
        // keep its original cwd and stay invisible to the target worktree.
        anyhow::bail!("forking conversations is not supported for codex")
    }

    fn resume_args(&self, session_id: &str) -> Vec<String> {
        vec!["resume".to_string(), session_id.to_string()]
    }
}

/// Resolve a conversation forker for the given agent name.
/// Returns None if the agent's session store can't be read from disk
/// (SQLite- or hash-keyed stores); such agents can still resume via their
/// profile's continue flag.
pub fn resolve_forker(agent_name: &str) -> Option<Box<dyn ConversationForker>> {
    // Normalize: strip path, take basename
    let basename = agent_name.rsplit('/').next().unwrap_or(agent_name);
    let name = basename
        .split_whitespace()
        .next()
        .unwrap_or(basename)
        .to_lowercase();

    match name.as_str() {
        "claude" => Some(Box::new(ClaudeForker::new())),
        "pi" => Some(Box::new(PiStyleForker::pi())),
        "omp" => Some(Box::new(PiStyleForker::omp())),
        "codex" => Some(Box::new(CodexForker::new())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_encode_path() {
        assert_eq!(
            ClaudeForker::encode_path(Path::new("/Users/raine/code/myproject")),
            "-Users-raine-code-myproject"
        );
    }

    #[test]
    fn test_encode_path_worktree() {
        assert_eq!(
            ClaudeForker::encode_path(Path::new("/Users/raine/code/myproject__worktrees/feature")),
            "-Users-raine-code-myproject--worktrees-feature"
        );
    }

    #[test]
    fn test_encode_path_dots_and_underscores() {
        assert_eq!(
            ClaudeForker::encode_path(Path::new("/home/user/.config/my_app")),
            "-home-user--config-my-app"
        );
    }

    #[test]
    fn test_resolve_forker_claude() {
        assert!(resolve_forker("claude").is_some());
        assert!(resolve_forker("Claude").is_some());
        assert!(resolve_forker("/usr/bin/claude --flag").is_some());
    }

    #[test]
    fn test_resolve_forker_unknown() {
        assert!(resolve_forker("unknown-agent").is_none());
        // SQLite/hash-keyed stores: no forker, resume happens via continue flag
        assert!(resolve_forker("opencode").is_none());
    }

    #[test]
    fn test_resolve_forker_pi_omp_codex() {
        assert!(resolve_forker("pi").is_some());
        assert!(resolve_forker("omp").is_some());
        assert!(resolve_forker("codex").is_some());
    }

    #[test]
    fn test_pi_encode_path() {
        assert_eq!(
            PiStyleForker::encode_path(Path::new("/home/esc2/repos/foo")),
            "--home-esc2-repos-foo--"
        );
        // dots and underscores survive, unlike claude's encoding
        assert_eq!(
            PiStyleForker::encode_path(Path::new("/home/u/.pi/my_app")),
            "--home-u-.pi-my_app--"
        );
    }

    #[test]
    fn test_pi_find_latest_and_resume_args() {
        let tmp = tempfile::tempdir().unwrap();
        let forker = PiStyleForker {
            sessions_dir: tmp.path().to_path_buf(),
        };
        let project_dir = forker.project_dir_for(Path::new("/test/project"));
        fs::create_dir_all(&project_dir).unwrap();

        let old = project_dir.join("2026-01-01T00-00-00-000Z_aaaa1111.jsonl");
        fs::write(&old, "{}").unwrap();
        filetime::set_file_mtime(
            &old,
            filetime::FileTime::from_system_time(
                std::time::SystemTime::now() - std::time::Duration::from_secs(10),
            ),
        )
        .unwrap();
        fs::write(
            project_dir.join("2026-02-02T00-00-00-000Z_bbbb2222.jsonl"),
            "{}",
        )
        .unwrap();

        let latest = forker
            .find_latest_conversation(Path::new("/test/project"))
            .unwrap()
            .unwrap();
        assert_eq!(latest.id, "bbbb2222");
        assert_eq!(
            forker.resume_args(&latest.id),
            vec!["--session".to_string(), "bbbb2222".to_string()]
        );

        // find by partial id
        let found = forker
            .find_conversation(Path::new("/test/project"), "aaaa")
            .unwrap()
            .unwrap();
        assert_eq!(found.id, "aaaa1111");
    }

    #[test]
    fn test_codex_matches_sessions_by_recorded_cwd() {
        let tmp = tempfile::tempdir().unwrap();
        let forker = CodexForker {
            sessions_dir: tmp.path().to_path_buf(),
        };
        let day = tmp.path().join("2026/08/19");
        fs::create_dir_all(&day).unwrap();

        let meta = |cwd: &str, id: &str| {
            format!(
                "{{\"timestamp\":\"t\",\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\",\"cwd\":\"{cwd}\"}}}}\n{{\"type\":\"other\"}}"
            )
        };
        fs::write(
            day.join("rollout-2026-08-19T10-00-00-11111111-aaaa.jsonl"),
            meta("/my/worktree", "11111111-aaaa"),
        )
        .unwrap();
        fs::write(
            day.join("rollout-2026-08-19T11-00-00-22222222-bbbb.jsonl"),
            meta("/other/dir", "22222222-bbbb"),
        )
        .unwrap();

        let latest = forker
            .find_latest_conversation(Path::new("/my/worktree"))
            .unwrap()
            .unwrap();
        assert_eq!(latest.id, "11111111-aaaa");
        assert!(
            forker
                .find_latest_conversation(Path::new("/nowhere"))
                .unwrap()
                .is_none()
        );
        assert_eq!(
            forker.resume_args("11111111-aaaa"),
            vec!["resume".to_string(), "11111111-aaaa".to_string()]
        );
    }

    #[test]
    fn test_list_sessions_ordering() {
        let tmp = tempfile::tempdir().unwrap();
        let forker = ClaudeForker {
            config_dir: tmp.path().to_path_buf(),
        };
        let project_dir = forker.project_dir_for(Path::new("/test/project"));
        fs::create_dir_all(&project_dir).unwrap();

        // Create two session files with a small delay to ensure different mtimes
        let old_file = project_dir.join("old-session.jsonl");
        fs::write(&old_file, "{}").unwrap();

        // Set the old file's mtime to the past
        let old_time = std::time::SystemTime::now() - std::time::Duration::from_secs(10);
        filetime::set_file_mtime(&old_file, filetime::FileTime::from_system_time(old_time))
            .unwrap();

        let new_file = project_dir.join("new-session.jsonl");
        fs::write(&new_file, "{}").unwrap();

        let sessions = forker.list_sessions(&project_dir).unwrap();
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].id, "new-session");
        assert_eq!(sessions[1].id, "old-session");
    }

    #[test]
    fn test_list_sessions_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let forker = ClaudeForker {
            config_dir: tmp.path().to_path_buf(),
        };
        let sessions = forker
            .list_sessions(Path::new("/nonexistent/path"))
            .unwrap();
        assert!(sessions.is_empty());
    }

    #[test]
    fn test_fork_conversation_copies_files() {
        let tmp = tempfile::tempdir().unwrap();
        let forker = ClaudeForker {
            config_dir: tmp.path().to_path_buf(),
        };

        // Create source project dir with a session
        let source_dir = forker.project_dir_for(Path::new("/source/project"));
        fs::create_dir_all(&source_dir).unwrap();
        let session_file = source_dir.join("abc123.jsonl");
        fs::write(&session_file, "{\"test\": true}").unwrap();

        // Create session subdirectory with data
        let session_subdir = source_dir.join("abc123");
        fs::create_dir_all(&session_subdir).unwrap();
        fs::write(session_subdir.join("data.json"), "{}").unwrap();

        let session = SessionInfo {
            id: "abc123".to_string(),
            path: session_file,
            timestamp: std::time::SystemTime::now(),
        };

        let result = forker
            .fork_conversation(&session, Path::new("/target/project"))
            .unwrap();
        assert_eq!(result, "abc123");

        // Verify files were copied
        let target_dir = forker.project_dir_for(Path::new("/target/project"));
        assert!(target_dir.join("abc123.jsonl").exists());
        assert!(target_dir.join("abc123").join("data.json").exists());
    }

    #[test]
    fn test_find_conversation_by_prefix() {
        let tmp = tempfile::tempdir().unwrap();
        let forker = ClaudeForker {
            config_dir: tmp.path().to_path_buf(),
        };
        let project_dir = forker.project_dir_for(Path::new("/test/project"));
        fs::create_dir_all(&project_dir).unwrap();

        fs::write(project_dir.join("abc123-def456.jsonl"), "{}").unwrap();

        let session = forker
            .find_conversation(Path::new("/test/project"), "abc123")
            .unwrap();
        assert!(session.is_some());
        assert_eq!(session.unwrap().id, "abc123-def456");
    }
}
