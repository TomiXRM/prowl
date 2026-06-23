//! OSの名前解決(`getnameinfo`)で逆引きする Enricher（FR-03 / blink方式）。
//!
//! macOS では `getnameinfo` が内部で mDNSResponder を経由するため、`.local` 名も拾える。
//! 自前の unicast DNS 直接問い合わせ（旧 hickory 実装）より広く名前が取れるのが利点。
//! 名前解決チェーンの一段目。

use std::net::IpAddr;
use std::time::Duration;

use super::Enricher;
use crate::model::Host;

/// 遅い PTR に引きずられないための頭打ち。
const TIMEOUT: Duration = Duration::from_millis(1500);

pub struct SystemDnsEnricher;

#[async_trait::async_trait]
impl Enricher for SystemDnsEnricher {
    async fn enrich(&self, host: &mut Host) {
        if host.hostname.is_some() {
            return;
        }
        let ip = IpAddr::V4(host.ip);
        // getnameinfo はブロッキング。専用スレッド＋timeout で頭打ち。
        let fut = tokio::task::spawn_blocking(move || dns_lookup::lookup_addr(&ip));
        if let Ok(Ok(Ok(name))) = tokio::time::timeout(TIMEOUT, fut).await {
            // 名前が無いと数値IPがそのまま返る実装があるので除外する
            let name = name.trim_end_matches('.');
            if !name.is_empty() && name != host.ip.to_string() {
                host.hostname = Some(name.to_string());
            }
        }
    }
}
