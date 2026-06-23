//! mDNS(.local) 逆引きでホスト名を引く Enricher（FR-03）。
//!
//! `224.0.0.251:5353` に PTR クエリ（QUビット付き=ユニキャスト応答要求）を投げ、
//! 返ってきたホスト名を採用する。root 不要。

use std::net::Ipv4Addr;
use std::time::Duration;

use hickory_proto::op::{Message, MessageType, OpCode, Query};
use hickory_proto::rr::{Name, RData, RecordType};
use tokio::net::UdpSocket;

use super::Enricher;
use crate::model::Host;

const MDNS_ADDR: &str = "224.0.0.251:5353";
const WAIT: Duration = Duration::from_millis(800);

pub struct MdnsEnricher;

#[async_trait::async_trait]
impl Enricher for MdnsEnricher {
    async fn enrich(&self, host: &mut Host) {
        if host.hostname.is_some() {
            return;
        }
        if let Some(name) = mdns_reverse(host.ip).await {
            host.hostname = Some(name);
        }
    }
}

/// `192.168.10.9` -> `9.10.168.192.in-addr.arpa.`
fn reverse_arpa(ip: Ipv4Addr) -> String {
    let o = ip.octets();
    format!("{}.{}.{}.{}.in-addr.arpa.", o[3], o[2], o[1], o[0])
}

async fn mdns_reverse(ip: Ipv4Addr) -> Option<String> {
    let qname = Name::from_ascii(reverse_arpa(ip)).ok()?;

    let mut msg = Message::new();
    msg.set_id(0)
        .set_message_type(MessageType::Query)
        .set_op_code(OpCode::Query)
        .set_recursion_desired(false)
        .add_query(Query::query(qname, RecordType::PTR));

    let mut bytes = msg.to_vec().ok()?;
    // mDNS の QU ビット（ユニキャスト応答要求）= qclass 最上位ビットを立てる
    let n = bytes.len();
    bytes[n - 2] |= 0x80;

    let sock = UdpSocket::bind("0.0.0.0:0").await.ok()?;
    sock.send_to(&bytes, MDNS_ADDR).await.ok()?;

    let mut buf = [0u8; 4096];
    let (len, _) = tokio::time::timeout(WAIT, sock.recv_from(&mut buf))
        .await
        .ok()?
        .ok()?;

    let resp = Message::from_vec(&buf[..len]).ok()?;
    for rec in resp.answers() {
        if let Some(RData::PTR(ptr)) = rec.data() {
            let name = ptr.to_string();
            let name = name.trim_end_matches('.');
            if !name.is_empty() {
                return Some(name.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reverse_arpa_format() {
        let ip = Ipv4Addr::new(192, 168, 10, 9);
        assert_eq!(reverse_arpa(ip), "9.10.168.192.in-addr.arpa.");
    }

    #[test]
    fn query_sets_qu_bit() {
        let qname = Name::from_ascii(reverse_arpa(Ipv4Addr::new(192, 168, 10, 9))).unwrap();
        let mut msg = Message::new();
        msg.set_id(0)
            .set_message_type(MessageType::Query)
            .set_op_code(OpCode::Query)
            .add_query(Query::query(qname, RecordType::PTR));
        let mut bytes = msg.to_vec().unwrap();
        let n = bytes.len();
        bytes[n - 2] |= 0x80;
        // QUビットが立っていること
        assert_ne!(bytes[n - 2] & 0x80, 0);
        // 1問だけ含むこと
        assert_eq!(Message::from_vec(&bytes).unwrap().queries().len(), 1);
    }
}
