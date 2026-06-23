//! 無権限フルスキャンの動作確認（FR-01〜03、blink方式）。**root不要**。
//! 実行: `cargo run -p prowl-core --example scan_once`

use prowl_core::discovery::ping_neigh::PingNeighborDiscovery;
use prowl_core::discovery::Discovery;
use prowl_core::enrich::{
    mdns::MdnsEnricher, netbios::NetBiosEnricher, oui::OuiEnricher, system_dns::SystemDnsEnricher,
    Enricher,
};
use prowl_core::model::Subnet;
use prowl_core::net;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let local = net::detect()?;
    println!("subnet: {}\n", local.subnet().cidr);

    let disc = PingNeighborDiscovery::new(local);
    let mut hosts = disc.discover(&Subnet::new("")).await?;
    hosts.sort_by_key(|h| h.ip);

    let enrichers: Vec<Box<dyn Enricher>> = vec![
        Box::new(OuiEnricher::from_bundled()?),
        Box::new(SystemDnsEnricher),
        Box::new(MdnsEnricher),
        Box::new(NetBiosEnricher),
    ];
    for h in &mut hosts {
        for e in &enrichers {
            e.enrich(h).await;
        }
    }

    println!("{:<15} {:<18} {:<14} HOSTNAME", "IP", "MAC", "VENDOR");
    for h in &hosts {
        println!(
            "{:<15} {:<18} {:<14} {}",
            h.ip.to_string(),
            h.mac.map(|m| m.to_string()).unwrap_or_default(),
            h.vendor.clone().unwrap_or_default(),
            h.hostname.clone().unwrap_or_default(),
        );
    }
    println!("\nfound {} hosts", hosts.len());
    Ok(())
}
