//! 名前解決(逆引きDNS / mDNS / NetBIOS)の動作確認（FR-03）。root 不要。
//! 実行: `cargo run -p prowl-core --example name_probe [IP...]`
//! 引数なしなら、検出した自分のIPとサブネットの .1 を対象にする。

use std::net::Ipv4Addr;

use prowl_core::enrich::mdns::MdnsEnricher;
use prowl_core::enrich::netbios::NetBiosEnricher;
use prowl_core::enrich::system_dns::SystemDnsEnricher;
use prowl_core::enrich::Enricher;
use prowl_core::model::Host;
use prowl_core::net;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args: Vec<Ipv4Addr> = std::env::args()
        .skip(1)
        .filter_map(|s| s.parse().ok())
        .collect();
    let ips = if args.is_empty() {
        let local = net::detect()?;
        let o = local.network.network().octets();
        vec![local.ipv4, Ipv4Addr::new(o[0], o[1], o[2], 1)]
    } else {
        args
    };

    for ip in ips {
        let mut h = Host::new(ip);
        SystemDnsEnricher.enrich(&mut h).await;
        let rdns = h.hostname.clone();

        let mut h = Host::new(ip);
        MdnsEnricher.enrich(&mut h).await;
        let md = h.hostname.clone();

        let mut h = Host::new(ip);
        NetBiosEnricher.enrich(&mut h).await;
        let nb = h.hostname.clone();

        println!("{ip:<15} reverse={rdns:?}  mdns={md:?}  netbios={nb:?}");
    }
    Ok(())
}
