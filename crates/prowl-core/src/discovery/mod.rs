//! 発見手法の差し替え口（内側の軸）。
//! ARP（P1）も将来の IPv6 NDP もこのトレイトを実装すれば差し込める（既存コード無改変）。

use crate::model::{Host, Subnet};

pub mod arp;
pub mod mock;
pub mod ping_neigh;

#[async_trait::async_trait]
pub trait Discovery: Send + Sync {
    /// 対象サブネットから生存ホストを発見して返す。
    async fn discover(&self, target: &Subnet) -> anyhow::Result<Vec<Host>>;
}
