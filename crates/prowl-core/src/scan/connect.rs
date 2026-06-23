//! 無権限 TCP connect スキャナ（FR-06）＋ 軽量バナー取得（FR-07）。
//!
//! 各ポートへ `TcpStream::connect` を試み、繋がれば開放とみなす。要 root なし。
//! 開放ポートは即時バナー（SSH/FTP/SMTP 等）を待ち、HTTP系は HEAD を投げて1行取る。

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;

use futures_util::stream::{self, StreamExt};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use super::{service_name, OpenPort, PortScanner};

const BANNER_WAIT: Duration = Duration::from_millis(600);

pub struct ConnectScanner {
    pub connect_timeout: Duration,
    pub concurrency: usize,
}

impl Default for ConnectScanner {
    fn default() -> Self {
        Self {
            connect_timeout: Duration::from_millis(900),
            concurrency: 256,
        }
    }
}

#[async_trait::async_trait]
impl PortScanner for ConnectScanner {
    async fn scan(&self, ip: Ipv4Addr, ports: &[u16]) -> Vec<OpenPort> {
        let to = self.connect_timeout;
        let mut open: Vec<OpenPort> = stream::iter(ports.iter().copied())
            .map(|port| async move { probe(ip, port, to).await })
            .buffer_unordered(self.concurrency)
            .filter_map(|r| async move { r })
            .collect()
            .await;
        open.sort_by_key(|p| p.port);
        open
    }
}

async fn probe(ip: Ipv4Addr, port: u16, to: Duration) -> Option<OpenPort> {
    let addr = SocketAddr::new(IpAddr::V4(ip), port);
    let mut stream = tokio::time::timeout(to, TcpStream::connect(addr))
        .await
        .ok()?
        .ok()?;
    let service = service_name(port).map(str::to_string);
    let banner = grab_banner(&mut stream, port).await;
    Some(OpenPort {
        port,
        service,
        banner,
    })
}

async fn grab_banner(stream: &mut TcpStream, port: u16) -> Option<String> {
    let mut buf = [0u8; 256];

    // まずサーバ主導で送られてくる挨拶（SSH/FTP/SMTP/POP3/IMAP 等）を待つ
    if let Ok(Ok(n)) = tokio::time::timeout(BANNER_WAIT, stream.read(&mut buf)).await {
        if n > 0 {
            return Some(first_clean_line(&buf[..n]));
        }
    }

    // HTTP系なら HEAD を投げて Server 行（無ければステータス行）を取る
    if matches!(port, 80 | 81 | 8000 | 8008 | 8080 | 8081 | 8888 | 9000) {
        let _ = stream.write_all(b"HEAD / HTTP/1.0\r\n\r\n").await;
        if let Ok(Ok(n)) = tokio::time::timeout(BANNER_WAIT, stream.read(&mut buf)).await {
            if n > 0 {
                return Some(http_summary(&buf[..n]));
            }
        }
    }
    None
}

/// 受信バイト列の最初の行を、印字可能ASCIIだけにして 60 文字で切る。
fn first_clean_line(bytes: &[u8]) -> String {
    let line: Vec<u8> = bytes
        .iter()
        .take_while(|&&b| b != b'\r' && b != b'\n')
        .copied()
        .collect();
    let s: String = String::from_utf8_lossy(&line)
        .chars()
        .filter(|c| !c.is_control())
        .collect();
    truncate(s.trim(), 60)
}

/// HTTP応答から `Server:` 行（無ければ先頭のステータス行）を要約する。
fn http_summary(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    for line in text.lines() {
        if let Some(rest) = line
            .strip_prefix("Server:")
            .or_else(|| line.strip_prefix("server:"))
        {
            return truncate(&format!("Server:{rest}"), 60);
        }
    }
    truncate(text.lines().next().unwrap_or("").trim(), 60)
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max).collect();
        format!("{cut}…")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpListener;

    #[test]
    fn banner_line_cleaned() {
        assert_eq!(first_clean_line(b"SSH-2.0-OpenSSH_9.0\r\nrest"), "SSH-2.0-OpenSSH_9.0");
    }

    #[test]
    fn http_server_extracted() {
        let resp = b"HTTP/1.0 200 OK\r\nServer: nginx/1.25\r\n\r\n";
        assert_eq!(http_summary(resp), "Server: nginx/1.25");
    }

    #[tokio::test]
    async fn detects_open_port_with_banner() {
        // ローカルにバナーを送る簡易サーバを立てて connect スキャンする
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            if let Ok((mut s, _)) = listener.accept().await {
                let _ = s.write_all(b"220 test-banner ready\r\n").await;
            }
        });

        let scanner = ConnectScanner::default();
        let open = scanner.scan(Ipv4Addr::new(127, 0, 0, 1), &[port]).await;
        assert_eq!(open.len(), 1);
        assert_eq!(open[0].port, port);
        assert_eq!(open[0].banner.as_deref(), Some("220 test-banner ready"));
    }

    #[tokio::test]
    async fn closed_port_not_reported() {
        // 使われていなさそうな高位ポート（基本 connect 失敗）
        let scanner = ConnectScanner {
            connect_timeout: Duration::from_millis(300),
            concurrency: 16,
        };
        let open = scanner.scan(Ipv4Addr::new(127, 0, 0, 1), &[1]).await;
        assert!(open.is_empty());
    }
}
