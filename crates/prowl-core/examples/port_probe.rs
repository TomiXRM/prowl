//! ポートスキャンの動作確認（FR-06/07）。**root不要**。
//! 実行: `cargo run -p prowl-core --example port_probe 192.168.10.1`

use std::net::Ipv4Addr;

use prowl_core::scan::{ConnectScanner, PortScanner, COMMON_PORTS};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let ip: Ipv4Addr = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "192.168.10.1".to_string())
        .parse()?;
    println!("scanning {ip}  ({} ports)…\n", COMMON_PORTS.len());

    let scanner = ConnectScanner::default();
    let open = scanner.scan(ip, COMMON_PORTS).await;

    for p in &open {
        println!(
            "{:>5}/tcp  {:<12} {}",
            p.port,
            p.service.clone().unwrap_or_default(),
            p.banner.clone().unwrap_or_default(),
        );
    }
    println!("\n{} open ports", open.len());
    Ok(())
}
