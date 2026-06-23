//! prowl-tui — ratatui フロントエンド（方針A）。
//!
//! 制約 C-03: `watch<AppState>` 駆動の再描画 + TEA風の「入力 → Command 翻訳」。
//! エンジンとは `prowl-app` の契約だけで繋がる（コアには非依存）。

use std::io::{self, Stdout};

use crossterm::event::{Event as CtEvent, EventStream, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use futures_util::StreamExt;
use prowl_app::{AppState, Command, EngineHandle, Frontend, HostRow, HostStatus, PortScanState};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState};
use ratatui::{Frame, Terminal};

type Term = Terminal<CrosstermBackend<Stdout>>;

/// ratatui ベースのフロントエンド。
#[derive(Default)]
pub struct TuiFrontend;

#[async_trait::async_trait]
impl Frontend for TuiFrontend {
    async fn run(self: Box<Self>, engine: EngineHandle) -> anyhow::Result<()> {
        let mut term = setup_terminal()?;
        let res = event_loop(&mut term, engine).await;
        restore_terminal(&mut term)?;
        res
    }
}

async fn event_loop(term: &mut Term, engine: EngineHandle) -> anyhow::Result<()> {
    let EngineHandle {
        commands,
        mut state,
        ..
    } = engine;

    let mut input = EventStream::new();
    let mut filter_mode = false;
    let mut selected: usize = 0;

    loop {
        let snapshot = state.borrow().clone();
        let visible = snapshot.visible_hosts();
        if selected >= visible.len() {
            selected = visible.len().saturating_sub(1);
        }
        let selected_ip = visible.get(selected).map(|h| h.ip);

        term.draw(|f| draw(f, &snapshot, &visible, selected, filter_mode))?;

        tokio::select! {
            changed = state.changed() => {
                if changed.is_err() {
                    break;
                }
            }
            maybe_ev = input.next() => {
                let Some(Ok(CtEvent::Key(key))) = maybe_ev else { continue };
                if key.kind != KeyEventKind::Press {
                    continue;
                }

                if filter_mode {
                    match key.code {
                        KeyCode::Esc | KeyCode::Enter => filter_mode = false,
                        KeyCode::Backspace => {
                            let mut f = snapshot.filter.clone();
                            f.pop();
                            let _ = commands.send(Command::SetFilter(f)).await;
                        }
                        KeyCode::Char(c) => {
                            let mut f = snapshot.filter.clone();
                            f.push(c);
                            let _ = commands.send(Command::SetFilter(f)).await;
                        }
                        _ => {}
                    }
                    continue;
                }

                match key.code {
                    KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                        let _ = commands.send(Command::Quit).await;
                        break;
                    }
                    KeyCode::Char('q') => {
                        let _ = commands.send(Command::Quit).await;
                        break;
                    }
                    KeyCode::Char('r') => {
                        let _ = commands.send(Command::Rescan).await;
                    }
                    KeyCode::Char('/') => filter_mode = true,
                    KeyCode::Char('m') => {
                        let _ = commands.send(Command::ToggleMonitor).await;
                    }
                    KeyCode::Down | KeyCode::Char('j') => {
                        if !visible.is_empty() {
                            selected = (selected + 1).min(visible.len() - 1);
                        }
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        selected = selected.saturating_sub(1);
                    }
                    KeyCode::Enter | KeyCode::Char('s') => {
                        if let Some(ip) = selected_ip {
                            let _ = commands.send(Command::SelectHost(ip)).await;
                            let _ = commands.send(Command::ScanPorts(ip)).await;
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}

fn draw(f: &mut Frame, state: &AppState, visible: &[&HostRow], selected: usize, filter_mode: bool) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // ヘッダ
            Constraint::Min(3),    // 本体
            Constraint::Length(3), // フッタ
        ])
        .split(f.area());

    draw_header(f, rows[0], state);

    let body = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
        .split(rows[1]);

    draw_host_table(f, body[0], visible, selected);
    draw_detail(f, body[1], state, visible.get(selected).copied());
    draw_footer(f, rows[2], state, filter_mode);
}

fn draw_header(f: &mut Frame, area: ratatui::layout::Rect, state: &AppState) {
    let subnet = state.subnet.clone().unwrap_or_else(|| "(未検出)".into());
    let scanning = if state.scanning { " [スキャン中…]" } else { "" };
    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            "prowl",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  subnet: "),
        Span::styled(subnet, Style::default().fg(Color::Yellow)),
        Span::styled(scanning, Style::default().fg(Color::Green)),
    ]))
    .block(Block::default().borders(Borders::ALL));
    f.render_widget(header, area);
}

fn draw_host_table(f: &mut Frame, area: ratatui::layout::Rect, visible: &[&HostRow], selected: usize) {
    let rows = visible.iter().map(|h| {
        let dash = || "-".to_string();
        let mark = match h.status {
            HostStatus::Up => " ",
            HostStatus::New => "+",
            HostStatus::Down => "×",
        };
        Row::new(vec![
            Cell::from(format!("{mark}{}", h.ip)),
            Cell::from(h.vendor.clone().unwrap_or_else(dash)),
            Cell::from(h.hostname.clone().unwrap_or_else(dash)),
        ])
        .style(status_style(h.status))
    });
    let table = Table::new(
        rows,
        [
            Constraint::Length(16),
            Constraint::Length(18),
            Constraint::Min(8),
        ],
    )
    .header(
        Row::new([" IP", "Vendor", "Host"])
            .style(Style::default().add_modifier(Modifier::BOLD | Modifier::REVERSED)),
    )
    .row_highlight_style(Style::default().fg(Color::Black).bg(Color::Cyan))
    .highlight_symbol("▌")
    .block(
        Block::default()
            .borders(Borders::ALL)
            .title(format!(" hosts ({}) ", visible.len())),
    );

    let mut ts = TableState::default();
    if !visible.is_empty() {
        ts.select(Some(selected));
    }
    f.render_stateful_widget(table, area, &mut ts);
}

fn draw_detail(
    f: &mut Frame,
    area: ratatui::layout::Rect,
    state: &AppState,
    host: Option<&HostRow>,
) {
    let mut lines: Vec<Line> = Vec::new();

    match host {
        None => lines.push(Line::raw("(ホスト未選択)")),
        Some(h) => {
            let (label, color) = status_label(h.status);
            lines.push(Line::from(vec![
                Span::styled("IP   ", Style::default().fg(Color::DarkGray)),
                Span::styled(h.ip.to_string(), Style::default().fg(Color::Yellow)),
                Span::raw("  "),
                Span::styled(format!("[{label}]"), Style::default().fg(color)),
            ]));
            lines.push(kv("MAC  ", h.mac.clone().unwrap_or_else(|| "-".into())));
            lines.push(kv("Vendor", h.vendor.clone().unwrap_or_else(|| "-".into())));
            lines.push(kv("Host ", h.hostname.clone().unwrap_or_else(|| "-".into())));
            lines.push(Line::raw(""));

            let ps = &state.port_scan;
            if ps.target == Some(h.ip) {
                match ps.state {
                    PortScanState::Scanning => {
                        lines.push(Line::styled(
                            "ポートスキャン中…",
                            Style::default().fg(Color::Green),
                        ));
                    }
                    PortScanState::Done => {
                        lines.push(Line::styled(
                            format!("開放ポート: {}", ps.open.len()),
                            Style::default().add_modifier(Modifier::BOLD),
                        ));
                        if ps.open.is_empty() {
                            lines.push(Line::raw("  (なし)"));
                        }
                        for p in &ps.open {
                            let svc = p.service.clone().unwrap_or_default();
                            let mut spans = vec![
                                Span::styled(
                                    format!("  {:>5}/tcp ", p.port),
                                    Style::default().fg(Color::Cyan),
                                ),
                                Span::raw(format!("{svc:<12} ")),
                            ];
                            if let Some(b) = &p.banner {
                                spans.push(Span::styled(
                                    b.clone(),
                                    Style::default().fg(Color::DarkGray),
                                ));
                            }
                            lines.push(Line::from(spans));
                        }
                    }
                    PortScanState::Idle => lines.push(scan_hint()),
                }
            } else {
                lines.push(scan_hint());
            }
        }
    }

    let detail = Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" detail "));
    f.render_widget(detail, area);
}

fn draw_footer(f: &mut Frame, area: ratatui::layout::Rect, state: &AppState, filter_mode: bool) {
    let line = if filter_mode {
        Line::from(vec![
            Span::styled("filter> ", Style::default().fg(Color::Magenta)),
            Span::raw(state.filter.clone()),
            Span::styled("  (Enter/Esc で確定)", Style::default().fg(Color::DarkGray)),
        ])
    } else {
        let mon = if state.monitoring {
            Span::styled("●監視中", Style::default().fg(Color::Green))
        } else {
            Span::styled("‖監視停止", Style::default().fg(Color::DarkGray))
        };
        Line::from(vec![
            mon,
            Span::raw("  "),
            Span::raw(state.status.clone()),
            Span::styled(
                "   [↑↓]選択 [Enter]ポート [m]監視 [r]再 [/]絞込 [q]終了",
                Style::default().fg(Color::DarkGray),
            ),
        ])
    };
    f.render_widget(
        Paragraph::new(line).block(Block::default().borders(Borders::ALL)),
        area,
    );
}

fn status_style(s: HostStatus) -> Style {
    match s {
        HostStatus::Up => Style::default(),
        HostStatus::New => Style::default().fg(Color::Green),
        HostStatus::Down => Style::default().fg(Color::Red).add_modifier(Modifier::DIM),
    }
}

fn status_label(s: HostStatus) -> (&'static str, Color) {
    match s {
        HostStatus::Up => ("稼働", Color::Green),
        HostStatus::New => ("新規", Color::Green),
        HostStatus::Down => ("離脱", Color::Red),
    }
}

fn kv(key: &'static str, val: String) -> Line<'static> {
    Line::from(vec![
        Span::styled(key, Style::default().fg(Color::DarkGray)),
        Span::raw(" "),
        Span::raw(val),
    ])
}

fn scan_hint() -> Line<'static> {
    Line::styled(
        "Enter でポートスキャン",
        Style::default().fg(Color::DarkGray),
    )
}

fn setup_terminal() -> anyhow::Result<Term> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    Ok(Terminal::new(backend)?)
}

fn restore_terminal(term: &mut Term) -> anyhow::Result<()> {
    disable_raw_mode()?;
    execute!(term.backend_mut(), LeaveAlternateScreen)?;
    term.show_cursor()?;
    Ok(())
}
