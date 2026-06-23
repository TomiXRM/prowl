//! 開発用のモック発見器。実スキャン（ARP）実装までの仮置き。

use std::net::Ipv4Addr;
use std::time::Duration;

use super::Discovery;
use crate::model::{Host, MacAddr, Subnet};

/// 数台のダミーホストを少し遅延して返す。TUIの土台確認用。
pub struct MockDiscovery;

#[async_trait::async_trait]
impl Discovery for MockDiscovery {
    async fn discover(&self, _target: &Subnet) -> anyhow::Result<Vec<Host>> {
        // スキャンにかかる時間を模擬（NFR-03: UIが固まらないことの確認用）
        tokio::time::sleep(Duration::from_millis(600)).await;

        let samples: &[(u8, [u8; 6])] = &[
            (1, [0x00, 0x1a, 0x11, 0x22, 0x33, 0x44]),
            (10, [0x3c, 0x22, 0xfb, 0xaa, 0xbb, 0xcc]),
            (23, [0xb8, 0x27, 0xeb, 0x12, 0x34, 0x56]),
            (42, [0xdc, 0xa6, 0x32, 0x99, 0x88, 0x77]),
            (105, [0x00, 0x50, 0x56, 0x0a, 0x0b, 0x0c]),
        ];

        let hosts = samples
            .iter()
            .map(|(last, mac)| {
                let mut h = Host::new(Ipv4Addr::new(192, 168, 1, *last));
                h.mac = Some(MacAddr(*mac));
                h
            })
            .collect();
        Ok(hosts)
    }
}
