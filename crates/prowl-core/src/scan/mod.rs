//! ポートスキャンの差し替え口（内側の軸）。
//! 無権限の TCP connect（[`connect::ConnectScanner`]）が既定。
//! 将来 SYN スキャン（要root）も同じトレイトで足せる。

use std::net::Ipv4Addr;

pub mod connect;
pub mod mock;

pub use connect::ConnectScanner;

/// 開放ポート1件。
#[derive(Clone, Debug)]
pub struct OpenPort {
    pub port: u16,
    pub service: Option<String>,
    pub banner: Option<String>,
}

#[async_trait::async_trait]
pub trait PortScanner: Send + Sync {
    /// 指定ホストの指定ポート群をスキャンし、開放ポートを返す。
    async fn scan(&self, ip: Ipv4Addr, ports: &[u16]) -> Vec<OpenPort>;
}

/// 既定でスキャンする「よく使われるポート」一覧（高速・高シグナル寄りに厳選）。
pub const COMMON_PORTS: &[u16] = &[
    21, 22, 23, 25, 53, 80, 81, 88, 110, 111, 123, 135, 139, 143, 161, 389, 443, 445, 465, 514,
    515, 543, 548, 554, 587, 631, 993, 995, 1080, 1433, 1521, 1723, 1883, 2049, 2082, 2083, 3000,
    3306, 3389, 3690, 4444, 5000, 5001, 5060, 5353, 5432, 5555, 5900, 5901, 6000, 6379, 7070, 7777,
    8000, 8008, 8080, 8081, 8443, 8888, 9000, 9100, 9200, 9999, 10000, 27017, 32400, 49152,
];

/// ポート番号 → よく知られたサービス名。
pub fn service_name(port: u16) -> Option<&'static str> {
    Some(match port {
        20 | 21 => "ftp",
        22 => "ssh",
        23 => "telnet",
        25 => "smtp",
        53 => "dns",
        80 | 81 | 8008 => "http",
        88 => "kerberos",
        110 => "pop3",
        111 => "rpcbind",
        123 => "ntp",
        135 => "msrpc",
        139 => "netbios-ssn",
        143 => "imap",
        161 => "snmp",
        389 => "ldap",
        443 => "https",
        445 => "smb",
        465 => "smtps",
        514 => "syslog",
        515 => "printer",
        543 => "klogin",
        548 => "afp",
        554 => "rtsp",
        587 => "submission",
        631 => "ipp",
        993 => "imaps",
        995 => "pop3s",
        1080 => "socks",
        1433 => "mssql",
        1521 => "oracle",
        1723 => "pptp",
        1883 => "mqtt",
        2049 => "nfs",
        2082 | 2083 => "cpanel",
        3000 => "http-dev",
        3306 => "mysql",
        3389 => "rdp",
        3690 => "svn",
        5000 | 5001 => "upnp",
        5060 => "sip",
        5353 => "mdns",
        5432 => "postgresql",
        5900 | 5901 => "vnc",
        6000 => "x11",
        6379 => "redis",
        7070 => "rtsp-alt",
        8000 | 8080 | 8081 | 8888 | 9000 => "http-alt",
        8443 => "https-alt",
        9100 => "jetdirect",
        9200 => "elasticsearch",
        10000 => "webmin",
        27017 => "mongodb",
        32400 => "plex",
        49152 => "upnp",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_ports_map() {
        assert_eq!(service_name(22), Some("ssh"));
        assert_eq!(service_name(443), Some("https"));
        assert_eq!(service_name(32400), Some("plex"));
        assert_eq!(service_name(11111), None);
    }
}
