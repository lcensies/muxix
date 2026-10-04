use anyhow::Context as _;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::debug;

use crate::{cmd, git, nerdfont};
use which::{which, which_in};

pub mod edit;
pub mod include;
pub mod profiles;
pub mod resolve;
pub mod secrets;

#[cfg(test)]
mod load_tests;

#[cfg(test)]
mod merge_differential;

/// Default script for cleaning up node_modules directories before worktree deletion.
/// This script moves node_modules to a temporary location and deletes them in the background,
/// making the muxix remove command return almost instantly.
const NODE_MODULES_CLEANUP_SCRIPT: &str = include_str!("scripts/cleanup_node_modules.sh");

/// Configuration for file operations during worktree creation
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct FileConfig {
    /// Glob patterns for files to copy from the repo root to the new worktree
    #[serde(default)]
    pub copy: Option<Vec<String>>,

    /// Glob patterns for files to symlink from the repo root into the new worktree
    #[serde(default)]
    pub symlink: Option<Vec<String>>,
}

/// Configuration for a single MCP (Model Context Protocol) server, declared in
/// the `mcp:` section of `.muxix.yaml`.
///
/// muxix renders these into a project `.mcp.json` (Claude Code's native
/// format) which is propagated into each worktree, making the server available
/// to any agent that reads a project-level `.mcp.json`. The schema is
/// agent-agnostic so the same declaration can drive other agents' formats later.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct McpServerConfig {
    /// Executable that launches the server (e.g. `npx`).
    pub command: String,

    /// Arguments passed to `command` (e.g. `["-y", "socraticode"]`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,

    /// Environment variables set for the server process.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env: Option<BTreeMap<String, String>>,

    /// Whether the server is enabled. Defaults to `true`; set `false` to keep
    /// the declaration but omit it from the generated `.mcp.json`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,

    /// Agents this server applies to (short id like `pi` or lowercased display
    /// name like `claude code`). Absent means all agents. Caveat: Claude, pi,
    /// and omp share the project `.mcp.json`, so a server scoped to any one of
    /// them is still visible to the others through that file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agents: Option<Vec<String>>,

    /// What the server needs: npm packages muxix installs, executables it
    /// asserts on PATH. Lets `command` name the binary instead of `npx -y`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires: Option<crate::deps::Requires>,
}

impl McpServerConfig {
    /// Whether this server should be materialized (enabled defaults to true).
    pub fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }

    /// Whether this server applies to `agent` (no `agents` list means all).
    pub fn applies_to(&self, agent: crate::agent::setup::Agent) -> bool {
        match &self.agents {
            None => true,
            Some(keys) => keys.iter().any(|k| agent.matches_config_key(k)),
        }
    }
}

/// Configuration for agent status icons displayed in tmux window bar
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct StatusIcons {
    /// Icon shown when agent is working. Default: 🤖
    pub working: Option<String>,
    /// Icon shown when agent is waiting for input. Default: 💬
    pub waiting: Option<String>,
    /// Icon shown when agent is done. Default: ✅
    pub done: Option<String>,
}

impl StatusIcons {
    pub fn working(&self) -> &str {
        self.working.as_deref().unwrap_or("🤖")
    }

    pub fn waiting(&self) -> &str {
        self.waiting.as_deref().unwrap_or("💬")
    }

    pub fn done(&self) -> &str {
        self.done.as_deref().unwrap_or("✅")
    }
}

/// Configuration for LLM-based branch name generation
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct AutoNameConfig {
    /// Custom command to use instead of `llm` for branch name generation.
    /// The command string is split into program and arguments (e.g., "claude -p").
    /// The composed prompt is piped via stdin at execution time.
    /// When set, `model` is ignored.
    pub command: Option<String>,

    /// Model to use with llm CLI (e.g., "gpt-4o-mini", "claude-3-5-sonnet").
    /// If not set, uses llm's default model. Ignored when `command` is set.
    pub model: Option<String>,

    /// Custom system prompt for branch name generation.
    /// If not set, uses the default prompt that asks for a kebab-case branch name.
    pub system_prompt: Option<String>,

    /// Whether to always run in background mode when using --auto-name.
    /// If true, the window will be created but not focused.
    pub background: Option<bool>,
}

/// Configuration for dashboard actions (commit, merge keybindings)
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct DashboardConfig {
    /// Text to send to agent for commit action (c key).
    /// Default: "Commit staged changes with a descriptive message"
    pub commit: Option<String>,

    /// Text to send to agent for merge action (m key).
    /// Default: "!muxix merge"
    pub merge: Option<String>,

    /// Size of the preview pane as a percentage of terminal height (1-90).
    /// Default: 60 (60% for preview, 40% for table)
    pub preview_size: Option<u8>,

    /// Show check pass/total counts alongside check icon (default: false)
    #[serde(default)]
    pub show_check_counts: Option<bool>,
}

impl DashboardConfig {
    pub fn commit(&self) -> &str {
        self.commit
            .as_deref()
            .unwrap_or("Commit staged changes with a descriptive message")
    }

    pub fn merge(&self) -> &str {
        self.merge.as_deref().unwrap_or("!muxix merge")
    }

    /// Get the preview size percentage (clamped to 1-90, matching the
    /// documented `preview_size` range and the `--preview` CLI flag).
    /// Default: 60
    pub fn preview_size(&self) -> u8 {
        self.preview_size.unwrap_or(60).clamp(1, 90)
    }

    /// Whether to show check pass/total counts alongside check icons.
    /// Default: false
    pub fn show_check_counts(&self) -> bool {
        self.show_check_counts.unwrap_or(false)
    }
}

/// Per-mode template strings for sidebar rendering.
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct TemplatesConfig {
    /// Single-line template for compact mode.
    pub compact: Option<String>,
    /// Multi-line templates for tile mode (one string per line).
    pub tiles: Option<Vec<String>>,
    /// Multi-line templates for horizontal bar chips (one string per line).
    #[serde(alias = "top")]
    pub horizontal: Option<Vec<String>>,
}

/// Detailed per-agent icon override: `{ icon, color }`.
///
/// `deny_unknown_fields` catches typos like `colour:` instead of silently
/// dropping them.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentIconDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// Per-agent icon and color override.
///
/// Backwards compatible: a bare string (`claude: "C"`) parses as `Plain`.
/// Detailed form (`claude: { icon: "C", color: "#d97757" }`) parses as
/// `Detailed`. Bare key with no value (`claude:`) parses as `Null` and
/// behaves like no override.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum AgentIconConfig {
    Plain(String),
    Detailed(AgentIconDetails),
    Null,
}

impl AgentIconConfig {
    /// Icon override, if any.
    pub fn icon(&self) -> Option<&str> {
        match self {
            Self::Plain(s) => Some(s.as_str()),
            Self::Detailed(d) => d.icon.as_deref(),
            Self::Null => None,
        }
    }

    /// Color override, if any. The string is unparsed; callers parse and
    /// validate at config-load time.
    pub fn color(&self) -> Option<&str> {
        match self {
            Self::Plain(_) | Self::Null => None,
            Self::Detailed(d) => d.color.as_deref(),
        }
    }
}

/// Per-agent icon overrides. Maps agent kind (e.g. "claude", "codex") to
/// either a bare icon string or `{ icon, color }`.
pub type AgentIcons = BTreeMap<String, AgentIconConfig>;

/// Configuration for horizontal sidebar rendering.
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct HorizontalSidebarConfig {
    /// Maximum width of each horizontal item in columns. Default: 24.
    pub item_width: Option<u16>,
}

impl HorizontalSidebarConfig {
    pub fn item_width(&self) -> usize {
        self.item_width.unwrap_or(24).clamp(12, 80) as usize
    }
}

/// Configuration for the sidebar.
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct SidebarConfig {
    /// Position of the sidebar pane. Default: left.
    pub position: Option<SidebarPosition>,

    /// Width of the left sidebar. Can be an absolute column count (e.g. 40)
    /// or a percentage of terminal width (e.g. "15%").
    /// Default: "10%" (clamped to 25-50 columns)
    pub width: Option<SidebarWidth>,

    /// Height of the top sidebar. Can be an absolute row count (e.g. 3)
    /// or a percentage of terminal height (e.g. "10%").
    /// Default: "10%" (clamped to 1-5 rows)
    pub height: Option<SidebarHeight>,

    /// Layout mode: "compact" or "tiles". Default: "tiles"
    pub layout: Option<String>,

    /// Horizontal bar configuration.
    #[serde(default)]
    pub horizontal: HorizontalSidebarConfig,

    /// Custom templates for sidebar rendering.
    pub templates: Option<TemplatesConfig>,

    /// Per-agent icon overrides.
    pub agent_icons: Option<AgentIcons>,

    /// Default scope for the sidebar toggle command: "global" or "session".
    /// When "session", plain `wng sidebar` activates only the current session.
    /// Use `wng sidebar --global` to force global scope regardless of this setting.
    pub default_scope: Option<SidebarDefaultScope>,
}

/// Sidebar pane position.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SidebarPosition {
    #[default]
    Left,
    Top,
}

/// Default scope for the sidebar toggle command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SidebarDefaultScope {
    /// Sidebar toggle affects all sessions (current default).
    #[default]
    Global,
    /// Sidebar toggle affects only the current session.
    Session,
}

impl IdleDuration {
    pub fn as_duration(self) -> std::time::Duration {
        self.0
    }
}

/// Renders back into the human-readable form it parses, so a config that is
/// read and rewritten (merge resolver, `config resolve`) stays round-trippable.
/// Idle duration parsed from human-readable strings like "5m", "30s", "2h".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdleDuration(pub std::time::Duration);

impl std::fmt::Display for IdleDuration {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let secs = self.0.as_secs();
        if secs.is_multiple_of(3600) {
            write!(f, "{}h", secs / 3600)
        } else if secs.is_multiple_of(60) {
            write!(f, "{}m", secs / 60)
        } else {
            write!(f, "{secs}s")
        }
    }
}

impl Serialize for IdleDuration {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.to_string())
    }
}

impl<'de> serde::Deserialize<'de> for IdleDuration {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        parse_idle_duration(&s)
            .map(IdleDuration)
            .map_err(serde::de::Error::custom)
    }
}

fn parse_idle_duration(s: &str) -> Result<std::time::Duration, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty duration".into());
    }
    let mut total_secs: u64 = 0;
    let mut num_buf = String::new();
    for ch in s.chars() {
        match ch {
            '0'..='9' => num_buf.push(ch),
            'h' | 'H' => {
                let n: u64 = num_buf
                    .parse()
                    .map_err(|_| format!("invalid number in '{s}'"))?;
                total_secs += n * 3600;
                num_buf.clear();
            }
            'm' | 'M' => {
                let n: u64 = num_buf
                    .parse()
                    .map_err(|_| format!("invalid number in '{s}'"))?;
                total_secs += n * 60;
                num_buf.clear();
            }
            's' | 'S' => {
                let n: u64 = num_buf
                    .parse()
                    .map_err(|_| format!("invalid number in '{s}'"))?;
                total_secs += n;
                num_buf.clear();
            }
            _ => return Err(format!("unexpected character '{ch}' in duration '{s}'")),
        }
    }
    if !num_buf.is_empty() {
        return Err(format!("duration '{s}' has trailing number without unit"));
    }
    if total_secs == 0 {
        return Err(format!("duration '{s}' resolved to zero"));
    }
    Ok(std::time::Duration::from_secs(total_secs))
}

/// Daemon-level configuration (applies globally, not per-project).
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct DaemonConfig {
    /// Auto-freeze idle Done agents to reclaim CPU.
    #[serde(default)]
    pub auto_freeze: Option<AutoFreezeConfig>,
}

/// Configuration for the auto-freeze feature.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct AutoFreezeConfig {
    /// Freeze a Done agent's process tree after it has been idle (unfocused)
    /// for this long. The process is SIGSTOPed and SIGCONTed when its pane
    /// regains focus. Examples: "5m", "30s", "2h".
    pub after_idle: IdleDuration,
}

/// Sidebar width: either absolute columns or a percentage of terminal width.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidebarWidth {
    Absolute(u16),
    Percent(u16),
}

impl SidebarWidth {
    /// Resolve to an absolute column count given the terminal width.
    pub fn resolve(&self, terminal_width: u16) -> u16 {
        match self {
            SidebarWidth::Absolute(w) => *w,
            SidebarWidth::Percent(p) => {
                if terminal_width == 0 {
                    25
                } else {
                    // Compute in u32 to avoid u16 overflow on wide terminals
                    // (e.g. width 700 * 100 = 70000 > u16::MAX).
                    ((terminal_width as u32 * *p as u32) / 100) as u16
                }
            }
        }
    }
}

impl Serialize for SidebarWidth {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            SidebarWidth::Absolute(w) => serializer.serialize_u16(*w),
            SidebarWidth::Percent(p) => serializer.serialize_str(&format!("{}%", p)),
        }
    }
}

impl<'de> Deserialize<'de> for SidebarWidth {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de;

        struct SidebarWidthVisitor;

        impl<'de> de::Visitor<'de> for SidebarWidthVisitor {
            type Value = SidebarWidth;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a number (columns) or a string like \"15%\"")
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(SidebarWidth::Absolute(v as u16))
            }

            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                if v < 0 {
                    return Err(de::Error::custom("width cannot be negative"));
                }
                Ok(SidebarWidth::Absolute(v as u16))
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                if let Some(pct) = v.strip_suffix('%') {
                    let p: u16 = pct
                        .trim()
                        .parse()
                        .map_err(|_| de::Error::custom("invalid percentage"))?;
                    if p == 0 || p > 100 {
                        return Err(de::Error::custom("percentage must be 1-100"));
                    }
                    Ok(SidebarWidth::Percent(p))
                } else {
                    let w: u16 = v
                        .trim()
                        .parse()
                        .map_err(|_| de::Error::custom("invalid width"))?;
                    Ok(SidebarWidth::Absolute(w))
                }
            }
        }

        deserializer.deserialize_any(SidebarWidthVisitor)
    }
}

/// Sidebar height: either absolute rows or a percentage of terminal height.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidebarHeight {
    Absolute(u16),
    Percent(u16),
}

impl SidebarHeight {
    /// Resolve to an absolute row count given the terminal height.
    pub fn resolve(&self, terminal_height: u16) -> u16 {
        match self {
            SidebarHeight::Absolute(h) => *h,
            SidebarHeight::Percent(p) => {
                if terminal_height == 0 {
                    return 0;
                }
                // Compute in u32 to avoid u16 overflow on tall terminals.
                ((terminal_height as u32 * *p as u32) / 100) as u16
            }
        }
    }
}

impl Serialize for SidebarHeight {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            SidebarHeight::Absolute(h) => serializer.serialize_u16(*h),
            SidebarHeight::Percent(p) => serializer.serialize_str(&format!("{}%", p)),
        }
    }
}

impl<'de> Deserialize<'de> for SidebarHeight {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de;

        struct SidebarHeightVisitor;

        impl<'de> de::Visitor<'de> for SidebarHeightVisitor {
            type Value = SidebarHeight;

            fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
                formatter.write_str("a number (rows) or a string like \"10%\"")
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(SidebarHeight::Absolute(v as u16))
            }

            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                if v < 0 {
                    return Err(de::Error::custom("height cannot be negative"));
                }
                Ok(SidebarHeight::Absolute(v as u16))
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                if let Some(pct) = v.strip_suffix('%') {
                    let p: u16 = pct
                        .trim()
                        .parse()
                        .map_err(|_| de::Error::custom("invalid percentage"))?;
                    if p == 0 || p > 100 {
                        return Err(de::Error::custom("percentage must be 1-100"));
                    }
                    Ok(SidebarHeight::Percent(p))
                } else {
                    let h: u16 = v
                        .trim()
                        .parse()
                        .map_err(|_| de::Error::custom("invalid height"))?;
                    Ok(SidebarHeight::Absolute(h))
                }
            }
        }

        deserializer.deserialize_any(SidebarHeightVisitor)
    }
}

/// Configuration for a single window within a session (session mode only)
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct WindowConfig {
    /// Optional window name. If omitted, tmux auto-names based on running command.
    #[serde(default)]
    pub name: Option<String>,

    /// Panes within this window. Same schema as top-level `panes`.
    #[serde(default)]
    pub panes: Option<Vec<PaneConfig>>,
}

/// A declared agent config profile. The name is the map key; the source tree
/// lives at `~/.config/muxix/agent-profiles/<name>/` and holds the deltas
/// layered over the agent's base config dir.
#[derive(Debug, Deserialize, Serialize, Default, Clone, PartialEq, Eq)]
pub struct AgentProfile {
    /// Human-readable note shown in listings. Optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// Per-agent declarative deltas (keyed by agent id, e.g. `pi`), applied by
    /// generating the agent's settings file into the derived overlay. Agents
    /// without an entry get the plain file overlay.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub agents: BTreeMap<String, AgentProfileAgent>,
}

/// Declarative per-agent deltas for one profile.
#[derive(Debug, Deserialize, Serialize, Default, Clone, PartialEq, Eq)]
pub struct AgentProfileAgent {
    /// Plugin specs added to the base package list. Paths are absolutized
    /// (`~/` → home, relative → base agent dir); scheme specs pass verbatim.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub add_plugins: Vec<String>,

    /// Base package entries removed from the overlay (exact string match).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude_plugins: Vec<String>,

    /// Skill sources linked into the overlay at `skills/<basename>`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub add_skills: Vec<String>,

    /// Installed skill dir names omitted from the overlay's `skills/`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude_skills: Vec<String>,

    /// Prompt components added to the overlay's rendered prompt.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub add_prompt_components: Vec<String>,

    /// Prompt components removed from the overlay's rendered prompt.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude_prompt_components: Vec<String>,

    /// Feature keys whose per-agent artifacts (plugin spec and/or fallback
    /// prompt component) are dropped from the overlay.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude_features: Vec<String>,

    /// Agent-dir-relative paths (files or dirs) omitted from the overlay
    /// entirely — skills, extension files, anything base carries that this
    /// profile must not see. Exact path match, no globs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude_paths: Vec<String>,

    /// RFC 7386 merge patch applied to the generated settings file after the
    /// package delta: objects merge recursively, scalars/arrays replace,
    /// `null` deletes the key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settings: Option<serde_json::Value>,
}

impl AgentProfileAgent {
    /// True when no delta is declared — the overlay is a plain file merge.
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

/// Configuration for the muxix tool, read from .muxix.yaml
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct Config {
    /// Other config files merged in beneath this one, in declaration order.
    ///
    /// Directive, not configuration: the loader expands and strips it, so a
    /// fully-resolved config never carries it. Present on the struct so that a
    /// config using it still round-trips through serde.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include: Vec<include::IncludeEntry>,

    /// Named partial configs, overlaid on the resolved base when selected by
    /// `--profile`, `MUXIX_PROFILE`, or `default_profile`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub profiles: BTreeMap<String, serde_yaml::Value>,

    /// Profile(s) applied when nothing higher-precedence selects one.
    /// Comma-separated for several, applied left to right.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_profile: Option<String>,

    /// Named agent config profiles: lightweight, host-override overlays of an
    /// agent's skills/extensions/config, layered on top of the real config dir.
    /// Materialized by `muxix setup`; selected by `muxix exec --profile`.
    /// A different axis from `profiles:` (which layers *muxix* config).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub agent_profiles: BTreeMap<String, AgentProfile>,

    /// Agent profile `muxix exec` falls back to when `--profile` is omitted.
    /// Note: only `exec` honors this; a bare agent launched outside muxix
    /// still uses the untouched base config dir.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_agent_profile: Option<String>,

    /// The primary branch to merge into (optional, auto-detected if not set)
    #[serde(default)]
    pub main_branch: Option<String>,

    /// Default base branch/commit to branch from when creating new worktrees.
    /// Used as fallback when --base is not passed to `muxix add`.
    #[serde(default)]
    pub base_branch: Option<String>,

    /// Directory where worktrees should be created (optional, defaults to <project>__worktrees pattern)
    /// Can be relative to repo root or absolute path
    #[serde(default)]
    pub worktree_dir: Option<String>,

    /// Prefix for tmux window names (optional, defaults to "mx-")
    #[serde(default)]
    pub window_prefix: Option<String>,

    /// Tmux pane configuration (single window layout, mutually exclusive with `windows`)
    #[serde(default)]
    pub panes: Option<Vec<PaneConfig>>,

    /// Named pane layouts, selectable with `-l/--layout`.
    #[serde(default)]
    pub layouts: Option<HashMap<String, LayoutConfig>>,

    /// Multiple window configuration (session mode only, mutually exclusive with `panes`)
    #[serde(default)]
    pub windows: Option<Vec<WindowConfig>>,

    /// Commands to run after creating the worktree
    /// How `muxix start` multiplexes tracked projects.
    #[serde(default)]
    pub project_mux: Option<ProjectMux>,

    /// Which worktrees `muxix start` / `muxix project open` restore.
    /// Default: `active` (only worktrees that have agent state).
    #[serde(default)]
    pub project_open: Option<ProjectOpenFilter>,

    /// Window in days for `project_open: recent`. Default: 7.
    #[serde(default)]
    pub project_open_days: Option<u64>,

    #[serde(default)]
    pub post_create: Option<Vec<String>>,

    /// Commands to run before merging (e.g., linting, tests)
    #[serde(default)]
    pub pre_merge: Option<Vec<String>>,

    /// Commands to run before removing the worktree (e.g., for backups)
    #[serde(default)]
    pub pre_remove: Option<Vec<String>>,

    /// The agent command to use (e.g., "claude", "gemini")
    #[serde(default)]
    pub agent: Option<String>,

    /// Spawn worktrees with the same agent muxix is running inside.
    ///
    /// When enabled, an agent detected from the environment (see
    /// `agent::identity::detect_parent_agent`) takes precedence over `agent`,
    /// so running `muxix add` from inside Claude Code spawns Claude Code
    /// rather than the configured default. An explicit `--agent` flag still
    /// wins. Defaults to false — without it, `agent` is used as before.
    #[serde(default)]
    pub inherit_agent: Option<bool>,

    /// Per-project agent selection by path regex, first match wins.
    ///
    /// Matched against the project's *main worktree* root, so every worktree
    /// of a project resolves the same agent. Ranks below an `agent:` set by a
    /// project config or profile and above the global `agent:`. Global-only:
    /// a rule names a command muxix will execute.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agent_rules: Vec<AgentRule>,

    /// Default merge strategy for `muxix merge`
    #[serde(default)]
    pub merge_strategy: Option<MergeStrategy>,

    /// Keep worktree, window, and branch by default after `muxix merge`
    #[serde(default)]
    pub merge_keep: Option<bool>,

    /// Strategy for deriving worktree/window names from branch names
    #[serde(default)]
    pub worktree_naming: WorktreeNaming,

    /// Prefix for worktree directory and window names
    #[serde(default)]
    pub worktree_prefix: Option<String>,

    /// File operations to perform after creating the worktree
    #[serde(default)]
    pub files: FileConfig,

    /// Whether to auto-apply muxix status to tmux window format.
    /// Default: true
    #[serde(default)]
    pub status_format: Option<bool>,

    /// Custom icons for agent status display.
    #[serde(default)]
    pub status_icons: StatusIcons,

    /// Configuration for LLM-based branch name generation
    #[serde(default)]
    pub auto_name: Option<AutoNameConfig>,

    /// Dashboard actions configuration
    #[serde(default)]
    pub dashboard: DashboardConfig,

    /// Sidebar configuration
    #[serde(default)]
    pub sidebar: SidebarConfig,

    /// Whether to use nerdfont icons (None = prompt user on first run)
    #[serde(default)]
    pub nerdfont: Option<bool>,

    /// Color theme for the dashboard
    #[serde(default)]
    pub theme: ThemeConfig,

    /// Mode for tmux operations: window (default) or session
    /// None means "use default" (Window), Some means explicitly set
    #[serde(default)]
    pub mode: Option<MuxMode>,

    /// Automatically check for updates in the background. Default: true
    #[serde(default)]
    pub auto_update_check: Option<bool>,

    /// Write prompt files without injecting into agent commands.
    /// Useful when your editor has an embedded agent that reads prompt files directly.
    #[serde(default)]
    pub prompt_file_only: Option<bool>,

    /// Named agent commands. Maps short names to command strings or
    /// `{ command, type }` objects. Global-only for security.
    #[serde(default)]
    pub agents: BTreeMap<String, AgentEntry>,

    /// Resolved agent type override from the agents map.
    /// Set internally during config loading, not deserialized.
    #[serde(skip)]
    pub agent_type: Option<String>,

    /// What decided `agent`. Set during loading so `muxix config agent which`
    /// reports the real decision instead of re-deriving it.
    #[serde(skip)]
    pub agent_source: Option<AgentSource>,

    /// Container sandbox configuration
    #[serde(default)]
    pub sandbox: SandboxConfig,

    /// Submodule worktree strategy configuration
    #[serde(default)]
    pub submodules: SubmoduleConfig,

    /// MCP (Model Context Protocol) servers to expose to agents in this project.
    /// Rendered into a project `.mcp.json` (Claude format) by `muxix mcp sync`
    /// and propagated into each worktree. Reusable for any MCP server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp: Option<BTreeMap<String, McpServerConfig>>,

    /// Agent bootstrap configuration: uniform setup for plugins, skills, and prompts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bootstrap: Option<crate::bootstrap::BootstrapConfig>,

    /// Unified, provider-centric model registry: declare each provider (source)
    /// and the models it serves, with context/compaction limits, once. Resolved
    /// by provider id or logical name everywhere (a model can come from several
    /// providers).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub providers: Option<crate::model::ProviderRegistry>,

    /// Proxy chain configuration for per-worktree agentgateway + RTK.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_chain: Option<crate::proxy::ProxyChainConfig>,

    /// Pipeline event tracing controls (`muxix::event`): level, master switch, and
    /// per-kind/group enable/disable.
    #[serde(default)]
    pub events: EventsConfig,

    /// Named agent capability profiles (inline definitions). Resolved by
    /// `agent_ref:` in harness YAML nodes. Local overrides beat file-based defs.
    /// Alternatively declare them in `.muxix/agents/<name>.yaml` files.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub agent_defs: BTreeMap<String, crate::agent::definition::AgentDefinition>,

    /// Named prompt templates (inline). Resolved by `prompt_ref:` in agent
    /// definitions or directly in harness nodes. Files in `.muxix/prompts/`
    /// are also scanned (inline wins on conflict).
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub prompt_defs: BTreeMap<String, crate::prompt::PromptTemplate>,

    /// Registry sources for resolving agent definitions and prompts by name.
    /// `.muxix/agents/` and `.muxix/prompts/` are always searched first.
    /// Additional sources (git repos, URLs) are searched in declaration order.
    /// Phase 1: only `local:` sources are supported; git/url are stubs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub agent_registries: Vec<crate::agent::registry::RegistrySource>,

    /// Default agent runtime for this project: the name of an `ade` entry, or
    /// `local` (the default) to run agents in a worktree and tmux pane here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_runtime: Option<String>,

    /// Agent-development environments muxix can drive, by name.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub ade: BTreeMap<String, AdeConfig>,

    /// Daemon-level settings (sidebar daemon, auto-freeze, etc.).
    /// Global-only: project configs cannot override these.
    #[serde(default)]
    pub daemon: DaemonConfig,

    /// Enterprise provision server settings. Global-only — project config cannot
    /// override this to prevent a malicious repo from redirecting policy fetch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provision: Option<crate::provision::ProvisionConfig>,
}

#[allow(dead_code)]
fn default_true() -> bool {
    true
}

/// Kill-switches for the OpenSpec integration. Both default to on; disabling
/// `enabled` reverts the loop to pre-OpenSpec behavior, and disabling
/// `writeback` keeps the import while never touching the user's tasks.md.
#[derive(Debug, Deserialize, Serialize, Clone)]
#[allow(dead_code)]
pub struct OpenspecConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub writeback: bool,
}

impl Default for OpenspecConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            writeback: true,
        }
    }
}

/// Ordering strategy for ready tasks.
#[derive(Debug, Deserialize, Serialize, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum SelectionOrder {
    /// Preserve task-graph file order (current behaviour).
    #[default]
    Graph,
    /// Highest `priority` first; ties keep graph order.
    Priority,
    /// Stable ascending order by task id.
    Fifo,
}

#[allow(dead_code)]
fn default_max_runtime() -> IdleDuration {
    IdleDuration(std::time::Duration::from_secs(6 * 3600))
}

/// Merge policy for completed tasks in the loop.
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
#[allow(dead_code)]
pub struct LoopMergeConfig {
    /// Whether the loop merges completed tasks automatically. When unset, the
    /// `--auto-merge` CLI flag decides. When false, tasks stop at `done`.
    #[serde(default)]
    pub auto_merge: Option<bool>,

    /// Merge strategy for loop merges. Falls back to the top-level
    /// `merge_strategy`, then a standard merge commit.
    #[serde(default)]
    pub strategy: Option<MergeStrategy>,

    /// What to do when a deterministic merge hits conflicts.
    #[serde(default)]
    pub on_conflict: ConflictPolicy,

    /// Shell commands run in the feature worktree before each task merge.
    #[serde(default)]
    pub pre_merge: Vec<String>,
}

/// How the loop reacts to a merge conflict during its deterministic merge step.
#[derive(Debug, Deserialize, Serialize, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code)]
pub enum ConflictPolicy {
    /// Spawn an agent in the worktree to resolve the conflict (default).
    #[default]
    Agent,
    /// Mark the task blocked/failed and surface it for manual handling.
    Manual,
}

/// Configuration for submodule worktree management.
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct SubmoduleConfig {
    /// When true, muxix creates/removes/merges matching worktrees for each
    /// git submodule alongside the parent worktree. Default: false.
    #[serde(default)]
    pub worktrees: bool,
}

/// Which step of the precedence chain supplied the resolved agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentSource {
    /// The `--agent` flag.
    Flag,
    /// An `agent:` from a project config or a selected profile.
    ProjectConfig,
    /// An `agent_rules` entry, by index and pattern.
    Rule { index: usize, pattern: String },
    /// Detected from the parent process under `inherit_agent`.
    Inherited,
    /// The global config's `agent:`.
    Global,
    /// The built-in fallback.
    Builtin,
}

impl std::fmt::Display for AgentSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Flag => write!(f, "--agent flag"),
            Self::ProjectConfig => write!(f, "project config or profile"),
            Self::Rule { index, pattern } => write!(f, "agent_rules[{index}] match {pattern:?}"),
            Self::Inherited => write!(f, "inherited from parent agent"),
            Self::Global => write!(f, "global config agent:"),
            Self::Builtin => write!(f, "built-in default"),
        }
    }
}

/// One `agent_rules` entry: a path regex and the agent to use when it matches.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct AgentRule {
    /// Regex matched against the project's main worktree root path. A leading
    /// `~/` is expanded to the home directory. Unanchored: `wng-workspace`
    /// matches any path containing it.
    #[serde(rename = "match")]
    pub pattern: String,
    /// Agent name (a key of `agents`) or a bare command.
    pub agent: String,
}

impl AgentRule {
    /// Compile the pattern with `~/` expanded.
    ///
    /// The tilde is expanded after an optional leading `^`, since an anchored
    /// home-relative pattern (`^~/repos/work/`) is the common hand-written form.
    pub fn compile(&self) -> Result<regex::Regex, regex::Error> {
        let (anchor, rest) = match self.pattern.strip_prefix('^') {
            Some(rest) => ("^", rest),
            None => ("", self.pattern.as_str()),
        };
        let expanded = match (rest.strip_prefix("~/"), home::home_dir()) {
            (Some(tail), Some(home)) => {
                format!("{anchor}{}/{tail}", regex::escape(&home.to_string_lossy()))
            }
            _ => self.pattern.clone(),
        };
        regex::Regex::new(&expanded)
    }
}

/// First rule whose pattern matches `path`, with its index. Patterns that fail
/// to compile are reported on stderr and skipped: one bad rule must not take
/// the others down.
pub fn match_agent_rule<'a>(rules: &'a [AgentRule], path: &Path) -> Option<(usize, &'a AgentRule)> {
    let path = path.to_string_lossy();
    rules
        .iter()
        .enumerate()
        .find(|(_, rule)| match rule.compile() {
            Ok(re) => re.is_match(&path),
            Err(e) => {
                eprintln!("muxix: invalid agent_rules pattern {:?}: {e}", rule.pattern);
                false
            }
        })
}

/// A named agent entry: either a plain command string or a `{ command, type }` object.
///
/// Deserializes from:
/// - `"claude --flags"` (string shorthand)
/// - `{ command: "/path/to/wrapper", type: "claude" }` (explicit type override)
#[derive(Debug, Clone, Serialize)]
pub struct AgentEntry {
    pub command: String,
    /// Explicit agent type override for profile detection.
    /// When set, profile resolution uses this instead of the executable stem.
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
}

impl<'de> Deserialize<'de> for AgentEntry {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum RawEntry {
            String(String),
            Map {
                command: String,
                #[serde(rename = "type")]
                agent_type: Option<String>,
            },
        }

        match RawEntry::deserialize(deserializer)? {
            RawEntry::String(s) => Ok(AgentEntry {
                command: s,
                agent_type: None,
            }),
            RawEntry::Map {
                command,
                agent_type,
            } => Ok(AgentEntry {
                command,
                agent_type,
            }),
        }
    }
}

/// Configuration for a single tmux pane
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct PaneConfig {
    /// A command to run when the pane is created. The pane will remain open
    /// with an interactive shell after the command completes. If not provided,
    /// the pane will start with the default shell.
    #[serde(default)]
    pub command: Option<String>,

    /// Whether this pane should receive focus after creation
    #[serde(default)]
    pub focus: bool,

    /// Split direction from the previous pane (horizontal or vertical)
    #[serde(default)]
    pub split: Option<SplitDirection>,

    /// The size of the new pane in lines (for vertical splits) or cells (for horizontal splits).
    /// Mutually exclusive with `percentage`.
    #[serde(default)]
    pub size: Option<u16>,

    /// The size of the new pane as a percentage of the available space.
    /// Mutually exclusive with `size`.
    #[serde(default)]
    pub percentage: Option<u8>,

    /// The 0-based index of the pane to split.
    /// If not specified, splits the most recently created pane.
    /// Only used when `split` is specified.
    #[serde(default)]
    pub target: Option<usize>,

    /// Whether this pane should be zoomed (fullscreen) after creation.
    /// Implies `focus: true`.
    #[serde(default)]
    pub zoom: bool,
}

/// A named pane layout, selectable with `-l/--layout` at add-time.
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct LayoutConfig {
    /// Pane configuration for this layout.
    pub panes: Vec<PaneConfig>,
}

#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SplitDirection {
    Horizontal,
    Vertical,
}

#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum MergeStrategy {
    #[default]
    Merge,
    Rebase,
    Squash,
}

/// Dark or light mode for the dashboard
#[derive(Debug, Serialize, Clone, Copy, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    #[default]
    Dark,
    Light,
}

impl<'de> serde::Deserialize<'de> for ThemeMode {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        match s.to_lowercase().as_str() {
            "light" => Ok(ThemeMode::Light),
            _ => Ok(ThemeMode::Dark),
        }
    }
}

/// Named color scheme for the dashboard
#[derive(Debug, Serialize, Clone, Copy, Default, PartialEq, Eq)]
pub enum ThemeScheme {
    #[default]
    Default,
    Emberforge,
    GlacierSignal,
    ObsidianPop,
    SlateGarden,
    PhosphorArcade,
    Lasergrid,
    Mossfire,
    NightSorbet,
    GraphiteCode,
    FestivalCircuit,
    TealDrift,
}

impl ThemeScheme {
    pub const ALL: [ThemeScheme; 12] = [
        ThemeScheme::Default,
        ThemeScheme::Emberforge,
        ThemeScheme::GlacierSignal,
        ThemeScheme::ObsidianPop,
        ThemeScheme::SlateGarden,
        ThemeScheme::PhosphorArcade,
        ThemeScheme::Lasergrid,
        ThemeScheme::Mossfire,
        ThemeScheme::NightSorbet,
        ThemeScheme::GraphiteCode,
        ThemeScheme::FestivalCircuit,
        ThemeScheme::TealDrift,
    ];

    pub fn next(self) -> Self {
        let idx = Self::ALL.iter().position(|&s| s == self).unwrap_or(0);
        Self::ALL[(idx + 1) % Self::ALL.len()]
    }

    #[allow(dead_code)]
    pub fn name(&self) -> &'static str {
        match self {
            ThemeScheme::Default => "Default",
            ThemeScheme::Emberforge => "Emberforge",
            ThemeScheme::GlacierSignal => "Glacier Signal",
            ThemeScheme::ObsidianPop => "Obsidian Pop",
            ThemeScheme::SlateGarden => "Slate Garden",
            ThemeScheme::PhosphorArcade => "Phosphor Arcade",
            ThemeScheme::Lasergrid => "Lasergrid",
            ThemeScheme::Mossfire => "Mossfire",
            ThemeScheme::NightSorbet => "Night Sorbet",
            ThemeScheme::GraphiteCode => "Graphite Code",
            ThemeScheme::FestivalCircuit => "Festival Circuit",
            ThemeScheme::TealDrift => "Teal Drift",
        }
    }

    pub fn slug(&self) -> &'static str {
        match self {
            ThemeScheme::Default => "default",
            ThemeScheme::Emberforge => "emberforge",
            ThemeScheme::GlacierSignal => "glacier-signal",
            ThemeScheme::ObsidianPop => "obsidian-pop",
            ThemeScheme::SlateGarden => "slate-garden",
            ThemeScheme::PhosphorArcade => "phosphor-arcade",
            ThemeScheme::Lasergrid => "lasergrid",
            ThemeScheme::Mossfire => "mossfire",
            ThemeScheme::NightSorbet => "night-sorbet",
            ThemeScheme::GraphiteCode => "graphite-code",
            ThemeScheme::FestivalCircuit => "festival-circuit",
            ThemeScheme::TealDrift => "teal-drift",
        }
    }

    pub fn from_slug(s: &str) -> Option<Self> {
        Self::ALL.iter().find(|v| v.slug() == s).copied()
    }
}

impl<'de> serde::Deserialize<'de> for ThemeScheme {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        Ok(Self::from_slug(&s.to_lowercase()).unwrap_or_default())
    }
}

/// Custom color overrides for the theme palette.
/// Each field corresponds to a `ThemePalette` field and accepts a CSS hex color (e.g. "#51afef").
/// Shorthand aliases: `bg` for `current_row_bg`, `fg` for `text`, `error` for `danger`.
#[derive(Debug, Serialize, Deserialize, Clone, Default, PartialEq, Eq)]
pub struct CustomThemeColors {
    #[serde(default, alias = "bg")]
    pub current_row_bg: Option<String>,
    #[serde(default)]
    pub highlight_row_bg: Option<String>,
    #[serde(default)]
    pub current_worktree_fg: Option<String>,
    #[serde(default)]
    pub dimmed: Option<String>,
    #[serde(default, alias = "fg")]
    pub text: Option<String>,
    #[serde(default)]
    pub border: Option<String>,
    #[serde(default)]
    pub help_border: Option<String>,
    #[serde(default)]
    pub help_muted: Option<String>,
    #[serde(default)]
    pub header: Option<String>,
    #[serde(default)]
    pub keycap: Option<String>,
    #[serde(default)]
    pub info: Option<String>,
    #[serde(default)]
    pub success: Option<String>,
    #[serde(default)]
    pub warning: Option<String>,
    #[serde(default, alias = "error")]
    pub danger: Option<String>,
    #[serde(default)]
    pub accent: Option<String>,
}

/// Theme configuration: scheme + optional mode override + custom color overrides.
/// Supports deserializing from:
///   - `theme: emberforge` (scheme name, auto-detect mode)
///   - `theme: dark` or `theme: light` (legacy mode override)
///   - `theme: { scheme: emberforge, mode: dark }` (structured)
///   - `theme: { scheme: emberforge, custom: { accent: "#51afef" } }` (with overrides)
#[derive(Debug, Serialize, Clone, Default, PartialEq, Eq)]
pub struct ThemeConfig {
    pub scheme: ThemeScheme,
    /// None = auto-detect from terminal background
    pub mode: Option<ThemeMode>,
    /// Custom color overrides applied on top of the base scheme
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom: Option<CustomThemeColors>,
}

impl<'de> serde::Deserialize<'de> for ThemeConfig {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de;

        struct ThemeVisitor;

        impl<'de> de::Visitor<'de> for ThemeVisitor {
            type Value = ThemeConfig;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("a theme scheme name, \"dark\", \"light\", or a {scheme, mode} map")
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<ThemeConfig, E> {
                let lower = v.to_lowercase();
                match lower.as_str() {
                    "dark" => Ok(ThemeConfig {
                        scheme: ThemeScheme::Default,
                        mode: Some(ThemeMode::Dark),
                        custom: None,
                    }),
                    "light" => Ok(ThemeConfig {
                        scheme: ThemeScheme::Default,
                        mode: Some(ThemeMode::Light),
                        custom: None,
                    }),
                    _ => Ok(ThemeConfig {
                        scheme: ThemeScheme::from_slug(&lower).unwrap_or_default(),
                        mode: None,
                        custom: None,
                    }),
                }
            }

            fn visit_map<M: de::MapAccess<'de>>(self, mut map: M) -> Result<ThemeConfig, M::Error> {
                let mut scheme = None;
                let mut mode = None;
                let mut custom = None;
                while let Some(key) = map.next_key::<String>()? {
                    match key.as_str() {
                        "scheme" => {
                            let s: String = map.next_value()?;
                            scheme = ThemeScheme::from_slug(&s.to_lowercase());
                        }
                        "mode" => {
                            let s: String = map.next_value()?;
                            mode = Some(match s.to_lowercase().as_str() {
                                "light" => ThemeMode::Light,
                                _ => ThemeMode::Dark,
                            });
                        }
                        "custom" => {
                            custom = Some(map.next_value::<CustomThemeColors>()?);
                        }
                        _ => {
                            let _ = map.next_value::<serde::de::IgnoredAny>()?;
                        }
                    }
                }
                Ok(ThemeConfig {
                    scheme: scheme.unwrap_or_default(),
                    mode,
                    custom,
                })
            }
        }

        d.deserialize_any(ThemeVisitor)
    }
}

/// Mode for multiplexer operations: create windows within the current session or create new sessions
#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum MuxMode {
    /// Create windows within the current tmux session (default)
    #[default]
    Window,
    /// Create new tmux sessions for each worktree
    Session,
}

/// Strategy for deriving worktree/window names from branch names
#[derive(Debug, Deserialize, Serialize, Clone, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum WorktreeNaming {
    /// Use the full branch name (slashes become dashes after slugification)
    #[default]
    Full,
    /// Use only the part after the last `/` (e.g., `prj-123/feature` → `feature`)
    Basename,
}

/// Sandbox backend type
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SandboxBackend {
    /// Docker/Podman containers (default)
    #[default]
    Container,
    /// Lima VM backend
    Lima,
    /// Microsandbox microVM backend (libkrun, no TAP networking required)
    MicroSandbox,
}

/// Checkpoint strategy for sandbox agents.
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum CheckpointStrategy {
    /// Checkpoint before switching permission modes (e.g. plan → implement).
    #[default]
    ModeSwitch,
    /// Checkpoint at a fixed time interval (requires `interval_secs`).
    Periodic,
    /// Only checkpoint when explicitly requested via CLI.
    Manual,
}

/// Checkpoint/restore configuration for sandbox agents.
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct CheckpointConfig {
    /// Enable checkpointing. Default: false
    #[serde(default)]
    pub enabled: Option<bool>,

    /// Checkpoint strategy. Default: mode-switch
    #[serde(default)]
    pub strategy: CheckpointStrategy,

    /// Interval in seconds between periodic checkpoints. Only used when
    /// strategy is `periodic`.
    #[serde(default)]
    pub interval_secs: Option<u64>,

    /// Directory to store checkpoint snapshots.
    /// Default: `~/.local/state/muxix/checkpoints/`
    #[serde(default)]
    pub dir: Option<String>,

    /// Maximum number of snapshots to keep per agent sandbox.
    /// Older snapshots are deleted automatically after each new checkpoint.
    /// Default: 1 (keep only the most recent snapshot).
    /// Set to 0 to disable pruning entirely.
    #[serde(default)]
    pub keep: Option<usize>,
}

impl CheckpointConfig {
    pub fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(false)
    }

    /// Number of snapshots to retain per sandbox. 0 means unlimited.
    pub fn keep(&self) -> usize {
        self.keep.unwrap_or(1)
    }
}

/// Microsandbox-specific configuration.
#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct MicroSandboxConfig {
    /// Container image to use (e.g. `ubuntu:22.04`).
    #[serde(default)]
    pub image: Option<String>,

    /// Number of vCPUs per sandbox VM. Default: 2
    #[serde(default)]
    pub cpus: Option<u32>,

    /// Memory per sandbox VM (e.g. `4G`). Default: `4G`
    #[serde(default)]
    pub memory: Option<String>,

    /// Disk size per sandbox VM (e.g. `20G`). Default: `20G`
    #[serde(default)]
    pub disk: Option<String>,
}

impl MicroSandboxConfig {
    pub fn resolved_image(&self) -> &str {
        self.image.as_deref().unwrap_or("ubuntu:22.04")
    }

    pub fn cpus(&self) -> u32 {
        self.cpus.unwrap_or(2)
    }

    #[allow(dead_code)]
    pub fn memory(&self) -> &str {
        self.memory.as_deref().unwrap_or("4G")
    }
}

/// Container runtime for sandbox
#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[serde(rename_all = "lowercase")]
pub enum SandboxRuntime {
    /// Docker
    Docker,
    /// Podman (default — its CRIU checkpoint/restore is what powers agent
    /// resume; requires a rootful runtime, see `detect`).
    #[default]
    Podman,
    /// Apple Container (macOS only, uses `container` binary)
    #[serde(rename = "apple-container")]
    AppleContainer,
}

impl SandboxRuntime {
    /// Auto-detect container runtime by checking PATH.
    ///
    /// On macOS, prefers Apple Container (`container`) over Docker/Podman.
    /// On Linux, prefers Podman over Docker: Podman is the default runtime for
    /// agent sandboxes because its CRIU checkpoint/restore is what enables
    /// seamless agent resume. Note this requires Podman to run *rootful*
    /// (rootless Podman refuses to checkpoint: "checkpointing a container
    /// requires root") — run muxix as root or point it at a rootful Podman
    /// service. Falls back to Podman if nothing is found (fails later with a
    /// clear "command not found" error).
    pub fn detect() -> Self {
        #[cfg(target_os = "macos")]
        if which("container").is_ok() {
            return SandboxRuntime::AppleContainer;
        }

        if which("podman").is_ok() {
            SandboxRuntime::Podman
        } else if which("docker").is_ok() {
            SandboxRuntime::Docker
        } else {
            debug!("no container runtime found in PATH, defaulting to podman");
            SandboxRuntime::Podman
        }
    }

    /// Returns the binary name for this runtime.
    pub fn binary_name(&self) -> &'static str {
        match self {
            SandboxRuntime::Docker => "docker",
            SandboxRuntime::Podman => "podman",
            SandboxRuntime::AppleContainer => "container",
        }
    }

    /// Human-readable name for user-facing messages.
    pub fn display_name(&self) -> &'static str {
        match self {
            SandboxRuntime::Docker => "docker",
            SandboxRuntime::Podman => "podman",
            SandboxRuntime::AppleContainer => "apple-container",
        }
    }

    /// Whether this runtime needs `--add-host host.docker.internal:host-gateway`.
    /// Only Docker requires this.
    pub fn needs_add_host(&self) -> bool {
        matches!(self, SandboxRuntime::Docker)
    }

    /// Whether this runtime needs `--userns=keep-id`.
    /// Only Podman requires this.
    pub fn needs_userns_keep_id(&self) -> bool {
        matches!(self, SandboxRuntime::Podman)
    }

    /// Whether this runtime needs `--cap-add=NET_ADMIN` and `--security-opt
    /// no-new-privileges` in network deny mode. Apple Container runs each
    /// container as a full VM where root already has all capabilities.
    pub fn needs_deny_mode_caps(&self) -> bool {
        matches!(self, SandboxRuntime::Docker | SandboxRuntime::Podman)
    }

    /// Whether this runtime supports binding individual files (not just directories).
    /// Apple Container only supports directory mounts via virtiofs.
    pub fn supports_file_mounts(&self) -> bool {
        !matches!(self, SandboxRuntime::AppleContainer)
    }

    /// Returns the arguments for pulling an image.
    /// Apple Container uses `image pull`, others use `pull`.
    pub fn pull_args(&self, image: &str) -> Vec<String> {
        match self {
            SandboxRuntime::AppleContainer => {
                vec!["image".into(), "pull".into(), image.into()]
            }
            _ => vec!["pull".into(), image.into()],
        }
    }

    /// Returns the default hostname that a container guest should use to reach the host.
    ///
    /// - Docker: `host.docker.internal` (Docker Desktop built-in)
    /// - Podman: `host.containers.internal` (Podman built-in)
    /// - Apple Container: `192.168.64.1` (default gateway for Apple VMs)
    pub fn rpc_host_address(&self) -> &'static str {
        match self {
            SandboxRuntime::Docker => "host.docker.internal",
            SandboxRuntime::Podman => "host.containers.internal",
            SandboxRuntime::AppleContainer => "192.168.64.1",
        }
    }

    /// Returns the default memory limit for this runtime, if one should be applied
    /// when the user hasn't configured an explicit value.
    ///
    /// Apple Container defaults to 1 GB RAM per VM which is insufficient for most
    /// workloads. Since memory is a ceiling (not an upfront allocation), a generous
    /// default is safe. Docker/Podman use host resources directly and don't need this.
    pub fn default_memory(&self) -> Option<&'static str> {
        match self {
            SandboxRuntime::AppleContainer => Some("16G"),
            _ => None,
        }
    }

    /// Returns the serde name for this runtime (used for state store serialization).
    pub fn serde_name(&self) -> &'static str {
        match self {
            SandboxRuntime::Docker => "docker",
            SandboxRuntime::Podman => "podman",
            SandboxRuntime::AppleContainer => "apple-container",
        }
    }

    /// Parse a runtime from its serde name. Returns None for unrecognized values.
    pub fn from_serde_name(s: &str) -> Option<Self> {
        match s {
            "docker" => Some(SandboxRuntime::Docker),
            "podman" => Some(SandboxRuntime::Podman),
            "apple-container" => Some(SandboxRuntime::AppleContainer),
            _ => None,
        }
    }
}

/// Isolation level for Lima backend
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum IsolationLevel {
    /// Single shared VM for all projects (fastest)
    Shared,
    /// One VM per git repository (default, balanced)
    #[default]
    Project,
}

/// Which panes to sandbox
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum SandboxTarget {
    /// Only sandbox agent panes (default, recommended)
    #[default]
    Agent,
    /// Sandbox all panes
    All,
}

/// Toolchain integration mode for Lima sandboxes.
/// Controls whether devbox.json/flake.nix are detected and used
/// to wrap agent commands with the appropriate environment.
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ToolchainMode {
    /// Auto-detect devbox.json or flake.nix and wrap commands (default)
    #[default]
    Auto,
    /// Disable toolchain integration
    Off,
    /// Force Devbox mode (use devbox.json)
    Devbox,
    /// Force Nix flake mode (use flake.nix)
    Flake,
}

/// An extra mount point for the sandbox.
///
/// Supports two forms:
/// - Simple string: `"~/my-notes"` (read-only, mirrored path)
/// - Detailed spec: `{ host_path: "~/data", guest_path: "/mnt/data", writable: true }`
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
#[serde(untagged)]
pub enum ExtraMount {
    /// Simple host path (read-only, guest path mirrors host path)
    Path(String),
    /// Detailed mount specification
    Spec {
        host_path: String,
        #[serde(default)]
        guest_path: Option<String>,
        #[serde(default)]
        writable: Option<bool>,
    },
}

impl ExtraMount {
    /// Resolve the mount to (host_path, guest_path, read_only).
    /// Expands `~` in host_path to the user's home directory.
    /// Returns an error if host_path or guest_path is not absolute after expansion.
    pub fn resolve(&self) -> anyhow::Result<(PathBuf, PathBuf, bool)> {
        let (host_str, guest_str, writable) = match self {
            Self::Path(p) => (p.as_str(), None, false),
            Self::Spec {
                host_path,
                guest_path,
                writable,
            } => (
                host_path.as_str(),
                guest_path.as_deref(),
                writable.unwrap_or(false),
            ),
        };

        let host_path = crate::util::expand_tilde(host_str);
        if !host_path.is_absolute() {
            anyhow::bail!(
                "extra_mounts: host path must be absolute (got '{}'). Use an absolute path or ~/.",
                host_str
            );
        }

        let guest_path = guest_str
            .map(PathBuf::from)
            .unwrap_or_else(|| host_path.clone());
        if !guest_path.is_absolute() {
            anyhow::bail!(
                "extra_mounts: guest_path must be absolute (got '{}')",
                guest_str.unwrap_or("")
            );
        }

        let read_only = !writable;
        Ok((host_path, guest_path, read_only))
    }
}

/// Lima-specific sandbox configuration.
/// Nested under `sandbox.lima` in YAML.
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct LimaConfig {
    /// Isolation level. Default: project
    #[serde(default)]
    pub isolation: Option<IsolationLevel>,

    /// Projects directory for shared isolation (required when isolation: shared)
    #[serde(default)]
    pub projects_dir: Option<PathBuf>,

    /// Number of CPUs for Lima VMs. Default: 4 (Lima default)
    #[serde(default)]
    pub cpus: Option<u32>,

    /// Memory for Lima VMs (e.g. "4GiB", "8GiB"). Default: "4GiB" (Lima default)
    #[serde(default)]
    pub memory: Option<String>,

    /// Disk size for Lima VMs (e.g. "100GiB"). Default: "100GiB" (Lima default)
    #[serde(default)]
    pub disk: Option<String>,

    /// Custom user provision script run once during Lima VM creation,
    /// after built-in system and user provisioning steps.
    /// Runs as user (not root). Use `sudo` for system-level commands.
    #[serde(default)]
    pub provision: Option<String>,

    /// Skip built-in provisioning scripts (system dependencies and tool installation).
    /// Useful when using a custom image that already has everything pre-installed.
    /// Custom `provision` script still runs if specified.
    #[serde(default)]
    pub skip_default_provision: Option<bool>,
}

impl LimaConfig {
    pub fn isolation(&self) -> IsolationLevel {
        self.isolation.clone().unwrap_or_default()
    }

    pub fn cpus(&self) -> u32 {
        self.cpus.unwrap_or(4)
    }

    pub fn memory(&self) -> &str {
        self.memory.as_deref().unwrap_or("4GiB")
    }

    pub fn disk(&self) -> &str {
        self.disk.as_deref().unwrap_or("100GiB")
    }

    pub fn provision_script(&self) -> Option<&str> {
        self.provision.as_deref().filter(|s| !s.trim().is_empty())
    }

    pub fn skip_default_provision(&self) -> bool {
        self.skip_default_provision.unwrap_or(false)
    }
}

/// Host device mapping for container sandboxes.
///
/// Supports two YAML forms:
/// - string: `"/dev/kvm"`, `"/dev/dri:/dev/dri"`, `"/dev/dri:/dev/dri:rwm"`
/// - struct: `{ host_path, guest_path?, permissions? }`
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
#[serde(untagged)]
pub enum ContainerDevice {
    String(String),
    Struct {
        host_path: String,
        #[serde(default)]
        guest_path: Option<String>,
        #[serde(default)]
        permissions: Option<String>,
    },
}

impl ContainerDevice {
    /// Render as the value for a `--device` flag.
    pub fn to_arg(&self) -> String {
        match self {
            Self::String(s) => s.clone(),
            Self::Struct {
                host_path,
                guest_path,
                permissions,
            } => {
                let gp = guest_path.as_deref().unwrap_or(host_path.as_str());
                match permissions.as_deref() {
                    Some(p) => format!("{host_path}:{gp}:{p}"),
                    None if guest_path.is_some() => format!("{host_path}:{gp}"),
                    None => host_path.clone(),
                }
            }
        }
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        match self {
            Self::String(s) => {
                if s.is_empty() {
                    anyhow::bail!("empty device entry");
                }
                if s.chars().any(char::is_whitespace) {
                    anyhow::bail!("device entry must not contain whitespace: {s}");
                }
                let mut parts = s.split(':');
                let host = parts.next().unwrap_or("");
                if !host.starts_with('/') {
                    anyhow::bail!("device host path must be absolute: {s}");
                }
                if let Some(second) = parts.next() {
                    let third = parts.next();
                    if parts.next().is_some() {
                        anyhow::bail!("device entry has too many ':' separators: {s}");
                    }
                    // second is either a guest path (absolute) or a permissions token.
                    // third, if present, must be a permissions token.
                    let is_perms = |tok: &str| {
                        !tok.is_empty() && tok.chars().all(|c| matches!(c, 'r' | 'w' | 'm'))
                    };
                    match third {
                        Some(perms) => {
                            if !second.starts_with('/') {
                                anyhow::bail!("device guest path must be absolute: {s}");
                            }
                            if !is_perms(perms) {
                                anyhow::bail!(
                                    "invalid device permissions (expected subset of r/w/m): {s}"
                                );
                            }
                        }
                        None => {
                            if !second.starts_with('/') && !is_perms(second) {
                                anyhow::bail!(
                                    "device entry second token must be an absolute path or r/w/m permissions: {s}"
                                );
                            }
                        }
                    }
                }
                Ok(())
            }
            Self::Struct {
                host_path,
                guest_path,
                permissions,
            } => {
                if host_path.is_empty()
                    || !host_path.starts_with('/')
                    || host_path.chars().any(char::is_whitespace)
                {
                    anyhow::bail!(
                        "device host_path must be an absolute path with no whitespace: {host_path}"
                    );
                }
                if let Some(gp) = guest_path
                    && (gp.is_empty()
                        || !gp.starts_with('/')
                        || gp.chars().any(char::is_whitespace))
                {
                    anyhow::bail!(
                        "device guest_path must be an absolute path with no whitespace: {gp}"
                    );
                }
                if let Some(p) = permissions
                    && (p.is_empty() || !p.chars().all(|c| matches!(c, 'r' | 'w' | 'm')))
                {
                    anyhow::bail!("invalid device permissions (expected subset of r/w/m): {p}");
                }
                Ok(())
            }
        }
    }
}

pub(crate) fn validate_group_add_entry(group: &str) -> anyhow::Result<()> {
    let trimmed = group.trim();
    if trimmed.is_empty() {
        anyhow::bail!("empty group_add entry");
    }
    if trimmed
        .chars()
        .any(|c| c.is_whitespace() || c == ',' || c == ':')
    {
        anyhow::bail!("group_add entry must not contain whitespace or separators: {group}");
    }
    Ok(())
}

/// Container-specific sandbox configuration.
/// Nested under `sandbox.container` in YAML.
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct ContainerConfig {
    /// Container runtime. Auto-detected from PATH if not set.
    #[serde(default)]
    pub runtime: Option<SandboxRuntime>,

    /// Number of CPUs for the container. Only passed when explicitly set.
    /// Apple Container defaults to 4 CPUs which is sufficient for most workloads.
    #[serde(default)]
    pub cpus: Option<u32>,

    /// Memory limit for the container (e.g. "8G", "16G").
    /// For Apple Container, defaults to "16G" when not set (the VM's 1 GB default
    /// is too low). For Docker/Podman, only passed when explicitly set.
    #[serde(default)]
    pub memory: Option<String>,

    /// Host device nodes exposed to the container sandbox (docker `--device`).
    /// Global-only: ignored in project config.
    #[serde(default)]
    pub devices: Option<Vec<ContainerDevice>>,

    /// Supplementary groups added to the sandboxed process
    /// (docker `--group-add`). Values can be group names or numeric GIDs.
    /// Global-only: ignored in project config.
    #[serde(default)]
    pub group_add: Option<Vec<String>>,

    /// Files (relative to the worktree root) to mask out of the container's
    /// worktree bind mounts. Each path is shadowed by bind-mounting `/dev/null`
    /// over it so agents running inside the container cannot read the host
    /// file. Useful for keeping `.env` and other secret-bearing files out of
    /// the sandbox without restructuring the project.
    ///
    /// Masking applies to both the current worktree and, where applicable, the
    /// main-worktree mount (which muxix adds for symlink resolution), so a
    /// symlinked secret cannot be read via the alias path.
    ///
    /// Only existing regular files are masked; missing paths are skipped with
    /// a warning. Directories are not supported.
    ///
    /// Security: this field is global-only. It is ignored when set in a
    /// project's `.muxix.yaml`, and muxix fails fast rather than running
    /// with excluded_files on a runtime that lacks file-level bind mounts
    /// (Apple Container).
    #[serde(default)]
    pub excluded_files: Option<Vec<String>>,
}

impl ContainerConfig {
    pub fn runtime(&self) -> SandboxRuntime {
        self.runtime.unwrap_or_else(SandboxRuntime::detect)
    }

    pub fn devices(&self) -> &[ContainerDevice] {
        self.devices.as_deref().unwrap_or(&[])
    }

    pub fn group_add(&self) -> &[String] {
        self.group_add.as_deref().unwrap_or(&[])
    }

    /// Files to mask out of the worktree bind mount (relative to worktree root).
    /// Returns empty slice when unset.
    pub fn excluded_files(&self) -> &[String] {
        self.excluded_files.as_deref().unwrap_or(&[])
    }

    /// Structural validation for hardware access fields. Called at config load.
    pub fn validate(&self) -> anyhow::Result<()> {
        for d in self.devices() {
            d.validate()?;
        }
        for g in self.group_add() {
            validate_group_add_entry(g)?;
        }
        Ok(())
    }
}

/// Network restriction policy for sandboxed containers.
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum NetworkPolicy {
    /// No network restrictions (default).
    Allow,
    /// Block all outbound except whitelisted domains via CONNECT proxy.
    Deny,
}

/// Detailed allowed domain rule: `{ host, allow_private_ips }`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AllowedDomainDetails {
    pub host: String,
    #[serde(default)]
    pub allow_private_ips: bool,
}

/// Allowed outbound HTTPS domain entry.
///
/// Backwards compatible: a bare string parses as a public-only rule.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(untagged)]
pub enum AllowedDomainEntry {
    Plain(String),
    Detailed(AllowedDomainDetails),
}

impl AllowedDomainEntry {
    pub fn host(&self) -> &str {
        match self {
            Self::Plain(host) => host,
            Self::Detailed(details) => &details.host,
        }
    }

    pub fn allow_private_ips(&self) -> bool {
        match self {
            Self::Plain(_) => false,
            Self::Detailed(details) => details.allow_private_ips,
        }
    }
}

/// Runtime form of an allowed domain rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllowedDomainRule {
    pub host: String,
    pub allow_private_ips: bool,
}

/// Network restriction configuration for the container sandbox.
///
/// When `policy` is `deny`, all outbound connections are blocked except those
/// to whitelisted domains via an HTTP CONNECT proxy. An iptables firewall
/// inside the container enforces that only the proxy and RPC ports are
/// reachable, preventing bypass via direct connections.
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct NetworkConfig {
    /// Network restriction policy. Default: allow (no restrictions).
    /// Set to "deny" to block all outbound except whitelisted domains.
    #[serde(default)]
    pub policy: Option<NetworkPolicy>,

    /// Allowed outbound HTTPS domains when policy is "deny".
    /// Supports exact matches and wildcard prefixes (e.g., "*.googleapis.com").
    /// The host RPC endpoint is always allowed regardless of this list.
    #[serde(default)]
    pub allowed_domains: Option<Vec<AllowedDomainEntry>>,
}

impl NetworkConfig {
    /// Get the effective network policy. Default: Allow.
    pub fn policy(&self) -> NetworkPolicy {
        self.policy.clone().unwrap_or(NetworkPolicy::Allow)
    }

    /// Get the allowed domains list (empty if not set).
    pub fn allowed_domains(&self) -> &[AllowedDomainEntry] {
        self.allowed_domains.as_deref().unwrap_or(&[])
    }

    /// Get normalized allowed domain rules.
    pub fn allowed_domain_rules(&self) -> Vec<AllowedDomainRule> {
        self.allowed_domains()
            .iter()
            .map(|entry| AllowedDomainRule {
                host: entry.host().to_string(),
                allow_private_ips: entry.allow_private_ips(),
            })
            .collect()
    }

    /// Validate all domain entries. Called at config load time.
    pub fn validate(&self) -> anyhow::Result<()> {
        for entry in self.allowed_domains() {
            let host = entry.host();
            validate_domain(host)?;
            if entry.allow_private_ips() && host.starts_with("*.") {
                anyhow::bail!(
                    "allow_private_ips is only allowed for exact domains: {}",
                    host
                );
            }
        }
        Ok(())
    }
}

/// Validate a single domain entry for the allowed_domains list.
fn validate_domain(domain: &str) -> anyhow::Result<()> {
    use std::net::IpAddr;
    // Reject IP literals
    if domain.parse::<IpAddr>().is_ok() {
        anyhow::bail!("IP literals not allowed in allowed_domains: {}", domain);
    }
    // Reject trailing dots
    if domain.ends_with('.') {
        anyhow::bail!("trailing dot not allowed in domain: {}", domain);
    }
    // Wildcard must be *.suffix form only
    if domain.contains('*') && !domain.starts_with("*.") {
        anyhow::bail!("invalid wildcard pattern (must be *.suffix): {}", domain);
    }
    // Empty domains
    if domain.is_empty() {
        anyhow::bail!("empty domain not allowed in allowed_domains");
    }
    Ok(())
}

/// Configuration for structured pipeline event tracing (target `muxix::event`).
///
/// `MUXIX_EVENTS` / `RUST_LOG` set the level; this section lets `.muxix.yaml`
/// set that level too and, beyond it, silence specific event kinds or whole
/// groups. See `docs/reference/events.md`.
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct EventsConfig {
    /// Master switch. Default: true. `false` silences all `muxix::event` output.
    #[serde(default)]
    pub enabled: Option<bool>,

    /// Trace level for `muxix::event`: `off` | `info` | `debug` | `trace`.
    /// Applied only when `MUXIX_EVENTS` is unset (env wins).
    #[serde(default)]
    pub level: Option<String>,

    /// Allowlist of event kinds/groups to emit. Non-empty => only these.
    /// Group-aware: `pane.probe` matches `pane.probe.*`.
    #[serde(default)]
    pub only: Vec<String>,

    /// Denylist of event kinds/groups to silence (applied after `only`).
    /// Group-aware: `pane.probe` matches `pane.probe.*`.
    #[serde(default)]
    pub disable: Vec<String>,
}

impl EventsConfig {
    /// Whether event tracing is enabled. Default: true.
    pub fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(true)
    }

    /// Build the runtime per-kind filter installed into the event system.
    pub fn to_filter(&self) -> crate::signals::event::EventFilter {
        crate::signals::event::EventFilter {
            enabled: self.is_enabled(),
            only: self.only.clone(),
            disable: self.disable.clone(),
        }
    }
}

/// Configuration for sandboxing (Container or Lima)
#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct SandboxConfig {
    /// Enable sandboxing. Default: false
    #[serde(default)]
    pub enabled: Option<bool>,

    /// Sandbox backend. Default: container
    #[serde(default)]
    pub backend: Option<SandboxBackend>,

    /// Which panes to sandbox. Default: agent
    #[serde(default)]
    pub target: Option<SandboxTarget>,

    /// Container/VM image. For containers: Docker image name.
    /// For Lima: qcow2 image URL or file:// path.
    #[serde(default)]
    pub image: Option<String>,

    /// Environment variables to pass to sandbox.
    /// Default: []
    #[serde(default)]
    pub env_passthrough: Option<Vec<String>>,

    /// Environment variables to set in the sandbox with explicit values.
    /// Unlike env_passthrough (which reads from host), these are set directly.
    /// Global-only: project config cannot set this to prevent a sandboxed agent
    /// from injecting env vars into its next session via .muxix.yaml.
    #[serde(default)]
    pub env: Option<HashMap<String, String>>,

    /// Override the hostname used by containers to reach the host RPC server.
    /// Defaults to `host.docker.internal` (Docker) or `host.containers.internal` (Podman).
    /// Useful for non-standard Podman or custom networking setups.
    #[serde(default)]
    pub rpc_host: Option<String>,

    /// Toolchain integration mode for sandboxes.
    /// Controls automatic detection and use of devbox.json/flake.nix.
    /// Default: auto (detect and wrap automatically)
    #[serde(default)]
    pub toolchain: Option<ToolchainMode>,

    /// Commands to proxy from guest to host via host-exec RPC.
    /// When set, shims are created in the guest VM that forward these
    /// commands to the host's toolchain environment.
    #[serde(default)]
    pub host_commands: Option<Vec<String>>,

    /// Extra mount points for the sandbox.
    /// Paths are mounted read-only by default. Supports simple string paths
    /// or detailed specs with guest_path and writable options.
    #[serde(default)]
    pub extra_mounts: Option<Vec<ExtraMount>>,

    /// Custom host directory for agent config (mounted instead of the default).
    /// Supports `{agent}` placeholder, e.g. `~/sandbox-config/{agent}`.
    /// When not set, defaults to the agent's standard config directory
    /// (e.g. `~/.claude/`, `~/.gemini/`).
    #[serde(default)]
    pub agent_config_dir: Option<String>,

    /// Lima-specific configuration
    #[serde(default)]
    pub lima: LimaConfig,

    /// Container-specific configuration
    #[serde(default)]
    pub container: ContainerConfig,

    /// Microsandbox-specific configuration (microVM backend via libkrun).
    #[serde(default)]
    pub microsandbox: MicroSandboxConfig,

    /// Checkpoint/restore configuration for sandbox agents.
    #[serde(default)]
    pub checkpoint: CheckpointConfig,

    /// Network restriction configuration (container backend only).
    #[serde(default)]
    pub network: NetworkConfig,

    /// Allow host-exec to run without bwrap sandboxing on Linux.
    /// Default: false (fail closed -- refuse to run if bwrap is missing).
    /// When true, falls back to unsandboxed execution with a warning.
    #[serde(default)]
    pub dangerously_allow_unsandboxed_host_exec: Option<bool>,
}

impl SandboxConfig {
    pub fn is_enabled(&self) -> bool {
        self.enabled.unwrap_or(false)
    }

    pub fn backend(&self) -> SandboxBackend {
        self.backend.clone().unwrap_or_default()
    }

    pub fn runtime(&self) -> SandboxRuntime {
        self.container.runtime()
    }

    pub fn target(&self) -> SandboxTarget {
        self.target.clone().unwrap_or_default()
    }

    /// Get the image name, falling back to the default ghcr.io image for the agent.
    ///
    /// `agent` must be a canonical agent name (e.g. "claude", "codex"), not a raw
    /// command string. Use `resolve_profile().name()` to obtain it.
    pub fn resolved_image(&self, agent: &str) -> String {
        match &self.image {
            Some(image) => image.clone(),
            None => format!("{}:{}", crate::sandbox::DEFAULT_IMAGE_REGISTRY, agent),
        }
    }

    pub fn env_passthrough(&self) -> Vec<&str> {
        self.env_passthrough
            .as_ref()
            .map(|v| v.iter().map(|s| s.as_str()).collect())
            .unwrap_or_default()
    }

    /// Get explicit environment variables to set in the sandbox.
    pub fn env_vars(&self) -> Vec<(&str, &str)> {
        self.env
            .as_ref()
            .map(|m| m.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect())
            .unwrap_or_default()
    }

    /// Get the RPC host address, using config override or runtime default.
    pub fn resolved_rpc_host(&self) -> String {
        self.rpc_host
            .clone()
            .unwrap_or_else(|| self.runtime().rpc_host_address().to_string())
    }

    pub fn toolchain(&self) -> ToolchainMode {
        self.toolchain.clone().unwrap_or_default()
    }

    pub fn host_commands(&self) -> &[String] {
        self.host_commands.as_deref().unwrap_or(&[])
    }

    pub fn extra_mounts(&self) -> &[ExtraMount] {
        self.extra_mounts.as_deref().unwrap_or(&[])
    }

    pub fn allow_unsandboxed_host_exec(&self) -> bool {
        self.dangerously_allow_unsandboxed_host_exec
            .unwrap_or(false)
    }

    /// Returns true if network policy is deny (restrictions active).
    pub fn network_policy_is_deny(&self) -> bool {
        self.network.policy() == NetworkPolicy::Deny
    }

    /// Returns the resolved agent config directory path for the given agent.
    /// Performs `{agent}` substitution and tilde expansion on the configured path.
    /// Falls back to the agent's default config directory when not configured.
    pub fn resolved_agent_config_dir(&self, agent: &str) -> Option<PathBuf> {
        if let Some(ref dir) = self.agent_config_dir {
            let expanded = dir.replace("{agent}", agent);
            Some(crate::util::expand_tilde(&expanded))
        } else {
            use crate::agent::setup::{copilot, omp, pi, prime};
            let home = home::home_dir()?;
            match agent {
                "claude" => Some(home.join(".claude")),
                // Honors COPILOT_HOME / PI_CODING_AGENT_DIR respectively, so a
                // relocated base is the base a profile overlays.
                "copilot" => copilot::copilot_home(),
                "omp" => omp::agent_dir(),
                "prime-agent" => prime::agent_dir(),
                "pi" => pi::agent_dir(),
                "gemini" => Some(home.join(".gemini")),
                "codex" => Some(home.join(".codex")),
                "opencode" => Some(home.join(".local/share/opencode")),
                _ => None,
            }
        }
    }
}

/// Result of config discovery, including the relative path from repo root
#[derive(Debug, Clone)]
pub struct ConfigLocation {
    /// Absolute path to the config file
    pub config_path: PathBuf,
    /// Absolute path to the directory containing the config
    pub config_dir: PathBuf,
    /// Relative path from repo root to config dir (e.g., "backend" for backend/.muxix.yaml)
    /// Empty if config is at repo root
    pub rel_dir: PathBuf,
}

/// Find the nearest .muxix.yaml by walking up from start_dir to repo root.
/// Returns ConfigLocation with the relative path computed at discovery time.
pub fn find_project_config(start_dir: &Path) -> anyhow::Result<Option<ConfigLocation>> {
    let config_names = [".muxix.yaml", ".muxix.yml"];

    let repo_root = match git::get_repo_root_for(start_dir) {
        Ok(root) => root,
        Err(_) => return Ok(None),
    };

    // Canonicalize both paths to handle symlinks and ensure consistent comparison
    let repo_root = repo_root.canonicalize().unwrap_or(repo_root);
    let mut dir = start_dir
        .canonicalize()
        .unwrap_or_else(|_| start_dir.to_path_buf());

    // Safety: ensure we're inside the repo
    if !dir.starts_with(&repo_root) {
        return Ok(None);
    }

    // Walk upward from start_dir to repo_root (inclusive)
    loop {
        for name in &config_names {
            let candidate = dir.join(name);
            if candidate.exists() {
                let rel_dir = dir
                    .strip_prefix(&repo_root)
                    .map(|p| p.to_path_buf())
                    .unwrap_or_default();
                debug!(
                    path = %candidate.display(),
                    rel_dir = %rel_dir.display(),
                    "config:found project config"
                );
                return Ok(Some(ConfigLocation {
                    config_path: candidate,
                    config_dir: dir,
                    rel_dir,
                }));
            }
        }
        if dir == repo_root {
            break;
        }
        if !dir.pop() {
            break;
        }
    }

    // Fallback: check main worktree root (preserves existing behavior for linked worktrees)
    if let Ok(main_root) = git::get_main_worktree_root() {
        let main_root = main_root.canonicalize().unwrap_or(main_root);
        if main_root != repo_root {
            for name in &config_names {
                let candidate = main_root.join(name);
                if candidate.exists() {
                    debug!(path = %candidate.display(), "config:found main-worktree config");
                    return Ok(Some(ConfigLocation {
                        config_path: candidate,
                        config_dir: main_root.clone(),
                        rel_dir: PathBuf::new(), // Main worktree root = empty rel_dir
                    }));
                }
            }
        }
    }

    Ok(None)
}

/// Locate the `.muxix/` state/workflow directory for a project.
///
/// Uses `find_project_config` to find the `.muxix.yaml` config file and returns
/// its sibling `.muxix/` directory. Falls back to `project_dir/.muxix` when no
/// config file is found (e.g. in a bare checkout or a repo that hasn't run `muxix init`).
pub fn find_muxix_dir(project_dir: &Path) -> std::path::PathBuf {
    if let Ok(Some(loc)) = find_project_config(project_dir) {
        loc.config_dir.join(".muxix")
    } else {
        project_dir.join(".muxix")
    }
}

impl WorktreeNaming {
    /// Derive a name from a branch name using this strategy
    pub fn derive_name(&self, branch: &str) -> String {
        match self {
            Self::Full => branch.to_string(),
            Self::Basename => branch
                .trim_end_matches('/')
                .rsplit('/')
                .next()
                .unwrap_or(branch)
                .to_string(),
        }
    }
}

/// Validate windows configuration
pub fn validate_windows_config(windows: &[WindowConfig]) -> anyhow::Result<()> {
    if windows.is_empty() {
        anyhow::bail!("'windows' list must not be empty.");
    }
    for (i, window) in windows.iter().enumerate() {
        if let Some(panes) = &window.panes {
            validate_panes_config(panes).map_err(|e| {
                anyhow::anyhow!(
                    "Window {} ({}): {}",
                    i,
                    window.name.as_deref().unwrap_or("unnamed"),
                    e
                )
            })?;
        }
    }
    Ok(())
}

/// Validate pane configuration
pub fn validate_panes_config(panes: &[PaneConfig]) -> anyhow::Result<()> {
    for (i, pane) in panes.iter().enumerate() {
        if i == 0 {
            // First pane cannot have a split or size
            if pane.split.is_some() {
                anyhow::bail!("First pane (index 0) cannot have a 'split' direction.");
            }
            if pane.size.is_some() || pane.percentage.is_some() {
                anyhow::bail!("First pane (index 0) cannot have 'size' or 'percentage'.");
            }
        } else {
            // Subsequent panes must have a split
            if pane.split.is_none() {
                anyhow::bail!("Pane {} must have a 'split' direction specified.", i);
            }
        }

        // size and percentage are mutually exclusive
        if pane.size.is_some() && pane.percentage.is_some() {
            anyhow::bail!(
                "Pane {} cannot have both 'size' and 'percentage' specified.",
                i
            );
        }

        // Validate percentage range
        if let Some(p) = pane.percentage
            && !(1..=100).contains(&p)
        {
            anyhow::bail!(
                "Pane {} has invalid percentage {}. Must be between 1 and 100.",
                i,
                p
            );
        }

        // If target is specified, validate it's a valid index
        if let Some(target) = pane.target
            && target >= i
        {
            anyhow::bail!(
                "Pane {} has invalid target {}. Target must reference a previously created pane (0-{}).",
                i,
                target,
                i.saturating_sub(1)
            );
        }
    }

    // Only one pane can have zoom
    let zoom_count = panes.iter().filter(|p| p.zoom).count();
    if zoom_count > 1 {
        anyhow::bail!(
            "Only one pane can have 'zoom: true' (found {}).",
            zoom_count
        );
    }

    Ok(())
}

/// Validate layouts configuration by validating each layout's panes.
#[cfg(test)]
pub fn validate_layouts_config(layouts: &HashMap<String, LayoutConfig>) -> anyhow::Result<()> {
    for (name, layout) in layouts {
        validate_panes_config(&layout.panes)
            .map_err(|e| anyhow::anyhow!("Invalid panes in layout '{}': {}", name, e))?;
    }
    Ok(())
}

/// Get the path to the global config file.
///
/// Resolves via `$XDG_CONFIG_HOME/muxix/` (default `~/.config/muxix/`).
/// If a custom `XDG_CONFIG_HOME` is set and no config exists there yet,
/// falls back to the legacy `~/.config/muxix/` location for reading.
/// Prefers existing .yml file to avoid shadowing, otherwise defaults to .yaml.
pub fn global_config_path() -> Option<PathBuf> {
    let xdg_dir = crate::xdg::config_dir().ok()?;
    let yaml = xdg_dir.join("config.yaml");
    let yml = xdg_dir.join("config.yml");

    // Check XDG location first
    if yml.exists() && !yaml.exists() {
        return Some(yml);
    }
    if yaml.exists() {
        return Some(yaml);
    }

    // Legacy fallback: if XDG_CONFIG_HOME points elsewhere, check ~/.config/muxix/
    if let Some(home) = home::home_dir() {
        let legacy_dir = home.join(".config/muxix");
        if legacy_dir != xdg_dir {
            let legacy_yml = legacy_dir.join("config.yml");
            let legacy_yaml = legacy_dir.join("config.yaml");
            if legacy_yml.exists() && !legacy_yaml.exists() {
                return Some(legacy_yml);
            }
            if legacy_yaml.exists() {
                return Some(legacy_yaml);
            }
        }
    }

    // Default to XDG path for new config files
    Some(yaml)
}

/// How `muxix start` multiplexes tracked projects.
#[derive(Debug, Deserialize, Serialize, Clone, Copy, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProjectMux {
    /// One tmux session per tracked project (the only strategy for now).
    #[default]
    Session,
}

/// Which worktrees `muxix start` / `muxix project open` restore.
#[derive(Debug, Deserialize, Serialize, Clone, Copy, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectOpenFilter {
    /// Every worktree of the project.
    All,
    /// Worktrees whose agent state shows work stopped mid-flight: no
    /// `completion` and a status other than `done`.
    Unfinished,
    /// Worktrees that have any agent state at all.
    #[default]
    Active,
    /// Worktrees touched within `project_open_days` days.
    Recent,
}

impl ProjectOpenFilter {
    /// Name as written in config and `--worktrees`, for skip messages.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Unfinished => "unfinished",
            Self::Active => "active",
            Self::Recent => "recent",
        }
    }
}

/// Which way project records flow between muxix and an ADE.
#[derive(Debug, Deserialize, Serialize, Clone, Copy, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SyncDirection {
    /// No syncing at all. The default: syncing edits two registries at once,
    /// so it is opt-in.
    #[default]
    Off,
    /// Adopt the ADE's projects into muxix.
    Pull,
    /// Publish muxix's projects to the ADE.
    Push,
    Bidirectional,
}

/// What to do when both sides disagree about the same project.
///
/// Distinct from [`ConflictPolicy`], which is about git merge conflicts in the
/// orchestrate loop.
#[derive(Debug, Deserialize, Serialize, Clone, Copy, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum SyncConflictPolicy {
    /// Report and change nothing. The default: a wrong automatic resolution
    /// here silently renames or drops a project.
    #[default]
    Manual,
    Muxix,
    Ade,
    Newest,
}

/// Project-sync policy for one ADE.
///
/// The defaults are deliberately timid: identity is the canonical root path,
/// removals do not propagate, and names are not compared. Naive bidirectional
/// sync is how registries lose entries or ping-pong forever.
#[derive(Debug, Deserialize, Serialize, Clone, Default, PartialEq, Eq)]
pub struct ProjectSyncConfig {
    #[serde(default)]
    pub direction: SyncDirection,
    #[serde(default)]
    pub conflict: SyncConflictPolicy,
    /// Propagate removals. Only ever inferred from a recorded last-synced set.
    #[serde(default)]
    pub removals: bool,
    /// Treat a differing display name as a conflict.
    #[serde(default)]
    pub names: bool,
}

/// How an ADE exposes its project registry.
#[derive(Debug, Deserialize, Serialize, Clone, Default, PartialEq, Eq)]
pub struct AdeProjectsConfig {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub list_args: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub add_args: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub remove_args: Vec<String>,
    /// JSON field holding a project's display name.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name_field: String,
    /// JSON field holding a project's root path -- the identity muxix syncs on.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub root_field: String,
    #[serde(default)]
    pub sync: ProjectSyncConfig,
}

impl AdeProjectsConfig {
    /// Overlay this block on a preset, field by field.
    ///
    /// A user block tweaks a preset rather than replacing it: blanking the
    /// preset's command lists because the user only wanted to set a sync
    /// policy leaves the ADE looking broken with no visible cause.
    pub fn merge_over(mut self, preset: &AdeProjectsConfig) -> AdeProjectsConfig {
        if self.list_args.is_empty() {
            self.list_args = preset.list_args.clone();
        }
        if self.add_args.is_empty() {
            self.add_args = preset.add_args.clone();
        }
        if self.remove_args.is_empty() {
            self.remove_args = preset.remove_args.clone();
        }
        if self.name_field.is_empty() {
            self.name_field = preset.name_field.clone();
        }
        if self.root_field.is_empty() {
            self.root_field = preset.root_field.clone();
        }
        self
    }
}

/// Which of an ADE's own status names map onto each neutral status.
///
/// `failed` is separate from `done` on purpose: a manager reporting an error
/// must never render as successful completion. An unmapped state becomes
/// `unknown` rather than a guess.
#[derive(Debug, Deserialize, Serialize, Clone, Default, PartialEq, Eq)]
pub struct AdeStatusMap {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub starting: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub working: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub waiting: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub done: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub failed: Vec<String>,
}

/// An agent-development-environment muxix drives through its CLI.
///
/// Driving the CLI rather than a wire protocol keeps zero protocol code here
/// and survives the ADE's schema churn, at the cost of polled status instead of
/// pushed events -- the same trade muxix already makes with tmux and git.
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Eq)]
pub struct AdeConfig {
    /// Executable to invoke.
    #[serde(default)]
    pub command: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub start_args: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub send_args: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub list_args: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stop_args: Vec<String>,
    /// Adopt an already-running agent into muxix.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub import_args: Vec<String>,
    /// JSON field holding an agent's id.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id_field: String,
    /// JSON field holding an agent's status.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub status_field: String,
    #[serde(default)]
    pub status_map: AdeStatusMap,
    /// Whether the ADE creates and owns the worktree itself.
    #[serde(default)]
    pub owns_worktree: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projects: Option<AdeProjectsConfig>,
}

/// Defaults to the paseo preset rather than an empty struct.
///
/// An `AdeConfig` with no status map maps every state to `unknown`, which
/// renders every agent as statusless in the UI. Deserialization is unaffected:
/// serde uses each field's own `#[serde(default)]`, not this impl.
impl Default for AdeConfig {
    fn default() -> Self {
        Self::paseo()
    }
}

impl AdeConfig {
    /// Overlay this block on a preset, field by field. See
    /// [`AdeProjectsConfig::merge_over`] for why this merges rather than
    /// replaces.
    pub fn merge_over(mut self, preset: &AdeConfig) -> AdeConfig {
        if self.command.is_empty() {
            self.command = preset.command.clone();
        }
        for (mine, theirs) in [
            (&mut self.start_args, &preset.start_args),
            (&mut self.send_args, &preset.send_args),
            (&mut self.list_args, &preset.list_args),
            (&mut self.stop_args, &preset.stop_args),
            (&mut self.import_args, &preset.import_args),
        ] {
            if mine.is_empty() {
                *mine = theirs.clone();
            }
        }
        if self.id_field.is_empty() {
            self.id_field = preset.id_field.clone();
        }
        if self.status_field.is_empty() {
            self.status_field = preset.status_field.clone();
        }
        if self.status_map == AdeStatusMap::default() {
            self.status_map = preset.status_map.clone();
        }
        self.projects = match (self.projects.take(), preset.projects.as_ref()) {
            (Some(mine), Some(theirs)) => Some(mine.merge_over(theirs)),
            (mine, theirs) => mine.or_else(|| theirs.cloned()),
        };
        self
    }

    /// The built-in paseo preset.
    pub fn paseo() -> AdeConfig {
        let a = |v: &[&str]| v.iter().map(|s| (*s).to_string()).collect::<Vec<_>>();
        AdeConfig {
            command: "paseo".to_string(),
            start_args: a(&["agent", "run", "--json", "--cwd", "{cwd}", "{prompt}"]),
            send_args: a(&["agent", "send", "{id}", "{text}"]),
            list_args: a(&["agent", "ls", "--json"]),
            stop_args: a(&["agent", "stop", "{id}"]),
            import_args: a(&["agent", "ls", "--json"]),
            id_field: "id".to_string(),
            status_field: "status".to_string(),
            status_map: AdeStatusMap {
                starting: a(&["initializing"]),
                working: a(&["running", "thinking"]),
                waiting: a(&["idle", "waiting", "needs_input"]),
                done: a(&["closed", "completed"]),
                failed: a(&["error"]),
            },
            owns_worktree: true,
            projects: Some(AdeProjectsConfig {
                list_args: a(&["project", "ls", "--json"]),
                add_args: a(&["project", "create", "{name}", "{root}"]),
                remove_args: a(&["project", "delete", "{name}"]),
                name_field: "name".to_string(),
                root_field: "root".to_string(),
                sync: ProjectSyncConfig::default(),
            }),
        }
    }

    /// Every built-in preset, by name.
    pub fn presets() -> BTreeMap<String, AdeConfig> {
        BTreeMap::from([("paseo".to_string(), AdeConfig::paseo())])
    }
}

/// Top-level keys a config may declare that are not `Config` fields.
///
/// `include`, `profiles`, and `default_profile` are directives consumed by the
/// loader; they are `Config` fields too, so they are covered by the field list
/// itself and listed here only for clarity.
const DIRECTIVE_KEYS: &[&str] = &["include", "profiles", "default_profile"];

/// The cached org policy, when one is usable.
///
/// Fresh applies silently; stale applies with a warning; expired contributes
/// nothing. Reading the cache is local-only — no command outside
/// `muxix provision` ever makes a network request for policy.
fn cached_policy(
    provision: Option<&crate::provision::ProvisionConfig>,
) -> Option<crate::provision::types::OrgPolicy> {
    use crate::provision::cache::{CacheStatus, load_policy};

    let grace_secs = provision
        .and_then(|p| p.grace_period_secs)
        .unwrap_or(72 * 3600);

    match load_policy(grace_secs) {
        Ok(CacheStatus::Fresh(policy)) => Some(policy),
        Ok(CacheStatus::Stale(policy)) => {
            tracing::warn!(
                "org policy {} is stale -- run 'muxix provision sync' to refresh",
                policy.policy_version
            );
            Some(policy)
        }
        Ok(CacheStatus::Expired) => {
            tracing::warn!("org policy cache expired -- run 'muxix provision sync' to refresh");
            None
        }
        Ok(CacheStatus::Missing) => None,
        Err(e) => {
            tracing::debug!("could not load policy cache: {}", e);
            None
        }
    }
}

/// Every top-level key the config schema accepts.
///
/// Read out of this file's own `struct Config` definition rather than out of a
/// serialized `Config::default()`: a field carrying `skip_serializing_if` is
/// absent from a default serialization, so that approach silently reported real
/// keys (`bootstrap`, `mcp`, `providers`, ...) as typos. Parsing the source
/// cannot drift from the struct the way a hand-maintained list would.
fn known_config_keys() -> std::collections::BTreeSet<String> {
    const SOURCE: &str = include_str!("config.rs");

    let mut keys: std::collections::BTreeSet<String> =
        DIRECTIVE_KEYS.iter().map(|s| (*s).to_string()).collect();

    let Some(body) = SOURCE
        .split_once("pub struct Config {")
        .and_then(|(_, rest)| rest.split_once("\n}"))
        .map(|(body, _)| body)
    else {
        return keys;
    };

    for line in body.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("pub ") else {
            continue;
        };
        if let Some((name, _)) = rest.split_once(':') {
            keys.insert(name.trim().to_string());
        }
    }
    keys
}

/// Top-level keys in `value` that the config schema does not recognize.
///
/// serde ignores unknown fields by default, which means a misspelled key is
/// silently dropped rather than reported. `muxix config validate` uses this
/// to surface them. Only the top level is checked: `deny_unknown_fields` on
/// every nested struct would be a much larger change, and top-level typos are
/// the common case.
pub fn unknown_top_level_keys(value: &serde_yaml::Value) -> Vec<String> {
    let serde_yaml::Value::Mapping(map) = value else {
        return Vec::new();
    };

    let known = known_config_keys();

    let mut unknown: Vec<String> = map
        .keys()
        .filter_map(|k| k.as_str())
        .filter(|k| !known.contains(*k))
        .map(str::to_owned)
        .collect();
    unknown.sort_unstable();
    unknown
}

/// One layer's identity, for `muxix config resolve --explain` output.
#[derive(Debug, Clone)]
pub struct LayerInfo {
    /// Stable id used as the provenance value, e.g. `global`, `profile:corp`.
    pub id: String,
    /// Where the layer came from: a file path, a URL, or a profile description.
    pub source: String,
}

impl Config {
    /// Resolve a model from the unified [`Config::providers`] registry by
    /// provider id or logical name. Returns `None` when no `providers` are
    /// configured or nothing matches.
    #[allow(dead_code)]
    pub fn resolve_model(&self, key: &str) -> Option<crate::model::ResolvedModel<'_>> {
        self.providers
            .as_ref()
            .and_then(|registry| crate::model::resolve(registry, key))
    }

    /// Load and merge global and project configurations.
    pub fn load(cli_agent: Option<&str>) -> anyhow::Result<Self> {
        Self::load_with_override(cli_agent, None)
    }

    pub fn load_with_override(
        cli_agent: Option<&str>,
        config_override: Option<&Path>,
    ) -> anyhow::Result<Self> {
        Self::load_with_location(cli_agent, config_override).map(|(cfg, _)| cfg)
    }

    /// Load and merge configs, returning the final config and project config location.
    /// The location indicates where the project config was found (for working dir calculation).
    pub fn load_with_location(
        cli_agent: Option<&str>,
        config_override: Option<&Path>,
    ) -> anyhow::Result<(Self, Option<ConfigLocation>)> {
        let start_dir = std::env::current_dir().unwrap_or_default();
        Self::load_with_location_from_override(&start_dir, cli_agent, config_override)
    }

    /// [`Config::load`], searching for the project config from `start_dir`
    /// instead of the process CWD. Required anywhere one process serves several
    /// projects -- the daemon -- since the CWD is global.
    pub fn load_from(start_dir: &Path, cli_agent: Option<&str>) -> anyhow::Result<Self> {
        Self::load_with_location_from(start_dir, cli_agent).map(|(cfg, _)| cfg)
    }

    /// Like `load_with_location`, but searches for the project config starting
    /// from `start_dir` instead of CWD.
    pub fn load_with_location_from(
        start_dir: &std::path::Path,
        cli_agent: Option<&str>,
    ) -> anyhow::Result<(Self, Option<ConfigLocation>)> {
        Self::load_with_location_from_override(start_dir, cli_agent, None)
    }

    pub fn load_with_location_from_override(
        start_dir: &std::path::Path,
        cli_agent: Option<&str>,
        config_override: Option<&Path>,
    ) -> anyhow::Result<(Self, Option<ConfigLocation>)> {
        // Every ordinary load honors the `--profile` flag recorded at startup;
        // `load_with_options` is for callers that select a profile explicitly.
        Self::load_with_options(
            start_dir,
            cli_agent,
            config_override,
            profiles::cli_profile(),
        )
    }

    /// The full load path: expand includes, merge layers, overlay profiles,
    /// then apply defaults.
    ///
    /// `cli_profile` is the `--profile` flag; see [`profiles::select`] for how
    /// it ranks against `MUXIX_PROFILE` and `default_profile`.
    pub fn load_with_options(
        start_dir: &std::path::Path,
        cli_agent: Option<&str>,
        config_override: Option<&Path>,
        cli_profile: Option<&str>,
    ) -> anyhow::Result<(Self, Option<ConfigLocation>)> {
        let result =
            Self::load_with_options_inner(start_dir, cli_agent, config_override, cli_profile);
        if let Err(e) = &result {
            // Many callers degrade to Config::default() on error, which would
            // otherwise turn a config typo into silently ignored settings.
            // Warn loudly, once per process; the Err still propagates.
            static WARNED: std::sync::Once = std::sync::Once::new();
            WARNED.call_once(|| {
                eprintln!("muxix: config error: {e:#} — commands may fall back to defaults");
            });
        }
        result
    }

    fn load_with_options_inner(
        start_dir: &std::path::Path,
        cli_agent: Option<&str>,
        config_override: Option<&Path>,
        cli_profile: Option<&str>,
    ) -> anyhow::Result<(Self, Option<ConfigLocation>)> {
        let (layers, location, agent_above_global) =
            Self::build_layers(start_dir, config_override, cli_profile)?;

        let resolved = resolve::resolve(layers, false);
        for warning in &resolved.warnings {
            tracing::warn!("{} ({})", warning.message, warning.source);
        }
        let merged: Self = serde_yaml::from_value(resolved.value)
            .map_err(|e| anyhow::anyhow!("Failed to load merged config: {}", e))?;

        let defaults_root = location
            .as_ref()
            .and_then(|loc| {
                let repo_root = git::get_repo_root_for(start_dir).ok()?;
                if loc.config_dir.starts_with(&repo_root) {
                    Some(loc.config_dir.clone())
                } else {
                    Some(repo_root)
                }
            })
            .or_else(|| git::get_repo_root_for(start_dir).ok())
            .unwrap_or_else(|| start_dir.to_path_buf());

        let config = Self::apply_defaults(
            merged,
            agent_above_global.as_deref(),
            cli_agent,
            &defaults_root,
        )?;

        debug!(
            agent = ?config.agent,
            has_location = location.is_some(),
            "config:loaded with location from"
        );
        Ok((config, location))
    }

    /// Resolve the effective config as a YAML value, without deserializing it
    /// into [`Config`] or applying defaults.
    ///
    /// This is what `muxix config resolve` prints. It returns the merged
    /// value plus the per-key provenance map when `track` is set, so callers
    /// can attribute each key to the layer that set it. Defaults are
    /// deliberately not applied: this shows what the config *files* say, which
    /// is what someone debugging a layering question needs to see.
    pub fn resolve_value(
        start_dir: &std::path::Path,
        config_override: Option<&Path>,
        cli_profile: Option<&str>,
        track: bool,
    ) -> anyhow::Result<(resolve::Resolved, Vec<LayerInfo>)> {
        let (layers, _location, _agent) =
            Self::build_layers(start_dir, config_override, cli_profile)?;
        let info: Vec<LayerInfo> = layers
            .iter()
            .map(|l| LayerInfo {
                id: l.id.clone(),
                source: l.source.clone(),
            })
            .collect();
        Ok((resolve::resolve(layers, track), info))
    }

    /// Resolve a single config file in isolation, ignoring the ambient global
    /// and project configs.
    ///
    /// This is what `muxix config validate --file` uses, and what the Nix
    /// module's build-time check needs: a rendered file must be judged on its
    /// own, not against whatever happens to be installed on the machine
    /// running the build.
    pub fn resolve_file(
        path: &Path,
        cli_profile: Option<&str>,
        track: bool,
    ) -> anyhow::Result<resolve::Resolved> {
        let value = Self::load_value_from_path(path)?
            .ok_or_else(|| anyhow::anyhow!("Config file not found: {}", path.display()))?;
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();

        // Trusted: a file named explicitly is the user's own choice, the same
        // standing as their global config.
        let mut layers = include::expand(&value, &dir, path, true, "file:")?;
        let mut value = value;
        include::strip_include_key(&mut value);
        layers.push(resolve::Layer::new(
            "file",
            path.display().to_string(),
            resolve::LayerKind::Global,
            value,
        ));

        let mut available = BTreeMap::new();
        let mut trust = BTreeMap::new();
        let mut default_profile = None;
        for layer in &mut layers {
            for (name, body) in profiles::declared(&layer.value)? {
                trust.insert(name.clone(), true);
                available.insert(name, body);
            }
            if let Some(v) = layer.value.get("default_profile").and_then(|v| v.as_str()) {
                default_profile = Some(v.to_string());
            }
            profiles::strip_profiles_key(&mut layer.value);
        }
        let selection = profiles::select(cli_profile, default_profile.as_deref());
        layers.extend(profiles::layers(&selection, &available, &trust)?);

        Ok(resolve::resolve(layers, track))
    }

    /// Names of every profile a config file declares, for validation.
    pub fn declared_profiles(path: &Path) -> anyhow::Result<Vec<String>> {
        let Some(value) = Self::load_value_from_path(path)? else {
            return Ok(Vec::new());
        };
        Ok(profiles::declared(&value)?.into_keys().collect())
    }

    /// Assemble every config layer in precedence order, lowest first.
    ///
    /// Returns the layers, where the project config was found, and the agent
    /// named by the highest layer above the global config (which
    /// `apply_defaults` needs and the merged config cannot express).
    fn build_layers(
        start_dir: &std::path::Path,
        config_override: Option<&Path>,
        cli_profile: Option<&str>,
    ) -> anyhow::Result<(Vec<resolve::Layer>, Option<ConfigLocation>, Option<String>)> {
        debug!(start_dir = %start_dir.display(), "config:loading with location from");
        let global_value = Self::load_global_value()?;
        let global_source = global_config_path()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "<global>".to_string());

        let (project_value, project_source, location) = if let Some(path) = config_override {
            let meta = std::fs::metadata(path)
                .with_context(|| format!("Config file not found: {}", path.display()))?;
            if meta.is_dir() {
                anyhow::bail!(
                    "--config path must be a file, not a directory: {}",
                    path.display()
                );
            }
            if !meta.is_file() {
                anyhow::bail!("--config path must be a regular file: {}", path.display());
            }
            let abs_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
            let value = Self::load_value_from_path(&abs_path)?
                .ok_or_else(|| anyhow::anyhow!("Config file not found: {}", path.display()))?;
            let config_dir = abs_path
                .parent()
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|| start_dir.to_path_buf());
            let source = abs_path.display().to_string();
            // rel_dir is intentionally empty: --config overrides don't imply a subproject layout,
            // so panes always open at the worktree root regardless of where the config file lives.
            let location = ConfigLocation {
                config_path: abs_path,
                config_dir: config_dir.clone(),
                rel_dir: PathBuf::new(),
            };
            (Some(value), source, Some(location))
        } else {
            let location = find_project_config(start_dir)?;
            let (value, source) = if let Some(ref loc) = location {
                (
                    Self::load_value_from_path(&loc.config_path)?,
                    loc.config_path.display().to_string(),
                )
            } else {
                (None, "<project>".to_string())
            };
            (value, source, location)
        };

        let mut layers = Vec::with_capacity(2);
        if let Some(value) = global_value {
            // Includes of the global config are trusted like the global config
            // itself: the user chose to pull them in from their own file.
            let dir = global_config_path()
                .and_then(|p| p.parent().map(Path::to_path_buf))
                .unwrap_or_default();
            let origin = global_config_path().unwrap_or_else(|| dir.join("config.yaml"));
            layers.extend(include::expand(&value, &dir, &origin, true, "global:")?);

            let mut value = value;
            include::strip_include_key(&mut value);
            layers.push(resolve::Layer::new(
                "global",
                global_source,
                resolve::LayerKind::Global,
                value,
            ));
        }
        if let Some(value) = project_value {
            // A project's includes inherit the project's trust level, so a repo
            // cannot launder a global-only key in through an included file.
            let (dir, origin) = match location.as_ref() {
                Some(loc) => (loc.config_dir.clone(), loc.config_path.clone()),
                None => (start_dir.to_path_buf(), start_dir.join(".muxix.yaml")),
            };
            layers.extend(include::expand(&value, &dir, &origin, false, "project:")?);

            let mut value = value;
            include::strip_include_key(&mut value);
            layers.push(resolve::Layer::new(
                "project",
                project_source,
                resolve::LayerKind::Project,
                value,
            ));
        }

        // The policy's defaults rank *below* profiles and CLI flags: a default is
        // a suggestion for a machine that has not decided, and must not beat a
        // deliberate project or profile choice. Its locks rank above everything
        // and are appended last.
        //
        // `provision` is global-only, so the settings that govern the cache are
        // read off the global layer rather than the not-yet-merged config.
        let provision_cfg: Option<crate::provision::ProvisionConfig> = layers
            .iter()
            .find(|l| l.kind == resolve::LayerKind::Global)
            .and_then(|l| l.value.get("provision"))
            .and_then(|v| serde_yaml::from_value(v.clone()).ok());
        let policy = cached_policy(provision_cfg.as_ref());
        if let Some(ref policy) = policy
            && let Some(layer) = crate::provision::layers::defaults_layer(policy)
        {
            layers.push(layer);
        }

        // Profiles are collected from every file layer, so an include or the
        // global config can declare one the project selects. Each records the
        // trust of the layer that declared it -- a project-declared profile is
        // no more privileged than the project config itself.
        let mut available: BTreeMap<String, serde_yaml::Value> = BTreeMap::new();
        let mut profile_trust: BTreeMap<String, bool> = BTreeMap::new();
        let mut default_profile: Option<String> = None;
        for layer in &mut layers {
            for (name, body) in profiles::declared(&layer.value)? {
                profile_trust.insert(name.clone(), layer.trusted);
                available.insert(name, body);
            }
            if let Some(value) = layer.value.get("default_profile").and_then(|v| v.as_str()) {
                default_profile = Some(value.to_string());
            }
            profiles::strip_profiles_key(&mut layer.value);
        }

        let selection = profiles::select(cli_profile, default_profile.as_deref());
        layers.extend(profiles::layers(&selection, &available, &profile_trust)?);

        // Locks win over every layer, including CLI flags: a lock is the one
        // thing an organization can rely on. Overrides are reported where a
        // user is looking (`config resolve --explain` attributes the key, and
        // `provision sync` names them) rather than on every config load.
        if let Some(ref policy) = policy
            && let Some(layer) = crate::provision::layers::locks_layer(policy)
        {
            layers.push(layer);
        }

        // An agent named by any layer above the global config outranks an agent
        // detected from the environment; one named only by the global config
        // does not. The merged config cannot express that difference, so it is
        // read off the layers here, highest precedence first.
        let agent_above_global = layers
            .iter()
            .rev()
            .take_while(|l| l.kind != resolve::LayerKind::Global)
            .find_map(|l| {
                l.value
                    .get("agent")
                    .and_then(|v| v.as_str())
                    .map(str::to_owned)
            });

        Ok((layers, location, agent_above_global))
    }

    /// Resolve the agent and apply repo-derived defaults to an already-merged
    /// config.
    ///
    /// Layer merging happens before this in [`resolve`]; everything here is
    /// post-processing that is not a merge. `agent_above_global` carries the one
    /// fact the merged config cannot express -- whether a layer above the global
    /// config named an agent -- which decides if environment detection may
    /// outrank the global default.
    fn apply_defaults(
        merged: Self,
        agent_above_global: Option<&str>,
        cli_agent: Option<&str>,
        defaults_root: &std::path::Path,
    ) -> anyhow::Result<Self> {
        let mut config = merged;
        let has_explicit_agent = cli_agent.is_some() || config.agent.is_some();

        // Opt-in agent inheritance: when `inherit_agent` is set, an agent
        // detected from the surrounding environment outranks the *global*
        // default, so `muxix add` run from inside Claude Code spawns Claude
        // Code rather than whatever the global config happens to name. An
        // explicit `--agent` flag and an agent named by any higher layer (a
        // project config or a selected profile) still win, and detection only
        // runs when neither is present -- it shells out to `tmux`/`ps`, so it
        // must stay off the common path.
        // Path rules: explicit per-project config, so they outrank parent
        // detection (a `tmux`/`ps` heuristic) but never a project's own config.
        // Resolved against the main worktree root so worktrees of a project
        // match the project's rule; skipped entirely when no rules exist, which
        // keeps the extra `git worktree list` off the common path.
        let rule_agent: Option<(String, AgentSource)> = if config.agent_rules.is_empty()
            || cli_agent.is_some()
            || agent_above_global.is_some()
        {
            None
        } else {
            let root = crate::git::get_main_worktree_root_in(Some(defaults_root))
                .unwrap_or_else(|_| defaults_root.to_path_buf());
            match_agent_rule(&config.agent_rules, &root).map(|(index, r)| {
                debug!(agent = %r.agent, pattern = %r.pattern, root = %root.display(), "config:agent from rule");
                (
                    r.agent.clone(),
                    AgentSource::Rule {
                        index,
                        pattern: r.pattern.clone(),
                    },
                )
            })
        };

        let inherit_agent = config.inherit_agent.unwrap_or(false);
        let detected_agent = if inherit_agent
            && cli_agent.is_none()
            && agent_above_global.is_none()
            && rule_agent.is_none()
        {
            crate::agent::identity::detect_parent_agent()
        } else {
            None
        };
        if let Some(detected) = &detected_agent {
            debug!(agent = %detected, "config:inherited agent from parent");
        }

        let (final_agent, agent_source) = cli_agent
            .map(|s| (s.to_string(), AgentSource::Flag))
            .or_else(|| agent_above_global.map(|a| (a.to_owned(), AgentSource::ProjectConfig)))
            .or_else(|| rule_agent.clone())
            .or_else(|| detected_agent.clone().map(|a| (a, AgentSource::Inherited)))
            .or_else(|| config.agent.clone().map(|a| (a, AgentSource::Global)))
            .unwrap_or_else(|| ("claude".to_string(), AgentSource::Builtin));
        config.agent_source = Some(agent_source);

        debug!(
            bootstrap = config.bootstrap.is_some(),
            "config:apply_defaults merged config"
        );

        // Resolve agent name through agents map
        if let Some(entry) = config.agents.get(&final_agent) {
            config.agent_type = entry.agent_type.clone();
            config.agent = Some(entry.command.clone());
        } else {
            config.agent = Some(final_agent);
        }

        if !defaults_root.as_os_str().is_empty() {
            let has_node_modules = defaults_root.join("pnpm-lock.yaml").exists()
                || defaults_root.join("package-lock.json").exists()
                || defaults_root.join("yarn.lock").exists();

            if config.panes.is_none() && config.windows.is_none() {
                if defaults_root.join("CLAUDE.md").exists() || has_explicit_agent {
                    config.panes = Some(Self::agent_default_panes());
                } else {
                    config.panes = Some(Self::default_panes());
                }
            }

            if config.pre_remove.is_none() && has_node_modules {
                config.pre_remove = Some(vec![NODE_MODULES_CLEANUP_SCRIPT.to_string()]);
            }
        } else if config.panes.is_none() && config.windows.is_none() {
            if has_explicit_agent {
                config.panes = Some(Self::agent_default_panes());
            } else {
                config.panes = Some(Self::default_panes());
            }
        }

        config
            .sandbox
            .network
            .validate()
            .context("Invalid sandbox network config")?;
        config
            .sandbox
            .container
            .validate()
            .context("Invalid sandbox container config")?;

        // Install pipeline event controls. Every config load routes through here,
        // so the per-kind filter and (env-permitting) the trace level stay current.
        crate::signals::event::set_filter(config.events.to_filter());
        if let Some(level) = &config.events.level {
            crate::logger::set_event_level(level);
        }

        // Validate against the cached org policy. The policy's `defaults` and
        // `locked` values were already applied as layers during resolution;
        // what is left here are deny-list assertions about the final config.
        if let Some(policy) = cached_policy(config.provision.as_ref()) {
            let violations = crate::provision::merge::validate_policy(&config, &policy);
            crate::provision::merge::report_violations(&violations);
        }

        Ok(config)
    }

    /// Read a config file as an unvalidated YAML value, for layer merging.
    ///
    /// Parsing to `Value` rather than to `Config` is what lets a later layer
    /// override a key without each layer having to be independently valid.
    fn load_value_from_path(path: &Path) -> anyhow::Result<Option<serde_yaml::Value>> {
        if !path.exists() {
            return Ok(None);
        }
        debug!(path = %path.display(), "config:reading file");
        let contents = fs::read_to_string(path)?;
        let value: serde_yaml::Value = serde_yaml::from_str(&contents)
            .map_err(|e| anyhow::anyhow!("Failed to parse config at {}: {}", path.display(), e))?;
        // An empty file parses as null; treat it as "no keys" so it merges as a
        // no-op instead of wiping the layers beneath it.
        if value.is_null() {
            return Ok(None);
        }
        Ok(Some(value))
    }

    /// Load the global configuration file as an unvalidated YAML value.
    ///
    /// Uses `global_config_path()` which resolves via XDG_CONFIG_HOME with
    /// legacy fallback.
    fn load_global_value() -> anyhow::Result<Option<serde_yaml::Value>> {
        if let Some(path) = global_config_path()
            && path.exists()
        {
            return Self::load_value_from_path(&path);
        }
        Ok(None)
    }

    /// Get default panes.
    fn default_panes() -> Vec<PaneConfig> {
        vec![
            PaneConfig {
                command: None, // Default shell
                focus: true,
                ..Default::default()
            },
            PaneConfig {
                command: Some("clear".to_string()),
                split: Some(SplitDirection::Horizontal),
                ..Default::default()
            },
        ]
    }

    /// Get default panes for a Claude project.
    fn agent_default_panes() -> Vec<PaneConfig> {
        vec![
            PaneConfig {
                command: Some("<agent>".to_string()),
                focus: true,
                ..Default::default()
            },
            PaneConfig {
                command: Some("clear".to_string()),
                split: Some(SplitDirection::Horizontal),
                ..Default::default()
            },
        ]
    }

    /// Get the window prefix to use.
    /// Priority: explicit window_prefix config > nerdfont icon > "mx-"
    pub fn window_prefix(&self) -> &str {
        if let Some(ref prefix) = self.window_prefix {
            prefix
        } else if nerdfont::is_enabled() {
            "\u{f418} " // nf-oct-git_branch
        } else {
            "mx-"
        }
    }

    /// Get the mode (window or session).
    /// Returns the configured value or defaults to Window.
    pub fn mode(&self) -> MuxMode {
        self.mode.unwrap_or(MuxMode::Window)
    }

    /// Window in days for `project_open: recent`. Default: 7.
    pub fn project_open_days(&self) -> u64 {
        self.project_open_days.unwrap_or(7)
    }

    /// Create an example .muxix.yaml configuration file
    pub fn init() -> anyhow::Result<()> {
        use std::path::PathBuf;

        let config_path = PathBuf::from(".muxix.yaml");

        if config_path.exists() {
            return Err(anyhow::anyhow!(
                ".muxix.yaml already exists. Remove it first if you want to regenerate it."
            ));
        }

        fs::write(&config_path, EXAMPLE_PROJECT_CONFIG)?;

        println!("✓ Created .muxix.yaml");
        println!("\nThis file provides project-specific overrides.");
        println!(
            "For global settings, edit {}",
            global_config_path()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "~/.config/muxix/config.yaml".to_string())
        );

        Ok(())
    }
}

/// Example project configuration with all options documented.
/// Used by `muxix init` and `muxix config show`.
pub const EXAMPLE_PROJECT_CONFIG: &str = r#"# muxix project configuration
# For global settings, edit ~/.config/muxix/config.yaml
# All options below are commented out - uncomment to override defaults.

#-------------------------------------------------------------------------------
# Appearance
#-------------------------------------------------------------------------------

# Color scheme for the dashboard. Press T (shift+t) in the dashboard to cycle.
# Options: default, emberforge, glacier-signal, obsidian-pop, slate-garden,
#          phosphor-arcade, lasergrid, mossfire, night-sorbet, graphite-code,
#          festival-circuit, teal-drift
# theme: default
#
# Or with explicit dark/light mode (otherwise auto-detected from terminal):
# theme:
#   scheme: emberforge
#   mode: dark

#-------------------------------------------------------------------------------
# Git
#-------------------------------------------------------------------------------

# The primary branch to merge into.
# Default: Auto-detected from remote HEAD, falls back to main/master.
# main_branch: main

# Default base branch/commit to branch from when creating new worktrees.
# The --base CLI flag always overrides this.
# Default: The currently checked out branch.
# base_branch: main

# Default merge strategy for `muxix merge`.
# Options: merge (default), rebase, squash
# CLI flags (--rebase, --squash) always override this.
# merge_strategy: rebase

# Keep the worktree, window, and branch after `muxix merge` by default.
# Keep and cleanup CLI flags always override this.
# merge_keep: true

#-------------------------------------------------------------------------------
# Naming & Paths
#-------------------------------------------------------------------------------

# Directory where worktrees are created.
# Can be relative to repo root or absolute. Supports `~` for home directory
# and `{project}` for the main worktree's directory name, so a global config
# can namespace each repo, e.g. `~/.muxix/{project}`.
# Default: Sibling directory '<project>__worktrees'.
# worktree_dir: .worktrees

# Strategy for deriving names from branch names.
# Options: full (default), basename (part after last '/').
# worktree_naming: basename

# Prefix added to worktree directories and tmux window names.
# worktree_prefix: ""

# Prefix for tmux window names.
# Default: "mx-"
# window_prefix: "mx-"

#-------------------------------------------------------------------------------
# Tmux
#-------------------------------------------------------------------------------

# Mode for tmux operations: window (default) or session.
# - window: Create windows within the current tmux session
# - session: Create new tmux sessions for each worktree (useful for session-per-project workflows)
# mode: session

# Custom tmux pane layout (mutually exclusive with 'windows').
# Default: Two-pane layout with shell and clear command.
# panes:
#   - command: pnpm install
#     focus: true
#   - split: horizontal
#   - command: clear
#     split: vertical
#     size: 5

# Multiple windows per session (session mode only, mutually exclusive with 'panes').
# Each window can have its own pane layout. Unnamed windows get tmux's
# automatic naming based on the running command.
# windows:
#   - name: editor
#     panes:
#       - command: <agent>
#         focus: true
#       - split: horizontal
#         size: 20
#   - name: tests
#     panes:
#       - command: just test --watch
#   - panes:
#       - command: tail -f app.log

# Auto-apply agent status icons to tmux window format.
# Default: true
# status_format: true

# Custom icons for agent status display.
# status_icons:
#   working: "🤖"
#   waiting: "💬"
#   done: "✅"

#-------------------------------------------------------------------------------
# Projects
#-------------------------------------------------------------------------------

# How `muxix start` multiplexes tracked projects.
# Options: session (default) — one session per tracked project.
# project_mux: session

# Which worktrees `muxix start` and `muxix project open` restore.
# Options:
#   all        - every worktree of the project
#   unfinished - worktrees whose agent stopped mid-work (no completion,
#                status not done)
#   active     - worktrees that have any agent state at all (default)
#   recent     - worktrees touched within `project_open_days` days
# The --worktrees <mode> flag always overrides this.
# Default: active
# project_open: active

# Window in days for `project_open: recent`.
# Default: 7
# project_open_days: 7

#-------------------------------------------------------------------------------
# Agent & AI
#-------------------------------------------------------------------------------

# Agent command for '<agent>' placeholder in pane commands.
# Default: "claude"
# agent: claude

# Per-project agent selection by path regex (global config only).
# Matched against the project's main worktree root, first match wins. Ranks
# below an `agent:` in a project .muxix.yaml and above the global `agent:`.
# Edit with `muxix config agent set|unset`; inspect with
# `muxix config agent list|which`.
# agent_rules:
#   - match: "^~/repos/work/"
#     agent: opencode
#   - match: "^~/repos/muxix(/|$)"
#     agent: pi

# LLM-based branch name generation (`muxix add -A`).
# auto_name:
#   model: "gpt-4o-mini"
#   system_prompt: "Generate a kebab-case git branch name."
#   background: true  # Always run in background when using --auto-name

#-------------------------------------------------------------------------------
# Hooks
#-------------------------------------------------------------------------------

# Commands to run in new worktree before tmux window opens.
# These block window creation - use for short tasks only.
# Use "<global>" to inherit from global config.
# Set to empty list to disable: `post_create: []`
# post_create:
#   - "<global>"
#   - mise use

# Commands to run before merging (e.g., linting, tests).
# Aborts the merge if any command fails.
# Use "<global>" to inherit from global config.
# Environment variables available:
#   - MUXIX_BRANCH_NAME: The name of the branch being merged
#   - MUXIX_TARGET_BRANCH: The name of the target branch (e.g., main)
#   - MUXIX_WORKTREE_PATH: Absolute path to the worktree
#   - MUXIX_PROJECT_ROOT: Absolute path of the main project directory
#   - MUXIX_HANDLE: The worktree handle/window name
# pre_merge:
#   - "<global>"
#   - cargo test
#   - cargo clippy -- -D warnings

# Commands to run before worktree removal (during merge or remove).
# Useful for backing up gitignored files before cleanup.
# Default: Auto-detects Node.js projects and fast-deletes node_modules.
# Set to empty list to disable: `pre_remove: []`
# Environment variables available:
#   - MUXIX_HANDLE: The worktree handle (directory name)
#   - MUXIX_WORKTREE_PATH: Absolute path of the worktree being deleted
#   - MUXIX_PROJECT_ROOT: Absolute path of the main project directory
# pre_remove:
#   - mkdir -p "$MUXIX_PROJECT_ROOT/artifacts/$MUXIX_HANDLE"
#   - cp -r test-results/ "$MUXIX_PROJECT_ROOT/artifacts/$MUXIX_HANDLE/"

#-------------------------------------------------------------------------------
# Files
#-------------------------------------------------------------------------------

# File operations when creating a worktree.
# files:
#   # Files to copy (useful for .env files that need to be unique).
#   copy:
#     - .env.local
#
#   # Files/directories to symlink (saves disk space, shares caches).
#   # Default: None.
#   # Use "<global>" to inherit from global config.
#   symlink:
#     - "<global>"
#     - node_modules

#-------------------------------------------------------------------------------
# Dashboard
#-------------------------------------------------------------------------------

# Actions for dashboard keybindings (c = commit, m = merge).
# Values are sent to the agent's pane. Use ! prefix for shell commands.
# Preview size (10-90): larger = more preview, less table. Use +/- keys to adjust.
# dashboard:
#   commit: "Commit staged changes with a descriptive message"
#   merge: "!muxix merge"
#   preview_size: 60

#-------------------------------------------------------------------------------
# Sidebar
#-------------------------------------------------------------------------------

# sidebar:
#   # Position: left (default) or top.
#   position: left
#
#   # Left sidebar width: absolute columns or percentage of terminal width.
#   # Default: "10%" (clamped to 25-50 columns).
#   # Explicit values are not clamped (minimum 10 columns).
#   width: 40       # absolute columns
#   # width: "15%"  # percentage of terminal width (takes precedence over any saved interactive size)
#
#   # Top bar height in rows.
#   height: 3
#
#   # Default scope for the sidebar toggle command: "global" or "session".
#   # "session" makes plain `wng sidebar` activate only the current tmux session.
#   # Use `wng sidebar --global` to override to global regardless of this setting.
#   # default_scope: session
#
#   # Layout mode for the left sidebar: "compact" or "tiles" (cards).
#   # Default: "tiles". Can be toggled at runtime with 'v' key.
#   layout: tiles
#
#   horizontal:
#     item_width: 24  # horizontal chip width in columns, clamped 12-80
#
#   templates:
#     horizontal:
#       - "{status_icon} {primary} {pane_suffix} {activity} {fill} {elapsed}"
#       - "{secondary} {fill} {git_stats}"
#       - "{pane_title}"

#-------------------------------------------------------------------------------
# Sandbox
#-------------------------------------------------------------------------------

# sandbox:
#   enabled: false
#   backend: lima
#   # host_commands: ["just", "cargo", "npm"]
#   # container:
#   #   runtime: docker          # docker | podman | apple-container
#   #   # memory: 16G            # VM memory limit (apple-container default: 16G)
#   #   # cpus: 4                # VM CPU count (only passed when set)
#   #   # Mask files out of the worktree bind mounts (paths relative to the
#   #   # worktree root). Each listed file is shadowed by /dev/null so the
#   #   # sandboxed agent cannot read it. Missing files are skipped.
#   #   # GLOBAL-ONLY: ignored when set in a project .muxix.yaml.
#   #   # excluded_files:
#   #   #   - .env
#   #   #   - .env.local
#   # lima:
#   #   isolation: project
#   #   cpus: 4
#   #   memory: 4GiB
#   #   # Custom provision script (runs once on VM creation, as user).
#   #   # Use sudo for system commands.
#   #   # provision: |
#   #   #   sudo apt-get install -y ripgrep fd-find jq
#   # Extra mount points (read-only by default).
#   # Supports simple paths or detailed specs with guest_path and writable.
#   # extra_mounts:
#   #   - ~/my-notes
#   #   - host_path: ~/data
#   #     guest_path: /mnt/data
#   #     writable: true

# Pipeline event tracing (`muxix::event`, logged to ~/.local/state/muxix.log).
# Controls the structured event timeline the runner/orchestrator emit. The
# `MUXIX_EVENTS` env var still overrides `level` when set.
# events:
#   # Master switch. false silences every muxix::event. Default: true.
#   enabled: true
#   # Trace level: off | info | debug | trace. Ignored if MUXIX_EVENTS is set.
#   level: info
#   # Silence specific kinds or whole groups (group-aware: `pane.probe` matches
#   # pane.probe.ok, pane.probe.retry, ...).
#   disable:
#     - pane.probe
#     - turn.poll
#   # Allowlist: when non-empty, ONLY these kinds/groups are emitted (still
#   # subject to `disable`). Empty = all.
#   only: []

#-------------------------------------------------------------------------------
# Agent bootstrap
#-------------------------------------------------------------------------------

# Uniform per-agent setup, applied by `muxix setup`: skills, plugins, and
# system-prompt components. Keeps "what the project needs" in one place
# instead of in each agent's own config.
# bootstrap:
#   # Skills installed into every skills-capable agent's skills directory
#   # (~/.claude/skills, ~/.config/opencode/skills, ~/.pi/agent/skills,
#   # ~/.omp/agent/skills). Each entry is a path to a directory containing a
#   # SKILL.md, relative to the project root or absolute. Codex, Copilot, and
#   # Gemini have no skills directory and are skipped.
#   skills:
#     - ./skills/muxix
#     # Table form: what the skill needs. `npm` packages are installed by
#     # muxix into `npm_prefix` (pinned when `@version` is given, pruned when
#     # no entity declares them any more); `bin` names are only asserted on
#     # PATH -- provide them via the system. Same block works on `mcp.<name>`.
#     - path: ./skills/openspec-taskflow
#       requires:
#         npm: ["@fission-ai/openspec@1.6.0"]
#         bin: [python3]
#
#   # npm global prefix for `requires.npm` installs (bins in <prefix>/bin).
#   npm_prefix: ~/.local
#   # true: every package under the prefix that no entity declares is pruned,
#   # not only the ones muxix installed. Hand installs get reverted.
#   deps_strict: false
#
#   # Prompt components merged into each agent's system prompt. A bare name
#   # loads .muxix/prompt-components/<name>.md; anything containing `/` is a
#   # path to a .md file (absolute, ~, or relative to the project root).
#   prompt_components:
#     - house-style
#     - ~/repos/harness/prompt-components/jj.md
#
#   # Plugins installed for agents that support them (pi, omp).
#   plugins:
#     - npm:pi-web-access
#
#   # Agent-agnostic capabilities: a feature resolves to that agent's plugin
#   # when one is declared, otherwise to the `default` prompt component.
#   features:
#     ponytail:
#       pi: git:github.com/DietrichGebert/ponytail
#       default: ponytail
#
#   # Per-agent additions. Keys are agent display names, lowercased
#   # ("claude code", "opencode", "pi", "omp", "codex", "gemini cli").
#   agents:
#     claude code:
#       add_skills:
#         - ./skills/worktree
#       add_prompt_components:
#         - code-review
#       exclude_prompt_components:
#         - ponytail
"#;

/// Resolves an executable name or path to its full absolute path.
///
/// For absolute paths, returns as-is. For relative paths, resolves against current directory.
/// For plain executable names (e.g., "claude"), searches first in tmux's global PATH
/// (since panes will run in tmux's environment), then falls back to the current shell's PATH.
/// Returns None if the executable cannot be found.
pub fn resolve_executable_path(executable: &str) -> Option<String> {
    let exec_path = Path::new(executable);

    if exec_path.is_absolute() {
        return Some(exec_path.to_string_lossy().into_owned());
    }

    if executable.contains(std::path::MAIN_SEPARATOR)
        || executable.contains('/')
        || executable.contains('\\')
    {
        if let Ok(current_dir) = env::current_dir() {
            return Some(current_dir.join(exec_path).to_string_lossy().into_owned());
        }
    } else {
        if let Some(tmux_path) = tmux_global_path() {
            let cwd = env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            if let Ok(found) = which_in(executable, Some(tmux_path.as_str()), &cwd) {
                return Some(found.to_string_lossy().into_owned());
            }
        }

        if let Ok(found) = which(executable) {
            return Some(found.to_string_lossy().into_owned());
        }
    }

    None
}

pub fn tmux_global_path() -> Option<String> {
    let output = cmd::Cmd::new("tmux")
        .args(&["show-environment", "-g", "PATH"])
        .run_and_capture_stdout()
        .ok()?;
    output.strip_prefix("PATH=").map(|s| s.to_string())
}

pub fn split_first_token(command: &str) -> Option<(&str, &str)> {
    let trimmed = command.trim_start();
    if trimmed.is_empty() {
        return None;
    }
    Some(
        trimmed
            .split_once(char::is_whitespace)
            .unwrap_or((trimmed, "")),
    )
}

/// Checks if a command string corresponds to the given agent command.
///
/// Returns true if:
/// 1. The command is the literal placeholder "<agent>"
/// 2. The command's executable stem matches the agent's executable stem
///    (e.g., "claude" matches "/usr/bin/claude")
///
/// Looks past `env` wrappers and `VAR=value` assignments to find the
/// real executable in both the command and agent strings.
pub fn is_agent_command(command_line: &str, agent_command: &str) -> bool {
    use crate::agent::profile::find_executable_token;

    let trimmed = command_line.trim();
    if trimmed.is_empty() {
        return false;
    }

    // Allow <agent> token regardless of what follows (e.g., "<agent> --verbose")
    let cmd_token = find_executable_token(trimmed);
    if cmd_token == "<agent>" {
        return true;
    }

    let agent_token = find_executable_token(agent_command);
    if agent_token.is_empty() {
        return false;
    }

    let resolved_cmd = resolve_executable_path(cmd_token).unwrap_or_else(|| cmd_token.to_string());
    let resolved_agent =
        resolve_executable_path(agent_token).unwrap_or_else(|| agent_token.to_string());

    let cmd_stem = Path::new(&resolved_cmd).file_stem();
    let agent_stem = Path::new(&resolved_agent).file_stem();

    cmd_stem.is_some() && cmd_stem == agent_stem
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::{
        AgentIconConfig, AgentIconDetails, AllowedDomainDetails, AllowedDomainEntry, Config,
        ContainerConfig, ContainerDevice, EventsConfig, ExtraMount, LayoutConfig, LimaConfig,
        NetworkConfig, NetworkPolicy, PaneConfig, ProjectOpenFilter, SandboxConfig, SandboxRuntime,
        SandboxTarget, SidebarHeight, SidebarPosition, SidebarWidth, SplitDirection, ToolchainMode,
        is_agent_command, split_first_token, validate_domain, validate_group_add_entry,
        validate_layouts_config,
    };

    #[test]
    fn agent_profile_deltas_roundtrip() {
        let yaml = r#"
agent_profiles:
  corp:
    description: corp litellm
    agents:
      pi:
        add_plugins: ["~/repos/x"]
        exclude_plugins: ["npm:pi-cliproxyapi"]
        exclude_features: ["taskflow"]
        exclude_paths: ["extensions/x.ts"]
        settings:
          defaultProvider: litellm
          taskflow: null
  plain:
    description: file-overlay only
"#;
        let cfg: Config = serde_yaml::from_str(yaml).unwrap();
        let corp = &cfg.agent_profiles["corp"];
        let pi = &corp.agents["pi"];
        assert_eq!(pi.exclude_plugins, vec!["npm:pi-cliproxyapi"]);
        assert_eq!(pi.settings.as_ref().unwrap()["defaultProvider"], "litellm");
        assert!(pi.settings.as_ref().unwrap()["taskflow"].is_null());
        assert!(!pi.is_empty());
        assert!(cfg.agent_profiles["plain"].agents.is_empty());

        // Round-trips without inventing fields.
        let out = serde_yaml::to_string(&cfg.agent_profiles).unwrap();
        let back: std::collections::BTreeMap<String, super::AgentProfile> =
            serde_yaml::from_str(&out).unwrap();
        assert_eq!(back, cfg.agent_profiles);
    }

    /// Merge two typed configs through the value-level resolver.
    ///
    /// The merge tests below predate the resolver and were written against the
    /// typed `Config::merge` it replaced. Routing them through this helper keeps
    /// every one of them asserting against the code path that actually runs.
    ///
    /// Serializing a `Config` emits its unset fields as nulls and its empty
    /// collections as empty maps; both are stripped here, because at the value
    /// level an explicit null means "remove this key" and would wipe the layer
    /// underneath. A config built with `..Default::default()` must merge as
    /// "sets nothing", which is what the typed merge did.
    fn merge_configs(global: Config, project: Config) -> Config {
        use super::resolve::{Layer, LayerKind, resolve};

        fn prune(value: serde_yaml::Value) -> Option<serde_yaml::Value> {
            match value {
                serde_yaml::Value::Null => None,
                serde_yaml::Value::Mapping(map) => {
                    let pruned: serde_yaml::Mapping = map
                        .into_iter()
                        .filter_map(|(k, v)| prune(v).map(|v| (k, v)))
                        .collect();
                    if pruned.is_empty() {
                        None
                    } else {
                        Some(serde_yaml::Value::Mapping(pruned))
                    }
                }
                other => Some(other),
            }
        }

        let to_layer = |config: &Config, id: &'static str, kind: LayerKind| {
            let value = prune(serde_yaml::to_value(config).expect("Config serializes"))
                .unwrap_or_else(|| serde_yaml::Value::Mapping(serde_yaml::Mapping::new()));
            Layer::new(id, id, kind, value)
        };

        let layers = vec![
            to_layer(&global, "global", LayerKind::Global),
            to_layer(&project, "project", LayerKind::Project),
        ];
        serde_yaml::from_value(resolve(layers, false).value)
            .expect("resolved value deserializes as Config")
    }

    #[test]
    fn sidebar_width_percent_no_overflow_on_wide_terminal() {
        // 700 cols * 100 = 70000 would overflow u16 if computed in u16.
        assert_eq!(SidebarWidth::Percent(100).resolve(700), 700);
        assert_eq!(SidebarWidth::Percent(50).resolve(700), 350);
        assert_eq!(SidebarWidth::Percent(25).resolve(0), 25);
        assert_eq!(SidebarWidth::Absolute(40).resolve(700), 40);
    }

    #[test]
    fn sidebar_height_percent_no_overflow_and_zero_guard() {
        assert_eq!(SidebarHeight::Percent(100).resolve(900), 900);
        assert_eq!(SidebarHeight::Percent(33).resolve(300), 99);
        // Zero terminal height must not divide a bogus value; returns 0.
        assert_eq!(SidebarHeight::Percent(50).resolve(0), 0);
        assert_eq!(SidebarHeight::Absolute(10).resolve(0), 10);
    }

    #[test]
    fn project_open_parses_every_mode_and_defaults() {
        for (yaml, want) in [
            ("project_open: all", ProjectOpenFilter::All),
            ("project_open: unfinished", ProjectOpenFilter::Unfinished),
            ("project_open: active", ProjectOpenFilter::Active),
            ("project_open: recent", ProjectOpenFilter::Recent),
        ] {
            let cfg: Config = serde_yaml::from_str(yaml).unwrap();
            assert_eq!(cfg.project_open, Some(want), "{yaml}");
            assert_eq!(want.as_str(), yaml.trim_start_matches("project_open: "));
        }

        let empty: Config = serde_yaml::from_str("{}").unwrap();
        assert_eq!(empty.project_open, None);
        assert_eq!(ProjectOpenFilter::default(), ProjectOpenFilter::Active);
        assert_eq!(empty.project_open_days(), 7);

        let days: Config = serde_yaml::from_str("project_open_days: 30").unwrap();
        assert_eq!(days.project_open_days(), 30);
    }

    #[test]
    fn merge_keep_parses_boolean_values() {
        let enabled: Config = serde_yaml::from_str("merge_keep: true").unwrap();
        assert_eq!(enabled.merge_keep, Some(true));

        let disabled: Config = serde_yaml::from_str("merge_keep: false").unwrap();
        assert_eq!(disabled.merge_keep, Some(false));
    }

    #[test]
    fn merge_keep_project_false_overrides_global_true() {
        let global = Config {
            merge_keep: Some(true),
            ..Default::default()
        };
        let project = Config {
            merge_keep: Some(false),
            ..Default::default()
        };

        let merged = merge_configs(global, project);
        assert_eq!(merged.merge_keep, Some(false));
    }

    #[test]
    fn agent_icon_config_parses_legacy_string() {
        let v: AgentIconConfig = serde_yaml::from_str("\"C\"").unwrap();
        assert_eq!(v, AgentIconConfig::Plain("C".to_string()));
    }

    #[test]
    fn agent_icon_config_parses_detailed_with_both() {
        let v: AgentIconConfig = serde_yaml::from_str("{ icon: \"X\", color: \"#fff\" }").unwrap();
        assert_eq!(
            v,
            AgentIconConfig::Detailed(AgentIconDetails {
                icon: Some("X".to_string()),
                color: Some("#fff".to_string()),
            })
        );
    }

    #[test]
    fn agent_icon_config_parses_detailed_with_color_only() {
        let v: AgentIconConfig = serde_yaml::from_str("{ color: red }").unwrap();
        assert_eq!(
            v,
            AgentIconConfig::Detailed(AgentIconDetails {
                icon: None,
                color: Some("red".to_string()),
            })
        );
    }

    #[test]
    fn agent_icon_config_parses_empty_object_as_detailed() {
        let v: AgentIconConfig = serde_yaml::from_str("{}").unwrap();
        assert_eq!(
            v,
            AgentIconConfig::Detailed(AgentIconDetails {
                icon: None,
                color: None,
            })
        );
    }

    #[test]
    fn agent_icon_config_parses_null_as_null_variant() {
        let v: AgentIconConfig = serde_yaml::from_str("~").unwrap();
        assert_eq!(v, AgentIconConfig::Null);
    }

    #[test]
    fn agent_icon_config_rejects_unknown_field() {
        let err = serde_yaml::from_str::<AgentIconConfig>("{ colour: red }");
        assert!(err.is_err(), "expected unknown-field rejection");
    }

    #[test]
    fn agent_icons_merge_extends_per_key() {
        // Project key overrides global for that key; other global keys survive.
        let yaml_global = r##"
sidebar:
  agent_icons:
    claude: "C"
    codex:
      color: cyan
"##;
        let yaml_project = r##"
sidebar:
  agent_icons:
    claude:
      color: "#ff8c00"
"##;
        let global: Config = serde_yaml::from_str(yaml_global).unwrap();
        let project: Config = serde_yaml::from_str(yaml_project).unwrap();
        let merged = merge_configs(global, project);
        let icons = merged.sidebar.agent_icons.unwrap();
        // codex (only in global) survives.
        assert!(icons.contains_key("codex"));
        // claude is replaced by the project entry (per-key replacement).
        assert_eq!(
            icons.get("claude"),
            Some(&AgentIconConfig::Detailed(AgentIconDetails {
                icon: None,
                color: Some("#ff8c00".to_string()),
            }))
        );
    }

    #[test]
    fn sidebar_position_and_height_parse() {
        let yaml = r#"
sidebar:
  position: top
  height: "10%"
  horizontal:
    item_width: 32
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();

        assert_eq!(config.sidebar.position, Some(SidebarPosition::Top));
        assert_eq!(config.sidebar.height, Some(SidebarHeight::Percent(10)));
        assert_eq!(config.sidebar.horizontal.item_width, Some(32));
        assert_eq!(config.sidebar.horizontal.item_width(), 32);
    }

    #[test]
    fn sidebar_position_and_height_merge_per_field() {
        let global: Config = serde_yaml::from_str(
            r#"
sidebar:
  position: top
  width: 40
  height: 3
  horizontal:
    item_width: 36
"#,
        )
        .unwrap();
        let project: Config = serde_yaml::from_str(
            r#"
sidebar:
  height: 4
"#,
        )
        .unwrap();

        let merged = merge_configs(global, project);

        assert_eq!(merged.sidebar.position, Some(SidebarPosition::Top));
        assert_eq!(merged.sidebar.width, Some(SidebarWidth::Absolute(40)));
        assert_eq!(merged.sidebar.height, Some(SidebarHeight::Absolute(4)));
        assert_eq!(merged.sidebar.horizontal.item_width, Some(36));
    }

    #[test]
    fn split_first_token_single_word() {
        assert_eq!(split_first_token("claude"), Some(("claude", "")));
    }

    #[test]
    fn split_first_token_with_args() {
        assert_eq!(
            split_first_token("claude --verbose"),
            Some(("claude", "--verbose"))
        );
    }

    #[test]
    fn split_first_token_multiple_spaces() {
        assert_eq!(
            split_first_token("claude   --verbose"),
            Some(("claude", "  --verbose"))
        );
    }

    #[test]
    fn split_first_token_leading_whitespace() {
        assert_eq!(
            split_first_token("  claude --verbose"),
            Some(("claude", "--verbose"))
        );
    }

    #[test]
    fn split_first_token_empty_string() {
        assert_eq!(split_first_token(""), None);
    }

    #[test]
    fn split_first_token_only_whitespace() {
        assert_eq!(split_first_token("   "), None);
    }

    #[test]
    fn is_agent_command_placeholder() {
        assert!(is_agent_command("<agent>", "claude"));
        assert!(is_agent_command("  <agent>  ", "gemini"));
        // <agent> with arguments should also match
        assert!(is_agent_command("<agent> --verbose", "claude"));
        assert!(is_agent_command("<agent> -p foo", "gemini"));
    }

    #[test]
    fn is_agent_command_exact_match() {
        assert!(is_agent_command("claude", "claude"));
        assert!(is_agent_command("gemini", "gemini"));
    }

    #[test]
    fn is_agent_command_with_args() {
        assert!(is_agent_command("claude --verbose", "claude"));
        assert!(is_agent_command("gemini -i", "gemini --model foo"));
    }

    #[test]
    fn is_agent_command_mismatch() {
        assert!(!is_agent_command("claude", "gemini"));
        assert!(!is_agent_command("vim", "claude"));
        assert!(!is_agent_command("clear", "claude"));
    }

    #[test]
    fn is_agent_command_empty() {
        assert!(!is_agent_command("", "claude"));
        assert!(!is_agent_command("   ", "claude"));
    }

    #[test]
    fn is_agent_command_env_wrapped() {
        assert!(is_agent_command("env -u FOO claude", "claude"));
        assert!(is_agent_command("claude", "env -u FOO claude"));
        assert!(is_agent_command("env -u FOO claude", "env -u BAR claude"));
        assert!(is_agent_command("FOO=bar claude", "claude"));
    }

    #[test]
    fn is_agent_command_env_wrapped_mismatch() {
        assert!(!is_agent_command("env -u FOO claude", "gemini"));
        assert!(!is_agent_command("env -u FOO vim", "claude"));
    }

    #[test]
    fn agents_deserialize_string_form() {
        let yaml = r#"
agents:
  cc-work: "claude --dangerously-skip-permissions"
  cc-bedrock: "env -u CLAUDE_CODE_USE_BEDROCK claude"
  cod: "codex --yolo"
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.agents.len(), 3);
        assert_eq!(
            config.agents.get("cc-work").unwrap().command,
            "claude --dangerously-skip-permissions"
        );
        assert!(config.agents.get("cc-work").unwrap().agent_type.is_none());
        assert_eq!(
            config.agents.get("cc-bedrock").unwrap().command,
            "env -u CLAUDE_CODE_USE_BEDROCK claude"
        );
        assert_eq!(config.agents.get("cod").unwrap().command, "codex --yolo");
    }

    #[test]
    fn agents_deserialize_map_form_with_type() {
        let yaml = r#"
agents:
  cc-smart:
    command: "/path/to/smart-picker"
    type: claude
  cod-plain: "codex"
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.agents.len(), 2);
        let smart = config.agents.get("cc-smart").unwrap();
        assert_eq!(smart.command, "/path/to/smart-picker");
        assert_eq!(smart.agent_type.as_deref(), Some("claude"));
        let cod = config.agents.get("cod-plain").unwrap();
        assert_eq!(cod.command, "codex");
        assert!(cod.agent_type.is_none());
    }

    #[test]
    fn agents_empty_by_default() {
        let yaml = "agent: claude";
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert!(config.agents.is_empty());
    }

    use super::find_project_config;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn find_project_config_from_subdir() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        // Initialize git repo
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(root)
            .output()
            .unwrap();

        // Create nested structure: root/backend/.muxix.yaml
        let backend = root.join("backend");
        fs::create_dir_all(&backend).unwrap();
        fs::write(backend.join(".muxix.yaml"), "agent: claude").unwrap();

        // Create deeper directory: root/backend/src
        let src = backend.join("src");
        fs::create_dir_all(&src).unwrap();

        // Find from src should find backend/.muxix.yaml
        let result = find_project_config(&src).unwrap();
        assert!(result.is_some());
        let loc = result.unwrap();
        assert!(loc.config_path.ends_with("backend/.muxix.yaml"));
        assert_eq!(loc.rel_dir, std::path::PathBuf::from("backend"));
    }

    #[test]
    fn find_project_config_nearest_wins() {
        let temp = TempDir::new().unwrap();
        let root = temp.path();

        // Initialize git repo
        std::process::Command::new("git")
            .args(["init"])
            .current_dir(root)
            .output()
            .unwrap();

        // Create root config
        fs::write(root.join(".muxix.yaml"), "agent: root").unwrap();

        // Create nested config
        let backend = root.join("backend");
        fs::create_dir_all(&backend).unwrap();
        fs::write(backend.join(".muxix.yaml"), "agent: backend").unwrap();

        // Find from backend should find backend config, not root
        let result = find_project_config(&backend).unwrap();
        assert!(result.is_some());
        let loc = result.unwrap();
        assert!(loc.config_path.ends_with("backend/.muxix.yaml"));
    }

    #[test]
    fn sandbox_config_defaults() {
        let config = SandboxConfig::default();
        assert!(!config.is_enabled());
        assert_eq!(config.target(), SandboxTarget::Agent);
        assert!(config.env_passthrough().is_empty());
    }

    #[test]
    fn sandbox_runtime_explicit_overrides_detect() {
        let config = ContainerConfig {
            runtime: Some(SandboxRuntime::Podman),
            ..Default::default()
        };
        assert_eq!(config.runtime(), SandboxRuntime::Podman);

        let config = ContainerConfig {
            runtime: Some(SandboxRuntime::Docker),
            ..Default::default()
        };
        assert_eq!(config.runtime(), SandboxRuntime::Docker);
    }

    #[test]
    fn sandbox_runtime_detect_when_unset() {
        let config = ContainerConfig {
            runtime: None,
            ..Default::default()
        };
        // Should auto-detect from PATH; result depends on environment
        // but should not panic
        let _runtime = config.runtime();
    }

    #[test]
    fn sandbox_config_merge() {
        let global = Config {
            sandbox: SandboxConfig {
                enabled: Some(true),
                container: ContainerConfig {
                    runtime: Some(SandboxRuntime::Docker),
                    ..Default::default()
                },
                image: Some("global-image".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let project = Config {
            sandbox: SandboxConfig {
                image: Some("project-image".to_string()),
                container: ContainerConfig {
                    runtime: Some(SandboxRuntime::Podman),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };

        let merged = merge_configs(global, project);
        assert!(merged.sandbox.is_enabled()); // from global
        assert_eq!(merged.sandbox.resolved_image("claude"), "project-image"); // project overrides global
        assert_eq!(merged.sandbox.runtime(), SandboxRuntime::Podman); // from project
    }

    #[test]
    fn sandbox_provision_merge_override() {
        let global = Config {
            sandbox: SandboxConfig {
                lima: LimaConfig {
                    provision: Some("echo global".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let project = Config {
            sandbox: SandboxConfig {
                lima: LimaConfig {
                    provision: Some("echo project".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };

        let merged = merge_configs(global, project);
        assert_eq!(merged.sandbox.lima.provision_script(), Some("echo project"));
    }

    #[test]
    fn sandbox_provision_merge_fallback() {
        let global = Config {
            sandbox: SandboxConfig {
                lima: LimaConfig {
                    provision: Some("echo global".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let project = Config::default();

        let merged = merge_configs(global, project);
        assert_eq!(merged.sandbox.lima.provision_script(), Some("echo global"));
    }

    #[test]
    fn sandbox_provision_empty_disables_global() {
        let global = Config {
            sandbox: SandboxConfig {
                lima: LimaConfig {
                    provision: Some("echo global".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let project = Config {
            sandbox: SandboxConfig {
                lima: LimaConfig {
                    provision: Some("".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };

        let merged = merge_configs(global, project);
        // Empty string wins over global (project explicitly set it)
        assert_eq!(merged.sandbox.lima.provision, Some("".to_string()));
        // But provision_script() filters it out
        assert_eq!(merged.sandbox.lima.provision_script(), None);
    }

    #[test]
    fn sandbox_skip_default_provision_defaults_false() {
        let config = LimaConfig::default();
        assert!(!config.skip_default_provision());
    }

    #[test]
    fn sandbox_skip_default_provision_merge() {
        let global = Config {
            sandbox: SandboxConfig {
                lima: LimaConfig {
                    skip_default_provision: Some(true),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let project = Config::default();

        let merged = merge_configs(global, project);
        assert!(merged.sandbox.lima.skip_default_provision());
    }

    #[test]
    fn sandbox_skip_default_provision_project_overrides() {
        let global = Config {
            sandbox: SandboxConfig {
                lima: LimaConfig {
                    skip_default_provision: Some(true),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let project = Config {
            sandbox: SandboxConfig {
                lima: LimaConfig {
                    skip_default_provision: Some(false),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };

        let merged = merge_configs(global, project);
        assert!(!merged.sandbox.lima.skip_default_provision());
    }

    #[test]
    fn test_rpc_host_address_defaults() {
        assert_eq!(
            SandboxRuntime::Docker.rpc_host_address(),
            "host.docker.internal"
        );
        assert_eq!(
            SandboxRuntime::Podman.rpc_host_address(),
            "host.containers.internal"
        );
    }

    #[test]
    fn test_resolved_rpc_host_uses_override() {
        let config = SandboxConfig {
            rpc_host: Some("custom.host.local".to_string()),
            ..Default::default()
        };
        assert_eq!(config.resolved_rpc_host(), "custom.host.local");
    }

    #[test]
    fn test_resolved_rpc_host_falls_back_to_runtime() {
        let config = SandboxConfig {
            container: ContainerConfig {
                runtime: Some(SandboxRuntime::Podman),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(config.resolved_rpc_host(), "host.containers.internal");
    }

    #[test]
    fn sandbox_toolchain_defaults_to_auto() {
        let config = SandboxConfig::default();
        assert_eq!(config.toolchain(), ToolchainMode::Auto);
    }

    #[test]
    fn sandbox_toolchain_merge_project_overrides() {
        let global = Config {
            sandbox: SandboxConfig {
                toolchain: Some(ToolchainMode::Auto),
                ..Default::default()
            },
            ..Default::default()
        };
        let project = Config {
            sandbox: SandboxConfig {
                toolchain: Some(ToolchainMode::Off),
                ..Default::default()
            },
            ..Default::default()
        };
        let merged = merge_configs(global, project);
        assert_eq!(merged.sandbox.toolchain(), ToolchainMode::Off);
    }

    #[test]
    fn sandbox_toolchain_merge_fallback_to_global() {
        let global = Config {
            sandbox: SandboxConfig {
                toolchain: Some(ToolchainMode::Devbox),
                ..Default::default()
            },
            ..Default::default()
        };
        let project = Config::default();
        let merged = merge_configs(global, project);
        assert_eq!(merged.sandbox.toolchain(), ToolchainMode::Devbox);
    }

    #[test]
    fn test_sandbox_host_commands_default_empty() {
        let config = SandboxConfig::default();
        assert!(config.host_commands().is_empty());
    }

    #[test]
    fn test_sandbox_host_commands_global_only() {
        // Project config is ignored -- only global matters
        let global = Config {
            sandbox: SandboxConfig {
                host_commands: Some(vec!["just".to_string(), "cargo".to_string()]),
                ..Default::default()
            },
            ..Default::default()
        };
        let project = Config {
            sandbox: SandboxConfig {
                host_commands: Some(vec!["npm".to_string()]),
                ..Default::default()
            },
            ..Default::default()
        };

        let merged = merge_configs(global, project);
        assert_eq!(
            merged.sandbox.host_commands(),
            &["just".to_string(), "cargo".to_string()]
        );
    }

    #[test]
    fn test_sandbox_host_commands_project_ignored_when_no_global() {
        let global = Config::default(); // no host_commands
        let project = Config {
            sandbox: SandboxConfig {
                host_commands: Some(vec!["rm".to_string()]),
                ..Default::default()
            },
            ..Default::default()
        };

        let merged = merge_configs(global, project);
        assert!(merged.sandbox.host_commands().is_empty());
    }

    #[test]
    fn test_sandbox_host_commands_uses_global() {
        let global = Config {
            sandbox: SandboxConfig {
                host_commands: Some(vec!["just".to_string()]),
                ..Default::default()
            },
            ..Default::default()
        };
        let project = Config::default();

        let merged = merge_configs(global, project);
        assert_eq!(merged.sandbox.host_commands(), &["just".to_string()]);
    }

    #[test]
    fn test_allow_unsandboxed_host_exec_defaults_false() {
        let config = SandboxConfig::default();
        assert!(!config.allow_unsandboxed_host_exec());
    }

    #[test]
    fn test_allow_unsandboxed_host_exec_global_only() {
        let global = Config {
            sandbox: SandboxConfig {
                dangerously_allow_unsandboxed_host_exec: Some(true),
                ..Default::default()
            },
            ..Default::default()
        };
        // Project tries to set it -- should be ignored
        let project = Config {
            sandbox: SandboxConfig {
                dangerously_allow_unsandboxed_host_exec: Some(false),
                ..Default::default()
            },
            ..Default::default()
        };

        let merged = merge_configs(global, project);
        assert!(merged.sandbox.allow_unsandboxed_host_exec());
    }

    #[test]
    fn test_allow_unsandboxed_host_exec_not_set_in_project() {
        let global = Config::default();
        let project = Config {
            sandbox: SandboxConfig {
                dangerously_allow_unsandboxed_host_exec: Some(true),
                ..Default::default()
            },
            ..Default::default()
        };

        let merged = merge_configs(global, project);
        // Project value should be ignored
        assert!(!merged.sandbox.allow_unsandboxed_host_exec());
    }

    #[test]
    fn test_sandbox_rpc_host_global_only() {
        // Project config is ignored -- only global matters
        let global = Config {
            sandbox: SandboxConfig {
                rpc_host: Some("trusted.host".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let project = Config {
            sandbox: SandboxConfig {
                rpc_host: Some("evil.attacker.com".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };

        let merged = merge_configs(global, project);
        assert_eq!(merged.sandbox.rpc_host, Some("trusted.host".to_string()));
    }

    #[test]
    fn test_sandbox_rpc_host_project_ignored_when_no_global() {
        let global = Config::default(); // no rpc_host
        let project = Config {
            sandbox: SandboxConfig {
                rpc_host: Some("evil.attacker.com".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };

        let merged = merge_configs(global, project);
        assert!(merged.sandbox.rpc_host.is_none());
    }

    #[test]
    fn test_sandbox_rpc_host_uses_global() {
        let global = Config {
            sandbox: SandboxConfig {
                rpc_host: Some("custom.host".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let project = Config::default();

        let merged = merge_configs(global, project);
        assert_eq!(merged.sandbox.rpc_host, Some("custom.host".to_string()));
    }

    #[test]
    fn test_sandbox_image_project_overrides_global() {
        let global = Config {
            sandbox: SandboxConfig {
                image: Some("global:latest".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let project = Config {
            sandbox: SandboxConfig {
                image: Some("custom:latest".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };

        let merged = merge_configs(global, project);
        assert_eq!(merged.sandbox.image, Some("custom:latest".to_string()));
    }

    #[test]
    fn test_sandbox_image_project_used_when_no_global() {
        let global = Config::default();
        let project = Config {
            sandbox: SandboxConfig {
                image: Some("custom:latest".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };

        let merged = merge_configs(global, project);
        assert_eq!(merged.sandbox.image, Some("custom:latest".to_string()));
    }

    #[test]
    fn test_sandbox_image_falls_back_to_global() {
        let global = Config {
            sandbox: SandboxConfig {
                image: Some("global:latest".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let project = Config::default();

        let merged = merge_configs(global, project);
        assert_eq!(merged.sandbox.image, Some("global:latest".to_string()));
    }

    #[test]
    fn test_excluded_files_default_empty() {
        let config = ContainerConfig::default();
        assert!(config.excluded_files().is_empty());
    }

    #[test]
    fn test_excluded_files_accessor() {
        let config = ContainerConfig {
            excluded_files: Some(vec![".env".into(), ".env.local".into()]),
            ..Default::default()
        };
        assert_eq!(config.excluded_files(), &[".env", ".env.local"]);
    }

    #[test]
    fn test_excluded_files_merge_project_is_ignored() {
        // Security: excluded_files is global-only. A project config (.muxix.yaml)
        // MUST NOT be able to weaken or replace the global list; otherwise a
        // malicious repo could delete user-level secret protections.
        let global = Config {
            sandbox: SandboxConfig {
                container: ContainerConfig {
                    excluded_files: Some(vec![".env".into()]),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let project = Config {
            sandbox: SandboxConfig {
                container: ContainerConfig {
                    excluded_files: Some(vec![".env.production".into()]),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };

        let merged = merge_configs(global, project);
        assert_eq!(
            merged.sandbox.container.excluded_files,
            Some(vec![".env".into()])
        );
    }

    #[test]
    fn test_excluded_files_merge_project_only_is_ignored() {
        // A project that sets excluded_files without any global list must NOT
        // take effect -- project config can never set this field.
        let global = Config::default();
        let project = Config {
            sandbox: SandboxConfig {
                container: ContainerConfig {
                    excluded_files: Some(vec![".env".into()]),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };

        let merged = merge_configs(global, project);
        assert_eq!(merged.sandbox.container.excluded_files, None);
    }

    #[test]
    fn test_excluded_files_merge_inherits_global() {
        let global = Config {
            sandbox: SandboxConfig {
                container: ContainerConfig {
                    excluded_files: Some(vec![".env".into()]),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let merged = merge_configs(global, Config::default());
        assert_eq!(
            merged.sandbox.container.excluded_files,
            Some(vec![".env".into()])
        );
    }

    #[test]
    fn test_sandbox_env_passthrough_global_only() {
        // Project config is ignored -- only global matters
        let global = Config {
            sandbox: SandboxConfig {
                env_passthrough: Some(vec!["GITHUB_TOKEN".to_string()]),
                ..Default::default()
            },
            ..Default::default()
        };
        let project = Config {
            sandbox: SandboxConfig {
                env_passthrough: Some(vec!["AWS_SECRET_ACCESS_KEY".to_string()]),
                ..Default::default()
            },
            ..Default::default()
        };

        let merged = merge_configs(global, project);
        assert_eq!(
            merged.sandbox.env_passthrough,
            Some(vec!["GITHUB_TOKEN".to_string()])
        );
    }

    #[test]
    fn test_sandbox_env_passthrough_project_ignored_when_no_global() {
        let global = Config::default();
        let project = Config {
            sandbox: SandboxConfig {
                env_passthrough: Some(vec!["AWS_SECRET_ACCESS_KEY".to_string()]),
                ..Default::default()
            },
            ..Default::default()
        };

        let merged = merge_configs(global, project);
        assert!(merged.sandbox.env_passthrough.is_none());
    }

    #[test]
    fn test_sandbox_env_passthrough_uses_global() {
        let global = Config {
            sandbox: SandboxConfig {
                env_passthrough: Some(vec!["GITHUB_TOKEN".to_string()]),
                ..Default::default()
            },
            ..Default::default()
        };
        let project = Config::default();

        let merged = merge_configs(global, project);
        assert_eq!(
            merged.sandbox.env_passthrough,
            Some(vec!["GITHUB_TOKEN".to_string()])
        );
    }

    #[test]
    fn sandbox_env_global_only() {
        // Project config is ignored -- only global matters
        let global = Config {
            sandbox: SandboxConfig {
                env: Some(HashMap::from([(
                    "GH_TOKEN".to_string(),
                    "global_token".to_string(),
                )])),
                ..Default::default()
            },
            ..Default::default()
        };
        let project = Config {
            sandbox: SandboxConfig {
                env: Some(HashMap::from([(
                    "GH_TOKEN".to_string(),
                    "project_token".to_string(),
                )])),
                ..Default::default()
            },
            ..Default::default()
        };

        let merged = merge_configs(global, project);
        let env = merged.sandbox.env.unwrap();
        assert_eq!(env.get("GH_TOKEN").unwrap(), "global_token");
    }

    #[test]
    fn sandbox_env_project_ignored_when_no_global() {
        let global = Config::default();
        let project = Config {
            sandbox: SandboxConfig {
                env: Some(HashMap::from([(
                    "GH_TOKEN".to_string(),
                    "project_token".to_string(),
                )])),
                ..Default::default()
            },
            ..Default::default()
        };

        let merged = merge_configs(global, project);
        assert!(merged.sandbox.env.is_none());
    }

    #[test]
    fn sandbox_env_uses_global() {
        let global = Config {
            sandbox: SandboxConfig {
                env: Some(HashMap::from([(
                    "GH_TOKEN".to_string(),
                    "global_token".to_string(),
                )])),
                ..Default::default()
            },
            ..Default::default()
        };
        let project = Config::default();

        let merged = merge_configs(global, project);
        let env = merged.sandbox.env.unwrap();
        assert_eq!(env.get("GH_TOKEN").unwrap(), "global_token");
    }

    #[test]
    fn sandbox_env_vars_accessor() {
        let config = SandboxConfig {
            env: Some(HashMap::from([("KEY".to_string(), "VALUE".to_string())])),
            ..Default::default()
        };
        let vars = config.env_vars();
        assert_eq!(vars.len(), 1);
        assert_eq!(vars[0], ("KEY", "VALUE"));
    }

    #[test]
    fn sandbox_env_vars_accessor_empty() {
        let config = SandboxConfig::default();
        assert!(config.env_vars().is_empty());
    }

    #[test]
    fn test_extra_mount_parse_simple_string() {
        let yaml = r#"extra_mounts: ["/tmp/notes"]"#;
        let config: SandboxConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.extra_mounts().len(), 1);
        let (host, guest, read_only) = config.extra_mounts()[0].resolve().unwrap();
        assert_eq!(host, std::path::PathBuf::from("/tmp/notes"));
        assert_eq!(guest, std::path::PathBuf::from("/tmp/notes"));
        assert!(read_only);
    }

    #[test]
    fn test_extra_mount_parse_spec() {
        let yaml = r#"
extra_mounts:
  - host_path: /tmp/data
    guest_path: /mnt/data
    writable: true
"#;
        let config: SandboxConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.extra_mounts().len(), 1);
        let (host, guest, read_only) = config.extra_mounts()[0].resolve().unwrap();
        assert_eq!(host, std::path::PathBuf::from("/tmp/data"));
        assert_eq!(guest, std::path::PathBuf::from("/mnt/data"));
        assert!(!read_only);
    }

    #[test]
    fn test_extra_mount_spec_defaults() {
        let yaml = r#"
extra_mounts:
  - host_path: /tmp/data
"#;
        let config: SandboxConfig = serde_yaml::from_str(yaml).unwrap();
        let (host, guest, read_only) = config.extra_mounts()[0].resolve().unwrap();
        assert_eq!(host, std::path::PathBuf::from("/tmp/data"));
        // guest defaults to host path
        assert_eq!(guest, std::path::PathBuf::from("/tmp/data"));
        // writable defaults to false (read_only = true)
        assert!(read_only);
    }

    #[test]
    fn test_extra_mount_tilde_expansion() {
        let mount = ExtraMount::Path("~/notes".to_string());
        let (host, guest, _) = mount.resolve().unwrap();
        // Should expand ~ to home dir
        assert!(!host.to_string_lossy().starts_with('~'));
        assert!(host.to_string_lossy().ends_with("/notes"));
        // Guest should mirror expanded host
        assert_eq!(host, guest);
    }

    #[test]
    fn test_extra_mount_mixed_list() {
        let yaml = r#"
extra_mounts:
  - /tmp/notes
  - host_path: /tmp/data
    guest_path: /mnt/data
    writable: true
"#;
        let config: SandboxConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.extra_mounts().len(), 2);

        let (host0, _, ro0) = config.extra_mounts()[0].resolve().unwrap();
        assert_eq!(host0, std::path::PathBuf::from("/tmp/notes"));
        assert!(ro0);

        let (host1, guest1, ro1) = config.extra_mounts()[1].resolve().unwrap();
        assert_eq!(host1, std::path::PathBuf::from("/tmp/data"));
        assert_eq!(guest1, std::path::PathBuf::from("/mnt/data"));
        assert!(!ro1);
    }

    #[test]
    fn test_extra_mounts_default_empty() {
        let config = SandboxConfig::default();
        assert!(config.extra_mounts().is_empty());
    }

    #[test]
    fn test_extra_mounts_global_only() {
        // Project config is ignored -- only global matters
        let global = Config {
            sandbox: SandboxConfig {
                extra_mounts: Some(vec![ExtraMount::Path("/global/path".to_string())]),
                ..Default::default()
            },
            ..Default::default()
        };
        let project = Config {
            sandbox: SandboxConfig {
                extra_mounts: Some(vec![ExtraMount::Path("/project/path".to_string())]),
                ..Default::default()
            },
            ..Default::default()
        };

        let merged = merge_configs(global, project);
        assert_eq!(merged.sandbox.extra_mounts().len(), 1);
        let (host, _, _) = merged.sandbox.extra_mounts()[0].resolve().unwrap();
        assert_eq!(host, std::path::PathBuf::from("/global/path"));
    }

    #[test]
    fn test_extra_mounts_project_ignored_when_no_global() {
        let global = Config::default(); // no extra_mounts
        let project = Config {
            sandbox: SandboxConfig {
                extra_mounts: Some(vec![ExtraMount::Path("/project/path".to_string())]),
                ..Default::default()
            },
            ..Default::default()
        };

        let merged = merge_configs(global, project);
        assert!(merged.sandbox.extra_mounts().is_empty());
    }

    #[test]
    fn test_extra_mounts_uses_global() {
        let global = Config {
            sandbox: SandboxConfig {
                extra_mounts: Some(vec![ExtraMount::Path("/global/path".to_string())]),
                ..Default::default()
            },
            ..Default::default()
        };
        let project = Config::default();

        let merged = merge_configs(global, project);
        assert_eq!(merged.sandbox.extra_mounts().len(), 1);
        let (host, _, _) = merged.sandbox.extra_mounts()[0].resolve().unwrap();
        assert_eq!(host, std::path::PathBuf::from("/global/path"));
    }

    #[test]
    fn test_resolved_agent_config_dir_with_placeholder() {
        let config = SandboxConfig {
            agent_config_dir: Some("~/sandbox/{agent}".to_string()),
            ..Default::default()
        };
        let dir = config.resolved_agent_config_dir("claude").unwrap();
        let home = home::home_dir().unwrap();
        assert_eq!(dir, home.join("sandbox/claude"));
    }

    #[test]
    fn test_resolved_agent_config_dir_without_placeholder() {
        let config = SandboxConfig {
            agent_config_dir: Some("~/my-config".to_string()),
            ..Default::default()
        };
        let dir = config.resolved_agent_config_dir("claude").unwrap();
        let home = home::home_dir().unwrap();
        assert_eq!(dir, home.join("my-config"));
    }

    #[test]
    fn test_resolved_agent_config_dir_default() {
        let config = SandboxConfig::default();
        let dir = config.resolved_agent_config_dir("claude").unwrap();
        let home = home::home_dir().unwrap();
        assert_eq!(dir, home.join(".claude"));
    }

    #[test]
    fn test_resolved_agent_config_dir_unknown_agent_default() {
        let config = SandboxConfig::default();
        assert!(config.resolved_agent_config_dir("unknown").is_none());
    }

    #[test]
    fn test_resolved_agent_config_dir_unknown_agent_custom() {
        let config = SandboxConfig {
            agent_config_dir: Some("/custom/{agent}".to_string()),
            ..Default::default()
        };
        // Custom dir always returns Some, even for unknown agents
        let dir = config.resolved_agent_config_dir("unknown").unwrap();
        assert_eq!(dir, std::path::PathBuf::from("/custom/unknown"));
    }

    #[test]
    fn test_agent_config_dir_global_only() {
        let global = Config {
            sandbox: SandboxConfig {
                agent_config_dir: Some("~/global/{agent}".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let project = Config {
            sandbox: SandboxConfig {
                agent_config_dir: Some("~/project/{agent}".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let merged = merge_configs(global, project);
        assert_eq!(
            merged.sandbox.agent_config_dir,
            Some("~/global/{agent}".to_string())
        );
    }

    #[test]
    fn test_agent_config_dir_project_ignored_when_no_global() {
        let global = Config::default();
        let project = Config {
            sandbox: SandboxConfig {
                agent_config_dir: Some("~/project/{agent}".to_string()),
                ..Default::default()
            },
            ..Default::default()
        };
        let merged = merge_configs(global, project);
        assert!(merged.sandbox.agent_config_dir.is_none());
    }

    #[test]
    fn test_extra_mount_rejects_relative_host_path() {
        let mount = ExtraMount::Path("relative/path".to_string());
        let result = mount.resolve();
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("must be absolute"));
    }

    #[test]
    fn test_extra_mount_rejects_relative_guest_path() {
        let mount = ExtraMount::Spec {
            host_path: "/tmp/data".to_string(),
            guest_path: Some("relative/guest".to_string()),
            writable: None,
        };
        let result = mount.resolve();
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("guest_path must be absolute"));
    }

    #[test]
    fn sandbox_nested_yaml_format() {
        let yaml = r#"
enabled: true
backend: lima
lima:
  isolation: shared
  cpus: 16
  memory: 16GiB
container:
  runtime: podman
"#;
        let config: SandboxConfig = serde_yaml::from_str(yaml).unwrap();

        assert!(config.is_enabled());
        assert_eq!(config.lima.isolation(), super::IsolationLevel::Shared);
        assert_eq!(config.lima.cpus(), 16);
        assert_eq!(config.lima.memory(), "16GiB");
        assert_eq!(config.container.runtime(), SandboxRuntime::Podman);
    }

    #[test]
    fn sandbox_lima_config_merge() {
        let global = Config {
            sandbox: SandboxConfig {
                lima: LimaConfig {
                    isolation: Some(super::IsolationLevel::Shared),
                    cpus: Some(4),
                    memory: Some("4GiB".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let project = Config {
            sandbox: SandboxConfig {
                lima: LimaConfig {
                    cpus: Some(8),
                    provision: Some("echo project".to_string()),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };

        let merged = merge_configs(global, project).sandbox.lima;
        // Project overrides
        assert_eq!(merged.cpus(), 8);
        assert_eq!(merged.provision_script(), Some("echo project"));
        // Global fallback
        assert_eq!(merged.isolation(), super::IsolationLevel::Shared);
        assert_eq!(merged.memory(), "4GiB");
    }

    #[test]
    fn sandbox_container_config_merge() {
        let global = Config {
            sandbox: SandboxConfig {
                container: ContainerConfig {
                    runtime: Some(SandboxRuntime::Docker),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let project = Config {
            sandbox: SandboxConfig {
                container: ContainerConfig {
                    runtime: Some(SandboxRuntime::Podman),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };

        let merged = merge_configs(global, project).sandbox.container;
        assert_eq!(merged.runtime(), SandboxRuntime::Podman);
    }

    // --- Network config tests ---

    #[test]
    fn network_policy_defaults_to_allow() {
        let config = SandboxConfig::default();
        assert_eq!(config.network.policy(), NetworkPolicy::Allow);
        assert!(!config.network_policy_is_deny());
    }

    #[test]
    fn network_policy_deny() {
        let config = SandboxConfig {
            network: NetworkConfig {
                policy: Some(NetworkPolicy::Deny),
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(config.network.policy(), NetworkPolicy::Deny);
        assert!(config.network_policy_is_deny());
    }

    #[test]
    fn network_allowed_domains_default_empty() {
        let config = NetworkConfig::default();
        assert!(config.allowed_domains().is_empty());
    }

    #[test]
    fn network_config_global_only() {
        let global = Config {
            sandbox: SandboxConfig {
                network: NetworkConfig {
                    policy: Some(NetworkPolicy::Deny),
                    allowed_domains: Some(vec![AllowedDomainEntry::Plain(
                        "api.anthropic.com".to_string(),
                    )]),
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let project = Config {
            sandbox: SandboxConfig {
                network: NetworkConfig {
                    policy: Some(NetworkPolicy::Allow),
                    allowed_domains: Some(vec![AllowedDomainEntry::Plain("evil.com".to_string())]),
                },
                ..Default::default()
            },
            ..Default::default()
        };

        let merged = merge_configs(global, project);
        // Global value should win
        assert_eq!(merged.sandbox.network.policy(), NetworkPolicy::Deny);
        assert_eq!(
            merged.sandbox.network.allowed_domains(),
            &[AllowedDomainEntry::Plain("api.anthropic.com".to_string())]
        );
    }

    #[test]
    fn network_config_project_ignored_when_no_global() {
        let global = Config::default();
        let project = Config {
            sandbox: SandboxConfig {
                network: NetworkConfig {
                    policy: Some(NetworkPolicy::Deny),
                    allowed_domains: Some(vec![AllowedDomainEntry::Plain("evil.com".to_string())]),
                },
                ..Default::default()
            },
            ..Default::default()
        };

        let merged = merge_configs(global, project);
        assert_eq!(merged.sandbox.network.policy(), NetworkPolicy::Allow);
        assert!(merged.sandbox.network.allowed_domains().is_empty());
    }

    #[test]
    fn network_config_uses_global() {
        let global = Config {
            sandbox: SandboxConfig {
                network: NetworkConfig {
                    policy: Some(NetworkPolicy::Deny),
                    allowed_domains: Some(vec![AllowedDomainEntry::Plain(
                        "github.com".to_string(),
                    )]),
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let project = Config::default();

        let merged = merge_configs(global, project);
        assert_eq!(merged.sandbox.network.policy(), NetworkPolicy::Deny);
        assert_eq!(
            merged.sandbox.network.allowed_domains(),
            &[AllowedDomainEntry::Plain("github.com".to_string())]
        );
    }

    #[test]
    fn validate_domain_rejects_ip_literal() {
        assert!(validate_domain("192.168.1.1").is_err());
        assert!(validate_domain("127.0.0.1").is_err());
        assert!(validate_domain("::1").is_err());
    }

    #[test]
    fn validate_domain_rejects_trailing_dot() {
        assert!(validate_domain("example.com.").is_err());
    }

    #[test]
    fn validate_domain_rejects_malformed_wildcard() {
        assert!(validate_domain("foo.*.com").is_err());
        assert!(validate_domain("*foo.com").is_err());
    }

    #[test]
    fn validate_domain_rejects_empty() {
        assert!(validate_domain("").is_err());
    }

    #[test]
    fn validate_domain_accepts_valid() {
        assert!(validate_domain("example.com").is_ok());
        assert!(validate_domain("api.anthropic.com").is_ok());
        assert!(validate_domain("*.googleapis.com").is_ok());
        assert!(validate_domain("*.github.com").is_ok());
    }

    #[test]
    fn network_config_validate_catches_bad_domains() {
        let config = NetworkConfig {
            policy: Some(NetworkPolicy::Deny),
            allowed_domains: Some(vec![
                AllowedDomainEntry::Plain("good.com".to_string()),
                AllowedDomainEntry::Plain("192.168.1.1".to_string()),
            ]),
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn network_config_validate_passes_good_domains() {
        let config = NetworkConfig {
            policy: Some(NetworkPolicy::Deny),
            allowed_domains: Some(vec![
                AllowedDomainEntry::Plain("api.anthropic.com".to_string()),
                AllowedDomainEntry::Plain("*.github.com".to_string()),
                AllowedDomainEntry::Plain("registry.npmjs.org".to_string()),
            ]),
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn network_config_yaml_roundtrip() {
        let yaml = r#"
network:
  policy: deny
  allowed_domains:
    - api.anthropic.com
    - "*.github.com"
"#;
        let config: SandboxConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.network.policy(), NetworkPolicy::Deny);
        assert_eq!(config.network.allowed_domains().len(), 2);
        assert_eq!(
            config.network.allowed_domain_rules(),
            vec![
                super::AllowedDomainRule {
                    host: "api.anthropic.com".to_string(),
                    allow_private_ips: false,
                },
                super::AllowedDomainRule {
                    host: "*.github.com".to_string(),
                    allow_private_ips: false,
                },
            ]
        );
    }

    #[test]
    fn network_config_yaml_parses_private_domain_rule() {
        let yaml = r#"
network:
  policy: deny
  allowed_domains:
    - host: artifactory.example.com
      allow_private_ips: true
"#;
        let config: SandboxConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            config.network.allowed_domains(),
            &[AllowedDomainEntry::Detailed(AllowedDomainDetails {
                host: "artifactory.example.com".to_string(),
                allow_private_ips: true,
            })]
        );
    }

    #[test]
    fn network_config_yaml_rejects_unknown_allowed_domain_field() {
        let yaml = r#"
network:
  policy: deny
  allowed_domains:
    - host: artifactory.example.com
      alow_private: true
"#;
        assert!(serde_yaml::from_str::<SandboxConfig>(yaml).is_err());
    }

    #[test]
    fn network_config_validate_rejects_wildcard_private_rule() {
        let config = NetworkConfig {
            policy: Some(NetworkPolicy::Deny),
            allowed_domains: Some(vec![AllowedDomainEntry::Detailed(AllowedDomainDetails {
                host: "*.example.com".to_string(),
                allow_private_ips: true,
            })]),
        };
        let err = config.validate().unwrap_err().to_string();
        assert!(err.contains("allow_private_ips"));
    }

    #[test]
    fn network_config_load_rejects_wildcard_private_rule() {
        let config = Config {
            sandbox: SandboxConfig {
                network: NetworkConfig {
                    policy: Some(NetworkPolicy::Deny),
                    allowed_domains: Some(vec![AllowedDomainEntry::Detailed(
                        AllowedDomainDetails {
                            host: "*.example.com".to_string(),
                            allow_private_ips: true,
                        },
                    )]),
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let err = Config::apply_defaults(config, None, None, std::path::Path::new("")).unwrap_err();
        assert!(err.to_string().contains("Invalid sandbox network config"));
    }

    // --- ContainerDevice / group_add tests ---

    #[test]
    fn container_device_string_form_parses() {
        let yaml = r#"
container:
  devices:
    - /dev/kvm
    - /dev/dri:/dev/dri
    - /dev/bus/usb/001/002:/dev/bus/usb/001/002:rwm
"#;
        let config: SandboxConfig = serde_yaml::from_str(yaml).unwrap();
        let devs = config.container.devices.unwrap();
        assert_eq!(devs.len(), 3);
        assert_eq!(devs[0].to_arg(), "/dev/kvm");
        assert_eq!(devs[1].to_arg(), "/dev/dri:/dev/dri");
        assert_eq!(
            devs[2].to_arg(),
            "/dev/bus/usb/001/002:/dev/bus/usb/001/002:rwm"
        );
    }

    #[test]
    fn container_device_struct_form_parses() {
        let yaml = r#"
container:
  devices:
    - host_path: /dev/bus/usb/001/002
      guest_path: /dev/bus/usb/001/002
      permissions: rw
    - host_path: /dev/kvm
"#;
        let config: SandboxConfig = serde_yaml::from_str(yaml).unwrap();
        let devs = config.container.devices.unwrap();
        assert_eq!(devs.len(), 2);
        assert_eq!(
            devs[0].to_arg(),
            "/dev/bus/usb/001/002:/dev/bus/usb/001/002:rw"
        );
        assert_eq!(devs[1].to_arg(), "/dev/kvm");
    }

    #[test]
    fn container_device_validation_rejects_relative_host_path() {
        let dev = ContainerDevice::String("ttyUSB0".to_string());
        assert!(dev.validate().is_err());
    }

    #[test]
    fn container_device_validation_rejects_whitespace() {
        let dev = ContainerDevice::String(" /dev/kvm".to_string());
        assert!(dev.validate().is_err());
        let dev = ContainerDevice::String("/dev/kvm ".to_string());
        assert!(dev.validate().is_err());
        let dev = ContainerDevice::String("/dev/kvm:/dev/kvm:r w".to_string());
        assert!(dev.validate().is_err());

        let dev = ContainerDevice::Struct {
            host_path: "/dev/kvm ".to_string(),
            guest_path: None,
            permissions: None,
        };
        assert!(dev.validate().is_err());
        let dev = ContainerDevice::Struct {
            host_path: "/dev/kvm".to_string(),
            guest_path: Some(" /dev/kvm".to_string()),
            permissions: None,
        };
        assert!(dev.validate().is_err());
    }

    #[test]
    fn container_device_validation_rejects_bad_permissions() {
        let dev = ContainerDevice::String("/dev/kvm:/dev/kvm:zzz".to_string());
        assert!(dev.validate().is_err());
    }

    #[test]
    fn container_device_validation_accepts_single_path() {
        let dev = ContainerDevice::String("/dev/kvm".to_string());
        assert!(dev.validate().is_ok());
    }

    #[test]
    fn container_device_validation_accepts_path_with_permissions_only() {
        let dev = ContainerDevice::String("/dev/kvm:rwm".to_string());
        assert!(dev.validate().is_ok());
    }

    #[test]
    fn container_device_validation_accepts_full_triple() {
        let dev = ContainerDevice::String("/dev/dri:/dev/dri:rw".to_string());
        assert!(dev.validate().is_ok());
    }

    #[test]
    fn container_device_struct_validation() {
        let bad = ContainerDevice::Struct {
            host_path: "dev/kvm".to_string(),
            guest_path: None,
            permissions: None,
        };
        assert!(bad.validate().is_err());

        let bad_perms = ContainerDevice::Struct {
            host_path: "/dev/kvm".to_string(),
            guest_path: None,
            permissions: Some("zzz".to_string()),
        };
        assert!(bad_perms.validate().is_err());

        let ok = ContainerDevice::Struct {
            host_path: "/dev/kvm".to_string(),
            guest_path: Some("/dev/kvm".to_string()),
            permissions: Some("rw".to_string()),
        };
        assert!(ok.validate().is_ok());
    }

    #[test]
    fn group_add_validation() {
        assert!(validate_group_add_entry("dialout").is_ok());
        assert!(validate_group_add_entry("20").is_ok());
        assert!(validate_group_add_entry("").is_err());
        assert!(validate_group_add_entry("dial out").is_err());
        assert!(validate_group_add_entry("a,b").is_err());
        assert!(validate_group_add_entry("a:b").is_err());
    }

    #[test]
    fn group_add_yaml_parses() {
        let yaml = r#"
container:
  group_add:
    - dialout
    - video
    - "46"
"#;
        let config: SandboxConfig = serde_yaml::from_str(yaml).unwrap();
        let groups = config.container.group_add.unwrap();
        assert_eq!(groups, vec!["dialout", "video", "46"]);
    }

    #[test]
    fn container_devices_are_global_only_in_merge() {
        let mut global = Config::default();
        global.sandbox.container.devices =
            Some(vec![ContainerDevice::String("/dev/kvm".to_string())]);
        let mut project = Config::default();
        project.sandbox.container.devices =
            Some(vec![ContainerDevice::String("/dev/ttyUSB0".to_string())]);

        let merged = merge_configs(global, project);
        let devs = merged.sandbox.container.devices.unwrap();
        assert_eq!(devs.len(), 1);
        assert_eq!(devs[0].to_arg(), "/dev/kvm");
    }

    #[test]
    fn container_devices_project_ignored_when_global_empty() {
        let global = Config::default();
        let mut project = Config::default();
        project.sandbox.container.devices =
            Some(vec![ContainerDevice::String("/dev/ttyUSB0".to_string())]);

        let merged = merge_configs(global, project);
        assert!(merged.sandbox.container.devices.is_none());
    }

    #[test]
    fn container_group_add_global_only_in_merge() {
        let mut global = Config::default();
        global.sandbox.container.group_add = Some(vec!["dialout".to_string()]);
        let mut project = Config::default();
        project.sandbox.container.group_add = Some(vec!["video".to_string()]);

        let merged = merge_configs(global, project);
        let groups = merged.sandbox.container.group_add.unwrap();
        assert_eq!(groups, vec!["dialout".to_string()]);
    }

    // --- WindowConfig tests ---

    use super::{WindowConfig, validate_windows_config};

    #[test]
    fn parse_windows_config_named() {
        let yaml = r#"
windows:
  - name: editor
    panes:
      - command: <agent>
        focus: true
      - split: horizontal
        size: 20
  - name: tests
    panes:
      - command: just test --watch
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        let windows = config.windows.unwrap();
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].name.as_deref(), Some("editor"));
        assert_eq!(windows[0].panes.as_ref().unwrap().len(), 2);
        assert_eq!(windows[1].name.as_deref(), Some("tests"));
        assert_eq!(windows[1].panes.as_ref().unwrap().len(), 1);
    }

    #[test]
    fn parse_windows_config_unnamed() {
        let yaml = r#"
windows:
  - panes:
      - command: tail -f app.log
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        let windows = config.windows.unwrap();
        assert_eq!(windows.len(), 1);
        assert!(windows[0].name.is_none());
    }

    #[test]
    fn parse_windows_config_mixed() {
        let yaml = r#"
windows:
  - name: editor
    panes:
      - command: <agent>
        focus: true
  - panes:
      - command: tail -f app.log
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        let windows = config.windows.unwrap();
        assert_eq!(windows.len(), 2);
        assert_eq!(windows[0].name.as_deref(), Some("editor"));
        assert!(windows[1].name.is_none());
    }

    #[test]
    fn validate_windows_config_empty_errors() {
        let result = validate_windows_config(&[]);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("must not be empty")
        );
    }

    #[test]
    fn validate_windows_config_valid() {
        let windows = vec![
            WindowConfig {
                name: Some("editor".to_string()),
                panes: Some(vec![super::PaneConfig {
                    command: Some("<agent>".to_string()),
                    focus: true,
                    ..Default::default()
                }]),
            },
            WindowConfig {
                name: None,
                panes: Some(vec![super::PaneConfig {
                    command: Some("tail -f app.log".to_string()),
                    ..Default::default()
                }]),
            },
        ];
        assert!(validate_windows_config(&windows).is_ok());
    }

    #[test]
    fn validate_windows_config_bad_pane_errors() {
        let windows = vec![WindowConfig {
            name: Some("bad".to_string()),
            panes: Some(vec![super::PaneConfig {
                split: Some(super::SplitDirection::Horizontal), // first pane cannot have split
                ..Default::default()
            }]),
        }];
        let result = validate_windows_config(&windows);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Window 0"));
    }

    #[test]
    fn merge_project_windows_overrides_global_panes() {
        let global = Config {
            panes: Some(vec![super::PaneConfig {
                command: Some("vim".to_string()),
                focus: true,
                ..Default::default()
            }]),
            ..Default::default()
        };
        let project = Config {
            windows: Some(vec![
                WindowConfig {
                    name: Some("editor".to_string()),
                    panes: None,
                },
                WindowConfig {
                    name: Some("tests".to_string()),
                    panes: None,
                },
            ]),
            ..Default::default()
        };

        let merged = merge_configs(global, project);
        // Project windows should win, panes should be cleared
        assert!(merged.windows.is_some());
        assert!(merged.panes.is_none());
        assert_eq!(merged.windows.unwrap().len(), 2);
    }

    #[test]
    fn merge_project_panes_overrides_global_windows() {
        let global = Config {
            windows: Some(vec![WindowConfig {
                name: Some("global-window".to_string()),
                panes: None,
            }]),
            ..Default::default()
        };
        let project = Config {
            panes: Some(vec![super::PaneConfig {
                command: Some("vim".to_string()),
                focus: true,
                ..Default::default()
            }]),
            ..Default::default()
        };

        let merged = merge_configs(global, project);
        // Project panes should win, windows should be cleared
        assert!(merged.panes.is_some());
        assert!(merged.windows.is_none());
    }

    #[test]
    fn merge_global_windows_inherited_when_no_project_layout() {
        let global = Config {
            windows: Some(vec![WindowConfig {
                name: Some("global-window".to_string()),
                panes: None,
            }]),
            ..Default::default()
        };
        let project = Config::default(); // no panes or windows

        let merged = merge_configs(global, project);
        assert!(merged.windows.is_some());
        assert!(merged.panes.is_none());
    }

    #[test]
    fn parse_runtime_apple_container() {
        let yaml = r#"
sandbox:
  container:
    runtime: apple-container
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(
            config.sandbox.container.runtime,
            Some(SandboxRuntime::AppleContainer)
        );
    }

    #[test]
    fn runtime_binary_names() {
        assert_eq!(SandboxRuntime::Docker.binary_name(), "docker");
        assert_eq!(SandboxRuntime::Podman.binary_name(), "podman");
        assert_eq!(SandboxRuntime::AppleContainer.binary_name(), "container");
    }

    #[test]
    fn runtime_rpc_host_addresses() {
        assert_eq!(
            SandboxRuntime::Docker.rpc_host_address(),
            "host.docker.internal"
        );
        assert_eq!(
            SandboxRuntime::Podman.rpc_host_address(),
            "host.containers.internal"
        );
        assert_eq!(
            SandboxRuntime::AppleContainer.rpc_host_address(),
            "192.168.64.1"
        );
    }

    #[test]
    fn runtime_capability_flags() {
        // needs_add_host: only Docker
        assert!(SandboxRuntime::Docker.needs_add_host());
        assert!(!SandboxRuntime::Podman.needs_add_host());
        assert!(!SandboxRuntime::AppleContainer.needs_add_host());

        // needs_userns_keep_id: only Podman
        assert!(!SandboxRuntime::Docker.needs_userns_keep_id());
        assert!(SandboxRuntime::Podman.needs_userns_keep_id());
        assert!(!SandboxRuntime::AppleContainer.needs_userns_keep_id());

        // needs_deny_mode_caps: Docker and Podman, not Apple Container
        assert!(SandboxRuntime::Docker.needs_deny_mode_caps());
        assert!(SandboxRuntime::Podman.needs_deny_mode_caps());
        assert!(!SandboxRuntime::AppleContainer.needs_deny_mode_caps());
    }

    #[test]
    fn runtime_pull_args() {
        assert_eq!(
            SandboxRuntime::Docker.pull_args("img:latest"),
            vec!["pull", "img:latest"]
        );
        assert_eq!(
            SandboxRuntime::Podman.pull_args("img:latest"),
            vec!["pull", "img:latest"]
        );
        assert_eq!(
            SandboxRuntime::AppleContainer.pull_args("img:latest"),
            vec!["image", "pull", "img:latest"]
        );
    }

    #[test]
    fn runtime_serde_name_roundtrip() {
        for runtime in [
            SandboxRuntime::Docker,
            SandboxRuntime::Podman,
            SandboxRuntime::AppleContainer,
        ] {
            let name = runtime.serde_name();
            let parsed = SandboxRuntime::from_serde_name(name).unwrap();
            assert_eq!(parsed, runtime);
        }
    }

    #[test]
    fn runtime_from_serde_name_unknown() {
        assert_eq!(SandboxRuntime::from_serde_name("unknown"), None);
        assert_eq!(SandboxRuntime::from_serde_name(""), None);
    }

    #[test]
    fn runtime_default_memory() {
        assert_eq!(SandboxRuntime::AppleContainer.default_memory(), Some("16G"));
        assert_eq!(SandboxRuntime::Docker.default_memory(), None);
        assert_eq!(SandboxRuntime::Podman.default_memory(), None);
    }

    #[test]
    fn container_config_merge_resources() {
        let global = Config {
            sandbox: SandboxConfig {
                container: ContainerConfig {
                    runtime: Some(SandboxRuntime::Docker),
                    memory: Some("8G".to_string()),
                    cpus: Some(4),
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let project = Config {
            sandbox: SandboxConfig {
                container: ContainerConfig {
                    runtime: None,
                    memory: Some("16G".to_string()),
                    cpus: None,
                    ..Default::default()
                },
                ..Default::default()
            },
            ..Default::default()
        };
        let merged = merge_configs(global, project).sandbox.container;
        assert_eq!(merged.memory.as_deref(), Some("16G")); // project overrides
        assert_eq!(merged.cpus, Some(4)); // falls back to global
        assert_eq!(merged.runtime, Some(SandboxRuntime::Docker));
    }

    // ── Theme config deserialization tests ───────────────────────

    use super::{ThemeConfig, ThemeMode, ThemeScheme};

    #[test]
    fn theme_scheme_slug_roundtrip() {
        for scheme in &ThemeScheme::ALL {
            let slug = scheme.slug();
            assert_eq!(
                ThemeScheme::from_slug(slug),
                Some(*scheme),
                "slug roundtrip failed for {:?}",
                scheme
            );
        }
    }

    #[test]
    fn theme_scheme_next_wraps() {
        let mut current = ThemeScheme::Default;
        for _ in 0..ThemeScheme::ALL.len() {
            current = current.next();
        }
        assert_eq!(current, ThemeScheme::Default);
    }

    #[test]
    fn theme_scheme_all_is_exhaustive() {
        // This match will fail to compile if a variant is added but not listed
        for scheme in &ThemeScheme::ALL {
            match scheme {
                ThemeScheme::Default
                | ThemeScheme::Emberforge
                | ThemeScheme::GlacierSignal
                | ThemeScheme::ObsidianPop
                | ThemeScheme::SlateGarden
                | ThemeScheme::PhosphorArcade
                | ThemeScheme::Lasergrid
                | ThemeScheme::Mossfire
                | ThemeScheme::NightSorbet
                | ThemeScheme::GraphiteCode
                | ThemeScheme::FestivalCircuit
                | ThemeScheme::TealDrift => {}
            }
        }
        assert_eq!(ThemeScheme::ALL.len(), 12);
    }

    #[test]
    fn theme_config_string_scheme() {
        let config: ThemeConfig = serde_yaml::from_str("emberforge").unwrap();
        assert_eq!(config.scheme, ThemeScheme::Emberforge);
        assert_eq!(config.mode, None);
    }

    #[test]
    fn theme_config_string_legacy_dark() {
        let config: ThemeConfig = serde_yaml::from_str("dark").unwrap();
        assert_eq!(config.scheme, ThemeScheme::Default);
        assert_eq!(config.mode, Some(ThemeMode::Dark));
    }

    #[test]
    fn theme_config_string_legacy_light() {
        let config: ThemeConfig = serde_yaml::from_str("light").unwrap();
        assert_eq!(config.scheme, ThemeScheme::Default);
        assert_eq!(config.mode, Some(ThemeMode::Light));
    }

    #[test]
    fn theme_config_structured() {
        let yaml = "scheme: glacier-signal\nmode: light";
        let config: ThemeConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.scheme, ThemeScheme::GlacierSignal);
        assert_eq!(config.mode, Some(ThemeMode::Light));
    }

    #[test]
    fn theme_config_structured_scheme_only() {
        let yaml = "scheme: night-sorbet";
        let config: ThemeConfig = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.scheme, ThemeScheme::NightSorbet);
        assert_eq!(config.mode, None);
    }

    #[test]
    fn theme_config_unknown_scheme_defaults() {
        let config: ThemeConfig = serde_yaml::from_str("nonexistent").unwrap();
        assert_eq!(config.scheme, ThemeScheme::Default);
        assert_eq!(config.mode, None);
    }

    #[test]
    fn theme_config_full_config_file() {
        let yaml = "agent: claude\ntheme: teal-drift\nnerdfont: true";
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.theme.scheme, ThemeScheme::TealDrift);
        assert_eq!(config.theme.mode, None);
    }

    #[test]
    fn theme_config_full_config_structured() {
        let yaml = "agent: claude\ntheme:\n  scheme: obsidian-pop\n  mode: dark\nnerdfont: true";
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.theme.scheme, ThemeScheme::ObsidianPop);
        assert_eq!(config.theme.mode, Some(ThemeMode::Dark));
    }

    #[test]
    fn theme_config_full_config_legacy() {
        let yaml = "agent: claude\ntheme: light";
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.theme.scheme, ThemeScheme::Default);
        assert_eq!(config.theme.mode, Some(ThemeMode::Light));
    }

    #[test]
    fn theme_config_missing_defaults() {
        let yaml = "agent: claude";
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.theme.scheme, ThemeScheme::Default);
        assert_eq!(config.theme.mode, None);
    }

    // --- Layout tests ---

    #[test]
    fn deserialize_layouts() {
        let yaml = r#"
layouts:
  design:
    panes:
      - command: "<agent:claude>"
        focus: true
      - command: "<agent:codex>"
        split: vertical
  review:
    panes:
      - command: "<agent:claude>"
"#;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        let layouts = config.layouts.unwrap();
        assert_eq!(layouts.len(), 2);
        assert!(layouts.contains_key("design"));
        assert_eq!(layouts["design"].panes.len(), 2);
        assert_eq!(layouts["review"].panes.len(), 1);
    }

    #[test]
    fn deserialize_layouts_absent() {
        let yaml = "agent: claude";
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert!(config.layouts.is_none());
    }

    #[test]
    fn validate_layouts_valid() {
        let mut layouts = HashMap::new();
        layouts.insert(
            "test".to_string(),
            LayoutConfig {
                panes: vec![
                    PaneConfig {
                        command: Some("<agent:claude>".into()),
                        focus: true,
                        ..Default::default()
                    },
                    PaneConfig {
                        command: Some("vim".into()),
                        split: Some(SplitDirection::Horizontal),
                        ..Default::default()
                    },
                ],
            },
        );
        assert!(validate_layouts_config(&layouts).is_ok());
    }

    #[test]
    fn validate_panes_multiple_zoom_fails() {
        let panes = vec![
            PaneConfig {
                command: Some("vim".to_string()),
                zoom: true,
                ..Default::default()
            },
            PaneConfig {
                command: Some("echo hi".to_string()),
                split: Some(SplitDirection::Horizontal),
                zoom: true,
                ..Default::default()
            },
        ];
        let err = super::validate_panes_config(&panes).unwrap_err();
        assert!(err.to_string().contains("Only one pane"));
    }

    #[test]
    fn validate_panes_single_zoom_ok() {
        let panes = vec![
            PaneConfig {
                command: Some("vim".to_string()),
                zoom: true,
                ..Default::default()
            },
            PaneConfig {
                command: Some("echo hi".to_string()),
                split: Some(SplitDirection::Horizontal),
                ..Default::default()
            },
        ];
        assert!(super::validate_panes_config(&panes).is_ok());
    }

    #[test]
    fn zoom_deserializes_from_yaml() {
        let yaml = r#"
panes:
  - command: vim
    zoom: true
  - command: echo hi
    split: horizontal
"#;
        let config: super::Config = serde_yaml::from_str(yaml).unwrap();
        let panes = config.panes.unwrap();
        assert!(panes[0].zoom);
        assert!(!panes[1].zoom);
    }

    #[test]
    fn validate_layouts_invalid_first_pane_has_split() {
        let mut layouts = HashMap::new();
        layouts.insert(
            "bad".to_string(),
            LayoutConfig {
                panes: vec![PaneConfig {
                    split: Some(SplitDirection::Horizontal),
                    ..Default::default()
                }],
            },
        );
        let err = validate_layouts_config(&layouts).unwrap_err();
        assert!(
            err.to_string().contains("layout 'bad'"),
            "error should mention layout name: {}",
            err
        );
    }

    #[test]
    fn merge_layouts_project_extends_global() {
        let global = Config {
            layouts: Some(HashMap::from([(
                "a".into(),
                LayoutConfig { panes: vec![] },
            )])),
            ..Default::default()
        };
        let project = Config {
            layouts: Some(HashMap::from([(
                "b".into(),
                LayoutConfig { panes: vec![] },
            )])),
            ..Default::default()
        };
        let merged = merge_configs(global, project);
        let layouts = merged.layouts.unwrap();
        // Project layouts extend global (both available)
        assert!(layouts.contains_key("a"));
        assert!(layouts.contains_key("b"));
    }

    #[test]
    fn merge_layouts_project_overrides_collision() {
        let global = Config {
            layouts: Some(HashMap::from([(
                "shared".into(),
                LayoutConfig {
                    panes: vec![PaneConfig {
                        command: Some("global-cmd".into()),
                        ..Default::default()
                    }],
                },
            )])),
            ..Default::default()
        };
        let project = Config {
            layouts: Some(HashMap::from([(
                "shared".into(),
                LayoutConfig {
                    panes: vec![PaneConfig {
                        command: Some("project-cmd".into()),
                        ..Default::default()
                    }],
                },
            )])),
            ..Default::default()
        };
        let merged = merge_configs(global, project);
        let layouts = merged.layouts.unwrap();
        // Project wins on collision
        assert_eq!(
            layouts["shared"].panes[0].command.as_deref(),
            Some("project-cmd")
        );
    }

    #[test]
    fn merge_layouts_global_used_when_project_has_none() {
        let global = Config {
            layouts: Some(HashMap::from([(
                "a".into(),
                LayoutConfig { panes: vec![] },
            )])),
            ..Default::default()
        };
        let project = Config::default();
        let merged = merge_configs(global, project);
        let layouts = merged.layouts.unwrap();
        assert!(layouts.contains_key("a"));
    }

    #[test]
    fn theme_config_with_custom_colors() {
        let yaml = r##"
theme:
  scheme: emberforge
  custom:
    accent: "#51afef"
    success: "#98be65"
"##;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.theme.scheme, super::ThemeScheme::Emberforge);
        let custom = config.theme.custom.unwrap();
        assert_eq!(custom.accent, Some("#51afef".to_string()));
        assert_eq!(custom.success, Some("#98be65".to_string()));
        assert_eq!(custom.danger, None);
    }

    #[test]
    fn theme_config_custom_with_aliases() {
        let yaml = r##"
theme:
  custom:
    bg: "#282c34"
    fg: "#bbc2cf"
    error: "#ff6c6b"
"##;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        let custom = config.theme.custom.unwrap();
        // bg alias maps to current_row_bg
        assert_eq!(custom.current_row_bg, Some("#282c34".to_string()));
        // fg alias maps to text
        assert_eq!(custom.text, Some("#bbc2cf".to_string()));
        // error alias maps to danger
        assert_eq!(custom.danger, Some("#ff6c6b".to_string()));
    }

    #[test]
    fn theme_config_custom_only() {
        let yaml = r##"
theme:
  custom:
    accent: "#51afef"
"##;
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.theme.scheme, super::ThemeScheme::Default);
        assert!(config.theme.custom.is_some());
    }

    #[test]
    fn theme_config_simple_string_no_custom() {
        let yaml = "theme: emberforge\n";
        let config: Config = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(config.theme.scheme, super::ThemeScheme::Emberforge);
        assert!(config.theme.custom.is_none());
    }

    #[test]
    fn theme_config_merge_custom_project_wins() {
        let global = Config {
            theme: super::ThemeConfig {
                scheme: super::ThemeScheme::Default,
                mode: None,
                custom: Some(super::CustomThemeColors {
                    accent: Some("#111111".to_string()),
                    ..Default::default()
                }),
            },
            ..Default::default()
        };
        let project = Config {
            theme: super::ThemeConfig {
                scheme: super::ThemeScheme::Default,
                mode: None,
                custom: Some(super::CustomThemeColors {
                    accent: Some("#222222".to_string()),
                    ..Default::default()
                }),
            },
            ..Default::default()
        };
        let merged = merge_configs(global, project);
        assert_eq!(
            merged.theme.custom.unwrap().accent,
            Some("#222222".to_string())
        );
    }

    #[test]
    fn theme_config_merge_custom_falls_back_to_global() {
        let global = Config {
            theme: super::ThemeConfig {
                scheme: super::ThemeScheme::Default,
                mode: None,
                custom: Some(super::CustomThemeColors {
                    accent: Some("#111111".to_string()),
                    ..Default::default()
                }),
            },
            ..Default::default()
        };
        let project = Config::default();
        let merged = merge_configs(global, project);
        assert_eq!(
            merged.theme.custom.unwrap().accent,
            Some("#111111".to_string())
        );
    }

    #[test]
    fn events_config_deserializes() {
        let yaml = r#"
enabled: true
level: debug
disable:
  - pane.probe
  - turn.poll
only:
  - node
"#;
        let cfg: EventsConfig = serde_yaml::from_str(yaml).unwrap();
        assert!(cfg.is_enabled());
        assert_eq!(cfg.level.as_deref(), Some("debug"));
        assert_eq!(cfg.disable, vec!["pane.probe", "turn.poll"]);
        assert_eq!(cfg.only, vec!["node"]);
    }

    #[test]
    fn events_config_defaults_enabled() {
        let cfg: EventsConfig = serde_yaml::from_str("{}").unwrap();
        assert!(cfg.is_enabled());
        assert!(cfg.level.is_none());
        assert!(cfg.disable.is_empty());
        assert!(cfg.only.is_empty());
    }

    #[test]
    fn events_to_filter_maps_fields() {
        let cfg = EventsConfig {
            enabled: Some(false),
            level: Some("debug".to_string()),
            only: vec!["node".to_string()],
            disable: vec!["pane.probe".to_string()],
        };
        let filter = cfg.to_filter();
        assert!(!filter.enabled);
        assert_eq!(filter.only, vec!["node"]);
        assert_eq!(filter.disable, vec!["pane.probe"]);
    }

    #[test]
    fn events_merge_project_overrides_global() {
        let global = Config {
            events: EventsConfig {
                enabled: Some(true),
                level: Some("info".to_string()),
                only: vec![],
                disable: vec!["global.kind".to_string()],
            },
            ..Default::default()
        };
        let project = Config {
            events: EventsConfig {
                enabled: None,
                level: Some("debug".to_string()),
                only: vec![],
                disable: vec!["pane.probe".to_string()],
            },
            ..Default::default()
        };
        let merged = merge_configs(global, project);
        // project level overrides global
        assert_eq!(merged.events.level.as_deref(), Some("debug"));
        // project disable (non-empty) overrides global disable
        assert_eq!(merged.events.disable, vec!["pane.probe"]);
        // enabled inherits global since project left it unset
        assert_eq!(merged.events.enabled, Some(true));
    }

    #[test]
    fn events_merge_inherits_global_when_project_empty() {
        let global = Config {
            events: EventsConfig {
                enabled: Some(false),
                level: Some("off".to_string()),
                only: vec!["node".to_string()],
                disable: vec!["pane.probe".to_string()],
            },
            ..Default::default()
        };
        let project = Config::default();
        let merged = merge_configs(global, project);
        assert_eq!(merged.events.enabled, Some(false));
        assert_eq!(merged.events.level.as_deref(), Some("off"));
        assert_eq!(merged.events.only, vec!["node"]);
        assert_eq!(merged.events.disable, vec!["pane.probe"]);
    }
}
