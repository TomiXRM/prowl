//! prowl-core — UI非依存のスキャンエンジン (制約 C-01)
//!
//! 拡張軸（内側）: [`discovery::Discovery`] / [`enrich::Enricher`] / (将来) `PortScanner`。
//! IPv6 や OS推定はこれらを実装して足す（既存コードは無改変で差し込める）。

pub mod discovery;
pub mod engine;
pub mod enrich;
pub mod model;
pub mod net;
pub mod scan;

pub use engine::{DiscoveryFactory, Engine};
pub use model::{Host, MacAddr, Subnet};
