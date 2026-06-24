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
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, Borders, Cell, Clear, List, ListItem, ListState, Paragraph, Row, Table, TableState,
};
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
    // NIC 選択ポップアップ（`i` で開く擬似プルダウン）。
    let mut iface_mode = false;
    let mut iface_cursor: usize = 0;
    // コピーモード（`y` で開始 → i/m/v/h で項目を選んでクリップボードへ）。
    let mut copy_mode = false;
    let mut copy_status: Option<String> = None;
    let mut selected: usize = 0;

    loop {
        let snapshot = state.borrow().clone();
        let visible = snapshot.visible_hosts();
        if selected >= visible.len() {
            selected = visible.len().saturating_sub(1);
        }
        let selected_ip = visible.get(selected).map(|h| h.ip);
        let selected_host = visible.get(selected).copied();

        term.draw(|f| {
            draw(
                f,
                &snapshot,
                &visible,
                selected,
                filter_mode,
                iface_mode,
                iface_cursor,
                copy_mode,
                copy_status.as_deref(),
            )
        })?;

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

                // コピーモード: i/m/v/h で項目を選んでクリップボードへ（GPUI 相当）。
                if copy_mode {
                    let field = match key.code {
                        KeyCode::Char('i') => selected_host.map(|h| h.ip.to_string()),
                        KeyCode::Char('m') => selected_host.and_then(|h| h.mac.clone()),
                        KeyCode::Char('v') => selected_host.and_then(|h| h.vendor.clone()),
                        KeyCode::Char('h') => selected_host.and_then(|h| h.hostname.clone()),
                        _ => {
                            copy_mode = false; // Esc 等で取消
                            continue;
                        }
                    };
                    copy_mode = false;
                    copy_status = Some(match field {
                        Some(text) => match copy_to_clipboard(&text) {
                            Ok(()) => format!("コピー: {text}"),
                            Err(e) => format!("コピー失敗: {e}"),
                        },
                        None => "(値なし)".to_string(),
                    });
                    continue;
                }

                // NIC 選択ポップアップ表示中はそちらに入力を吸わせる（擬似プルダウン）。
                if iface_mode {
                    let ifaces = &snapshot.interfaces;
                    match key.code {
                        KeyCode::Esc | KeyCode::Char('i') | KeyCode::Char('q') => iface_mode = false,
                        KeyCode::Down | KeyCode::Char('j') => {
                            if !ifaces.is_empty() {
                                iface_cursor = (iface_cursor + 1).min(ifaces.len() - 1);
                            }
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            iface_cursor = iface_cursor.saturating_sub(1);
                        }
                        KeyCode::Enter => {
                            if let Some(nic) = ifaces.get(iface_cursor) {
                                let _ = commands
                                    .send(Command::SelectInterface(nic.name.clone()))
                                    .await;
                            }
                            iface_mode = false;
                        }
                        _ => {}
                    }
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
                    KeyCode::Char('y') => {
                        // 選択ホストがあればコピーモードへ（i/m/v/h で項目選択）。
                        if selected_host.is_some() {
                            copy_mode = true;
                        }
                    }
                    KeyCode::Char('i') => {
                        // 候補が複数あるときだけ NIC ピッカーを開く。
                        if snapshot.interfaces.len() > 1 {
                            iface_mode = true;
                            iface_cursor = snapshot
                                .current_iface
                                .as_ref()
                                .and_then(|cur| {
                                    snapshot.interfaces.iter().position(|n| &n.name == cur)
                                })
                                .unwrap_or(0);
                        }
                    }
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

#[allow(clippy::too_many_arguments)]
fn draw(
    f: &mut Frame,
    state: &AppState,
    visible: &[&HostRow],
    selected: usize,
    filter_mode: bool,
    iface_mode: bool,
    iface_cursor: usize,
    copy_mode: bool,
    copy_status: Option<&str>,
) {
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
    draw_footer(f, rows[2], state, filter_mode, copy_mode, copy_status);

    // NIC ピッカーは最後にオーバーレイ描画する（擬似プルダウン）。
    if iface_mode {
        draw_iface_popup(f, state, iface_cursor);
    }
}

/// 中央寄せの `width`×`height` の矩形を返す（`area` をはみ出さないようクランプ）。
fn centered_rect(width: u16, height: u16, area: Rect) -> Rect {
    let w = width.min(area.width);
    let h = height.min(area.height);
    Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    }
}

/// NIC 選択ポップアップ（擬似プルダウン）。`●` が現在のNIC、`▌` がカーソル。
fn draw_iface_popup(f: &mut Frame, state: &AppState, cursor: usize) {
    let ifaces = &state.interfaces;
    let height = (ifaces.len() as u16).saturating_add(2).min(f.area().height);
    let popup = centered_rect(52, height, f.area());

    let items: Vec<ListItem> = ifaces
        .iter()
        .map(|nic| {
            let cur = if state.current_iface.as_deref() == Some(nic.name.as_str()) {
                "●"
            } else {
                " "
            };
            ListItem::new(format!("{cur} {:<12} {}", nic.name, nic.cidr))
        })
        .collect();

    let list = List::new(items)
        .block(
            Block::default()
                .borders(Borders::ALL)
                .title(" NIC を選択  (↑↓ / Enter 決定 / Esc 取消) "),
        )
        .highlight_style(Style::default().fg(Color::Black).bg(Color::Cyan))
        .highlight_symbol("▌");

    let mut ls = ListState::default();
    ls.select(Some(cursor.min(ifaces.len().saturating_sub(1))));

    f.render_widget(Clear, popup); // 背景を消してから重ねる
    f.render_stateful_widget(list, popup, &mut ls);
}

fn draw_header(f: &mut Frame, area: ratatui::layout::Rect, state: &AppState) {
    let subnet = state.subnet.clone().unwrap_or_else(|| "(未検出)".into());
    let scanning = if state.scanning {
        " [スキャン中…]"
    } else {
        ""
    };
    let nic = state
        .current_iface
        .clone()
        .map(|n| format!("  nic: {n}"))
        .unwrap_or_default();
    let header = Paragraph::new(Line::from(vec![
        Span::styled(
            "prowl",
            Style::default()
                .fg(Color::Cyan)
                .add_modifier(Modifier::BOLD),
        ),
        Span::raw("  subnet: "),
        Span::styled(subnet, Style::default().fg(Color::Yellow)),
        Span::styled(nic, Style::default().fg(Color::Magenta)),
        Span::styled(scanning, Style::default().fg(Color::Green)),
    ]))
    .block(Block::default().borders(Borders::ALL));
    f.render_widget(header, area);
}

fn draw_host_table(
    f: &mut Frame,
    area: ratatui::layout::Rect,
    visible: &[&HostRow],
    selected: usize,
) {
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
            lines.push(kv(
                "Host ",
                h.hostname.clone().unwrap_or_else(|| "-".into()),
            ));
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

    let detail =
        Paragraph::new(lines).block(Block::default().borders(Borders::ALL).title(" detail "));
    f.render_widget(detail, area);
}

fn draw_footer(
    f: &mut Frame,
    area: ratatui::layout::Rect,
    state: &AppState,
    filter_mode: bool,
    copy_mode: bool,
    copy_status: Option<&str>,
) {
    let line = if copy_mode {
        Line::from(vec![
            Span::styled("copy> ", Style::default().fg(Color::Cyan)),
            Span::styled(
                "[i]IP [m]MAC [v]Vendor [h]Host",
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled("  (Esc で取消)", Style::default().fg(Color::DarkGray)),
        ])
    } else if filter_mode {
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
        // コピー結果があればステータスの代わりに優先表示する。
        let status = match copy_status {
            Some(s) => Span::styled(s.to_string(), Style::default().fg(Color::Cyan)),
            None => Span::raw(state.status.clone()),
        };
        Line::from(vec![
            mon,
            Span::raw("  "),
            status,
            Span::styled(
                "   [↑↓]選択 [Enter]ポート [y]コピー [m]監視 [r]再 [/]絞込 [i]NIC [q]終了",
                Style::default().fg(Color::DarkGray),
            ),
        ])
    };
    f.render_widget(
        Paragraph::new(line).block(Block::default().borders(Borders::ALL)),
        area,
    );
}

/// 選択値をクリップボードへコピーする（依存ゼロ＝OS標準ツールにパイプ）。
/// macOS: `pbcopy` / Linux: `wl-copy` → `xclip` → `xsel` の順に試す。
fn copy_to_clipboard(text: &str) -> anyhow::Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    #[cfg(target_os = "macos")]
    let candidates: &[(&str, &[&str])] = &[("pbcopy", &[])];
    #[cfg(not(target_os = "macos"))]
    let candidates: &[(&str, &[&str])] = &[
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["-ib"]),
    ];

    let mut last_err = String::from("クリップボードツールが見つかりません");
    for (prog, args) in candidates {
        match Command::new(prog)
            .args(*args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            Ok(mut child) => {
                if let Some(mut stdin) = child.stdin.take() {
                    let _ = stdin.write_all(text.as_bytes());
                    // stdin をここで閉じる（drop）とツールが処理を完了できる。
                }
                match child.wait() {
                    Ok(s) if s.success() => return Ok(()),
                    Ok(s) => last_err = format!("{prog} が異常終了 ({s})"),
                    Err(e) => last_err = format!("{prog}: {e}"),
                }
            }
            Err(_) => continue, // 次の候補へ
        }
    }
    anyhow::bail!("{last_err}")
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
