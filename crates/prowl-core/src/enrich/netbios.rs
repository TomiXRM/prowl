//! NetBIOS 名前問い合わせ（Node Status / NBSTAT, UDP137）で Windows 端末名を引く Enricher（FR-03）。
//!
//! 対象ホストの 137/udp に NBSTAT クエリを送り、応答の名前テーブルから
//! ユニークな Workstation(<00>) 名を採用する。root 不要・ベストエフォート。

use std::net::Ipv4Addr;
use std::time::Duration;

use tokio::net::UdpSocket;

use super::Enricher;
use crate::model::Host;

const WAIT: Duration = Duration::from_millis(800);

pub struct NetBiosEnricher;

#[async_trait::async_trait]
impl Enricher for NetBiosEnricher {
    async fn enrich(&self, host: &mut Host) {
        if host.hostname.is_some() {
            return;
        }
        if let Some(name) = nbstat(host.ip).await {
            host.hostname = Some(name);
        }
    }
}

/// NBSTAT（Node Status Request）クエリを組み立てる。
fn nbstat_query() -> Vec<u8> {
    let mut q = Vec::with_capacity(50);
    q.extend_from_slice(&[0x00, 0x00]); // transaction id
    q.extend_from_slice(&[0x00, 0x00]); // flags
    q.extend_from_slice(&[0x00, 0x01]); // QDCOUNT
    q.extend_from_slice(&[0x00, 0x00]); // ANCOUNT
    q.extend_from_slice(&[0x00, 0x00]); // NSCOUNT
    q.extend_from_slice(&[0x00, 0x00]); // ARCOUNT
                                        // 問い合わせ名: "*" を第一レベルエンコードした 32 バイト ("CK" + 'A'×30)
    q.push(0x20);
    q.push(b'C');
    q.push(b'K');
    q.extend_from_slice(&[b'A'; 30]);
    q.push(0x00); // 名前終端
    q.extend_from_slice(&[0x00, 0x21]); // QTYPE = NBSTAT
    q.extend_from_slice(&[0x00, 0x01]); // QCLASS = IN
    q
}

async fn nbstat(ip: Ipv4Addr) -> Option<String> {
    let sock = UdpSocket::bind("0.0.0.0:0").await.ok()?;
    sock.send_to(&nbstat_query(), (ip, 137)).await.ok()?;

    let mut buf = [0u8; 1024];
    let (len, _) = tokio::time::timeout(WAIT, sock.recv_from(&mut buf))
        .await
        .ok()?
        .ok()?;
    parse_nbstat(&buf[..len])
}

/// NBSTAT 応答から、ユニークな Workstation(<00>) 名を取り出す。
fn parse_nbstat(resp: &[u8]) -> Option<String> {
    // header(12) + 応答名(34) + type(2)+class(2)+ttl(4)+rdlen(2)=10 → RDATA は 56 から
    const RDATA: usize = 56;
    if resp.len() <= RDATA {
        return None;
    }
    let num = resp[RDATA] as usize;
    let mut off = RDATA + 1;
    for _ in 0..num {
        if off + 18 > resp.len() {
            break;
        }
        let name = &resp[off..off + 15];
        let suffix = resp[off + 15];
        let flags = u16::from_be_bytes([resp[off + 16], resp[off + 17]]);
        let is_group = flags & 0x8000 != 0;
        // ユニーク かつ Workstation(<00>) を端末名として採用
        if !is_group && suffix == 0x00 {
            let s = String::from_utf8_lossy(name)
                .trim_end_matches(['\0', ' '])
                .to_string();
            if !s.is_empty() {
                return Some(s);
            }
        }
        off += 18;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_shape() {
        let q = nbstat_query();
        assert_eq!(q.len(), 50);
        // QTYPE=NBSTAT(0x0021), QCLASS=IN(0x0001) が末尾
        assert_eq!(&q[q.len() - 4..], &[0x00, 0x21, 0x00, 0x01]);
    }

    #[test]
    fn parse_unique_workstation_name() {
        // header(12)+name(34)+rr-meta(10) = 56 バイトのダミー前置き
        let mut resp = vec![0u8; 56];
        resp.push(1); // num names = 1
        let mut entry = b"MYPC           ".to_vec(); // 15バイト ("MYPC" + 空白11)
        assert_eq!(entry.len(), 15);
        entry.push(0x00); // suffix <00> = Workstation
        entry.extend_from_slice(&[0x00, 0x00]); // flags: ユニーク
        resp.extend_from_slice(&entry);

        assert_eq!(parse_nbstat(&resp).as_deref(), Some("MYPC"));
    }

    #[test]
    fn parse_too_short_is_none() {
        assert_eq!(parse_nbstat(&[0u8; 10]), None);
    }
}
