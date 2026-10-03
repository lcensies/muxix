//! The result model shared by every `workmux setup` section.
//!
//! Sections return data rather than printing it, so the interactive TUI, the
//! non-interactive log, `--json`, and `--check` are four renderers over one
//! structure. Before this existed each section printed its own progress in its
//! own vocabulary, which is why `--check` and `--json` were not simply flags:
//! there was no value to render.

use serde::{Deserialize, Serialize};
use std::fmt;

/// What happened (or would happen) to one item.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// Did not exist; created.
    Installed,
    /// Existed with different content; rewritten.
    Updated,
    /// Already matches the declared state. No write.
    UpToDate,
    /// Deliberately not applied — an uninstalled agent, an unsupported
    /// capability, a source workmux cannot fetch. Not a failure.
    Skipped,
    /// Was installed by workmux, is no longer declared, and has been deleted.
    Removed,
    /// Tried and failed. The only outcome that makes the command exit non-zero.
    Failed,
}

impl Outcome {
    /// Whether this outcome represents a change to the machine.
    ///
    /// This is what `--check` reports as drift: an item that a real run would
    /// create or rewrite.
    pub fn is_drift(self) -> bool {
        matches!(
            self,
            Outcome::Installed | Outcome::Updated | Outcome::Removed
        )
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Outcome::Installed => "installed",
            Outcome::Updated => "updated",
            Outcome::UpToDate => "up-to-date",
            Outcome::Skipped => "skipped",
            Outcome::Removed => "removed",
            Outcome::Failed => "failed",
        }
    }
}

impl fmt::Display for Outcome {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The sections `workmux setup` can apply, in the order it applies them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Section {
    Hooks,
    Skills,
    /// Skill-declared and global hooks written into agents' native hook config.
    AgentHooks,
    Subagents,
    Plugins,
    /// Declared merge patches applied to agents' own settings files.
    AgentSettings,
    /// Registry providers with connection details rendered into agent configs.
    Providers,
    Prompts,
    Theme,
    Mcp,
    /// Derived agent-profile overlay dirs (base ⊕ profile source).
    AgentProfiles,
    /// `requires:` of skills and MCP servers: npm installs, PATH assertions.
    Deps,
}

impl Section {
    pub const ALL: &'static [Section] = &[
        Section::Hooks,
        Section::Skills,
        // After skills: a hook's sha256 is verified against the *installed*
        // script, which the skills section just wrote.
        Section::AgentHooks,
        Section::Subagents,
        Section::Plugins,
        // After plugins: a settings key often configures a plugin the previous
        // section just installed, and `pi install` rewrites the same file.
        Section::AgentSettings,
        Section::Providers,
        Section::Prompts,
        Section::Theme,
        Section::Mcp,
        Section::AgentProfiles,
        // Last: its inputs are the resolved skill and MCP sets.
        Section::Deps,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Section::Hooks => "hooks",
            Section::Skills => "skills",
            Section::AgentHooks => "agent-hooks",
            Section::Subagents => "subagents",
            Section::Plugins => "plugins",
            Section::AgentSettings => "agent-settings",
            Section::Providers => "providers",
            Section::Prompts => "prompts",
            Section::Theme => "theme",
            Section::Mcp => "mcp",
            Section::AgentProfiles => "agent-profiles",
            Section::Deps => "deps",
        }
    }

    pub fn parse(name: &str) -> Option<Section> {
        Section::ALL
            .iter()
            .copied()
            .find(|s| s.as_str() == name.trim().to_ascii_lowercase())
    }

    /// Comma-separated list of valid names, for error messages.
    pub fn all_names() -> String {
        Section::ALL
            .iter()
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl fmt::Display for Section {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One unit of work: a hook, a skill, a plugin, a prompt, an MCP sync.
#[derive(Debug, Clone, Serialize)]
pub struct ItemResult {
    pub section: Section,
    /// Agent this item belongs to, or `None` for agent-independent work.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// What the item is: a skill name, a plugin spec, a file path.
    pub name: String,
    pub outcome: Outcome,
    /// Why it was skipped or how it failed. Absent when there is nothing to say.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// Set when workmux owns this item: how to remove it later (a filesystem
    /// path, a plugin spec, a hook command). Internal provenance for the
    /// managed-state manifest, not part of the `--json` contract.
    #[serde(skip)]
    pub managed: Option<String>,
}

impl ItemResult {
    pub fn new(section: Section, agent: Option<&str>, name: impl Into<String>, outcome: Outcome) -> Self {
        Self {
            section,
            agent: agent.map(str::to_owned),
            name: name.into(),
            outcome,
            detail: None,
            managed: None,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    /// Mark this item as workmux-managed, recording how to remove it.
    pub fn managed_at(mut self, target: impl Into<String>) -> Self {
        self.managed = Some(target.into());
        self
    }

    pub fn skipped(section: Section, agent: Option<&str>, name: impl Into<String>, why: impl Into<String>) -> Self {
        Self::new(section, agent, name, Outcome::Skipped).with_detail(why)
    }

    pub fn failed(section: Section, agent: Option<&str>, name: impl Into<String>, why: impl Into<String>) -> Self {
        Self::new(section, agent, name, Outcome::Failed).with_detail(why)
    }
}

/// Everything one `workmux setup` run did (or would do).
#[derive(Debug, Default, Serialize)]
pub struct SetupReport {
    pub items: Vec<ItemResult>,
    /// True when this was a `--check` run, so nothing was written.
    pub check_only: bool,
}

impl SetupReport {
    pub fn push(&mut self, item: ItemResult) {
        self.items.push(item);
    }

    pub fn extend(&mut self, items: impl IntoIterator<Item = ItemResult>) {
        self.items.extend(items);
    }

    pub fn any_failed(&self) -> bool {
        self.items.iter().any(|i| i.outcome == Outcome::Failed)
    }

    /// Items a real run would create or rewrite.
    pub fn drifted(&self) -> Vec<&ItemResult> {
        self.items.iter().filter(|i| i.outcome.is_drift()).collect()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Process exit code: 1 on any failure, 2 on drift in check mode, else 0.
    ///
    /// Check mode reports drift as 2 rather than 1 so a caller can tell "the
    /// machine differs from the config" apart from "the check itself broke".
    pub fn exit_code(&self) -> i32 {
        if self.any_failed() {
            return 1;
        }
        if self.check_only && !self.drifted().is_empty() {
            return 2;
        }
        0
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "checkOnly": self.check_only,
            "exitCode": self.exit_code(),
            "items": self.items,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(section: Section, outcome: Outcome) -> ItemResult {
        ItemResult::new(section, Some("claude"), "thing", outcome)
    }

    #[test]
    fn section_names_round_trip() {
        for section in Section::ALL {
            assert_eq!(Section::parse(section.as_str()), Some(*section));
        }
    }

    #[test]
    fn section_parse_is_case_insensitive_and_trims() {
        assert_eq!(Section::parse("  MCP "), Some(Section::Mcp));
    }

    #[test]
    fn unknown_section_is_none() {
        assert_eq!(Section::parse("bogus"), None);
    }

    #[test]
    fn only_writes_count_as_drift() {
        assert!(Outcome::Installed.is_drift());
        assert!(Outcome::Updated.is_drift());
        assert!(!Outcome::UpToDate.is_drift());
        assert!(!Outcome::Skipped.is_drift());
        // A failure is reported through the exit code, not as drift.
        assert!(!Outcome::Failed.is_drift());
        // A removal changes the machine as much as an install does.
        assert!(Outcome::Removed.is_drift());
    }

    #[test]
    fn check_with_only_a_removal_exits_two() {
        let mut r = SetupReport {
            check_only: true,
            ..Default::default()
        };
        r.push(item(Section::Skills, Outcome::Removed));
        assert_eq!(r.exit_code(), 2);
    }

    #[test]
    fn clean_apply_exits_zero() {
        let mut r = SetupReport::default();
        r.push(item(Section::Hooks, Outcome::Installed));
        r.push(item(Section::Skills, Outcome::UpToDate));
        assert_eq!(r.exit_code(), 0);
    }

    #[test]
    fn failure_exits_one() {
        let mut r = SetupReport::default();
        r.push(item(Section::Plugins, Outcome::Failed));
        assert_eq!(r.exit_code(), 1);
    }

    #[test]
    fn check_with_drift_exits_two() {
        let mut r = SetupReport {
            check_only: true,
            ..Default::default()
        };
        r.push(item(Section::Skills, Outcome::Installed));
        assert_eq!(r.exit_code(), 2);
    }

    #[test]
    fn check_without_drift_exits_zero() {
        let mut r = SetupReport {
            check_only: true,
            ..Default::default()
        };
        r.push(item(Section::Skills, Outcome::UpToDate));
        r.push(item(Section::Plugins, Outcome::Skipped));
        assert_eq!(r.exit_code(), 0);
    }

    /// A failure outranks drift: something is broken, not merely stale.
    #[test]
    fn check_failure_exits_one_not_two() {
        let mut r = SetupReport {
            check_only: true,
            ..Default::default()
        };
        r.push(item(Section::Skills, Outcome::Installed));
        r.push(item(Section::Hooks, Outcome::Failed));
        assert_eq!(r.exit_code(), 1);
    }

    #[test]
    fn empty_report_exits_zero() {
        assert_eq!(SetupReport::default().exit_code(), 0);
    }

    #[test]
    fn json_shape_is_stable() {
        let mut r = SetupReport::default();
        r.push(
            ItemResult::new(Section::Skills, Some("claude"), "workmux", Outcome::Installed)
                .with_detail("created"),
        );
        let json = r.to_json();
        assert_eq!(json["exitCode"], 0);
        assert_eq!(json["checkOnly"], false);
        assert_eq!(json["items"][0]["section"], "skills");
        assert_eq!(json["items"][0]["agent"], "claude");
        assert_eq!(json["items"][0]["outcome"], "installed");
        assert_eq!(json["items"][0]["detail"], "created");
    }

    #[test]
    fn agentless_items_omit_the_agent_field() {
        let mut r = SetupReport::default();
        r.push(ItemResult::new(Section::Mcp, None, ".mcp.json", Outcome::UpToDate));
        let json = r.to_json();
        assert!(json["items"][0].get("agent").is_none());
    }
}
