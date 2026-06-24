//! 純粋なアップデートモデル（I/Oなし）。
//!
//! バージョン解析/比較、GitHub release JSON のパース、ホスト OS/arch に合う
//! アセット選定。ネットワークもファイルも触らない＝文字列だけで単体テスト可能。
//! `ureq` でのDL/検証/差し替えは [`crate::io`] 側。

use std::cmp::Ordering;

// ────────────────────────────────────────────────────────────
// Version
// ────────────────────────────────────────────────────────────

/// `major.minor.patch` ＋任意の pre-release タグからなるセマンティックバージョン。
///
/// 同じ `x.y.z` では正式版が pre-release より後ろに並ぶ（`1.0.0 > 1.0.0-beta`）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
    pub pre: Option<String>,
}

impl Version {
    /// `"v1.2.3"` / `"1.2.3"` / `"1.2.3-beta.1"` を解析。arity 不正や非数値で `None`。
    pub fn parse(s: &str) -> Option<Version> {
        let s = s.trim();
        let s = s.strip_prefix('v').unwrap_or(s);
        let (core, pre) = match s.split_once('-') {
            Some((c, p)) if !p.is_empty() => (c, Some(p.to_string())),
            _ => (s, None),
        };
        let mut it = core.split('.');
        let major = it.next()?.parse().ok()?;
        let minor = it.next()?.parse().ok()?;
        let patch = it.next()?.parse().ok()?;
        if it.next().is_some() {
            return None;
        }
        Some(Version {
            major,
            minor,
            patch,
            pre,
        })
    }

    /// 正式版（pre-release でない）なら `true`。
    pub fn is_stable(&self) -> bool {
        self.pre.is_none()
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(pre) = &self.pre {
            write!(f, "-{pre}")?;
        }
        Ok(())
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch)
            .cmp(&(other.major, other.minor, other.patch))
            .then_with(|| match (&self.pre, &other.pre) {
                (None, None) => Ordering::Equal,
                // 同じコアなら正式版が pre-release より上位。
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(a), Some(b)) => a.cmp(b),
            })
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

// ────────────────────────────────────────────────────────────
// Release モデル
// ────────────────────────────────────────────────────────────

/// DL 可能なリリースアセット1件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

/// `releases/latest` JSON から取り出した GitHub リリース。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReleaseInfo {
    pub tag: String,
    pub version: Version,
    pub notes: String,
    pub assets: Vec<Asset>,
}

/// `current` → 新リリースへ更新する具体計画（ホスト向けアセット込み）。[`plan_update`] が生成。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdatePlan {
    pub current: Version,
    pub latest: Version,
    pub tag: String,
    pub notes: String,
    pub asset: Asset,
}

/// ホスト OS/arch に合うアセットを選ぶ。
///
/// `os` = [`std::env::consts::OS`]（`"macos"`/`"linux"`）、`arch` =
/// [`std::env::consts::ARCH`]（`"aarch64"`/`"x86_64"`）。`release.yml` が吐く名前に対応:
/// `prowl-<v>-arm64.dmg` / `prowl-<v>-x86_64.dmg`、`prowl-<v>-<arch>.tar.gz`。
pub fn pick_asset<'a>(assets: &'a [Asset], os: &str, arch: &str) -> Option<&'a Asset> {
    match os {
        "macos" => {
            let a = if arch == "aarch64" { "arm64" } else { "x86_64" };
            assets
                .iter()
                .find(|x| x.name.ends_with(".dmg") && x.name.contains(a))
        }
        "linux" => {
            let a = if arch == "aarch64" {
                "aarch64"
            } else {
                "x86_64"
            };
            assets
                .iter()
                .find(|x| x.name.ends_with(".tar.gz") && x.name.contains(a))
        }
        _ => None,
    }
}

/// `release` が `current` より厳密に新しい正式版で、`skipped` タグでなく、ホスト向け
/// アセットが在る場合のみ [`UpdatePlan`] を返す。それ以外は `None`
/// （最新/pre-release/skip/アセット無し）。
pub fn plan_update(
    current: &Version,
    release: &ReleaseInfo,
    os: &str,
    arch: &str,
    skipped: Option<&str>,
) -> Option<UpdatePlan> {
    if !release.version.is_stable() {
        return None; // stable チャネルは pre-release を無視
    }
    if release.version <= *current {
        return None;
    }
    if skipped == Some(release.tag.as_str()) {
        return None;
    }
    let asset = pick_asset(&release.assets, os, arch)?.clone();
    Some(UpdatePlan {
        current: current.clone(),
        latest: release.version.clone(),
        tag: release.tag.clone(),
        notes: release.notes.clone(),
        asset,
    })
}

/// `SHA256SUMS` テキスト（連結可）から `asset_name` の小文字16進SHA-256を探す
/// （行: `<hex>␠␠<filename>`）。
pub fn find_checksum(checksums: &str, asset_name: &str) -> Option<String> {
    for line in checksums.lines() {
        let line = line.trim();
        let mut it = line.split_whitespace();
        let (Some(hex), Some(name)) = (it.next(), it.next()) else {
            continue;
        };
        // `shasum`/`sha256sum` はバイナリモードで名前に `*` を前置することがある。
        let name = name.trim_start_matches('*');
        if name == asset_name && hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Some(hex.to_ascii_lowercase());
        }
    }
    None
}

// ────────────────────────────────────────────────────────────
// Release JSON パース（serde 不使用・文字列を意識した手スキャン）
// ────────────────────────────────────────────────────────────

/// GitHub `releases/latest` JSON を [`ReleaseInfo`] へ。
///
/// 手書きだが文字列を意識する: `assets` 配列を括弧対応で取り出し各オブジェクトを
/// 個別走査するので、トップレベルの `"name"`/`"body"` がアセットの `"name"` と衝突しない。
pub fn parse_release_json(json: &str) -> Option<ReleaseInfo> {
    let tag = field_string(json, "tag_name")?;
    let version = Version::parse(&tag)?;
    let notes = field_string(json, "body").unwrap_or_default();
    let assets = parse_assets(json);
    Some(ReleaseInfo {
        tag,
        version,
        notes,
        assets,
    })
}

/// `json` 内で最初の `"key": "..."` の文字列値を読む。`\"` `\\` `\n` `\t` `\r`
/// `\/` `\uXXXX`(BMP) エスケープに対応。
fn field_string(json: &str, key: &str) -> Option<String> {
    let needle = format!("\"{}\"", key);
    let key_pos = json.find(&needle)?;
    let after = key_pos + needle.len();
    let colon = json[after..].find(':')? + after;
    let q = json[colon + 1..].find('"')? + colon + 1;
    scan_json_string(json.as_bytes(), q).map(|(v, _)| v)
}

/// `json` 内で最初の `"key": <digits>` の数値を読む。
fn field_u64(json: &str, key: &str) -> Option<u64> {
    let needle = format!("\"{}\"", key);
    let key_pos = json.find(&needle)?;
    let after = key_pos + needle.len();
    let colon = json[after..].find(':')? + after;
    let rest = json[colon + 1..].trim_start();
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

/// 開きクォートが `open_quote` にある JSON 文字列を走査。デコード値と閉じクォート直後の
/// インデックスを返す。
fn scan_json_string(bytes: &[u8], open_quote: usize) -> Option<(String, usize)> {
    let mut i = open_quote + 1;
    let mut out: Vec<u8> = Vec::new();
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => {
                i += 1;
                let e = *bytes.get(i)?;
                match e {
                    b'"' => out.push(b'"'),
                    b'\\' => out.push(b'\\'),
                    b'/' => out.push(b'/'),
                    b'n' => out.push(b'\n'),
                    b'r' => out.push(b'\r'),
                    b't' => out.push(b'\t'),
                    b'b' => out.push(0x08),
                    b'f' => out.push(0x0c),
                    b'u' => {
                        if i + 4 < bytes.len() {
                            if let Ok(hex) = std::str::from_utf8(&bytes[i + 1..i + 5]) {
                                if let Ok(cp) = u32::from_str_radix(hex, 16) {
                                    if let Some(ch) = char::from_u32(cp) {
                                        let mut buf = [0u8; 4];
                                        out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                                    }
                                }
                            }
                            i += 4;
                        }
                    }
                    other => out.push(other),
                }
                i += 1;
            }
            b'"' => return Some((String::from_utf8_lossy(&out).into_owned(), i + 1)),
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    None
}

/// `"assets":[ {…}, {…} ]` を [`Asset`] 群へ。文字列を意識した括弧対応で、値の中の
/// 括弧に惑わされない。
fn parse_assets(json: &str) -> Vec<Asset> {
    let Some((arr_start, arr_end)) = locate_array(json, "assets") else {
        return Vec::new();
    };
    let arr = &json[arr_start..=arr_end];
    let mut out = Vec::new();
    for obj in top_level_objects(arr) {
        if let Some(a) = parse_one_asset(obj) {
            out.push(a);
        }
    }
    out
}

fn parse_one_asset(obj: &str) -> Option<Asset> {
    let name = field_string(obj, "name")?;
    let url = field_string(obj, "browser_download_url")?;
    let size = field_u64(obj, "size").unwrap_or(0);
    Some(Asset { name, url, size })
}

/// `"key": [ ... ]` を見つけ `[`..`]` の包含バイト範囲を返す。
fn locate_array(json: &str, key: &str) -> Option<(usize, usize)> {
    let needle = format!("\"{}\"", key);
    let kp = json.find(&needle)?;
    let open = json[kp..].find('[')? + kp;
    let bytes = json.as_bytes();
    let (mut i, mut depth, mut in_str, mut esc) = (open, 0i32, false, false);
    while i < bytes.len() {
        let c = bytes[i];
        if in_str {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
        } else {
            match c {
                b'"' => in_str = true,
                b'[' | b'{' => depth += 1,
                b']' => {
                    depth -= 1;
                    if depth == 0 {
                        return Some((open, i));
                    }
                }
                b'}' => depth -= 1,
                _ => {}
            }
        }
        i += 1;
    }
    None
}

/// `[ {…}, {…} ]` をトップレベルの `{…}` 部分文字列へ分割。
fn top_level_objects(arr: &str) -> Vec<&str> {
    let bytes = arr.as_bytes();
    let (mut i, mut depth, mut start, mut in_str, mut esc) = (0usize, 0i32, 0usize, false, false);
    let mut out = Vec::new();
    while i < bytes.len() {
        let c = bytes[i];
        if in_str {
            if esc {
                esc = false;
            } else if c == b'\\' {
                esc = true;
            } else if c == b'"' {
                in_str = false;
            }
        } else {
            match c {
                b'"' => in_str = true,
                b'{' => {
                    if depth == 0 {
                        start = i;
                    }
                    depth += 1;
                }
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        out.push(&arr[start..=i]);
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_orders_versions() {
        assert_eq!(
            Version::parse("v0.3.3"),
            Some(Version {
                major: 0,
                minor: 3,
                patch: 3,
                pre: None
            })
        );
        assert_eq!(Version::parse("1.2.3").unwrap().major, 1);
        assert!(Version::parse("0.3").is_none());
        assert!(Version::parse("x.y.z").is_none());

        let v033 = Version::parse("0.3.3").unwrap();
        let v034 = Version::parse("0.3.4").unwrap();
        let v0310 = Version::parse("0.3.10").unwrap();
        assert!(v034 > v033);
        assert!(v0310 > v034); // 辞書順でなく数値順
        let beta = Version::parse("0.3.4-beta.1").unwrap();
        assert!(v034 > beta); // 正式版が自身の pre-release より上位
        assert!(beta > v033);
        assert!(!beta.is_stable());
        assert_eq!(v034.to_string(), "0.3.4");
        assert_eq!(beta.to_string(), "0.3.4-beta.1");
    }

    fn sample_json() -> &'static str {
        // 構造的に忠実に切り詰めた releases/latest JSON。トップの "name"/"body" と
        // アセットの "name" を混同しないことを検査。
        r#"{
          "tag_name": "v0.3.4",
          "name": "v0.3.4",
          "body": "Line one\nLine two with a \"quote\" and emoji 🚀",
          "assets": [
            {"name":"prowl-0.3.4-arm64.dmg","size":1111,"browser_download_url":"https://example.com/prowl-0.3.4-arm64.dmg"},
            {"name":"prowl-0.3.4-x86_64.tar.gz","size":2222,"browser_download_url":"https://example.com/prowl-0.3.4-x86_64.tar.gz"},
            {"name":"prowl-0.3.4-aarch64.tar.gz","size":3333,"browser_download_url":"https://example.com/prowl-0.3.4-aarch64.tar.gz"},
            {"name":"SHA256SUMS-macos-arm64.txt","size":55,"browser_download_url":"https://example.com/SHA256SUMS-macos-arm64.txt"}
          ]
        }"#
    }

    #[test]
    fn parses_release_json() {
        let r = parse_release_json(sample_json()).expect("parse");
        assert_eq!(r.tag, "v0.3.4");
        assert_eq!(r.version, Version::parse("0.3.4").unwrap());
        assert!(r.notes.contains("Line one\nLine two"));
        assert!(r.notes.contains("\"quote\""));
        assert!(r.notes.contains('🚀')); // \uXXXX デコード
        assert_eq!(r.assets.len(), 4);
        let dmg = r.assets.iter().find(|a| a.name.ends_with(".dmg")).unwrap();
        assert_eq!(dmg.url, "https://example.com/prowl-0.3.4-arm64.dmg");
        assert_eq!(dmg.size, 1111);
    }

    #[test]
    fn picks_per_platform_assets() {
        let r = parse_release_json(sample_json()).unwrap();
        assert!(pick_asset(&r.assets, "macos", "aarch64")
            .unwrap()
            .name
            .ends_with("arm64.dmg"));
        // x86_64 dmg はサンプルに無い → None
        assert!(pick_asset(&r.assets, "macos", "x86_64").is_none());
        assert_eq!(
            pick_asset(&r.assets, "linux", "x86_64").unwrap().name,
            "prowl-0.3.4-x86_64.tar.gz"
        );
        assert_eq!(
            pick_asset(&r.assets, "linux", "aarch64").unwrap().name,
            "prowl-0.3.4-aarch64.tar.gz"
        );
        assert!(pick_asset(&r.assets, "windows", "x86_64").is_none());
    }

    #[test]
    fn plan_update_gates() {
        let r = parse_release_json(sample_json()).unwrap();
        let cur = Version::parse("0.3.3").unwrap();
        // 新しい → 計画あり
        assert!(plan_update(&cur, &r, "macos", "aarch64", None).is_some());
        // 最新 → なし
        let same = Version::parse("0.3.4").unwrap();
        assert!(plan_update(&same, &r, "macos", "aarch64", None).is_none());
        // 手元が先 → なし
        let ahead = Version::parse("0.4.0").unwrap();
        assert!(plan_update(&ahead, &r, "macos", "aarch64", None).is_none());
        // skip → なし
        assert!(plan_update(&cur, &r, "macos", "aarch64", Some("v0.3.4")).is_none());
        // 不明プラットフォーム → なし（アセット無し）
        assert!(plan_update(&cur, &r, "freebsd", "x86_64", None).is_none());
    }

    #[test]
    fn finds_checksum_line() {
        let sums = "abc  not-it.txt\n\
            d4f1e2a3b4c5d6e7f80911223344556677889900aabbccddeeff001122334455  prowl-0.3.4-x86_64.tar.gz\n\
            00ff  short.txt\n";
        assert_eq!(
            find_checksum(sums, "prowl-0.3.4-x86_64.tar.gz").as_deref(),
            Some("d4f1e2a3b4c5d6e7f80911223344556677889900aabbccddeeff001122334455")
        );
        assert!(find_checksum(sums, "missing.zip").is_none());
        // バイナリモードの '*' 前置を許容
        let star = "aa11bb22cc33dd44ee55ff6677889900aabbccddeeff00112233445566778899 *prowl\n";
        assert!(find_checksum(star, "prowl").is_some());
    }

    #[test]
    fn pre_release_is_not_offered_on_stable() {
        let json = r#"{"tag_name":"v0.4.0-beta.1","name":"beta","body":"",
            "assets":[{"name":"prowl-0.4.0-x86_64.tar.gz","size":1,"browser_download_url":"https://e/x"}]}"#;
        let r = parse_release_json(json).unwrap();
        let cur = Version::parse("0.3.3").unwrap();
        assert!(plan_update(&cur, &r, "linux", "x86_64", None).is_none());
    }
}
