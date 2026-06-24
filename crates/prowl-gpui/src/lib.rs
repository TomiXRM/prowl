//! prowl-gpui (P4) — GPUI ネイティブフロントエンド。
//!
//! 方針A の3フロント目。TUI/Web と同じ `Command`/`AppState` 契約を、
//! GPUI のネイティブウィンドウに映す（lanscan風の Table ＋ 右の詳細ペイン）。
//! GPUI はメインスレッドを占有するため、`Frontend`(async) ではなく [`run`] を
//! メインスレッドで直接呼ぶ（tokio エンジンは背後ランタイムで動かし `watch` で橋渡し）。
//!
//! crates.io 版 gpui + `runtime_shaders`（実行時シェーダ）採用で、Metal のビルド時
//! コンパイル(フル Xcode)は不要。CommandLineTools だけでビルド/実行できる。

#![allow(dead_code)]

use std::net::Ipv4Addr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::*;
use gpui_component::notification::Notification;
use gpui_component::table::{Column, Table, TableDelegate, TableEvent, TableState};
use gpui_component::{button::*, *};
use prowl_app::{AppState, Command, EngineHandle, HostRow, HostStatus, PortScanState};
use prowl_update::{ReleaseInfo, UpdatePlan};
use tokio::sync::{mpsc, watch};

/// ヘッダの「更新」バナー状態。バックグラウンドスレッド（ネットワーク/インストール）と
/// ビュー（描画）の間で `Arc<Mutex<…>>` 共有し、既存の 150ms タイマで描画へ反映する。
#[derive(Clone)]
enum UpdateUi {
    /// チェック中、または情報なし（バナー非表示）。
    Idle,
    /// 最新（バナー非表示）。
    UpToDate,
    /// 新バージョンあり → 更新ボタンを出す。
    Available {
        plan: UpdatePlan,
        release: ReleaseInfo,
    },
    /// インストール中（進捗テキスト）。
    Installing(String),
    /// 失敗（現バージョンは温存）。
    Failed(String),
}

/// ホスト一覧テーブルのデータ供給（gpui-component の Table デリゲート）。
struct HostTableDelegate {
    hosts: Vec<HostRow>,
    columns: Vec<Column>,
}

impl HostTableDelegate {
    fn new() -> Self {
        Self {
            hosts: Vec::new(),
            columns: vec![
                Column::new("ip", "IP").width(px(120.)),
                Column::new("mac", "MAC").width(px(140.)),
                Column::new("vendor", "Vendor").width(px(150.)),
                Column::new("host", "Hostname").width(px(190.)),
            ],
        }
    }
}

impl TableDelegate for HostTableDelegate {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.hosts.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> &Column {
        &self.columns[col_ix]
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _window: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let Some(h) = self.hosts.get(row_ix) else {
            return div().into_any_element();
        };
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        match col_ix {
            // IP（死活で色分け）— クリックでコピー
            0 => {
                let (color, mark) = match h.status {
                    HostStatus::Up => (fg, " "),
                    HostStatus::New => (theme.green, "+"),
                    HostStatus::Down => (theme.red, "×"),
                };
                copyable_cell(
                    "ip",
                    row_ix,
                    format!("{mark}{}", h.ip),
                    h.ip.to_string(),
                    color,
                )
            }
            // MAC / Vendor — 値があればクリックでコピー
            1 => match &h.mac {
                Some(mac) => copyable_cell("mac", row_ix, mac.clone(), mac.clone(), muted),
                None => div().text_color(muted).child("-").into_any_element(),
            },
            2 => match &h.vendor {
                Some(v) => copyable_cell("vendor", row_ix, v.clone(), v.clone(), fg),
                None => div().text_color(muted).child("-").into_any_element(),
            },
            // Hostname — 値があればクリックでコピー
            3 => match &h.hostname {
                Some(host) => copyable_cell("host", row_ix, host.clone(), host.clone(), fg),
                None => div().text_color(muted).child("-").into_any_element(),
            },
            _ => div().into_any_element(),
        }
    }
}

/// ルートビュー。`AppState` をミラーし、入力を `Command` に翻訳する。
struct ProwlView {
    state: AppState,
    commands: mpsc::Sender<Command>,
    selected: Option<Ipv4Addr>,
    table: Entity<TableState<HostTableDelegate>>,
    /// セルフアップデートの状態（バックグラウンドスレッドと共有）。
    update_ui: Arc<Mutex<UpdateUi>>,
}

impl ProwlView {
    fn new(
        commands: mpsc::Sender<Command>,
        state_rx: watch::Receiver<AppState>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let state = state_rx.borrow().clone();

        // テーブル状態を作り、初期ホストを流し込む
        let table = cx.new(|cx| TableState::new(HostTableDelegate::new(), window, cx));
        table.update(cx, |t, cx| {
            t.delegate_mut().hosts = state.hosts.clone();
            cx.notify();
        });

        // 行の選択 or ダブルクリック → そのホストをポートスキャン。
        // 各セルはクリックでコピーを消費し得るので、ダブルクリックでも確実に走らせる。
        cx.subscribe(&table, |this, _table, event, cx| {
            let ix = match event {
                TableEvent::SelectRow(ix) | TableEvent::DoubleClickedRow(ix) => *ix,
                _ => return,
            };
            if let Some(h) = this.state.hosts.get(ix) {
                let ip = h.ip;
                this.select(ip, cx);
            }
        })
        .detach();

        // watch<AppState> をビューへ橋渡し。tokio の changed().await は gpui executor で
        // cross-executor wake が不確実なので、gpui の timer でポーリングして取り込む。
        cx.spawn(async move |this, cx| loop {
            cx.background_executor()
                .timer(Duration::from_millis(150))
                .await;
            let snapshot = state_rx.borrow().clone();
            if this
                .update(cx, |this, cx| {
                    this.table.update(cx, |t, cx| {
                        t.delegate_mut().hosts = snapshot.hosts.clone();
                        cx.notify();
                    });
                    this.state = snapshot;
                    cx.notify();
                })
                .is_err()
            {
                break;
            }
        })
        .detach();

        // 起動時にバックグラウンドで最新リリースを確認（best-effort・失敗は静か）。
        // ネットワークはブロッキングなので OS スレッドで回し、結果は共有状態へ。
        let update_ui = Arc::new(Mutex::new(UpdateUi::Idle));
        {
            let ui = update_ui.clone();
            std::thread::spawn(move || {
                let next = match prowl_update::check_for_update(None) {
                    Ok(Some((plan, release))) => UpdateUi::Available { plan, release },
                    _ => UpdateUi::UpToDate, // 失敗も静かに「最新」扱い
                };
                if let Ok(mut g) = ui.lock() {
                    *g = next;
                }
            });
        }

        Self {
            state,
            commands,
            selected: None,
            table,
            update_ui,
        }
    }

    fn send(&self, cmd: Command) {
        let _ = self.commands.try_send(cmd);
    }

    /// 「更新」ボタン押下 → 別スレッドで DL→検証→差し替えを実行し、成功で再起動する。
    /// 進捗は共有状態 [`UpdateUi::Installing`] に流れ、150ms タイマで描画へ反映される。
    fn start_install(&self) {
        let mut guard = match self.update_ui.lock() {
            Ok(g) => g,
            Err(_) => return,
        };
        let (plan, release) = match &*guard {
            UpdateUi::Available { plan, release } => (plan.clone(), release.clone()),
            _ => return, // 二重押下や状態変化はノーオペ
        };
        *guard = UpdateUi::Installing("準備中…".into());
        drop(guard);

        let ui = self.update_ui.clone();
        std::thread::spawn(move || {
            let log = |m: &str| {
                if let Ok(mut g) = ui.lock() {
                    *g = UpdateUi::Installing(m.to_string());
                }
            };
            match prowl_update::install(&plan, &release, &log) {
                Ok(relaunch) => relaunch.spawn_and_exit(), // 新プロセスを起動して終了（戻らない）
                Err(e) => {
                    if let Ok(mut g) = ui.lock() {
                        *g = UpdateUi::Failed(e);
                    }
                }
            }
        });
    }

    fn select(&mut self, ip: Ipv4Addr, cx: &mut Context<Self>) {
        self.selected = Some(ip);
        self.send(Command::SelectHost(ip));
        self.send(Command::ScanPorts(ip));
        cx.notify();
    }

    fn detail_lines(&self, fg: Hsla, muted: Hsla, accent_green: Hsla) -> Vec<AnyElement> {
        let Some(ip) = self.selected else {
            return vec![div()
                .text_color(muted)
                .child("← ホストを選んでポートスキャン")
                .into_any_element()];
        };
        let Some(h) = self.state.hosts.iter().find(|h| h.ip == ip) else {
            return vec![div()
                .text_color(muted)
                .child("(選択ホストが消えました)")
                .into_any_element()];
        };

        let mut out = vec![
            div()
                .font_weight(FontWeight::BOLD)
                .text_color(fg)
                .child(format!("{}  [{:?}]", h.ip, h.status))
                .into_any_element(),
            kv(
                "MAC",
                h.mac.clone().unwrap_or_else(|| "-".into()),
                muted,
                fg,
            ),
            kv(
                "Vendor",
                h.vendor.clone().unwrap_or_else(|| "-".into()),
                muted,
                fg,
            ),
            kv(
                "Host",
                h.hostname.clone().unwrap_or_else(|| "-".into()),
                muted,
                fg,
            ),
            div().h_2().into_any_element(),
        ];

        let ps = &self.state.port_scan;
        if ps.target == Some(ip) {
            match ps.state {
                PortScanState::Scanning => out.push(
                    div()
                        .text_color(accent_green)
                        .child("ポートスキャン中…")
                        .into_any_element(),
                ),
                PortScanState::Done => {
                    out.push(
                        div()
                            .font_weight(FontWeight::BOLD)
                            .child(format!("開放ポート: {}", ps.open.len()))
                            .into_any_element(),
                    );
                    for p in &ps.open {
                        let svc = p.service.clone().unwrap_or_default();
                        let ban = p.banner.clone().unwrap_or_default();
                        out.push(
                            div()
                                .text_color(muted)
                                .child(format!("{}/tcp  {svc}  {ban}", p.port))
                                .into_any_element(),
                        );
                    }
                }
                PortScanState::Idle => out.push(
                    div()
                        .text_color(muted)
                        .child("行クリックでポートスキャン")
                        .into_any_element(),
                ),
            }
        } else {
            out.push(
                div()
                    .text_color(muted)
                    .child("行クリックでポートスキャン")
                    .into_any_element(),
            );
        }
        out
    }
}

/// クリックでクリップボードへコピーし成功トーストを出すセル。
fn copyable_cell(
    key: &'static str,
    row_ix: usize,
    display: String,
    copy: String,
    color: Hsla,
) -> AnyElement {
    div()
        .id((key, row_ix))
        .cursor_pointer()
        .text_color(color)
        .child(display)
        .on_click(move |_, window, cx| {
            cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
            window.push_notification(Notification::success(format!("コピー: {copy}")), cx);
        })
        .into_any_element()
}

fn kv(key: &'static str, val: String, muted: Hsla, fg: Hsla) -> AnyElement {
    div()
        .flex()
        .gap_2()
        .child(div().w(px(64.)).text_color(muted).child(key))
        .child(div().text_color(fg).child(val))
        .into_any_element()
}

impl Render for ProwlView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 全体スケール: gpui は rem 単位。既定 16px の 0.7倍 = 11.2px で UI 全体を縮小。
        window.set_rem_size(px(11.2));
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let border = theme.border;
        let accent_green = theme.green;
        let accent_red = theme.red;
        let bg = theme.background;
        let panel = theme.sidebar;

        // --- セルフアップデートのバナー（状態に応じて出し分け）---
        let update_el: Option<AnyElement> = match self.update_ui.lock().ok().map(|g| g.clone()) {
            Some(UpdateUi::Available { plan, .. }) => Some(
                Button::new("update")
                    .small()
                    .label(format!("⬆ v{} に更新", plan.latest))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.start_install();
                        cx.notify();
                    }))
                    .into_any_element(),
            ),
            Some(UpdateUi::Installing(msg)) => Some(
                div()
                    .text_color(accent_green)
                    .child(format!("更新: {msg}"))
                    .into_any_element(),
            ),
            Some(UpdateUi::Failed(_)) => Some(
                div()
                    .text_color(accent_red)
                    .child("更新に失敗（再試行可）")
                    .into_any_element(),
            ),
            _ => None,
        };

        let subnet = self.state.subnet.clone().unwrap_or_else(|| "—".into());
        let monitoring = self.state.monitoring;
        let status = self.state.status.clone();

        // --- ヘッダ ---
        let header = h_flex()
            .gap_2()
            .px_2()
            .py_1()
            .border_b_1()
            .border_color(border)
            .child(div().font_weight(FontWeight::BOLD).child("prowl"))
            .child(div().text_color(muted).child(format!("subnet: {subnet}")))
            .child(
                div()
                    .text_color(if monitoring { accent_green } else { muted })
                    .child(if monitoring {
                        "● 監視中"
                    } else {
                        "‖ 停止"
                    }),
            )
            .child(
                Button::new("rescan")
                    .small()
                    .label("再スキャン")
                    .on_click(cx.listener(|this, _, _, _| this.send(Command::Rescan))),
            )
            .child(
                Button::new("monitor")
                    .small()
                    .label("監視")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.send(Command::ToggleMonitor);
                        cx.notify();
                    })),
            )
            .child(div().flex_1())
            .children(update_el)
            .child(div().text_color(muted).child(status));

        // --- 左: テーブル ---
        let table = div()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(Table::new(&self.table).stripe(true).xsmall());

        // --- 右: 詳細サイドペイン ---
        let detail = v_flex()
            .w(px(260.))
            .h_full()
            .p_2()
            .gap_1()
            .bg(panel)
            .border_l_1()
            .border_color(border)
            .children(self.detail_lines(fg, muted, accent_green));

        let body = v_flex()
            .size_full()
            .bg(bg)
            .text_color(fg)
            .font_family("Menlo")
            .text_xs()
            .child(header)
            .child(h_flex().flex_1().min_h_0().child(table).child(detail));

        // Root::render は通知レイヤを描かないので、アプリ側で重ねる（トースト表示用）。
        div()
            .relative()
            .size_full()
            .child(body)
            .children(Root::render_notification_layer(window, cx))
    }
}

/// ウィンドウを1枚開く（commands/state でエンジンに繋がる）。
fn open_window(cx: &mut App, commands: mpsc::Sender<Command>, state: watch::Receiver<AppState>) {
    let bounds = Bounds::centered(None, size(px(924.), px(420.)), cx);
    let opts = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
        titlebar: Some(TitlebarOptions {
            title: Some("prowl".into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    cx.open_window(opts, move |window, cx| {
        let view = cx.new(|cx| ProwlView::new(commands, state, window, cx));
        cx.new(|cx| Root::new(view, window, cx))
    })
    .expect("failed to open window");
}

/// GPUI アプリを起動する（メインスレッドを占有してブロックする）。
/// エンジンは別の tokio ランタイムで動かし、`handle` 経由で状態をやり取りする。
pub fn run(handle: EngineHandle) {
    let EngineHandle {
        commands, state, ..
    } = handle;

    let app = Application::new();

    // X でウィンドウを閉じてもプロセスは生き続ける（macOS）。Dock アイコンの再クリックで
    // ウィンドウが無ければ開き直す。無いと「Dockに居るのに開かない」状態になる。
    {
        let commands = commands.clone();
        let state = state.clone();
        app.on_reopen(move |cx| {
            if cx.windows().is_empty() {
                open_window(cx, commands.clone(), state.clone());
            }
            cx.activate(true);
        });
    }

    app.run(move |cx: &mut App| {
        gpui_component::init(cx);
        open_window(cx, commands, state);
    });
}
