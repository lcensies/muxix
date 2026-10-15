//! Muxix's own read-only view of an ADE session, and the pane that hosts it.
//!
//! An ADE agent has no pane (`features().panes` is false), so there is nothing
//! to switch to when one is selected. This opens a muxix-owned view instead:
//! chrome rendered from the ADE's state command, wrapped around its streaming
//! log output passed through verbatim.
//!
//! Every call this makes is a **client-side** call. The ADE's daemon owns the
//! provider process and is the only writer to the session, so this view sits
//! alongside the ADE's phone and desktop clients rather than displacing them —
//! all three can use one session at the same time. That is also why the preset
//! streams with `logs -f` rather than `attach`: attach claims the invoking
//! terminal's stdin, which this pane needs for itself.

use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, anyhow, bail};
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::agent::ade::{AdeAgentState, AdeRuntime};
use crate::agent::runtime::{AgentRef, RuntimeStatus};
use crate::config::Config;
use crate::multiplexer::Multiplexer;

/// Lines of session output kept in memory. The ADE holds the real transcript;
/// this is only what the pane can scroll.
const SCROLLBACK: usize = 4000;

/// Where the ref → view-pane mapping lives, so a second selection focuses the
/// pane already streaming a session instead of spawning another viewer.
fn map_path() -> Result<PathBuf> {
    Ok(crate::xdg::state_dir()?.join("ade-views.json"))
}

fn load_map() -> BTreeMap<String, String> {
    map_path()
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_map(map: &BTreeMap<String, String>) {
    if let Ok(path) = map_path() {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(body) = serde_json::to_string_pretty(map) {
            let _ = std::fs::write(path, body);
        }
    }
}

/// Open a view for a pane-less agent, or focus the one already open.
///
/// Returns the pane now showing the session. Errors are returned rather than
/// swallowed: a selection that silently does nothing is the bug this replaces.
pub fn open_or_focus(mux: &dyn Multiplexer, agent: &AgentRef) -> Result<String> {
    let cfg = Config::load(None).unwrap_or_default();
    let ade = crate::agent::ade::for_name(&cfg, &agent.runtime).ok_or_else(|| {
        anyhow!(
            "'{}' is not a configured agent manager, so its sessions cannot be opened",
            agent.runtime
        )
    })?;
    let view = ade.view().ok_or_else(|| {
        anyhow!(
            "runtime '{}' declares no `view` block, so muxix cannot open its sessions",
            agent.runtime
        )
    })?;

    let wire = agent.to_wire();

    // Desktop hand-off opens no pane at all.
    if view.on_select == crate::config::AdeViewOnSelect::Desktop {
        if view.desktop_args.is_empty() {
            bail!("runtime '{}' declares no desktop_args", agent.runtime);
        }
        let args = ade.render_args(&view.desktop_args, &[("id", agent.id.as_str())]);
        ade.run_client(&args)
            .with_context(|| format!("opening {wire} in the manager's desktop client"))?;
        return Ok(String::new());
    }

    let mut map = load_map();
    if let Some(pane) = map.get(&wire) {
        // A pane the user closed is forgotten rather than focused.
        if mux.capture_pane(pane, 1).is_some() {
            mux.switch_to_pane(pane, None)?;
            return Ok(pane.clone());
        }
        map.remove(&wire);
    }

    let argv = match view.on_select {
        // The ADE's own client, run directly in a plain pane.
        crate::config::AdeViewOnSelect::Ade => {
            let mut argv = vec![ade.command().to_string()];
            argv.extend(
                ade.view_command(agent)
                    .ok_or_else(|| anyhow!("runtime '{}' declares no log_args", agent.runtime))?,
            );
            argv
        }
        // Muxix's own view, re-execing this binary like the sidebar does.
        _ => vec![
            std::env::current_exe()?.display().to_string(),
            "_ade-view".to_string(),
            wire.clone(),
        ],
    };

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let pane = crate::multiplexer::launch::open_argv_pane(mux, &argv, &cwd)?;

    map.insert(wire, pane.clone());
    save_map(&map);
    mux.switch_to_pane(&pane, None)?;
    Ok(pane)
}

/// Shared, mutable state the render loop reads and the worker threads write.
struct Live {
    body: Mutex<Vec<String>>,
    state: Mutex<Option<AdeAgentState>>,
    notice: Mutex<Option<String>>,
}

enum Tick {
    Redraw,
    Input(Event),
}

/// Run the session view (the hidden `_ade-view` command).
pub fn run(wire_ref: &str) -> Result<()> {
    let agent = AgentRef::from_wire(wire_ref);
    let cfg = Config::load(None).unwrap_or_default();
    let ade = crate::agent::ade::for_name(&cfg, &agent.runtime)
        .ok_or_else(|| anyhow!("'{}' is not a configured agent manager", agent.runtime))?;
    let view = ade
        .view()
        .ok_or_else(|| anyhow!("runtime '{}' declares no `view` block", agent.runtime))?
        .clone();

    let live = Arc::new(Live {
        body: Mutex::new(Vec::new()),
        state: Mutex::new(None),
        notice: Mutex::new(None),
    });
    let (tx, rx) = mpsc::channel();

    // Session output. Streamed into scrollback verbatim: the ADE's `--json` is
    // a no-op on this subcommand, so there is no structured form to render and
    // scraping its text would be a parser for an undocumented format.
    if let Some(args) = ade.view_command(&agent) {
        let mut child = Command::new(ade.command())
            .args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .with_context(|| format!("{} {}", ade.command(), args.join(" ")))?;
        if let Some(out) = child.stdout.take() {
            spawn_reader(out, Arc::clone(&live), tx.clone());
        }
        if let Some(err) = child.stderr.take() {
            spawn_reader(err, Arc::clone(&live), tx.clone());
        }
    }

    // Chrome. Polled because the ADE pushes no events; the same trade the rest
    // of the ADE runtime already makes.
    {
        let live = Arc::clone(&live);
        let tx = tx.clone();
        let ade_poll = ade.clone();
        let agent_poll = agent.clone();
        std::thread::spawn(move || {
            loop {
                let next = ade_poll.view_state(&agent_poll).ok();
                if let Ok(mut slot) = live.state.lock() {
                    *slot = next;
                }
                if tx.send(Tick::Redraw).is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        });
    }

    {
        let tx = tx.clone();
        std::thread::spawn(move || {
            while let Ok(ev) = event::read() {
                if tx.send(Tick::Input(ev)).is_err() {
                    break;
                }
            }
        });
    }


    // Resolved before the alternate screen, so a backend-detection failure is
    // readable instead of landing under a TUI.
    let mux = crate::multiplexer::create_backend(crate::multiplexer::detect_backend());
    let mut arming = false;
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let _guard = TerminalGuard;
    let mut terminal = ratatui::Terminal::new(CrosstermBackend::new(stdout))?;

    loop {
        terminal.draw(|f| draw(f, &agent, &live))?;
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(Tick::Input(Event::Key(k))) if k.kind == KeyEventKind::Press => {
                // Any other key cancels a half-confirmed takeover, so a stray E
                // cannot sit armed until an unrelated keypress fires it.
                let confirming = std::mem::take(&mut arming);
                match k.code {
                    KeyCode::Char('q') | KeyCode::Esc => break,
                    KeyCode::Char('a') => {
                        client_action(&ade, &view.permit_args, &agent, &live, "approved");
                    }
                    KeyCode::Char('d') => {
                        client_action(&ade, &view.deny_args, &agent, &live, "denied");
                    }
                    // The second press is the consent `--stop-ade` carries on the
                    // command line: it interrupts whatever the session is doing,
                    // possibly on someone else's device.
                    KeyCode::Char('E') if confirming => {
                        let msg = match crate::command::ade_takeover::execute(
                            &ade,
                            &agent,
                            true,
                            mux.as_ref(),
                        ) {
                            Ok(m) => m,
                            Err(e) => format!("{e}"),
                        };
                        set_notice(&live, msg);
                    }
                    KeyCode::Char('E') => match crate::command::ade_takeover::offer(&ade, &agent)
                    {
                        Ok(m) => {
                            set_notice(&live, format!("{m} -- press E again to do it"));
                            arming = true;
                        }
                        Err(e) => set_notice(&live, format!("{e}")),
                    },
                    _ => {}
                }
            }
            Ok(_) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(())
}

fn spawn_reader<R: io::Read + Send + 'static>(stream: R, live: Arc<Live>, tx: mpsc::Sender<Tick>) {
    std::thread::spawn(move || {
        use io::BufRead;
        let reader = io::BufReader::new(stream);
        for line in reader.lines().map_while(Result::ok) {
            if let Ok(mut body) = live.body.lock() {
                body.push(line);
                let overflow = body.len().saturating_sub(SCROLLBACK);
                if overflow > 0 {
                    body.drain(0..overflow);
                }
            }
            if tx.send(Tick::Redraw).is_err() {
                break;
            }
        }
    });
}

fn set_notice(live: &Arc<Live>, msg: String) {
    if let Ok(mut slot) = live.notice.lock() {
        *slot = Some(msg);
    }
}

fn client_action(
    ade: &AdeRuntime,
    template: &[String],
    agent: &AgentRef,
    live: &Arc<Live>,
    done: &str,
) {
    if template.is_empty() {
        set_notice(live, format!("this manager exposes no way to be {done}"));
        return;
    }
    let args = ade.render_args(template, &[("id", agent.id.as_str())]);
    match ade.run_client(&args) {
        Ok(_) => set_notice(live, format!("{done}")),
        Err(e) => set_notice(live, format!("{e}")),
    }
}

fn draw(f: &mut ratatui::Frame, agent: &AgentRef, live: &Arc<Live>) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(f.area());

    let state = live.state.lock().ok().and_then(|s| s.clone());
    let mut chrome: Vec<Span> = vec![Span::styled(
        agent.to_wire(),
        Style::default().add_modifier(Modifier::BOLD),
    )];
    if let Some(s) = &state {
        chrome.push(Span::raw("  "));
        chrome.push(Span::styled(
            status_label(s.status),
            Style::default().fg(status_color(s.status)),
        ));
        for extra in [s.model.as_deref(), s.mode.as_deref()] {
            if let Some(v) = extra {
                chrome.push(Span::raw("  "));
                chrome.push(Span::raw(v.to_string()));
            }
        }
        chrome.push(Span::raw("  "));
        chrome.push(match s.pending_permissions {
            Some(0) => Span::raw(""),
            Some(n) => Span::styled(format!("{n} pending"), Style::default().fg(Color::Yellow)),
            // Not zero: a session blocked on a prompt must not look settled.
            None => Span::styled("pending: unknown", Style::default().fg(Color::Yellow)),
        });
    } else {
        chrome.push(Span::raw("  (state unavailable)"));
    }
    f.render_widget(
        Paragraph::new(Line::from(chrome))
            .block(Block::default().borders(Borders::ALL).title(" session ")),
        rows[0],
    );

    let body = live.body.lock().map(|b| b.clone()).unwrap_or_default();
    let height = rows[1].height as usize;
    let tail = body.len().saturating_sub(height);
    let text: Vec<Line> = body[tail..]
        .iter()
        .flat_map(|l| crate::command::dashboard::ansi::parse_ansi_to_lines(l))
        .collect();
    f.render_widget(Paragraph::new(text), rows[1]);

    let notice = live.notice.lock().ok().and_then(|n| n.clone());
    let footer = notice.unwrap_or_else(|| "q quit   E take over   a allow   d deny".to_string());
    f.render_widget(
        Paragraph::new(footer).style(Style::default().fg(Color::DarkGray)),
        rows[2],
    );
}

fn status_label(s: RuntimeStatus) -> String {
    format!("{s:?}").to_lowercase()
}

fn status_color(s: RuntimeStatus) -> Color {
    match s {
        RuntimeStatus::Working | RuntimeStatus::Starting => Color::Cyan,
        RuntimeStatus::Waiting => Color::Yellow,
        RuntimeStatus::Done => Color::Green,
        RuntimeStatus::Failed => Color::Red,
        RuntimeStatus::Unknown => Color::DarkGray,
    }
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen);
    }
}
