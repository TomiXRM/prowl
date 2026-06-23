//! 決定論的なモックスキャナ（`--mock` / e2eテスト用）。
//! 実ネットワークに触れず、固定の開放ポートを即返す。

use std::net::Ipv4Addr;

use super::{OpenPort, PortScanner};

pub struct MockScanner;

#[async_trait::async_trait]
impl PortScanner for MockScanner {
    async fn scan(&self, _ip: Ipv4Addr, _ports: &[u16]) -> Vec<OpenPort> {
        vec![
            OpenPort {
                port: 22,
                service: Some("ssh".into()),
                banner: Some("SSH-2.0-mock".into()),
            },
            OpenPort {
                port: 80,
                service: Some("http".into()),
                banner: Some("Server: mock".into()),
            },
        ]
    }
}
