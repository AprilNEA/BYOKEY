//! Terminal UI for inspecting BYOKEY state.
//!
//! Reads directly from the local `SQLite` store so it works whether or not the
//! background daemon is running. Liveness of the daemon itself is queried via
//! [`byokey_daemon::process::status`].

#![allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]

mod app;
mod ui;

use anyhow::Result;
use byokey_auth::AuthManager;
use byokey_store::SqliteTokenStore;
use crossterm::{
    event::{DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{
    io,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

use app::App;

/// Open the `SQLite` store at the given path (or the default if `None`).
async fn open_store(db: Option<PathBuf>) -> Result<SqliteTokenStore> {
    let path = match db {
        Some(p) => p,
        None => byokey_daemon::paths::db_path()?,
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let url = format!("sqlite://{}?mode=rwc", path.display());
    SqliteTokenStore::new(&url)
        .await
        .map_err(|e| anyhow::anyhow!("database error: {e}"))
}

/// Run the TUI until the user quits.
///
/// # Errors
///
/// Returns an error if the terminal cannot be initialized or if the underlying
/// store fails to open.
pub async fn run(db: Option<PathBuf>) -> Result<()> {
    let store = Arc::new(open_store(db).await?);
    let auth = Arc::new(AuthManager::new(store.clone(), rquest::Client::new()));

    let mut app = App::new(store, auth);
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
