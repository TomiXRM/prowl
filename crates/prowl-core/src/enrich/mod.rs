//! ホストに情報を後付けする差し替え口（内側の軸）。
//! ベンダー/名前/将来のOS推定はこのトレイトを実装して足す。

use crate::model::Host;

pub mod mdns;
pub mod netbios;
pub mod oui;
pub mod system_dns;

#[async_trait::async_trait]
pub trait Enricher: Send + Sync {
    /// ホストに情報を付与する（冪等であること: 既に埋まっている項目は壊さない）。
    async fn enrich(&self, host: &mut Host);
}
