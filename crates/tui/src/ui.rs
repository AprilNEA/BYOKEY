//! Drawing routines for each TUI tab.

use crate::app::{App, AuthState, ConnectionStatus, ProviderSnapshot, Tab, TokenState};
use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, List, ListItem, ListState, Paragraph, Row, Table, Tabs},
};

pub fn draw(f: &mut Frame, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header / tabs
            Constraint::Min(0),    // body
            Constraint::Length(1), // footer
        ])
        .split(f.area());

    draw_tabs(f, chunks[0], app);
    match app.tab {
        Tab::Status => draw_status(f, chunks[1], app),
        Tab::Accounts => draw_accounts(f, chunks[1], app),
        Tab::Usage => draw_usage(f, chunks[1], app),
    }
    draw_footer(f, chunks[2], app);
}

fn draw_tabs(f: &mut Frame, area: Rect, app: &App) {
    let titles: Vec<Line> = Tab::ALL
        .iter()
        .map(|t| Line::from(Span::raw(t.title())))
        .collect();
    let tabs = Tabs::new(titles)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" BYOKEY — Bring Your Own Keys "),
        )
        .select(app.tab.index())
        .style(Style::default().fg(Color::Gray))
        .highlight_style(
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        );
    f.render_widget(tabs, area);
}

fn draw_status(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(4), Constraint::Min(0)])
        .split(area);

    let server_line = match &app.server {
        ConnectionStatus::Connected { host, port } => Line::from(vec![
            Span::styled(
                "● connected",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::raw(format!("  {host}:{port}")),
        ]),
        ConnectionStatus::Disconnected => Line::from(Span::styled(
            "○ disconnected",
            Style::default().fg(Color::DarkGray),
        )),
    };
    let server = Paragraph::new(vec![server_line]).block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Management API "),
    );
    f.render_widget(server, chunks[0]);

    let rows: Vec<Row> = app
        .providers
        .iter()
        .map(|p| {
            let (state_label, state_style) = state_cell(p);
            Row::new(vec![
                Cell::from(p.id.clone()).style(Style::default().fg(Color::Cyan)),
                Cell::from(p.display_name.clone()),
                Cell::from(state_label).style(state_style),
                Cell::from(if p.enabled { "yes" } else { "no" }),
                Cell::from(p.accounts.len().to_string()).style(Style::default().fg(Color::White)),
                Cell::from(p.models_count.to_string()),
            ])
        })
        .collect();

    let header = Row::new(vec!["id", "name", "state", "enabled", "accounts", "models"])
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .bottom_margin(1);

    let table = Table::new(
        rows,
        [
            Constraint::Length(14),
            Constraint::Min(20),
            Constraint::Length(20),
            Constraint::Length(9),
            Constraint::Length(10),
            Constraint::Length(8),
        ],
    )
    .header(header)
    .block(Block::default().borders(Borders::ALL).title(" Providers "))
    .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut t_state = ratatui::widgets::TableState::default();
    t_state.select(Some(app.selected));
    f.render_stateful_widget(table, chunks[1], &mut t_state);
}

fn state_cell(p: &ProviderSnapshot) -> (String, Style) {
    match p.auth_state {
        AuthState::Authenticated => (
            "authenticated".to_string(),
            Style::default().fg(Color::Green),
        ),
        AuthState::Expired => (
            "expired (refresh)".to_string(),
            Style::default().fg(Color::Yellow),
        ),
        AuthState::NotConfigured => (
            "not configured".to_string(),
            Style::default().fg(Color::DarkGray),
        ),
        AuthState::Unknown => ("unknown".to_string(), Style::default().fg(Color::Red)),
    }
}

fn draw_accounts(f: &mut Frame, area: Rect, app: &App) {
    let mut items: Vec<ListItem> = Vec::new();
    for p in &app.providers {
        items.push(ListItem::new(Line::from(vec![Span::styled(
            format!("[{}] {}", p.id, p.display_name),
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        )])));
        if p.accounts.is_empty() {
            items.push(ListItem::new(Line::from(vec![
                Span::raw("    "),
                Span::styled("(no accounts)", Style::default().fg(Color::DarkGray)),
            ])));
        } else {
            for a in &p.accounts {
                let active = if a.is_active { " (active)" } else { "" };
                let label = a
                    .label
                    .as_deref()
                    .map(|l| format!(" — {l}"))
                    .unwrap_or_default();
                items.push(ListItem::new(Line::from(vec![
                    Span::raw("    • "),
                    Span::styled(a.account_id.clone(), Style::default().fg(Color::White)),
                    Span::styled(label, Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        active,
                        Style::default()
                            .fg(Color::Green)
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        format!(" [{}]", token_state_label(a.token_state)),
                        token_state_style(a.token_state),
                    ),
                ])));
            }
        }
    }

    let list = List::new(items)
        .block(Block::default().borders(Borders::ALL).title(" Accounts "))
        .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
    let mut state = ListState::default();
    state.select(Some(app.selected));
    f.render_stateful_widget(list, area, &mut state);
}

fn token_state_label(state: TokenState) -> &'static str {
    match state {
        TokenState::Valid => "valid",
        TokenState::Expired => "expired",
        TokenState::Invalid => "invalid",
        TokenState::Unknown => "unknown",
    }
}

fn token_state_style(state: TokenState) -> Style {
    match state {
        TokenState::Valid => Style::default().fg(Color::Green),
        TokenState::Expired => Style::default().fg(Color::Yellow),
        TokenState::Invalid => Style::default().fg(Color::Red),
        TokenState::Unknown => Style::default().fg(Color::DarkGray),
    }
}

fn draw_usage(f: &mut Frame, area: Rect, app: &App) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(5), Constraint::Min(0)])
        .split(area);

    let totals = Paragraph::new(vec![
        Line::from(vec![
            Span::styled(
                "Total requests: ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(app.usage.total_requests().to_string()),
        ]),
        Line::from(vec![
            Span::styled(
                "Input tokens:   ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(app.usage.total_input().to_string()),
        ]),
        Line::from(vec![
            Span::styled(
                "Output tokens:  ",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::raw(app.usage.total_output().to_string()),
        ]),
    ])
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(" Cumulative usage "),
    );
    f.render_widget(totals, chunks[0]);

    if app.usage.rows.is_empty() {
        let empty = Paragraph::new("no usage recorded yet")
            .alignment(Alignment::Center)
            .block(Block::default().borders(Borders::ALL).title(" Per-model "));
        f.render_widget(empty, chunks[1]);
        return;
    }

    let rows: Vec<Row> = app
        .usage
        .rows
        .iter()
        .map(|b| {
            Row::new(vec![
                Cell::from(b.model.clone()).style(Style::default().fg(Color::Cyan)),
                Cell::from(b.request_count.to_string()),
                Cell::from(b.input_tokens.to_string()),
                Cell::from(b.output_tokens.to_string()),
            ])
        })
        .collect();

    let header = Row::new(vec!["model", "requests", "input", "output"])
        .style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .bottom_margin(1);

    let table = Table::new(
        rows,
        [
            Constraint::Min(20),
            Constraint::Length(10),
            Constraint::Length(12),
            Constraint::Length(12),
        ],
    )
    .header(header)
    .block(Block::default().borders(Borders::ALL).title(" Per-model "))
    .row_highlight_style(Style::default().add_modifier(Modifier::REVERSED));

    let mut state = ratatui::widgets::TableState::default();
    state.select(Some(app.selected));
    f.render_stateful_widget(table, chunks[1], &mut state);
}

fn draw_footer(f: &mut Frame, area: Rect, app: &App) {
    let mut spans = vec![
        Span::styled("q", Style::default().fg(Color::Cyan)),
        Span::raw(" quit  "),
        Span::styled("Tab", Style::default().fg(Color::Cyan)),
        Span::raw("/"),
        Span::styled("←→", Style::default().fg(Color::Cyan)),
        Span::raw(" switch  "),
        Span::styled("↑↓", Style::default().fg(Color::Cyan)),
        Span::raw("/"),
        Span::styled("jk", Style::default().fg(Color::Cyan)),
        Span::raw(" scroll  "),
        Span::styled("r", Style::default().fg(Color::Cyan)),
        Span::raw(" refresh  "),
    ];
    if let Some(err) = app.last_error.as_ref() {
        spans.push(Span::styled(
            format!("⚠ {err}"),
            Style::default().fg(Color::Red),
        ));
    }
    let footer = Paragraph::new(Line::from(spans)).style(Style::default().fg(Color::DarkGray));
    f.render_widget(footer, area);
}
