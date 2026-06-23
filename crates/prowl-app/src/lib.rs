//! prowl-app — UI非依存の契約層 (制約 C-02)
//!
//! フロントエンド(TUI/GPUI/...) と コア(エンジン) は **この層だけ** を共有する。
//! - [`Command`] — ユーザの操作意図（フロント → エンジン）
//! - [`AppState`] — 画面に出す状態スナップショット（エンジン → フロント）
//! - [`Event`] — 一時的な通知（将来: モニタ差分アラート等）
//! - [`EngineHandle`] — 両者を繋ぐハンドル
//! - [`Frontend`] — フロントの差し替え口（方針A: 各フロントが自前ランタイムを持つ）

use std::net::Ipv4Addr;

use tokio::sync::{broadcast, mpsc, watch};

/// ホストの識別子。P1では IPv4 アドレスで一意とする。
pub type HostId = Ipv4Addr;

/// ホストの死活ステータス（継続モニタ用）。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HostStatus {
    /// 生存中。
    #[default]
    Up,
    /// 今回初めて見つかった。
    New,
    /// 以前は居たが応答しなくなった（離脱）。
    Down,
}

/// 一覧に表示する1行（ビューモデル）。
/// コアの rich な `Host` 型ではなく、表示に必要な分だけを持つ＝境界の単純化。
#[derive(Clone, Debug)]
pub struct HostRow {
    pub ip: Ipv4Addr,
    pub mac: Option<String>,
    pub hostname: Option<String>,
    pub vendor: Option<String>,
    pub status: HostStatus,
}

/// 開放ポート1件（表示用）。
#[derive(Clone, Debug)]
pub struct PortInfo {
    pub port: u16,
    pub service: Option<String>,
    pub banner: Option<String>,
}

/// 選択ホストのポートスキャン進捗。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum PortScanState {
    #[default]
    Idle,
    Scanning,
    Done,
}

/// 選択ホストのポートスキャン結果。
#[derive(Clone, Debug, Default)]
pub struct PortScan {
    pub target: Option<HostId>,
    pub state: PortScanState,
    pub open: Vec<PortInfo>,
}

/// 画面に出す「今の全状態」。フロントはこれを描くだけ。
#[derive(Clone, Debug, Default)]
pub struct AppState {
    pub subnet: Option<String>,
    pub hosts: Vec<HostRow>,
    pub selected: Option<HostId>,
    pub scanning: bool,
    pub monitoring: bool,
    pub filter: String,
    pub status: String,
    pub port_scan: PortScan,
}

impl AppState {
    /// `filter` を反映した表示対象の行を返す（ip/mac/vendor/hostname を部分一致）。
    pub fn visible_hosts(&self) -> Vec<&HostRow> {
        if self.filter.is_empty() {
            return self.hosts.iter().collect();
        }
        let f = self.filter.to_lowercase();
        self.hosts
            .iter()
            .filter(|h| {
                h.ip.to_string().contains(&f)
                    || h.mac
                        .as_deref()
                        .is_some_and(|m| m.to_lowercase().contains(&f))
                    || h.vendor
                        .as_deref()
                        .is_some_and(|v| v.to_lowercase().contains(&f))
                    || h.hostname
                        .as_deref()
                        .is_some_and(|n| n.to_lowercase().contains(&f))
            })
            .collect()
    }
}

/// ユーザの操作意図。各フロントは自分の入力をこれに翻訳する。
#[derive(Clone, Debug)]
pub enum Command {
    Rescan,
    SelectHost(HostId),
    ScanPorts(HostId),
    SetFilter(String),
    ToggleMonitor,
    Quit,
}

/// 一時的な通知（将来のモニタ差分アラートなどに使う）。
#[derive(Clone, Debug)]
pub enum Event {
    ScanStarted,
    ScanFinished { found: usize },
    Error(String),
}

/// エンジンとフロントを繋ぐハンドル。
/// フロントは `commands` に操作を投げ、`state` から最新状態を読む。
pub struct EngineHandle {
    pub commands: mpsc::Sender<Command>,
    pub state: watch::Receiver<AppState>,
    pub events: broadcast::Receiver<Event>,
}

/// フロントエンドの差し替え口 (制約 C-02 / 方針A)。
/// 各フロントは [`EngineHandle`] を受け取り、自分のランタイム/ループを回す。
#[async_trait::async_trait]
pub trait Frontend: Send {
    async fn run(self: Box<Self>, engine: EngineHandle) -> anyhow::Result<()>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(ip: [u8; 4], vendor: &str) -> HostRow {
        HostRow {
            ip: Ipv4Addr::new(ip[0], ip[1], ip[2], ip[3]),
            mac: None,
            hostname: None,
            vendor: Some(vendor.to_string()),
            status: HostStatus::Up,
        }
    }

    #[test]
    fn empty_filter_shows_all() {
        let s = AppState {
            hosts: vec![
                row([192, 168, 1, 1], "Apple"),
                row([192, 168, 1, 2], "VMware"),
            ],
            ..Default::default()
        };
        assert_eq!(s.visible_hosts().len(), 2);
    }

    #[test]
    fn filter_matches_vendor_and_ip_case_insensitively() {
        let s = AppState {
            hosts: vec![row([192, 168, 1, 1], "Apple"), row([10, 0, 0, 9], "VMware")],
            filter: "apple".to_string(),
            ..Default::default()
        };
        let v = s.visible_hosts();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].vendor.as_deref(), Some("Apple"));
    }
}
