//! ARP 発見器（FR-02）。要 root/sudo（NFR-06）。
//!
//! 対象サブネットの各アドレスへ ARP リクエストをブロードキャストし、
//! リプライから IP/MAC を収集する。pnet のデータリンクI/Oはブロッキングなので、
//! `spawn_blocking` 上で実行して UI を止めない（NFR-03）。

use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use pnet::datalink::{self, Channel, Config};
use pnet::packet::arp::{ArpHardwareTypes, ArpOperations, ArpPacket, MutableArpPacket};
use pnet::packet::ethernet::{EtherTypes, EthernetPacket, MutableEthernetPacket};
use pnet::packet::Packet;
use pnet::util::MacAddr as PnetMac;

use super::Discovery;
use crate::model::{Host, MacAddr, Subnet};
use crate::net::LocalNet;

/// 1スキャンで投げる最大ターゲット数（巨大プレフィックス対策）。
const MAX_TARGETS: usize = 4096;
/// ARP リプライを収集する待ち時間。
const RECV_WINDOW: Duration = Duration::from_millis(1500);

pub struct ArpDiscovery {
    local: LocalNet,
}

impl ArpDiscovery {
    pub fn new(local: LocalNet) -> Self {
        Self { local }
    }
}

#[async_trait::async_trait]
impl Discovery for ArpDiscovery {
    async fn discover(&self, _target: &Subnet) -> Result<Vec<Host>> {
        let local = self.local.clone();
        tokio::task::spawn_blocking(move || arp_scan(&local))
            .await
            .context("ARPスキャンタスクの join に失敗")?
    }
}

fn arp_scan(local: &LocalNet) -> Result<Vec<Host>> {
    let targets = local.targets(MAX_TARGETS);

    let config = Config {
        read_timeout: Some(Duration::from_millis(200)),
        ..Default::default()
    };
    let (mut tx, mut rx) = match datalink::channel(&local.interface, config) {
        Ok(Channel::Ethernet(tx, rx)) => (tx, rx),
        Ok(_) => bail!("未対応のデータリンクチャネル種別です"),
        Err(e) => {
            return Err(anyhow!(
                "データリンクを開けません ({e})。raw socket には root/sudo が必要です (NFR-06)"
            ))
        }
    };

    // 全ターゲットへ ARP リクエストを送信
    for target in &targets {
        let mut eth_buf = [0u8; 42]; // Ethernet(14) + ARP(28)
        let mut eth = MutableEthernetPacket::new(&mut eth_buf).unwrap();
        eth.set_destination(PnetMac(0xff, 0xff, 0xff, 0xff, 0xff, 0xff));
        eth.set_source(local.mac);
        eth.set_ethertype(EtherTypes::Arp);

        let mut arp_buf = [0u8; 28];
        let mut arp = MutableArpPacket::new(&mut arp_buf).unwrap();
        arp.set_hardware_type(ArpHardwareTypes::Ethernet);
        arp.set_protocol_type(EtherTypes::Ipv4);
        arp.set_hw_addr_len(6);
        arp.set_proto_addr_len(4);
        arp.set_operation(ArpOperations::Request);
        arp.set_sender_hw_addr(local.mac);
        arp.set_sender_proto_addr(local.ipv4);
        arp.set_target_hw_addr(PnetMac(0, 0, 0, 0, 0, 0));
        arp.set_target_proto_addr(*target);

        eth.set_payload(arp.packet());
        let _ = tx.send_to(eth.packet(), None);
    }

    // 一定時間 ARP リプライを収集（IP で重複排除）
    let mut found: BTreeMap<Ipv4Addr, PnetMac> = BTreeMap::new();
    let start = Instant::now();
    while start.elapsed() < RECV_WINDOW {
        match rx.next() {
            Ok(frame) => {
                let Some(eth) = EthernetPacket::new(frame) else {
                    continue;
                };
                if eth.get_ethertype() != EtherTypes::Arp {
                    continue;
                }
                let Some(arp) = ArpPacket::new(eth.payload()) else {
                    continue;
                };
                if arp.get_operation() == ArpOperations::Reply {
                    found.insert(arp.get_sender_proto_addr(), arp.get_sender_hw_addr());
                }
            }
            Err(e)
                if e.kind() == std::io::ErrorKind::TimedOut
                    || e.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(e) => return Err(anyhow!("ARP受信エラー: {e}")),
        }
    }

    let hosts = found
        .into_iter()
        .map(|(ip, mac)| {
            let mut h = Host::new(ip);
            h.mac = Some(MacAddr([mac.0, mac.1, mac.2, mac.3, mac.4, mac.5]));
            h
        })
        .collect();
    Ok(hosts)
}
