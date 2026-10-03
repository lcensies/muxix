//! Task graph state and operations for the dashboard Tasks tab.

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow};
use ratatui::style::Style;
use ratatui::widgets::{Block, TableState};
use tui_textarea::TextArea;

use crate::tasks::graph;
use crate::tasks::types::{GraphTask, STATUS_DONE, STATUS_FAILED, STATUS_IN_PROGRESS, STATUS_TODO};
use crate::workflow;

use super::App;

pub(super) const TASK_RELOAD_INTERVAL: Duration = Duration::from_secs(3);

pub fn textarea_value(ta: &TextArea) -> String {
    ta.lines().join("\n")
}

pub fn make_textarea(initial: &str) -> TextArea<'static> {
    let mut ta = TextArea::from([initial]);
    ta.set_cursor_line_style(Style::default());
    ta.set_block(Block::default());
    ta
}

pub fn make_description_textarea(initial: &str) -> TextArea<'static> {
    let mut ta = TextArea::from([initial]);
    ta.set_cursor_line_style(Style::default());
    ta.set_block(Block::default());
    ta.word_wrap = true;
    ta
}

// ── Status filter ─────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TaskStatusFilter {
    All,
    Todo,
    InProgress,
    Done,
    Failed,
}

impl TaskStatusFilter {
    pub fn next(self) -> Self {
        match self {
            Self::All => Self::Todo,
            Self::Todo => Self::InProgress,
            Self::InProgress => Self::Done,
            Self::Done => Self::Failed,
            Self::Failed => Self::All,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Todo => "todo",
            Self::InProgress => "in_progress",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }

    pub fn matches(self, status: &str) -> bool {
        match self {
            Self::All => true,
            Self::Todo => status == STATUS_TODO,
            Self::InProgress => status == STATUS_IN_PROGRESS,
            Self::Done => status == STATUS_DONE,
            Self::Failed => status == STATUS_FAILED,
        }
    }
}

// ── Form ──────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TaskFormField {
    Id,
    Title,
    Description,
    DependsOn,
    Worktree,
    Status,
}

pub const TASK_FORM_FIELDS: [TaskFormField; 6] = [
    TaskFormField::Title,
    TaskFormField::Description,
    TaskFormField::Id,
    TaskFormField::DependsOn,
    TaskFormField::Worktree,
    TaskFormField::Status,
];

pub struct TaskForm {
    pub is_edit: bool,
    pub original_id: Option<String>,
    pub id: TextArea<'static>,
    pub title: TextArea<'static>,
    pub description: TextArea<'static>,
    pub depends_on: TextArea<'static>,
    pub worktree: TextArea<'static>,
    pub status: String,
    pub focused: usize,
    pub error: Option<String>,
    /// When true, ID is auto-derived from title and updated as user types.
    pub id_auto: bool,
    /// Filtered task IDs shown as suggestions when DependsOn field is focused.
    pub dep_suggestions: Vec<String>,
    pub dep_suggestion_cursor: usize,
}

impl TaskForm {
    pub fn new_add() -> Self {
        Self {
            is_edit: false,
            original_id: None,
            id: make_textarea(""),
            title: make_description_textarea(""),
            description: make_description_textarea(""),
            depends_on: make_textarea(""),
            worktree: make_textarea(""),
            status: STATUS_TODO.to_string(),
            focused: 0,
            error: None,
            id_auto: true,
            dep_suggestions: Vec::new(),
            dep_suggestion_cursor: 0,
        }
    }

    pub fn new_edit(task: &GraphTask) -> Self {
        Self {
            is_edit: true,
            original_id: Some(task.id.clone()),
            id: make_textarea(&task.id),
            title: make_description_textarea(&task.title),
            description: make_description_textarea(&task.description),
            depends_on: make_textarea(&task.depends_on.join(" ")),
            worktree: make_textarea(task.worktree.as_deref().unwrap_or("")),
            status: task.status.clone(),
            focused: 0,
            error: None,
            id_auto: false,
            dep_suggestions: Vec::new(),
            dep_suggestion_cursor: 0,
        }
    }

    pub fn focused_field(&self) -> TaskFormField {
        TASK_FORM_FIELDS[self.focused]
    }

    pub fn focused_textarea_mut(&mut self) -> Option<&mut TextArea<'static>> {
        match self.focused_field() {
            TaskFormField::Id => Some(&mut self.id),
            TaskFormField::Title => Some(&mut self.title),
            TaskFormField::Description => Some(&mut self.description),
            TaskFormField::DependsOn => Some(&mut self.depends_on),
            TaskFormField::Worktree => Some(&mut self.worktree),
            TaskFormField::Status => None,
        }
    }

    pub fn to_task(&self) -> GraphTask {
        GraphTask {
            id: textarea_value(&self.id).trim().to_string(),
            title: textarea_value(&self.title).trim().to_string(),
            description: textarea_value(&self.description).trim().to_string(),
            depends_on: textarea_value(&self.depends_on)
                .split_whitespace()
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect(),
            status: self.status.clone(),
            worktree: {
                let w = textarea_value(&self.worktree).trim().to_string();
                if w.is_empty() { None } else { Some(w) }
            },
            ..Default::default()
        }
    }

    pub fn validate(&self, all_tasks: &[GraphTask]) -> Option<String> {
        let id_val = textarea_value(&self.id);
        let id = id_val.trim();
        if id.is_empty() {
            return Some("ID cannot be empty".to_string());
        }
        let title_val = textarea_value(&self.title);
        if title_val.trim().is_empty() {
            return Some("Title cannot be empty".to_string());
        }
        let is_dupe = all_tasks.iter().any(|t| {
            t.id == id
                && self
                    .original_id
                    .as_ref()
                    .map(|oid| oid != id)
                    .unwrap_or(true)
        });
        if is_dupe {
            return Some(format!("Task ID '{}' already exists", id));
        }
        let known_ids: std::collections::HashSet<&str> =
            all_tasks.iter().map(|t| t.id.as_str()).collect();
        let depends_on_val = textarea_value(&self.depends_on);
        for dep in depends_on_val.split_whitespace() {
            if dep != id && !known_ids.contains(dep) {
                return Some(format!("Unknown dependency: '{}'", dep));
            }
        }
        if depends_on_val.split_whitespace().any(|d| d == id) {
            return Some("Task cannot depend on itself".to_string());
        }
        let valid = [STATUS_TODO, STATUS_IN_PROGRESS, STATUS_DONE, STATUS_FAILED];
        if !valid.contains(&self.status.as_str()) {
            return Some(format!("Invalid status: '{}'", self.status));
        }
        None
    }

    pub fn cycle_status(&mut self) {
        self.status = match self.status.as_str() {
            STATUS_TODO => STATUS_IN_PROGRESS.to_string(),
            STATUS_IN_PROGRESS => STATUS_DONE.to_string(),
            STATUS_DONE => STATUS_FAILED.to_string(),
            _ => STATUS_TODO.to_string(),
        };
    }

    /// Recompute dep_suggestions based on the current last token in depends_on.
    pub fn update_dep_suggestions(&mut self, all_tasks: &[GraphTask]) {
        if self.focused_field() != TaskFormField::DependsOn {
            self.dep_suggestions.clear();
            self.dep_suggestion_cursor = 0;
            return;
        }
        let depends_on_val = textarea_value(&self.depends_on);
        let already: std::collections::HashSet<&str> = depends_on_val.split_whitespace().collect();
        let id_val = textarea_value(&self.id);
        let own_id = self.original_id.as_deref().unwrap_or(&id_val);
        let prefix = depends_on_val
            .split_whitespace()
            .last()
            .unwrap_or("")
            .to_lowercase();
        // Only show suggestions when there's a partial token being typed
        if prefix.is_empty() && !depends_on_val.ends_with(char::is_whitespace) {
            self.dep_suggestions.clear();
            self.dep_suggestion_cursor = 0;
            return;
        }
        self.dep_suggestions = all_tasks
            .iter()
            .filter(|t| {
                t.id != own_id
                    && !already.contains(t.id.as_str())
                    && t.id.to_lowercase().starts_with(&prefix)
            })
            .map(|t| t.id.clone())
            .collect();
        if self.dep_suggestion_cursor >= self.dep_suggestions.len() {
            self.dep_suggestion_cursor = 0;
        }
    }

    /// Accept the currently highlighted suggestion into the depends_on field.
    pub fn accept_dep_suggestion(&mut self) {
        let Some(suggestion) = self
            .dep_suggestions
            .get(self.dep_suggestion_cursor)
            .cloned()
        else {
            return;
        };
        let current = textarea_value(&self.depends_on);
        let trimmed = current.trim_end().to_string();
        let base = if let Some(pos) = trimmed.rfind(char::is_whitespace) {
            trimmed[..=pos].to_string()
        } else {
            String::new()
        };
        let new_value = format!("{}{} ", base, suggestion);
        self.depends_on = make_textarea(&new_value);
        self.dep_suggestions.clear();
        self.dep_suggestion_cursor = 0;
    }
}

// ── ID auto-derivation ────────────────────────────────────────────────────────

/// Derive a slug-style ID from a title (like GitLab/Bitbucket short IDs).
pub fn derive_id(title: &str) -> String {
    let mut result = String::new();
    let mut prev_dash = true; // treat start as dash to suppress leading dashes
    for c in title.chars() {
        if c.is_alphanumeric() {
            result.push(c.to_ascii_lowercase());
            prev_dash = false;
        } else if !prev_dash {
            result.push('-');
            prev_dash = true;
        }
    }
    result.trim_end_matches('-').to_string()
}

// ── Modal ─────────────────────────────────────────────────────────────────────

pub struct DeleteTaskPlan {
    pub id: String,
    pub worktree: Option<String>,
    pub delete_worktree: bool,
}

// One modal is alive at a time, so the `Form` variant's size costs nothing a
// `Box` would buy back.
#[allow(clippy::large_enum_variant)]
pub enum TaskModal {
    Form(TaskForm),
    DeleteConfirm(DeleteTaskPlan),
    #[allow(dead_code)]
    Help,
}

// ── State fields (declared in App) ───────────────────────────────────────────

pub struct TaskState {
    pub graph_path: PathBuf,
    pub all_tasks: Vec<GraphTask>,
    pub filtered_indices: Vec<usize>,
    pub table_state: TableState,
    pub filter_text: String,
    pub filter_active: bool,
    pub status_filter: TaskStatusFilter,
    pub modal: Option<TaskModal>,
    pub last_reload: Instant,
}

impl TaskState {
    pub fn new(graph_path: PathBuf) -> Self {
        Self {
            graph_path,
            all_tasks: Vec::new(),
            filtered_indices: Vec::new(),
            table_state: TableState::default(),
            filter_text: String::new(),
            filter_active: false,
            status_filter: TaskStatusFilter::All,
            modal: None,
            last_reload: Instant::now() - Duration::from_secs(60),
        }
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

pub(super) fn write_atomic(path: &std::path::Path, content: &str) -> Result<()> {
    let tmp = path.with_extension(match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => format!("{ext}.tmp"),
        None => "tmp".to_string(),
    });
    fs::write(&tmp, content)?;
    fs::rename(&tmp, path)?;
    Ok(())
}

// ── App methods ───────────────────────────────────────────────────────────────

impl App {
    pub fn task_selected_task(&self) -> Option<&GraphTask> {
        let idx = self.tasks.table_state.selected()?;
        let task_idx = self.tasks.filtered_indices.get(idx)?;
        self.tasks.all_tasks.get(*task_idx)
    }

    pub fn task_selected_id(&self) -> Option<String> {
        self.task_selected_task().map(|t| t.id.clone())
    }

    pub fn task_navigate_up(&mut self) {
        if self.tasks.filtered_indices.is_empty() {
            return;
        }
        let i = self.tasks.table_state.selected().unwrap_or(0);
        let new = if i == 0 {
            self.tasks.filtered_indices.len() - 1
        } else {
            i - 1
        };
        self.tasks.table_state.select(Some(new));
    }

    pub fn task_navigate_down(&mut self) {
        if self.tasks.filtered_indices.is_empty() {
            return;
        }
        let i = self.tasks.table_state.selected().unwrap_or(0);
        let new = if i + 1 >= self.tasks.filtered_indices.len() {
            0
        } else {
            i + 1
        };
        self.tasks.table_state.select(Some(new));
    }

    pub fn task_apply_filters(&mut self) {
        let filter = self.tasks.filter_text.to_lowercase();
        let status_filter = self.tasks.status_filter;
        self.tasks.filtered_indices = (0..self.tasks.all_tasks.len())
            .filter(|&i| {
                let t = &self.tasks.all_tasks[i];
                if !status_filter.matches(&t.status) {
                    return false;
                }
                if filter.is_empty() {
                    return true;
                }
                t.id.to_lowercase().contains(&filter)
                    || t.title.to_lowercase().contains(&filter)
                    || t.description.to_lowercase().contains(&filter)
            })
            .collect();
        let len = self.tasks.filtered_indices.len();
        match self.tasks.table_state.selected() {
            Some(s) if s >= len && len > 0 => self.tasks.table_state.select(Some(len - 1)),
            None if len > 0 => self.tasks.table_state.select(Some(0)),
            _ if len == 0 => self.tasks.table_state.select(None),
            _ => {}
        }
    }

    pub fn task_reload(&mut self) {
        self.task_ensure_graph_file();
        match graph::load(&self.tasks.graph_path) {
            Ok(tasks) => {
                let selected_id = self.task_selected_id();
                self.tasks.all_tasks = tasks;
                self.task_apply_filters();
                if let Some(id) = selected_id
                    && let Some(pos) = self
                        .tasks
                        .filtered_indices
                        .iter()
                        .position(|&i| self.tasks.all_tasks[i].id == id)
                {
                    self.tasks.table_state.select(Some(pos));
                }
            }
            Err(e) => {
                self.status_message = Some((
                    format!("task reload failed: {e}"),
                    std::time::Instant::now(),
                ));
            }
        }
        self.tasks.last_reload = Instant::now();
    }

    pub fn task_maybe_autoreload(&mut self) {
        if self.tasks.modal.is_none() && self.tasks.last_reload.elapsed() >= TASK_RELOAD_INTERVAL {
            self.task_reload();
        }
    }

    pub fn task_cycle_status_filter(&mut self) {
        self.tasks.status_filter = self.tasks.status_filter.next();
        self.task_apply_filters();
    }

    pub fn task_cycle_status(&mut self) {
        let Some(id) = self.task_selected_id() else {
            return;
        };
        let Some(task) = self.tasks.all_tasks.iter().find(|t| t.id == id) else {
            return;
        };
        let new_status = match task.status.as_str() {
            STATUS_TODO => STATUS_IN_PROGRESS,
            STATUS_IN_PROGRESS => STATUS_DONE,
            STATUS_DONE => STATUS_FAILED,
            _ => STATUS_TODO,
        };
        match graph::update_status(&self.tasks.graph_path, &id, new_status) {
            Ok(()) => {
                self.status_message =
                    Some((format!("{id} → {new_status}"), std::time::Instant::now()));
                self.task_reload();
            }
            Err(e) => {
                self.status_message = Some((format!("error: {e}"), std::time::Instant::now()));
            }
        }
    }

    pub fn task_confirm_form(&mut self) {
        let (validation_err, task, is_edit, original_id) = match &self.tasks.modal {
            Some(TaskModal::Form(form)) => {
                let err = form.validate(&self.tasks.all_tasks);
                (err, form.to_task(), form.is_edit, form.original_id.clone())
            }
            _ => return,
        };
        if let Some(err) = validation_err {
            if let Some(TaskModal::Form(f)) = &mut self.tasks.modal {
                f.error = Some(err);
            }
            return;
        }
        self.tasks.modal = None;
        if is_edit {
            match self.task_save_edited(&original_id.unwrap(), task) {
                Ok(()) => self.status_message = Some(("task updated".to_string(), Instant::now())),
                Err(e) => self.status_message = Some((format!("save failed: {e}"), Instant::now())),
            }
        } else {
            match graph::add_task(&self.tasks.graph_path, &task) {
                Ok(()) => {
                    self.status_message =
                        Some((format!("task '{}' added", task.id), Instant::now()))
                }
                Err(e) => self.status_message = Some((format!("add failed: {e}"), Instant::now())),
            }
        }
        self.task_reload();
    }

    fn task_save_edited(&self, original_id: &str, task: GraphTask) -> Result<()> {
        let data = fs::read_to_string(&self.tasks.graph_path)?;
        let mut value: serde_json::Value = serde_json::from_str(&data)?;
        let arr = value
            .as_array_mut()
            .ok_or_else(|| anyhow!("task graph is not a JSON array"))?;
        let pos = arr
            .iter()
            .position(|t| t.get("id").and_then(|v| v.as_str()) == Some(original_id))
            .ok_or_else(|| anyhow!("task '{}' not found", original_id))?;
        arr[pos] = serde_json::to_value(&task)?;
        let out = serde_json::to_string_pretty(&value)?;
        write_atomic(&self.tasks.graph_path, &out)
    }

    pub fn task_delete(&mut self, plan: DeleteTaskPlan) {
        // Remove worktree + branch first if requested. A task may reference a
        // worktree that no longer exists (manually removed, etc.); in that case
        // skip removal and still delete the task instead of erroring out.
        if plan.delete_worktree
            && let Some(ref handle) = plan.worktree
            && let Ok(ctx) =
                workflow::WorkflowContext::new(self.config.clone(), self.mux.clone(), None)
        {
            // Only attempt removal if the worktree is still resolvable, either
            // via git's worktree list or the broken-metadata fallback path.
            let resolvable = crate::git::find_worktree(handle).is_ok()
                || matches!(workflow::fallback_worktree_path(handle, &ctx), Ok(Some(_)));
            if resolvable {
                if let Err(e) = workflow::remove(handle, true, false, &ctx) {
                    self.status_message =
                        Some((format!("worktree removal failed: {e}"), Instant::now()));
                    return;
                }
                self.trigger_worktree_refetch();
            }
        }
        let id = &plan.id;
        let result = (|| -> Result<()> {
            let data = fs::read_to_string(&self.tasks.graph_path)?;
            let mut value: serde_json::Value = serde_json::from_str(&data)?;
            let arr = value
                .as_array_mut()
                .ok_or_else(|| anyhow!("not a JSON array"))?;
            arr.retain(|t| t.get("id").and_then(|v| v.as_str()) != Some(id.as_str()));
            let out = serde_json::to_string_pretty(&value)?;
            write_atomic(&self.tasks.graph_path, &out)
        })();
        match result {
            Ok(()) => {
                self.status_message = Some((format!("task '{}' deleted", id), Instant::now()))
            }
            Err(e) => self.status_message = Some((format!("delete failed: {e}"), Instant::now())),
        }
        self.task_reload();
    }

    pub fn task_ensure_graph_file(&self) {
        if !self.tasks.graph_path.exists() {
            if let Some(dir) = self.tasks.graph_path.parent()
                && !dir.as_os_str().is_empty()
            {
                let _ = fs::create_dir_all(dir);
            }
            let _ = fs::write(&self.tasks.graph_path, "[]\n");
        }
    }
}

#[cfg(test)]
mod tests {}
