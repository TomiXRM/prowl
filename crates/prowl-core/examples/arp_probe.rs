//! 実ARPスキャンの動作確認（FR-02）。**要 root/sudo**。
//! 実行例:
//!   cargo build -p prowl-core --example arp_probe
//!   sudo ./target/debug/examples/arp_probe

use prowl_core::discovery::arp::ArpDiscovery;
use prowl_core::discovery::Discovery;
use prowl_core::enrich::oui::OuiEnricher;
use prowl_core::enrich::Enricher;
use prowl_core::net;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let local = net::detect()?;
    let subnet = local.subnet();
    println!("scanning {} ...", subnet.cidr);

    let arp = ArpDiscovery::new(local);
    let mut hosts = arp.discover(&subnet).await?;
    hosts.sort_by_key(|h| h.ip);

    let oui = OuiEnricher::from_bundled()?;
    for h in &mut hosts {
        oui.enrich(h).await;
        println!(
            "{:<15} {:<18} {}",
            h.ip,
            h.mac.map(|m| m.to_string()).unwrap_or_default(),
            h.vendor.clone().unwrap_or_default(),
        );
    }
    println!("found {} hosts", hosts.len());
    Ok(())
}
