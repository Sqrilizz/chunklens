pub mod app;
pub mod ui;

#[cfg(test)]
mod benchmarks;

use crate::{model::ScanResult, protocol::spark::SparkProfileSummary};
use anyhow::Result;
use app::{AppState, Tab};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{io, net::SocketAddr, time::Duration};

struct TerminalSession;
impl Drop for TerminalSession {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

pub fn run_tui(
    scan: ScanResult,
    server_host: Option<String>,
    spark_summary: Option<SparkProfileSummary>,
    rcon_addr: Option<SocketAddr>,
    rcon_password: Option<String>,
) -> Result<()> {
    let _session = TerminalSession;
    let mut terminal = ratatui::try_init()?;
    let mut state = AppState::new(scan, server_host, spark_summary, rcon_addr, rcon_password);
    run_loop(&mut terminal, &mut state)
}

fn run_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    state: &mut AppState,
) -> Result<()> {
    let mut dirty = true;
    while !state.should_quit {
        if !state.server_host.is_empty()
            && !state.poll_pending
            && state.last_poll.elapsed() >= state.poll_interval
        {
            state.poll_server();
            dirty = true;
        }
        dirty |= state.process_background();
        let had_notification = state.notification.is_some();
        state.clear_expired_notification();
        dirty |= had_notification != state.notification.is_some();
        if dirty {
            terminal.draw(|frame| ui::render(frame, state))?;
            dirty = false;
        }
        if event::poll(Duration::from_millis(50))? {
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    handle_key(state, key.code, key.modifiers);
                    dirty = true;
                }
                Event::Resize(_, _) => dirty = true,
                _ => {}
            }
        }
    }
    Ok(())
}

fn handle_key(state: &mut AppState, code: KeyCode, modifiers: KeyModifiers) {
    // Global interrupt
    if modifiers.contains(KeyModifiers::CONTROL) && code == KeyCode::Char('c') {
        state.should_quit = true;
        return;
    }

    if state.help_visible {
        state.help_visible = false;
        return;
    }
    if code == KeyCode::F(1)
        || (code == KeyCode::Char('?') && state.active_tab != Tab::Rcon && !state.is_searching)
    {
        state.help_visible = true;
        return;
    }

    // Modal Search Filter Input
    if state.is_searching {
        match code {
            KeyCode::Enter => {
                state.is_searching = false;
                state.refresh_filter();
            }
            KeyCode::Esc => {
                state.is_searching = false;
                state.search_query = state.search_before_edit.clone();
                state.refresh_filter();
            }
            KeyCode::Backspace => {
                state.search_query.pop();
                state.refresh_filter();
            }
            KeyCode::Char(c) => {
                state.search_query.push(c);
                state.refresh_filter();
            }
            _ => {}
        }
        return;
    }

    // Dedicated RCON Input Mode
    if state.active_tab == Tab::Rcon {
        match code {
            KeyCode::Up => {
                state.recall_rcon(true);
                return;
            }
            KeyCode::Down => {
                state.recall_rcon(false);
                return;
            }
            KeyCode::Tab => {
                state.next_tab();
                return;
            }
            KeyCode::BackTab => {
                state.prev_tab();
                return;
            }
            KeyCode::Enter => {
                state.send_rcon_command();
                return;
            }
            KeyCode::Backspace => {
                state.rcon_input.pop();
                return;
            }
            KeyCode::Esc => {
                if state.rcon_input.is_empty() {
                    state.set_tab(Tab::Monitor);
                } else {
                    state.rcon_input.clear();
                }
                return;
            }
            KeyCode::Char(c) => {
                state.rcon_input.push(c);
                return;
            }
            _ => {}
        }
    }

    // Standard Navigation
    match code {
        KeyCode::Char('q') | KeyCode::Char('Q') | KeyCode::Esc => {
            state.should_quit = true;
        }
        KeyCode::Tab => {
            state.next_tab();
        }
        KeyCode::BackTab => {
            state.prev_tab();
        }
        KeyCode::Char('1') => state.set_tab(Tab::Monitor),
        KeyCode::Char('2') => state.set_tab(Tab::Hotspots),
        KeyCode::Char('3') => state.set_tab(Tab::Heatmap),
        KeyCode::Char('4') => state.set_tab(Tab::Clusters),
        KeyCode::Char('5') => state.set_tab(Tab::Diagnostic),
        KeyCode::Char('6') => state.set_tab(Tab::Spark),
        KeyCode::Char('7') => state.set_tab(Tab::Rcon),

        // Table & list navigation
        KeyCode::Down | KeyCode::Char('j') => match state.active_tab {
            Tab::Clusters => state.next_cluster(),
            Tab::Heatmap => state.pan_heatmap(0.0, 5.0),
            _ => state.next_row(),
        },
        KeyCode::Up | KeyCode::Char('k') => match state.active_tab {
            Tab::Clusters => state.prev_cluster(),
            Tab::Heatmap => state.pan_heatmap(0.0, -5.0),
            _ => state.prev_row(),
        },
        KeyCode::PageDown => {
            state.page_down();
        }
        KeyCode::PageUp => {
            state.page_up();
        }

        KeyCode::Enter => {
            if state.active_tab == Tab::Hotspots {
                state.set_tab(Tab::Diagnostic);
            }
        }

        KeyCode::Char('c') | KeyCode::Char('C') => {
            state.copy_teleport_command();
        }

        KeyCode::Char('/') => {
            state.search_before_edit = state.search_query.clone();
            state.is_searching = true;
        }

        KeyCode::Char('p') | KeyCode::Char('r') if state.active_tab == Tab::Monitor => {
            state.poll_server();
            state.notify(
                if state.server_host.is_empty() {
                    "Use --host to enable monitoring"
                } else {
                    "Refreshing server status…"
                }
                .to_string(),
            );
        }

        // Heatmap navigation
        KeyCode::Left | KeyCode::Char('h') if state.active_tab == Tab::Heatmap => {
            state.pan_heatmap(-5.0, 0.0);
        }
        KeyCode::Right | KeyCode::Char('l') if state.active_tab == Tab::Heatmap => {
            state.pan_heatmap(5.0, 0.0);
        }
        KeyCode::Char('+') | KeyCode::Char('=') if state.active_tab == Tab::Heatmap => {
            state.zoom_heatmap(0.8);
        }
        KeyCode::Char('-') | KeyCode::Char('_') if state.active_tab == Tab::Heatmap => {
            state.zoom_heatmap(1.25);
        }

        _ => {}
    }
}
