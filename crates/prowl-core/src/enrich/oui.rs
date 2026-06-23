//! MAC OUI からベンダー名を引く Enricher（FR-04）。
//!
//! macaddress.io 由来の OUI DB（`mac_oui` に同梱）で MA-L/MA-M/MA-S を引く。
//! DB のパースは構築時に一度だけ行い、`Arc` で共有する。

use std::sync::Arc;

use mac_oui::Oui;

use super::Enricher;
use crate::model::Host;

pub struct OuiEnricher {
    db: Arc<Oui>,
}

impl OuiEnricher {
    /// 同梱の OUI DB をロードして構築する（CSVパースはここで一度だけ）。
    pub fn from_bundled() -> anyhow::Result<Self> {
        let db = Oui::default().map_err(|e| anyhow::anyhow!("OUI DBのロードに失敗: {e}"))?;
        Ok(Self { db: Arc::new(db) })
    }
}

#[async_trait::async_trait]
impl Enricher for OuiEnricher {
    async fn enrich(&self, host: &mut Host) {
        if host.vendor.is_some() {
            return;
        }
        let Some(mac) = host.mac else {
            return;
        };
        if let Ok(Some(entry)) = self.db.lookup_by_mac(&mac.to_string()) {
            if !entry.is_private && !entry.company_name.is_empty() {
                host.vendor = Some(entry.company_name.clone());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::MacAddr;
    use std::net::Ipv4Addr;

    #[tokio::test]
    async fn fills_known_vendor() {
        let oui = OuiEnricher::from_bundled().expect("bundled OUI DB loads");
        let mut h = Host::new(Ipv4Addr::new(192, 168, 0, 1));
        // 00:00:0C は歴史的に Cisco Systems
        h.mac = Some(MacAddr([0x00, 0x00, 0x0c, 0x11, 0x22, 0x33]));
        oui.enrich(&mut h).await;
        assert!(
            h.vendor.as_deref().unwrap_or("").contains("Cisco"),
            "got: {:?}",
            h.vendor
        );
    }

    #[tokio::test]
    async fn unknown_local_mac_stays_none() {
        let oui = OuiEnricher::from_bundled().expect("bundled OUI DB loads");
        let mut h = Host::new(Ipv4Addr::new(192, 168, 0, 2));
        // ローカル管理アドレス（02:..）は登録が無い
        h.mac = Some(MacAddr([0x02, 0x00, 0x00, 0x00, 0x00, 0x01]));
        oui.enrich(&mut h).await;
        assert_eq!(h.vendor, None);
    }
}
