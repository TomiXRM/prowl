//! prowl — 起動口。
//!
//! ランタイムを立て、エンジンを背後で起動し、選んだフロントを走らせるだけ。
//! ここだけが「具体的な発見器・付与器・フロント」を知る配線ポイント。
//!
//! フラグ:
//! - `--web [--port N]` : Web(DOM)フロント（無印は TUI）
//! - `--mock`           : 実ネットワークに触れない決定論モード（e2e/デモ用）

use std::sync::Arc;

use anyhow::{Context, Result};
use prowl_app::Frontend;
use prowl_core::discovery::mock::MockDiscovery;
use prowl_core::discovery::{ping_neigh::PingNeighborDiscovery, Discovery};
// use prowl_core::discovery::arp::ArpDiscovery; // sudo版（要root、より確実な場合あり）
use prowl_core::enrich::{
    mdns::MdnsEnricher, netbios::NetBiosEnricher, oui::OuiEnricher, system_dns::SystemDnsEnricher,
    Enricher,
};
use prowl_core::scan::{mock::MockScanner, ConnectScanner, PortScanner};
use prowl_core::{net, Engine, Subnet};
use prowl_tui::TuiFrontend;
use prowl_web::WebFrontend;

#[tokio::main]
async fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let use_web = args.iter().any(|a| a == "--web");
    let use_mock = args.iter().any(|a| a == "--mock");
    let port: u16 = args
        .iter()
        .position(|a| a == "--port")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(7878);

    // --- 内側の軸を組み立てる ---
    // 通常は無権限発見(blink方式)＋名前解決チェーン＋connectスキャン。
    // --mock は実NWに触れず固定データを返す（決定論的＝e2eテスト/デモ向き）。
    // FR-01: 通常モードはローカルNICからサブネットを自動検出（--mock は固定値）。
    let local = if use_mock {
        None
    } else {
        Some(net::detect().context("ネットワークインターフェースの自動検出に失敗")?)
    };
    let subnet = match &local {
        Some(l) => l.subnet(),
        None => Subnet::new("192.168.1.0/24"),
    };
    let discovery: Arc<dyn Discovery> = match local {
        Some(l) => Arc::new(PingNeighborDiscovery::new(l)),
        None => Arc::new(MockDiscovery),
    };
    let scanner: Arc<dyn PortScanner> = if use_mock {
        Arc::new(MockScanner)
    } else {
        Arc::new(ConnectScanner::default())
    };
    let mut enrichers: Vec<Arc<dyn Enricher>> = vec![Arc::new(OuiEnricher::from_bundled()?)];
    if !use_mock {
        enrichers.push(Arc::new(SystemDnsEnricher));
        enrichers.push(Arc::new(MdnsEnricher));
        enrichers.push(Arc::new(NetBiosEnricher));
    }

    let engine = Engine::new(subnet, discovery, enrichers, scanner);
    let handle = engine.spawn();

    // --- 外側の軸: フロントを選んで走らせる（方針A）---
    // `--web [--port N]` で Web(DOM)フロント、無印で TUI（U-04）。
    let frontend: Box<dyn Frontend> = if use_web {
        Box::new(WebFrontend::new(port))
    } else {
        Box::new(TuiFrontend)
    };
    frontend.run(handle).await
}
