//! prowl-gpui (P4) — GPUI ネイティブフロントエンド。
//!
//! 方針A の3フロント目。TUI/Web と同じ `Command`/`AppState` 契約を、
//! GPUI のネイティブウィンドウに映す。GPUI はメインスレッドを占有し独自 executor を
//! 持つため、`Frontend`(async) トレイトではなく [`run`] をメインスレッドで直接呼ぶ
//! （tokio エンジンは背後ランタイムで動かし、状態は `watch` 経由で橋渡しする）。
//!
//! ウィンドウ起動部 [`run`] は `desktop` feature (gpui_platform/Metal) 必須で、
//! macOS ではフル Xcode が要る。ビュー層(本ファイルの大半)は gpui コアのみで成立。

// ウィンドウ起動部は feature gate のため、未使用扱いになる項目を許容する。
#![allow(dead_code)]

use std::net::Ipv4Addr;

use gpui::prelude::FluentBuilder as _;
use gpui::*;
use gpui_component::{button::*, *};
use prowl_app::{AppState, Command, HostStatus, PortScanState};
use tokio::sync::{mpsc, watch};

#[cfg(feature = "desktop")]
use prowl_app::EngineHandle;

/// ルートビュー。`AppState` をミラーし、入力を `Command` に翻訳する。
struct ProwlView {
    state: AppState,
    commands: mpsc::Sender<Command>,
    selected: Option<Ipv4Addr>,
}

impl ProwlView {
    fn new(
        commands: mpsc::Sender<Command>,
        mut state_rx: watch::Receiver<AppState>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let state = state_rx.borrow().clone();

        // watch<AppState> をビューへ橋渡し（tokio sync は runtime非依存なので gpui executor で await 可）
        cx.spawn(async move |this, cx| loop {
            if state_rx.changed().await.is_err() {
                break;
            }
            let snapshot = state_rx.borrow().clone();
            if this
                .update(cx, |this, cx| {
                    this.state = snapshot;
                    cx.notify();
                })
                .is_err()
            {
                break;
            }
        })
        .detach();

        Self {
            state,
            commands,
            selected: None,
        }
    }

    fn send(&self, cmd: Command) {
        // try_send は非ブロッキング（gpui executor から呼べる）
        let _ = self.commands.try_send(cmd);
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
                .child("ホストをクリックでポートスキャン")
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
            kv("MAC", h.mac.clone().unwrap_or_else(|| "-".into()), muted, fg),
            kv("Vendor", h.vendor.clone().unwrap_or_else(|| "-".into()), muted, fg),
            kv("Host", h.hostname.clone().unwrap_or_else(|| "-".into()), muted, fg),
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
                                .child(format!("  {}/tcp  {svc}  {ban}", p.port))
                                .into_any_element(),
                        );
                    }
                }
                PortScanState::Idle => out.push(
                    div()
                        .text_color(muted)
                        .child("クリックでポートスキャン")
                        .into_any_element(),
                ),
            }
        } else {
            out.push(
                div()
                    .text_color(muted)
                    .child("クリックでポートスキャン")
                    .into_any_element(),
            );
        }
        out
    }
}

fn status_color(status: HostStatus, theme: &Theme) -> Hsla {
    match status {
        HostStatus::Up => theme.foreground,
        HostStatus::New => theme.green,
        HostStatus::Down => theme.red,
    }
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 必要な色を先取り（Hsla は Copy。後段の cx.listener と借用が衝突しないように）
        let theme = cx.theme();
        let fg = theme.foreground;
        let muted = theme.muted_foreground;
        let border = theme.border;
        let sel_bg = theme.secondary;
        let hover_bg = theme.accent;
        let accent_green = theme.green;
        let bg = theme.background;

        let subnet = self.state.subnet.clone().unwrap_or_else(|| "—".into());
        let monitoring = self.state.monitoring;

        // --- ヘッダ ---
        let header = h_flex()
            .gap_3()
            .px_3()
            .py_2()
            .border_b_1()
            .border_color(border)
            .child(div().font_weight(FontWeight::BOLD).child("prowl"))
            .child(div().text_color(muted).child(format!("subnet: {subnet}")))
            .child(
                div()
                    .text_color(if monitoring { accent_green } else { muted })
                    .child(if monitoring { "監視 ON" } else { "監視 OFF" }),
            )
            .child(
                Button::new("rescan")
                    .label("再スキャン")
                    .on_click(cx.listener(|this, _, _, _| this.send(Command::Rescan))),
            )
            .child(
                Button::new("monitor")
                    .label("監視 ON/OFF")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.send(Command::ToggleMonitor);
                        cx.notify();
                    })),
            );

        // --- ホスト一覧 ---
        let selected = self.selected;
        let rows: Vec<AnyElement> = self
            .state
            .hosts
            .iter()
            .map(|h| {
                let ip = h.ip;
                let is_sel = selected == Some(ip);
                let color = status_color(h.status, cx.theme());
                let mark = match h.status {
                    HostStatus::New => "+",
                    HostStatus::Down => "×",
                    HostStatus::Up => " ",
                };
                div()
                    .id(SharedString::from(ip.to_string()))
                    .flex()
                    .gap_2()
                    .px_3()
                    .py_1()
                    .cursor_pointer()
                    .when(is_sel, |d| d.bg(sel_bg))
                    .hover(|d| d.bg(hover_bg))
                    .on_click(cx.listener(move |this, _, _, cx| this.select(ip, cx)))
                    .child(
                        div()
                            .w(px(150.))
                            .text_color(color)
                            .child(format!("{mark}{ip}")),
                    )
                    .child(
                        div()
                            .w(px(150.))
                            .text_color(muted)
                            .child(h.vendor.clone().unwrap_or_else(|| "-".into())),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_color(fg)
                            .child(h.hostname.clone().unwrap_or_else(|| "-".into())),
                    )
                    .into_any_element()
            })
            .collect();

        let host_list = div()
            .id("hosts")
            .flex()
            .flex_col()
            .w(px(480.))
            .h_full()
            .overflow_y_scroll()
            .border_r_1()
            .border_color(border)
            .children(rows);

        // --- 詳細パネル ---
        let detail = v_flex()
            .flex_1()
            .p_3()
            .gap_1()
            .children(self.detail_lines(fg, muted, accent_green));

        v_flex()
            .size_full()
            .bg(bg)
            .text_color(fg)
            .child(header)
            .child(h_flex().flex_1().min_h_0().child(host_list).child(detail))
    }
}

/// GPUI アプリを起動する（メインスレッドを占有してブロックする）。
/// エンジンは別の tokio ランタイムで動かし、`handle` 経由で状態をやり取りする。
///
/// 要 `desktop` feature（gpui_platform/Metal）。macOS ではフル Xcode が必要。
#[cfg(feature = "desktop")]
pub fn run(handle: EngineHandle) {
    gpui_platform::application().run(move |cx| {
        gpui_component::init(cx);

        let EngineHandle {
            commands, state, ..
        } = handle;

        cx.spawn(async move |cx| {
            cx.open_window(WindowOptions::default(), move |window, cx| {
                let view = cx.new(|cx| ProwlView::new(commands, state, window, cx));
                cx.new(|cx| Root::new(view, window, cx).bg(cx.theme().background))
            })
            .expect("failed to open window");
        })
        .detach();
    });
}
