//! 無権限ホスト発見（blink方式・FR-02 の sudo不要版）。
//!
//! 各ターゲットへ UDP を撃ってカーネルに同一サブネット宛の ARP を解決させ、
//! その後 OS の近隣テーブル(`arp`/`ip neigh`)から IP/MAC を回収する。
//! raw socket を使わないので **root/sudo 不要**。

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use tokio::net::UdpSocket;

use super::Discovery;
use crate::model::{Host, MacAddr, Subnet};
use crate::net::LocalNet;

const MAX_TARGETS: usize = 4096;
/// ARP 解決を誘発するための送信先ポート（discard）。応答は不要。
const PROBE_PORT: u16 = 9;
/// 送信フェーズ全体の頭打ち（cold ARP で一部の send が詰まっても先へ進む）。
const SEND_BUDGET: Duration = Duration::from_secs(3);
/// 送信後に ARP 解決が落ち着くまでの待ち。
const SETTLE: Duration = Duration::from_millis(900);

pub struct PingNeighborDiscovery {
    local: LocalNet,
}

impl PingNeighborDiscovery {
    pub fn new(local: LocalNet) -> Self {
        Self { local }
    }
}

#[async_trait::async_trait]
impl Discovery for PingNeighborDiscovery {
    async fn discover(&self, _target: &Subnet) -> Result<Vec<Host>> {
        let targets = self.local.targets(MAX_TARGETS);

        // 1) 各ターゲットへ UDP を一斉に撃つ → カーネルが宛先MACを ARP 解決する。
        //    直列 await だと cold ARP 時に未解決ホスト分の遅延が積み上がるので、
        //    並列に投げて全体を SEND_BUDGET で頭打ちにする。
        let sock = Arc::new(
            UdpSocket::bind("0.0.0.0:0")
                .await
                .context("UDPソケットのbindに失敗")?,
        );
        let sends = targets.iter().map(|ip| {
            let sock = sock.clone();
            let ip = *ip;
            // 到達不能などの送信失敗は無視（狙いは ARP 解決のトリガー）
            async move {
                let _ = sock.send_to(&[0u8], (ip, PROBE_PORT)).await;
            }
        });
        let _ = tokio::time::timeout(SEND_BUDGET, futures_util::future::join_all(sends)).await;

        // 2) ARP 解決が落ち着くまで待つ
        tokio::time::sleep(SETTLE).await;

        // 3) OS の近隣テーブルを読む（arp/ip を叩くのでブロッキング）
        let table = tokio::task::spawn_blocking(netneighbours::get_table)
            .await
            .context("近隣テーブル取得タスクの join に失敗")?;

        // 4) 自サブネットの IPv4 だけ拾って Host 化
        //    （自分自身・ブロードキャストIP・ゼロ/マルチキャスト/ブロードキャストMACは除外）
        let net = self.local.network;
        let bcast = net.broadcast();
        let mut found: BTreeMap<Ipv4Addr, MacAddr> = BTreeMap::new();
        for (ip, mac) in table {
            let IpAddr::V4(v4) = ip else { continue };
            if v4 == self.local.ipv4 || v4 == bcast || !net.contains(v4) {
                continue;
            }
            if let Some(arr) = usable_mac(mac.as_bytes()) {
                found.insert(v4, MacAddr(arr));
            }
        }

        let hosts = found
            .into_iter()
            .map(|(ip, mac)| {
                let mut h = Host::new(ip);
                h.mac = Some(mac);
                h
            })
            .collect();
        Ok(hosts)
    }
}

/// 実ホストとして採用できる MAC のみ受理する。
/// ゼロ・ブロードキャスト(ff:ff:..)・マルチキャスト(最下位ビット=1)は除外。
fn usable_mac(b: &[u8]) -> Option<[u8; 6]> {
    if b.len() < 6 {
        return None;
    }
    let arr = [b[0], b[1], b[2], b[3], b[4], b[5]];
    if arr == [0u8; 6] || arr[0] & 0x01 != 0 {
        return None;
    }
    Some(arr)
}

#[cfg(test)]
mod tests {
    use super::usable_mac;

    #[test]
    fn rejects_zero_broadcast_multicast() {
        assert_eq!(usable_mac(&[0, 0, 0, 0, 0, 0]), None); // ゼロ
        assert_eq!(usable_mac(&[0xff; 6]), None); // ブロードキャスト
        assert_eq!(usable_mac(&[0x01, 0, 0x5e, 0, 0, 1]), None); // マルチキャスト
        assert_eq!(usable_mac(&[0xc0, 0x25]), None); // 短すぎ
    }

    #[test]
    fn accepts_unicast() {
        assert_eq!(
            usable_mac(&[0x88, 0x1f, 0xa1, 0x3c, 0xbb, 0x72]),
            Some([0x88, 0x1f, 0xa1, 0x3c, 0xbb, 0x72])
        );
    }
}
