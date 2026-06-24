//! ローカルNICの自動検出（FR-01）。ARPスキャンの送信元情報も供給する。

use std::net::Ipv4Addr;

use anyhow::{anyhow, Result};
use pnet::datalink::{self, NetworkInterface};
use pnet::ipnetwork::{IpNetwork, Ipv4Network};
use pnet::util::MacAddr;

use crate::model::Subnet;

/// 検出したローカルネットワーク情報。
#[derive(Clone)]
pub struct LocalNet {
    pub interface: NetworkInterface,
    pub ipv4: Ipv4Addr,
    pub mac: MacAddr,
    pub network: Ipv4Network,
}

impl LocalNet {
    /// 表示用の [`Subnet`]（ネットワークアドレス基準の CIDR）。
    pub fn subnet(&self) -> Subnet {
        Subnet::new(format!(
            "{}/{}",
            self.network.network(),
            self.network.prefix()
        ))
    }

    /// スキャン対象のホストアドレス列（ネットワーク/ブロードキャスト/自分自身は除外）。
    /// `max` 件で打ち切る（巨大プレフィックスでの暴走防止）。
    pub fn targets(&self, max: usize) -> Vec<Ipv4Addr> {
        let net = self.network.network();
        let bcast = self.network.broadcast();
        self.network
            .iter()
            .filter(|ip| *ip != net && *ip != bcast && *ip != self.ipv4)
            .take(max)
            .collect()
    }

    /// インターフェース名（`en0` など）。
    pub fn name(&self) -> &str {
        &self.interface.name
    }

    /// `--list-ifaces` 表示用の1行サマリ。
    pub fn describe(&self) -> String {
        format!(
            "{name:<12} ip={ip:<15} subnet={net}/{prefix}  mac={mac}",
            name = self.interface.name,
            ip = self.ipv4,
            net = self.network.network(),
            prefix = self.network.prefix(),
            mac = self.mac,
        )
    }
}

/// スキャンに使える候補インターフェースか（稼働中・非ループバック・MACあり・IPv4あり）。
fn is_candidate(i: &NetworkInterface) -> bool {
    i.is_up() && !i.is_loopback() && i.mac.is_some() && i.ips.iter().any(IpNetwork::is_ipv4)
}

/// `NetworkInterface` から [`LocalNet`] を組み立てる（IPv4/MAC を取り出す）。
fn from_interface(interface: NetworkInterface) -> Result<LocalNet> {
    let network = interface
        .ips
        .iter()
        .find_map(|ip| match ip {
            IpNetwork::V4(n) => Some(*n),
            IpNetwork::V6(_) => None,
        })
        .ok_or_else(|| anyhow!("インターフェース {} にIPv4がありません", interface.name))?;
    let mac = interface
        .mac
        .ok_or_else(|| anyhow!("インターフェース {} にMACがありません", interface.name))?;
    Ok(LocalNet {
        ipv4: network.ip(),
        mac,
        network,
        interface,
    })
}

/// 稼働中の主要インターフェース（非ループバック・IPv4あり・MACあり）を自動選択する。
pub fn detect() -> Result<LocalNet> {
    let interface = datalink::interfaces()
        .into_iter()
        .find(is_candidate)
        .ok_or_else(|| anyhow!("有効なIPv4インターフェースが見つかりません"))?;
    from_interface(interface)
}

/// 名前で明示指定したインターフェースを使う（`--iface en0` など）。
/// 見つからない場合は候補名を添えたエラーを返す。
pub fn detect_named(name: &str) -> Result<LocalNet> {
    let interface = datalink::interfaces()
        .into_iter()
        .find(|i| i.name == name)
        .ok_or_else(|| {
            let avail = list()
                .iter()
                .map(|l| l.interface.name.clone())
                .collect::<Vec<_>>()
                .join(", ");
            anyhow!("インターフェース '{name}' が見つかりません（候補: {avail}）")
        })?;
    from_interface(interface)
}

/// スキャンに使える候補インターフェースを全部列挙する（`--list-ifaces` 用）。
pub fn list() -> Vec<LocalNet> {
    datalink::interfaces()
        .into_iter()
        .filter(is_candidate)
        .filter_map(|i| from_interface(i).ok())
        .collect()
}
