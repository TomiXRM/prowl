//! エンジン本体。Discovery + Enricher を束ね、`AppState` を更新する。
//! フロントとは `Command`(操作)↓ / `AppState`(状態)↑ の契約だけでやり取りする。

use std::collections::{BTreeMap, HashSet};
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Duration;

use prowl_app::{
    AppState, Command, EngineHandle, Event, HostId, HostRow, HostStatus, PortInfo, PortScanState,
};
use tokio::sync::{broadcast, mpsc, watch};

use crate::discovery::Discovery;
use crate::enrich::Enricher;
use crate::model::{Host, Subnet};
use crate::scan::{PortScanner, COMMON_PORTS};

/// 継続モニタの再スキャン間隔。
const MONITOR_INTERVAL: Duration = Duration::from_secs(10);
/// このミス回数で Down 確定（チラつき防止）。
const DOWN_THRESHOLD: u8 = 2;

/// 既知ホスト1件の追跡状態。
struct Tracked {
    host: Host,
    status: HostStatus,
    misses: u8,
}

pub struct Engine {
    subnet: Subnet,
    discovery: Arc<dyn Discovery>,
    enrichers: Vec<Arc<dyn Enricher>>,
    scanner: Arc<dyn PortScanner>,
}

impl Engine {
    pub fn new(
        subnet: Subnet,
        discovery: Arc<dyn Discovery>,
        enrichers: Vec<Arc<dyn Enricher>>,
        scanner: Arc<dyn PortScanner>,
    ) -> Self {
        Self {
            subnet,
            discovery,
            enrichers,
            scanner,
        }
    }

    /// エンジンを背後タスクとして起動し、フロント用ハンドルを返す。
    pub fn spawn(self) -> EngineHandle {
        let (cmd_tx, cmd_rx) = mpsc::channel::<Command>(64);
        let (state_tx, state_rx) = watch::channel(AppState {
            subnet: Some(self.subnet.cidr.clone()),
            monitoring: true,
            status: "起動中…".to_string(),
            ..Default::default()
        });
        let (evt_tx, evt_rx) = broadcast::channel::<Event>(64);

        tokio::spawn(self.run(cmd_rx, state_tx, evt_tx));

        EngineHandle {
            commands: cmd_tx,
            state: state_rx,
            events: evt_rx,
        }
    }

    async fn run(
        self,
        mut cmd_rx: mpsc::Receiver<Command>,
        state_tx: watch::Sender<AppState>,
        evt_tx: broadcast::Sender<Event>,
    ) {
        // 既知ホストをスキャン横断で保持し、死活ステータスを追跡する（FR-10）。
        let mut known: BTreeMap<Ipv4Addr, Tracked> = BTreeMap::new();
        let mut monitoring = true;

        // 起動直後に一度スキャン
        self.scan_round(&mut known, &state_tx, &evt_tx).await;

        // 定期再スキャン用タイマー（FR-09）。最初の即時 tick は捨てる。
        let mut ticker = tokio::time::interval(MONITOR_INTERVAL);
        ticker.tick().await;

        loop {
            tokio::select! {
                cmd = cmd_rx.recv() => {
                    let Some(cmd) = cmd else { break };
                    match cmd {
                        Command::Rescan => self.scan_round(&mut known, &state_tx, &evt_tx).await,
                        Command::SetFilter(f) => state_tx.send_modify(|s| s.filter = f),
                        Command::SelectHost(id) => state_tx.send_modify(|s| s.selected = Some(id)),
                        Command::ScanPorts(id) => {
                            // ポートスキャンは数秒かかるので別タスクへ（コマンドループを塞がない）
                            let scanner = self.scanner.clone();
                            let state_tx = state_tx.clone();
                            tokio::spawn(scan_ports_task(id, scanner, state_tx));
                        }
                        Command::ToggleMonitor => {
                            monitoring = !monitoring;
                            state_tx.send_modify(|s| s.monitoring = monitoring);
                        }
                        Command::Quit => break,
                    }
                }
                _ = ticker.tick() => {
                    if monitoring {
                        self.scan_round(&mut known, &state_tx, &evt_tx).await;
                    }
                }
            }
        }
    }

    /// 1巡のスキャン: 発見 → 死活ステータス更新 → 新規のみ付与 → 反映。
    async fn scan_round(
        &self,
        known: &mut BTreeMap<Ipv4Addr, Tracked>,
        state_tx: &watch::Sender<AppState>,
        evt_tx: &broadcast::Sender<Event>,
    ) {
        state_tx.send_modify(|s| {
            s.scanning = true;
            s.status = format!("{} をスキャン中…", s.subnet.clone().unwrap_or_default());
        });
        let _ = evt_tx.send(Event::ScanStarted);

        let discovered = match self.discovery.discover(&self.subnet).await {
            Ok(hosts) => hosts,
            Err(err) => {
                state_tx.send_modify(|s| {
                    s.scanning = false;
                    s.status = format!("スキャン失敗: {err}");
                });
                let _ = evt_tx.send(Event::Error(err.to_string()));
                return;
            }
        };
        // 死活ステータスを更新し、新規ホストの IP を得る
        let fresh = update_statuses(known, discovered);

        // 第一報（発見直後に反映）
        publish(known, state_tx, false);

        // 3) 新規ホストだけ付与（既知は前回の名前/ベンダーを維持＝モニタを軽く保つ）
        if !fresh.is_empty() {
            let to_enrich: Vec<Host> = fresh
                .iter()
                .filter_map(|ip| known.get(ip).map(|t| t.host.clone()))
                .collect();
            for h in self.enrich_all(to_enrich).await {
                if let Some(t) = known.get_mut(&h.ip) {
                    t.host.hostname = h.hostname;
                    t.host.vendor = h.vendor;
                }
            }
        }

        // 確定報
        publish(known, state_tx, true);
        let up = known.values().filter(|t| t.status != HostStatus::Down).count();
        let _ = evt_tx.send(Event::ScanFinished { found: up });
    }

    /// 各ホストを並列に付与する（各ホスト内では Enricher を順番に適用）。
    async fn enrich_all(&self, hosts: Vec<Host>) -> Vec<Host> {
        let enrichers = self.enrichers.clone();
        let futs = hosts.into_iter().map(|mut h| {
            let enrichers = enrichers.clone();
            async move {
                for e in &enrichers {
                    e.enrich(&mut h).await;
                }
                h
            }
        });
        futures_util::future::join_all(futs).await
    }
}

/// 1ホストのポートスキャンを実行し、結果を `AppState` に反映する（別タスク）。
async fn scan_ports_task(
    id: HostId,
    scanner: Arc<dyn PortScanner>,
    state_tx: watch::Sender<AppState>,
) {
    state_tx.send_modify(|s| {
        s.port_scan.target = Some(id);
        s.port_scan.state = PortScanState::Scanning;
        s.port_scan.open.clear();
    });

    let open = scanner.scan(id, COMMON_PORTS).await;
    let infos: Vec<PortInfo> = open
        .into_iter()
        .map(|p| PortInfo {
            port: p.port,
            service: p.service,
            banner: p.banner,
        })
        .collect();

    state_tx.send_modify(|s| {
        // 走っている間に別ホストへ切り替わっていたら上書きしない
        if s.port_scan.target == Some(id) {
            s.port_scan.open = infos;
            s.port_scan.state = PortScanState::Done;
        }
    });
}

/// 発見結果から既知ホストの死活ステータスを更新し、新規ホストの IP 一覧を返す。
/// - 今回見つからない既知ホスト: ミス加算、[`DOWN_THRESHOLD`] 到達で Down
/// - 見つかった既知ホスト: Up に復帰・ミスリセット
/// - 未知ホスト: New として登録
fn update_statuses(known: &mut BTreeMap<Ipv4Addr, Tracked>, discovered: Vec<Host>) -> Vec<Ipv4Addr> {
    let current: HashSet<Ipv4Addr> = discovered.iter().map(|h| h.ip).collect();

    for (ip, t) in known.iter_mut() {
        if !current.contains(ip) {
            t.misses = t.misses.saturating_add(1);
            if t.misses >= DOWN_THRESHOLD {
                t.status = HostStatus::Down;
            }
        }
    }

    let mut fresh = Vec::new();
    for h in discovered {
        match known.get_mut(&h.ip) {
            Some(t) => {
                if let Some(mac) = h.mac {
                    t.host.mac = Some(mac);
                }
                t.status = HostStatus::Up;
                t.misses = 0;
            }
            None => {
                let ip = h.ip;
                known.insert(
                    ip,
                    Tracked {
                        host: h,
                        status: HostStatus::New,
                        misses: 0,
                    },
                );
                fresh.push(ip);
            }
        }
    }
    fresh
}

/// 既知ホスト一覧を `AppState` に反映する（IP昇順＝BTreeMapの順）。
fn publish(
    known: &BTreeMap<Ipv4Addr, Tracked>,
    state_tx: &watch::Sender<AppState>,
    done: bool,
) {
    let rows: Vec<HostRow> = known.values().map(|t| to_row(&t.host, t.status)).collect();
    let down = known.values().filter(|t| t.status == HostStatus::Down).count();
    let up = known.len() - down;
    state_tx.send_modify(|s| {
        s.hosts = rows;
        if done {
            s.scanning = false;
            s.status = if down > 0 {
                format!("{up} 台稼働 / {down} 台離脱")
            } else {
                format!("{up} 台稼働")
            };
        } else {
            s.status = format!("{up} 台発見、名前/ベンダー取得中…");
        }
    });
}

/// rich な `Host` を 表示用 `HostRow` に落とす（UI境界の変換点）。
fn to_row(h: &Host, status: HostStatus) -> HostRow {
    HostRow {
        ip: h.ip,
        mac: h.mac.map(|m| m.to_string()),
        hostname: h.hostname.clone(),
        vendor: h.vendor.clone(),
        status,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(last: u8) -> Host {
        Host::new(Ipv4Addr::new(192, 168, 0, last))
    }
    fn ip(last: u8) -> Ipv4Addr {
        Ipv4Addr::new(192, 168, 0, last)
    }

    #[test]
    fn lifecycle_new_up_down_recover() {
        let mut known: BTreeMap<Ipv4Addr, Tracked> = BTreeMap::new();

        // round1: A,B 発見 → どちらも New
        let fresh = update_statuses(&mut known, vec![host(1), host(2)]);
        assert_eq!(fresh.len(), 2);
        assert_eq!(known[&ip(1)].status, HostStatus::New);

        // round2: A,B 再発見 → Up（New→Up）、新規なし
        let fresh = update_statuses(&mut known, vec![host(1), host(2)]);
        assert!(fresh.is_empty());
        assert_eq!(known[&ip(2)].status, HostStatus::Up);

        // round3: B 消失（1ミス）→ まだ Down ではない
        update_statuses(&mut known, vec![host(1)]);
        assert_eq!(known[&ip(2)].status, HostStatus::Up);
        assert_eq!(known[&ip(2)].misses, 1);

        // round4: B 消失（2ミス）→ Down 確定
        update_statuses(&mut known, vec![host(1)]);
        assert_eq!(known[&ip(2)].status, HostStatus::Down);

        // round5: B 復活 → Up、ミスリセット
        update_statuses(&mut known, vec![host(1), host(2)]);
        assert_eq!(known[&ip(2)].status, HostStatus::Up);
        assert_eq!(known[&ip(2)].misses, 0);
    }
}
