//! Terminal UI for inspecting BYOKEY state.
//!
//! This crate is an upper-layer management API client. It does not read
//! `SQLite` stores, auth managers, or daemon internals directly; all BYOKEY
//! state is fetched through the `ConnectRPC` management API served by `proxy`.

#![allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]

mod app;
mod ui;

use anyhow::{Context as _, Result, bail};
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{
    io,
    time::{Duration, Instant},
};

use app::App;
use byokey_proto::client::ManagementClient;

const DEFAULT_MANAGEMENT_URL: &str = "http://127.0.0.1:8018";

fn management_client(endpoint: Option<String>) -> Result<ManagementClient> {
    let endpoint = endpoint.unwrap_or_else(|| DEFAULT_MANAGEMENT_URL.to_string());
    let uri: http::Uri = endpoint
        .parse()
        .with_context(|| format!("invalid management API URL {endpoint}"))?;
    if uri.scheme_str() != Some("http") {
        bail!("TUI management client currently supports local http:// URLs only");
    }
    if uri.authority().is_none() {
        bail!("management API URL must include host and port");
    }
    Ok(ManagementClient::local_http(uri))
}

/// Run the TUI until the user quits.
///
/// # Errors
///
/// Returns an error if the terminal cannot be initialized or if the underlying
/// management API URL is invalid.
pub async fn run(endpoint: Option<String>) -> Result<()> {
    let mut app = App::new(management_client(endpoint)?);
    app.refresh().await;

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_loop(&mut terminal, &mut app).await;

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    res
}

async fn run_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
) -> Result<()> {
    use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};

    let tick = Duration::from_millis(200);
    let auto_refresh = Duration::from_secs(5);
    let mut last_auto = Instant::now();

    loop {
        terminal.draw(|f| ui::draw(f, app))?;

        if event::poll(tick)?
            && let Event::Key(key) = event::read()?
            && key.kind == KeyEventKind::Press
        {
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => return Ok(()),
                KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    return Ok(());
                }
                KeyCode::Tab | KeyCode::Right => app.next_tab(),
                KeyCode::BackTab | KeyCode::Left => app.prev_tab(),
                KeyCode::Char('r') => app.refresh().await,
                KeyCode::Down | KeyCode::Char('j') => app.scroll_down(),
                KeyCode::Up | KeyCode::Char('k') => app.scroll_up(),
                _ => {}
            }
        }

        if last_auto.elapsed() >= auto_refresh {
            app.refresh().await;
            last_auto = Instant::now();
        }
    }
}
