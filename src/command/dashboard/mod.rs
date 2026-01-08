//! Dashboard TUI for monitoring and managing workmux agents.
//!
//! This module provides an interactive terminal UI that displays:
//! - All running agent panes across tmux sessions
//! - Git status for each worktree
//! - Agent status (working/waiting/done) with elapsed time
//! - Live preview of selected agent's terminal output
//!
//! # Module Structure
//!
//! - `app`: Application state and business logic
//! - `sort`: Sort mode enum and tmux persistence
//! - `ui`: TUI rendering with ratatui

mod app;
mod sort;
mod ui;

use anyhow::Result;
use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::backend::CrosstermBackend;
use std::io;
use std::time::Duration;

use crate::git;
use crate::tmux;

use self::app::{App, ViewMode};
use self::ui::{SPINNER_FRAME_COUNT, ui};

pub fn run() -> Result<()> {
    // Check if tmux is running
    if !tmux::is_running().unwrap_or(false) {
        println!("No tmux server running.");
        return Ok(());
    }

    // Setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = ratatui::Terminal::new(backend)?;

    // Create app state
    let mut app = App::new()?;

    // Main loop
    let tick_rate = Duration::from_millis(250);
    let mut last_tick = std::time::Instant::now();
    let refresh_interval = Duration::from_secs(2);
    let mut last_refresh = std::time::Instant::now();
    // Preview refreshes more frequently than the agent list
    // Use a faster refresh rate when in input mode for responsive typing feedback
    let preview_refresh_interval_normal = Duration::from_millis(500);
    let preview_refresh_interval_input = Duration::from_millis(100);
    let mut last_preview_refresh = std::time::Instant::now();

    loop {
        terminal.draw(|f| ui(f, &mut app))?;

        // Calculate timeout to respect the next scheduled preview refresh
        let current_preview_interval = if app.input_mode {
            preview_refresh_interval_input
        } else {
            preview_refresh_interval_normal
        };
        let time_until_preview =
            current_preview_interval.saturating_sub(last_preview_refresh.elapsed());
        let time_until_tick = tick_rate.saturating_sub(last_tick.elapsed());
        let timeout = time_until_tick.min(time_until_preview);

        if event::poll(timeout)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            // Check if we're in diff mode and need to toggle (read before mutable borrow)
            let diff_toggle = if let ViewMode::Diff(diff_view) = &app.view_mode {
                if key.code == KeyCode::Char('d') && !key.modifiers.contains(KeyModifiers::CONTROL)
                {
                    Some(!diff_view.is_branch_diff)
                } else {
                    None
                }
            } else {
                None
            };

            // Handle diff toggle separately to avoid borrow issues
            if let Some(toggle_to_branch) = diff_toggle {
                app.load_diff(toggle_to_branch);
            } else {
                match &mut app.view_mode {
                    ViewMode::Dashboard => {
                        if app.input_mode {
                            // In input mode: forward keys to the selected pane
                            match key.code {
                                KeyCode::Esc => {
                                    app.input_mode = false;
                                }
                                KeyCode::Enter => {
                                    app.send_key_to_selected("Enter");
                                }
                                KeyCode::Backspace => {
                                    app.send_key_to_selected("BSpace");
                                }
                                KeyCode::Tab => {
                                    app.send_key_to_selected("Tab");
                                }
                                KeyCode::Up => {
                                    app.send_key_to_selected("Up");
                                }
                                KeyCode::Down => {
                                    app.send_key_to_selected("Down");
                                }
                                KeyCode::Left => {
                                    app.send_key_to_selected("Left");
                                }
                                KeyCode::Right => {
                                    app.send_key_to_selected("Right");
                                }
                                KeyCode::Char(c) => {
                                    // Send the character to the pane
                                    app.send_key_to_selected(&c.to_string());
                                }
                                _ => {}
                            }
                            // Refresh preview immediately after sending input
                            app.refresh_preview();
                            last_preview_refresh = std::time::Instant::now();
                        } else {
                            // Normal dashboard mode: handle navigation and commands
                            match key.code {
                                KeyCode::Char('q') | KeyCode::Esc => app.should_quit = true,
                                KeyCode::Char('j') | KeyCode::Down => app.next(),
                                KeyCode::Char('k') | KeyCode::Up => app.previous(),
                                KeyCode::Enter => app.jump_to_selected(),
                                KeyCode::Char('p') => app.peek_selected(),
                                KeyCode::Char('s') => app.cycle_sort_mode(),
                                KeyCode::Char('i') => {
                                    // Enter input mode if an agent is selected
                                    if app.table_state.selected().is_some()
                                        && !app.agents.is_empty()
                                    {
                                        app.input_mode = true;
                                    }
                                }
                                // Preview scrolling with Ctrl+U/D
                                KeyCode::Char('u')
                                    if key.modifiers.contains(KeyModifiers::CONTROL) =>
                                {
                                    app.scroll_preview_up(
                                        app.preview_height,
                                        app.preview_line_count,
                                    );
                                }
                                KeyCode::Char('d')
                                    if key.modifiers.contains(KeyModifiers::CONTROL) =>
                                {
                                    app.scroll_preview_down(
                                        app.preview_height,
                                        app.preview_line_count,
                                    );
                                }
                                // Open diff view (starts with WIP, toggle with d)
                                KeyCode::Char('d') => {
                                    app.load_diff(false); // Start with WIP view
                                }
                                // Quick jump: 1-9 for rows 0-8
                                KeyCode::Char(c @ '1'..='9') => {
                                    app.jump_to_index((c as u8 - b'1') as usize);
                                }
                                _ => {}
                            }
                        }
                    }
                    ViewMode::Diff(diff_view) => {
                        // Diff view mode: handle scrolling and actions
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
                            // Toggle is handled above
                            KeyCode::Char('c') => app.send_commit_to_agent(),
                            KeyCode::Char('m') => app.trigger_merge(),
                            _ => {}
                        }
                    }
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            last_tick = std::time::Instant::now();
            // Advance spinner animation frame (wrap at frame count to avoid skip artifact)
            app.spinner_frame = (app.spinner_frame + 1) % SPINNER_FRAME_COUNT;
        }

        // Auto-refresh agent list every 2 seconds
        if last_refresh.elapsed() >= refresh_interval {
            app.refresh();
            last_refresh = std::time::Instant::now();
        }

        // Auto-refresh preview more frequently for live updates
        // Uses faster refresh rate in input mode (set at top of loop)
        if last_preview_refresh.elapsed() >= current_preview_interval {
            app.refresh_preview();
            last_preview_refresh = std::time::Instant::now();
        }

        if app.should_quit || app.should_jump {
            break;
        }
    }

    // Save git status cache before exiting
    git::save_status_cache(&app.git_statuses);

    // Restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    Ok(())
}
