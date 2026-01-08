# Plan: Dashboard Diff Browsing Feature

Add the ability to browse git diffs directly from the workmux dashboard TUI, enabling users to review agent changes and trigger commits/merges without leaving the dashboard.

## Use Case

When monitoring multiple AI agents working in different worktrees, users want to:
1. Press a hotkey to view the diff of changes in the selected worktree
2. Review uncommitted/staged changes the agent has made
3. Tell the agent to commit the changes
4. Trigger merge workflow

## UX Design

### Keybindings (in dashboard view)

| Key | Action |
|-----|--------|
| `d` | View uncommitted changes (`git diff HEAD`) |
| `D` | View branch changes vs main (`git diff main...HEAD`) |

### Keybindings (in diff modal)

| Key | Action |
|-----|--------|
| `j`/`k` | Scroll down/up (single line) |
| `Ctrl+d`/`Ctrl+u` | Page down/up |
| `PageDown`/`PageUp` | Page down/up (native keys) |
| `q`/`Esc` | Close modal, return to dashboard |
| `c` | Send commit command to agent, close modal |
| `m` | Trigger merge (run `workmux merge`), close modal |

### Visual Design

Full-screen modal overlay (90% width/height) with:
- Title bar showing diff type and worktree name
- Scrollable diff content with ANSI color support
- Footer with available keybindings

```
┌─────────────────────────────────────────────────────┐
│ Uncommitted Changes: fix-bug                        │
├─────────────────────────────────────────────────────┤
│ diff --git a/src/handler.rs b/src/handler.rs        │
│ index 1234567..abcdef0 100644                       │
│ --- a/src/handler.rs                                │
│ +++ b/src/handler.rs                                │
│ @@ -42,6 +42,10 @@ fn handle_request() {            │
│      let response = process(req);                   │
│ +    if !validate(&req) {                           │
│ +        return Err(ValidationError);               │
│ +    }                                              │
│      send_response(response)                        │
├─────────────────────────────────────────────────────┤
│ [j/k] scroll  [q] close  [c] commit  [m] merge      │
└─────────────────────────────────────────────────────┘
```

## Implementation Steps

### Step 1: Add ViewMode enum and DiffView struct

File: `src/command/dashboard/app.rs`

**Key design decisions:**
- Use `usize` for scroll/line_count internally (u16 max is 65,535, large diffs can exceed this)
- Store `viewport_height` in DiffView so scroll logic knows page size
- Put scroll methods on DiffView itself to keep App clean

```rust
#[derive(Debug, Default, PartialEq)]
pub enum ViewMode {
    #[default]
    Dashboard,
    Diff(DiffView),
}

#[derive(Debug, PartialEq)]
pub struct DiffView {
    /// The diff content (with ANSI colors)
    pub content: String,
    /// Current scroll offset (use usize to handle large diffs)
    pub scroll: usize,
    /// Total line count for scroll bounds
    pub line_count: usize,
    /// Viewport height (updated by UI during render for page scroll)
    pub viewport_height: u16,
    /// Title for the modal (e.g., "Uncommitted Changes: fix-bug")
    pub title: String,
    /// Path to the worktree (for commit/merge actions)
    pub worktree_path: PathBuf,
    /// Pane ID for sending commands to agent
    pub pane_id: String,
}

impl DiffView {
    pub fn scroll_up(&mut self) {
        self.scroll = self.scroll.saturating_sub(1);
    }

    pub fn scroll_down(&mut self) {
        let max_scroll = self.line_count.saturating_sub(self.viewport_height as usize);
        if self.scroll < max_scroll {
            self.scroll += 1;
        }
    }

    pub fn scroll_page_up(&mut self) {
        let page = self.viewport_height as usize;
        self.scroll = self.scroll.saturating_sub(page);
    }

    pub fn scroll_page_down(&mut self) {
        let page = self.viewport_height as usize;
        let max_scroll = self.line_count.saturating_sub(self.viewport_height as usize);
        self.scroll = (self.scroll + page).min(max_scroll);
    }
}
```

Add to `App` struct:
```rust
pub view_mode: ViewMode,
```

### Step 2: Add git diff fetching logic

File: `src/command/dashboard/app.rs`

**Performance consideration:** For MVP, synchronous git diff is acceptable since agent diffs
are typically small. If performance becomes an issue, consider async loading with a Loading state.

```rust
impl App {
    /// Load diff for the selected worktree
    /// - `branch_diff`: if true, diff against main branch; if false, diff HEAD (uncommitted)
    pub fn load_diff(&mut self, branch_diff: bool) {
        let Some(selected) = self.table_state.selected() else { return };
        let Some(agent) = self.agents.get(selected) else { return };

        let path = &agent.path;
        let pane_id = agent.pane_id.clone();
        let worktree_name = self.extract_worktree_name(agent).0;

        // Build git diff command
        let mut cmd = std::process::Command::new("git");
        cmd.arg("-C").arg(path)
           .arg("--no-pager")
           .arg("diff")
           .arg("--color=always");

        let title = if branch_diff {
            // Get the base branch from git status if available, fallback to "main"
            let base = self.git_statuses.get(path)
                .map(|s| s.base_branch.as_str())
                .filter(|b| !b.is_empty())
                .unwrap_or("main");
            cmd.arg(format!("{}...HEAD", base));
            format!("Branch Changes: {}", worktree_name)
        } else {
            cmd.arg("HEAD");
            format!("Uncommitted Changes: {}", worktree_name)
        };

        match cmd.output() {
            Ok(output) => {
                let content = String::from_utf8_lossy(&output.stdout).to_string();

                // Handle empty diff - don't open modal
                if content.trim().is_empty() {
                    // TODO: Show temporary status message "No changes"
                    return;
                }

                let line_count = content.lines().count();

                self.view_mode = ViewMode::Diff(DiffView {
                    content,
                    scroll: 0,
                    line_count,
                    viewport_height: 0, // Will be set by UI
                    title,
                    worktree_path: path.clone(),
                    pane_id,
                });
            }
            Err(e) => {
                // Show error in diff modal
                self.view_mode = ViewMode::Diff(DiffView {
                    content: format!("Error running git diff: {}", e),
                    scroll: 0,
                    line_count: 1,
                    viewport_height: 0,
                    title: "Error".to_string(),
                    worktree_path: path.clone(),
                    pane_id,
                });
            }
        }
    }

    pub fn close_diff(&mut self) {
        self.view_mode = ViewMode::Dashboard;
    }

    /// Send commit command to the agent pane and close diff modal
    pub fn send_commit_to_agent(&mut self) {
        if let ViewMode::Diff(diff) = &self.view_mode {
            // Send /commit command to the agent's pane
            // Note: This assumes the agent is ready to receive input
            let _ = tmux::send_keys(&diff.pane_id, "/commit\n");
        }
        self.close_diff();
    }

    /// Trigger merge workflow and close diff modal
    pub fn trigger_merge(&mut self) {
        if let ViewMode::Diff(diff) = &self.view_mode {
            // Run workmux merge in the worktree directory
            // Could also send a command to the agent pane instead
            let _ = std::process::Command::new("workmux")
                .arg("merge")
                .current_dir(&diff.worktree_path)
                .spawn();
        }
        self.close_diff();
        self.should_quit = true; // Exit dashboard after merge
    }
}
```

### Step 3: Update event loop for new keybindings

File: `src/command/dashboard/mod.rs`

Modify the key handling to check `app.view_mode`. Delegate scroll logic to DiffView methods.

```rust
match &mut app.view_mode {
    ViewMode::Dashboard => {
        // Existing dashboard keybindings...
        match key.code {
            // ... existing keys ...
            KeyCode::Char('d') => {
                if key.modifiers.contains(KeyModifiers::SHIFT) {
                    app.load_diff(true);  // Branch diff (D)
                } else {
                    app.load_diff(false); // Uncommitted diff (d)
                }
            }
            _ => {}
        }
    }
    ViewMode::Diff(diff_view) => {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => app.close_diff(),
            KeyCode::Char('j') | KeyCode::Down => diff_view.scroll_down(),
            KeyCode::Char('k') | KeyCode::Up => diff_view.scroll_up(),
            KeyCode::PageDown => diff_view.scroll_page_down(),
            KeyCode::PageUp => diff_view.scroll_page_up(),
            KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                diff_view.scroll_page_down();
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                diff_view.scroll_page_up();
            }
            KeyCode::Char('c') => app.send_commit_to_agent(),
            KeyCode::Char('m') => app.trigger_merge(),
            _ => {}
        }
    }
}
```

### Step 4: Add diff modal rendering

File: `src/command/dashboard/ui.rs`

**Key considerations:**
- Use `Clear` widget to prevent dashboard bleeding through
- Update `viewport_height` during render so scroll logic works
- ANSI parsing happens every frame - acceptable for typical diff sizes
- Cast scroll to u16 for ratatui API (clamping for safety)

```rust
use ratatui::widgets::Clear;

pub fn ui(f: &mut Frame, app: &mut App) {
    // ... existing dashboard rendering (table, preview, footer) ...

    // Render diff modal overlay if in diff mode
    if let ViewMode::Diff(ref mut diff_view) = app.view_mode {
        render_diff_modal(f, diff_view);
    }
}

fn render_diff_modal(f: &mut Frame, diff: &mut DiffView) {
    let area = f.area();
    let popup_area = centered_rect(area, 90, 90);

    // Update viewport height for scroll calculations (subtract 2 for borders)
    diff.viewport_height = popup_area.height.saturating_sub(2);

    // Clear background so dashboard doesn't bleed through
    f.render_widget(Clear, popup_area);

    // Create block with title
    let block = Block::bordered()
        .title(format!(" {} ", diff.title))
        .title_style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
        .border_style(Style::default().fg(Color::DarkGray));

    // Parse ANSI colors from diff content
    let text = diff.content.as_str().into_text()
        .unwrap_or_else(|_| Text::raw(&diff.content));

    // Render scrollable paragraph (cast scroll to u16, clamping for safety)
    let scroll_u16 = diff.scroll.min(u16::MAX as usize) as u16;
    let paragraph = Paragraph::new(text)
        .block(block)
        .scroll((scroll_u16, 0));

    f.render_widget(paragraph, popup_area);

    // Footer with keybindings (render below the modal or as part of block)
    // Option: Include in block title_bottom or render separately
}

/// Helper to create a centered rectangle
fn centered_rect(r: Rect, percent_x: u16, percent_y: u16) -> Rect {
    let popup_layout = Layout::vertical([
        Constraint::Percentage((100 - percent_y) / 2),
        Constraint::Percentage(percent_y),
        Constraint::Percentage((100 - percent_y) / 2),
    ])
    .split(r);

    Layout::horizontal([
        Constraint::Percentage((100 - percent_x) / 2),
        Constraint::Percentage(percent_x),
        Constraint::Percentage((100 - percent_x) / 2),
    ])
    .split(popup_layout[1])[1]
}
```

### Step 5: Update footer help text

Modify existing footer rendering to show different keybindings based on view mode:
- Dashboard mode: existing keybindings
- Diff mode: `[j/k] scroll  [Ctrl+d/u] page  [c] commit  [m] merge  [q] close`

### Step 6: Handle edge cases

| Case | Handling |
|------|----------|
| No changes | Don't open modal, optionally show status message |
| Large diffs | Scroll works with usize; ANSI parsing may lag on huge diffs |
| Git errors | Show error message in modal |
| No agent selected | `load_diff` returns early |
| Agent not ready for input | `c` key sends command anyway (best effort) |

## Configuration (Future)

Potentially add to `.workmux.yaml`:

```yaml
dashboard:
  commit_command: "/commit"  # Command sent to agent on 'c' key
```

## Future Enhancements

### Delta support for syntax-highlighted diffs

The MVP uses `git diff --color=always` which only provides diff-level coloring (red/green for +/-), not syntax highlighting of code content.

For proper syntax highlighting, add support for [delta](https://github.com/dandavison/delta):

```rust
// Check if delta is available and pipe diff through it
fn get_diff_command(path: &Path, branch_diff: bool) -> Command {
    let has_delta = Command::new("which").arg("delta").output()
        .map(|o| o.status.success()).unwrap_or(false);

    if has_delta {
        // Pipe git diff through delta
        // git -C <path> diff --color=always ... | delta --paging=never
    } else {
        // Fallback to plain git diff
    }
}
```

Could also add config option:
```yaml
dashboard:
  diff_highlighter: "delta"  # or "diff-so-fancy" or "none"
```

## Files to Modify

1. `src/command/dashboard/app.rs` - Add ViewMode, DiffView, diff loading logic
2. `src/command/dashboard/mod.rs` - Update event loop for new keybindings
3. `src/command/dashboard/ui.rs` - Add modal rendering
4. `docs/reference/commands/dashboard.md` - Document new keybindings

## Testing

1. Test `d` key opens diff modal with uncommitted changes
2. Test `D` key opens diff modal with branch changes
3. Test scrolling works correctly (j/k, Ctrl+d/u, PageUp/Down)
4. Test `q`/`Esc` closes modal
5. Test `c` sends command to agent pane
6. Test `m` triggers merge and exits dashboard
7. Test with no changes (modal should not open)
8. Test with large diff (scrolling performance)
9. Test with no agent selected (should do nothing)
10. Test with git errors (should show error in modal)
