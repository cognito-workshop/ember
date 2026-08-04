use std::sync::Arc;
use std::time::{Duration, Instant};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph, Tabs, Wrap},
    Frame, Terminal,
};

use crate::config::Config;
use crate::wisp::plugins::{Metrics, MetricsSnapshot};

pub struct TuiState {
    pub should_quit: bool,
    pub selected_tab: usize,
    pub scroll_offset: usize,
    pub metrics: MetricsSnapshot,
    pub logs: Vec<String>,
    pub start_time: Instant,
}

impl TuiState {
    fn new() -> Self {
        Self {
            should_quit: false,
            selected_tab: 0,
            scroll_offset: 0,
            metrics: MetricsSnapshot {
                connections_total: 0,
                connections_active: 0,
                streams_total: 0,
                streams_active: 0,
                bytes_in: 0,
                bytes_out: 0,
            },
            logs: Vec::new(),
            start_time: Instant::now(),
        }
    }

    fn uptime(&self) -> Duration {
        self.start_time.elapsed()
    }

    fn push_log(&mut self, msg: String) {
        self.logs.push(msg);
        if self.logs.len() > 500 {
            self.logs.remove(0);
        }
    }
}

const TAB_NAMES: &[&str] = &[
    "Overview",
    "Connections",
    "Streams",
    "Throughput",
    "Plugins",
    "Logs",
];

pub async fn run_tui(metrics: Arc<Metrics>, config: Config) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    enable_raw_mode()?;
    let mut stdout = std::io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut state = TuiState::new();
    state.push_log(format!("TUI started — Ember v{}", env!("CARGO_PKG_VERSION")));
    state.push_log(format!(
        "Listening on {}:{}",
        config.server.host, config.server.port
    ));
    if config.tls.enabled {
        state.push_log("TLS enabled".into());
    }

    let tick_rate = Duration::from_millis(250);
    let mut last_tick = Instant::now();

    loop {
        terminal.draw(|f| draw(f, &state))?;

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or(Duration::ZERO);

        if event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    handle_input(key.code, key.modifiers, &mut state, &metrics);
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            state.metrics = metrics.snapshot();
            last_tick = Instant::now();
        }

        if state.should_quit {
            break;
        }
    }

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn handle_input(key: KeyCode, modifiers: KeyModifiers, state: &mut TuiState, _metrics: &Metrics) {
    match key {
        KeyCode::Char('q') => {
            state.should_quit = true;
        }
        KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
            state.should_quit = true;
        }
        KeyCode::Tab => {
            state.selected_tab = (state.selected_tab + 1) % TAB_NAMES.len();
            state.scroll_offset = 0;
        }
        KeyCode::BackTab => {
            state.selected_tab = if state.selected_tab == 0 {
                TAB_NAMES.len() - 1
            } else {
                state.selected_tab - 1
            };
            state.scroll_offset = 0;
        }
        KeyCode::Up => {
            state.scroll_offset = state.scroll_offset.saturating_add(1);
        }
        KeyCode::Down => {
            if state.scroll_offset > 0 {
                state.scroll_offset -= 1;
            }
        }
        KeyCode::Home => {
            state.scroll_offset = 0;
        }
        KeyCode::End => {
            state.scroll_offset = usize::MAX;
        }
        KeyCode::Char('r') => {
            state.push_log("Metrics refreshed".into());
        }
        _ => {}
    }
}

fn draw(frame: &mut Frame, state: &TuiState) {
    let area = frame.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Min(0),   // body
            Constraint::Length(1), // footer
        ])
        .split(area);

    draw_header(frame, state, chunks[0]);
    draw_body(frame, state, chunks[1]);
    draw_footer(frame, state, chunks[2]);
}

fn draw_header(frame: &mut Frame, state: &TuiState, area: Rect) {
    let uptime = state.uptime();
    let uptime_str = format!(
        "{:02}:{:02}:{:02}",
        uptime.as_secs() / 3600,
        (uptime.as_secs() % 3600) / 60,
        uptime.as_secs() % 60
    );

    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            " Ember Wisp Server ",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("v1.0.0  "),
        Span::styled("uptime: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            &uptime_str,
            Style::default()
                .fg(Color::Green)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  |  conns: {}", state.metrics.connections_active),
            Style::default().fg(Color::Cyan),
        ),
        Span::styled(
            format!("  |  streams: {}", state.metrics.streams_active),
            Style::default().fg(Color::Cyan),
        ),
    ]))
    .block(
        Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Yellow))
            .title("Ember Dashboard"),
    );

    frame.render_widget(header, area);
}

fn draw_body(frame: &mut Frame, state: &TuiState, area: Rect) {
    let tab_bar = draw_tab_bar(state);
    let body_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(0)])
        .split(area);

    frame.render_widget(tab_bar, body_chunks[0]);

    match state.selected_tab {
        0 => draw_overview(frame, state, body_chunks[1]),
        1 => draw_connections_panel(frame, state, body_chunks[1]),
        2 => draw_streams_panel(frame, state, body_chunks[1]),
        3 => draw_throughput_panel(frame, state, body_chunks[1]),
        4 => draw_plugins_panel(frame, state, body_chunks[1]),
        5 => draw_logs_panel(frame, state, body_chunks[1]),
        _ => {}
    }
}

fn draw_tab_bar(state: &TuiState) -> Tabs<'static> {
    let titles: Vec<Line> = TAB_NAMES
        .iter()
        .map(|name| Line::from(Span::raw(*name)))
        .collect();

    Tabs::new(titles)
        .select(state.selected_tab)
        .style(Style::default().fg(Color::DarkGray))
        .highlight_style(
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        )
        .block(Block::default().borders(Borders::BOTTOM))
}

fn draw_overview(frame: &mut Frame, state: &TuiState, area: Rect) {
    let metrics = &state.metrics;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(8), // connections + streams
            Constraint::Length(6), // throughput
            Constraint::Min(0),   // recent logs
        ])
        .split(area);

    // Connections + Streams panel
    let top_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(chunks[0]);

    let conn_block = Block::default()
        .title("Connections")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let conn_text = vec![
        Line::from(vec![
            Span::styled("Active: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", metrics.connections_active),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Total:  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", metrics.connections_total),
                Style::default().fg(Color::White),
            ),
        ]),
    ];

    frame.render_widget(Paragraph::new(conn_text).block(conn_block), top_chunks[0]);

    let stream_block = Block::default()
        .title("Streams")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta));

    let stream_text = vec![
        Line::from(vec![
            Span::styled("Active: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", metrics.streams_active),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(vec![
            Span::styled("Total:  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", metrics.streams_total),
                Style::default().fg(Color::White),
            ),
        ]),
    ];

    frame.render_widget(
        Paragraph::new(stream_text).block(stream_block),
        top_chunks[1],
    );

    // Throughput panel
    let throughput_block = Block::default()
        .title("Throughput")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let throughput_text = vec![
        Line::from(vec![
            Span::styled("Bytes in:  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format_bytes(metrics.bytes_in),
                Style::default().fg(Color::Cyan),
            ),
        ]),
        Line::from(vec![
            Span::styled("Bytes out: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format_bytes(metrics.bytes_out),
                Style::default().fg(Color::Magenta),
            ),
        ]),
    ];

    frame.render_widget(
        Paragraph::new(throughput_text).block(throughput_block),
        chunks[1],
    );

    // Recent logs
    let log_block = Block::default()
        .title("Recent Logs")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let visible_height = chunks[2].height as usize;
    let start = if state.logs.len() > visible_height {
        state.logs.len() - visible_height
    } else {
        0
    };

    let log_lines: Vec<Line> = state.logs[start..]
        .iter()
        .map(|l| Line::from(Span::styled(l.as_str(), Style::default().fg(Color::White))))
        .collect();

    frame.render_widget(Paragraph::new(log_lines).block(log_block), chunks[2]);
}

fn draw_connections_panel(frame: &mut Frame, state: &TuiState, area: Rect) {
    let metrics = &state.metrics;

    let block = Block::default()
        .title("Connection Details")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let items = vec![
        Line::from(vec![
            Span::styled("Active connections: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", metrics.connections_active),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Total connections:  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", metrics.connections_total),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "Connection limit:   ",
                Style::default().fg(Color::DarkGray),
            ),
            Span::styled("10000", Style::default().fg(Color::Yellow)),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "No per-connection details available in this view.",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    frame.render_widget(Paragraph::new(items), inner);
}

fn draw_streams_panel(frame: &mut Frame, state: &TuiState, area: Rect) {
    let metrics = &state.metrics;

    let block = Block::default()
        .title("Stream Details")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Magenta));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let items = vec![
        Line::from(vec![
            Span::styled("Active streams: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", metrics.streams_active),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Total streams:  ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format!("{}", metrics.streams_total),
                Style::default().fg(Color::White),
            ),
        ]),
        Line::from(""),
        Line::from(Span::styled(
            "No per-stream details available in this view.",
            Style::default().fg(Color::DarkGray),
        )),
    ];

    frame.render_widget(Paragraph::new(items), inner);
}

fn draw_throughput_panel(frame: &mut Frame, state: &TuiState, area: Rect) {
    let metrics = &state.metrics;

    let block = Block::default()
        .title("Throughput")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Yellow));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let items = vec![
        Line::from(vec![
            Span::styled("Bytes received: ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format_bytes(metrics.bytes_in),
                Style::default().fg(Color::Cyan),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Bytes sent:     ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                format_bytes(metrics.bytes_out),
                Style::default().fg(Color::Magenta),
            ),
        ]),
        Line::from(""),
        Line::from(vec![
            Span::styled(
                "Throughput counters are cumulative since server start.",
                Style::default().fg(Color::DarkGray),
            ),
        ]),
    ];

    frame.render_widget(Paragraph::new(items), inner);
}

fn draw_plugins_panel(frame: &mut Frame, _state: &TuiState, area: Rect) {
    let block = Block::default()
        .title("Active Plugins")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Blue));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let plugins = vec![
        ListItem::new(Line::from(vec![
            Span::styled(
                " ● ",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("metrics", Style::default().fg(Color::White)),
            Span::styled(" — connection/stream/byte counters", Style::default().fg(Color::DarkGray)),
        ])),
        ListItem::new(Line::from(vec![
            Span::styled(
                " ● ",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("connection-limiter", Style::default().fg(Color::White)),
            Span::styled(" — global max connections", Style::default().fg(Color::DarkGray)),
        ])),
        ListItem::new(Line::from(vec![
            Span::styled(
                " ● ",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("logger", Style::default().fg(Color::White)),
            Span::styled(" — event logging", Style::default().fg(Color::DarkGray)),
        ])),
    ];

    frame.render_widget(List::new(plugins), inner);
}

fn draw_logs_panel(frame: &mut Frame, state: &TuiState, area: Rect) {
    let block = Block::default()
        .title(format!("Logs ({})", state.logs.len()))
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::DarkGray));

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let visible_height = inner.height as usize;
    let total = state.logs.len();

    let start = if state.scroll_offset >= total {
        0
    } else if state.scroll_offset > 0 {
        total - state.scroll_offset
    } else if total > visible_height {
        total - visible_height
    } else {
        0
    };

    let end = std::cmp::min(start + visible_height, total);

    let log_lines: Vec<Line> = state.logs[start..end]
        .iter()
        .map(|l| Line::from(Span::styled(l.as_str(), Style::default().fg(Color::White))))
        .collect();

    frame.render_widget(Paragraph::new(log_lines).wrap(Wrap { trim: false }), inner);

    if total > visible_height {
        let scroll_info = format!(
            " {}-{}/{} (↑/↓ to scroll) ",
            start + 1,
            end,
            total,
        );
        let footer_area = Rect {
            x: inner.x,
            y: inner.y + inner.height.saturating_sub(1),
            width: inner.width,
            height: 1,
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                scroll_info,
                Style::default().fg(Color::DarkGray),
            ))),
            footer_area,
        );
    }
}

fn draw_footer(frame: &mut Frame, _state: &TuiState, area: Rect) {
    let footer = Paragraph::new(Line::from(vec![
        Span::styled(
            " q",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(":quit  "),
        Span::styled(
            "Tab",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(":switch  "),
        Span::styled(
            "↑↓",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(":scroll  "),
        Span::styled(
            "r",
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw(":refresh"),
    ]))
    .style(Style::default().bg(Color::Black));

    frame.render_widget(footer, area);
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}
