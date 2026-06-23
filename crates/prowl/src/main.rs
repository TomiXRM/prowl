//! prowl — 起動口。
//!
//! ランタイムを立て、エンジンを背後で起動し、選んだフロントを走らせるだけ。
//! ここだけが「具体的な発見器・付与器・フロント」を知る配線ポイント。

use std::sync::Arc;

use anyhow::{Context, Result};
use prowl_app::Frontend;
use prowl_core::discovery::ping_neigh::PingNeighborDiscovery;
// use prowl_core::discovery::arp::ArpDiscovery; // sudo版（要root、より確実な場合あり）
use prowl_core::enrich::{
    mdns::MdnsEnricher, netbios::NetBiosEnricher, oui::OuiEnricher, system_dns::SystemDnsEnricher,
};
use prowl_core::scan::ConnectScanner;
use prowl_core::{net, Engine};
use prowl_tui::TuiFrontend;

#[tokio::main]
async fn main() -> Result<()> {
    // FR-01: ローカルNICからスキャン対象サブネットを自動検出
    let local = net::detect().context("ネットワークインターフェースの自動検出に失敗")?;
    let subnet = local.subnet();

    // --- 内側の軸: 無権限発見(blink方式) + OUI + 名前解決チェーン ---
    // 発見は sudo不要の PingNeighborDiscovery（要rootの ArpDiscovery に1行で差し替え可）。
    // 名前は best-effort: OSリゾルバ(getnameinfo) → mDNS → NetBIOS の順で最初に取れたものを採用
    // （各 Enricher は hostname が空のときだけ動く＝登録順が優先順位）。
    let engine = Engine::new(
        subnet,
        Arc::new(PingNeighborDiscovery::new(local)),
        vec![
            Arc::new(OuiEnricher::from_bundled()?),
            Arc::new(SystemDnsEnricher),
            Arc::new(MdnsEnricher),
            Arc::new(NetBiosEnricher),
        ],
        Arc::new(ConnectScanner::default()),
    );
    let handle = engine.spawn();

    // --- 外側の軸: フロントを選んで走らせる（方針A）---
    // TODO(U-04): --ui で TuiFrontend / GpuiFrontend を切り替え
    let frontend: Box<dyn Frontend> = Box::new(TuiFrontend);
    frontend.run(handle).await
}
