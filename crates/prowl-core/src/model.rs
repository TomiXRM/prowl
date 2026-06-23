//! コアのドメイン型。通信手段に依存しない。

use std::net::Ipv4Addr;

/// MACアドレス。
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct MacAddr(pub [u8; 6]);

impl MacAddr {
    /// ベンダー判定に使う OUI（先頭3バイト）。
    pub fn oui(&self) -> [u8; 3] {
        [self.0[0], self.0[1], self.0[2]]
    }
}

impl std::fmt::Display for MacAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let b = self.0;
        write!(
            f,
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            b[0], b[1], b[2], b[3], b[4], b[5]
        )
    }
}

impl std::fmt::Debug for MacAddr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self}")
    }
}

/// 発見した1ホストの rich なドメイン表現。
#[derive(Clone, Debug)]
pub struct Host {
    pub ip: Ipv4Addr,
    pub mac: Option<MacAddr>,
    pub hostname: Option<String>,
    pub vendor: Option<String>,
}

impl Host {
    pub fn new(ip: Ipv4Addr) -> Self {
        Self {
            ip,
            mac: None,
            hostname: None,
            vendor: None,
        }
    }
}

/// スキャン対象サブネット。P1では表示用 CIDR 文字列を保持する簡易版。
/// TODO(P1): ローカルNICから自動検出し、アドレス範囲を厳密に持つ（FR-01）。
#[derive(Clone, Debug)]
pub struct Subnet {
    pub cidr: String,
}

impl Subnet {
    pub fn new(cidr: impl Into<String>) -> Self {
        Self { cidr: cidr.into() }
    }
}
