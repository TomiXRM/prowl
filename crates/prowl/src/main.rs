//! prowl — 起動口。
//!
//! ランタイムを立て、エンジンを背後で起動し、選んだフロントを走らせるだけ。
//! ここだけが「具体的な発見器・付与器・フロント」を知る配線ポイント。
//!
//! フラグ:
//! - `--web [--port N]` : Web(DOM)フロント（無印は TUI）
//! - `--gpui`           : GPUI ネイティブフロント（要 `--features gpui` ＝ Xcode/Metal）
//! - `--mock`           : 実ネットワークに触れない決定論モード（e2e/デモ用）
//!
//! GPUI はメインスレッドを占有するため、`#[tokio::main]` ではなく手動ランタイムにし、
//! エンジンは背後のワーカーで動かす。

use std::sync::Arc;

use std::io::{IsTerminal, Write};

use anyhow::{anyhow, Context, Result};
use prowl_app::Frontend;
use prowl_core::discovery::mock::MockDiscovery;
use prowl_core::discovery::{ping_neigh::PingNeighborDiscovery, Discovery};
use prowl_core::enrich::{
    mdns::MdnsEnricher, netbios::NetBiosEnricher, oui::OuiEnricher, system_dns::SystemDnsEnricher,
    Enricher,
};
use prowl_core::scan::{mock::MockScanner, ConnectScanner, PortScanner};
use prowl_core::{net, Engine, Subnet};
use prowl_tui::TuiFrontend;
use prowl_web::WebFrontend;

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().collect();

    // --- セルフアップデート（フロント起動より前に処理して終了）---
    // `ureq` は同期なので tokio ランタイム不要。
    if args.iter().any(|a| a == "--check-update") {
        return run_update_cli(true);
    }
    if args.iter().any(|a| a == "--update") {
        return run_update_cli(false);
    }

    let use_web = args.iter().any(|a| a == "--web");
    let use_mock = args.iter().any(|a| a == "--mock");
    // .app/.desktop からの起動(端末なし)では GPUI を既定にする（gpui feature 有効時のみ）。
    let use_gpui = args.iter().any(|a| a == "--gpui")
        || (cfg!(feature = "gpui") && !use_web && !std::io::stdout().is_terminal());
    let port: u16 = args
        .iter()
        .position(|a| a == "--port")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.parse().ok())
        .unwrap_or(7878);

    // GPUI がメインスレッドを占有しても背後でエンジンが動くよう、tokio は手動構築。
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    // --- 内側の軸を組み立てる ---
    // 通常は無権限発見(blink方式)＋名前解決チェーン＋connectスキャン。
    // --mock は実NWに触れず固定データを返す（決定論的＝e2eテスト/デモ向き）。
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
    // engine.spawn() は内部で tokio::spawn するため、ランタイムコンテキストで呼ぶ。
    let handle = {
        let _guard = rt.enter();
        engine.spawn()
    };

    // --- 外側の軸: フロントを選んで走らせる（方針A）---
    if use_gpui {
        #[cfg(feature = "gpui")]
        {
            // GPUI がメインスレッドをブロック。エンジンは rt のワーカーで動き続ける。
            prowl_gpui::run(handle);
            return Ok(());
        }
        #[cfg(not(feature = "gpui"))]
        {
            let _ = handle;
            anyhow::bail!("--gpui には `--features gpui` でのビルドが必要です（要 Xcode/Metal）");
        }
    }

    // TUI / Web は async。手動ランタイムで走らせる。
    let frontend: Box<dyn Frontend> = if use_web {
        Box::new(WebFrontend::new(port))
    } else {
        Box::new(TuiFrontend)
    };
    rt.block_on(frontend.run(handle))
}

/// `--check-update` / `--update` の CLI フロー。
///
/// GitHub の最新リリースを確認し、`check_only` なら案内だけ、そうでなければ確認の上で
/// DL → SHA-256 検証 → バイナリ/`.app` をアトミック差し替え → 再起動する。
fn run_update_cli(check_only: bool) -> Result<()> {
    let current = prowl_update::current_version();
    let found = prowl_update::check_for_update(None).map_err(|e| anyhow!(e))?;

    let (plan, release) = match found {
        Some(pr) => pr,
        None => {
            println!("✓ prowl {current} は最新です。");
            return Ok(());
        }
    };

    let size_mb = plan.asset.size as f64 / 1_048_576.0;
    println!(
        "⬆ 新バージョンがあります: {} → {}",
        plan.current, plan.latest
    );
    println!("   asset : {} ({size_mb:.1} MB)", plan.asset.name);
    if !plan.notes.trim().is_empty() {
        println!("   notes :");
        for line in plan.notes.lines().take(8) {
            println!("     {line}");
        }
    }

    if check_only {
        println!("\n`prowl --update` で更新できます。");
        return Ok(());
    }

    // 対話端末でなければ自動承認しない（CI/パイプ誤実行の暴発防止）。
    if !std::io::stdin().is_terminal() {
        return Err(anyhow!(
            "端末ではないため自動更新を中止しました（対話端末で `prowl --update` を実行してください）"
        ));
    }
    print!("\n更新しますか？ [y/N] ");
    std::io::stdout().flush().ok();
    let mut ans = String::new();
    std::io::stdin().read_line(&mut ans)?;
    if !matches!(ans.trim(), "y" | "Y" | "yes") {
        println!("中止しました。現在のバージョンは変更していません。");
        return Ok(());
    }

    let relaunch = prowl_update::install(&plan, &release, &|m| println!("  {m}"))
        .map_err(|e| anyhow!("更新に失敗しました（現在のバージョンは温存）: {e}"))?;
    println!("✓ 更新完了。再起動します…");
    relaunch.spawn_and_exit();
}
