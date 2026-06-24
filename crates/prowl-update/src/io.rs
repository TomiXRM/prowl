//! セルフアップデートの I/O 層。
//!
//! 更新でネットワーク＋ファイルシステムに触れる**唯一**の層。純粋モデル
//! （バージョン比較・JSON パース・アセット/計画選定・チェックサム照合）は
//! [`crate::domain`]。呼び出し側（CLI/GPUI）は `ureq` や `std::fs` を直接触らず
//! この層を呼ぶ。
//!
//! 流れ: [`check_latest`]（GitHub API）→ `domain::plan_update` →
//! （ユーザ確認後）[`install`]（DL → SHA-256 検証 → 展開 → アトミック差し替え →
//! 再起動）。チェックは best-effort（失敗は静か）、インストールは常に確認済み・
//! チェックサム検証付き・アトミック書き込み・破壊的コマンド無し。

use std::path::Path;

use crate::domain::{self, Asset, ReleaseInfo, UpdatePlan, Version};

const REPO: &str = "TomiXRM/prowl";
const USER_AGENT: &str = concat!("prowl/", env!("CARGO_PKG_VERSION"), " (self-update)");

/// 更新成功後の再起動方法。
#[derive(Debug, Clone)]
pub struct Relaunch {
    pub program: String,
    pub args: Vec<String>,
}

impl Relaunch {
    /// 新プロセスを（デタッチで）起動し、自分は終了する。戻らない。
    pub fn spawn_and_exit(&self) -> ! {
        let _ = std::process::Command::new(&self.program)
            .args(&self.args)
            .spawn();
        std::process::exit(0);
    }
}

/// 実行中バージョン。`PROWL_UPDATE_FORCE_CURRENT`（例 `0.0.1`）でテスト上書き可。
pub fn current_version() -> Version {
    if let Ok(forced) = std::env::var("PROWL_UPDATE_FORCE_CURRENT") {
        if let Some(v) = Version::parse(&forced) {
            return v;
        }
    }
    Version::parse(env!("CARGO_PKG_VERSION")).expect("CARGO_PKG_VERSION is valid semver")
}

/// 最新の GitHub リリースを取得。best-effort: ネットワーク/パースエラーは `String` で
/// 返し、呼び出し側がログして無視する（更新は提示しない）。
pub fn check_latest() -> Result<ReleaseInfo, String> {
    let url = format!("https://api.github.com/repos/{REPO}/releases/latest");
    let body = http_text(&url)?;
    domain::parse_release_json(&body).ok_or_else(|| "could not parse release JSON".to_string())
}

/// 起動時チェックの便宜版: 取得 → 判定。計画と、その元リリース（チェックサム取得に
/// 必要なアセット一覧を持つ）を返す。`skipped` はユーザの「このバージョンをスキップ」。
pub fn check_for_update(
    skipped: Option<&str>,
) -> Result<Option<(UpdatePlan, ReleaseInfo)>, String> {
    let release = check_latest()?;
    let plan = domain::plan_update(
        &current_version(),
        &release,
        std::env::consts::OS,
        std::env::consts::ARCH,
        skipped,
    );
    Ok(plan.map(|p| (p, release)))
}

// ────────────────────────────────────────────────────────────
// HTTP（ureq, blocking, rustls）
// ────────────────────────────────────────────────────────────

fn http_text(url: &str) -> Result<String, String> {
    ureq::get(url)
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/vnd.github+json")
        .call()
        .map_err(|e| format!("GET {url}: {e}"))?
        .body_mut()
        .read_to_string()
        .map_err(|e| format!("read {url}: {e}"))
}

fn http_bytes(url: &str) -> Result<Vec<u8>, String> {
    ureq::get(url)
        .header("User-Agent", USER_AGENT)
        .call()
        .map_err(|e| format!("GET {url}: {e}"))?
        .body_mut()
        // リリースアセットは数十MB。既定の読み取り上限を大きく引き上げる。
        .with_config()
        .limit(512 * 1024 * 1024)
        .read_to_vec()
        .map_err(|e| format!("download {url}: {e}"))
}

// ────────────────────────────────────────────────────────────
// インストール（DL → 検証 → 展開 → 差し替え → 再起動）
// ────────────────────────────────────────────────────────────

/// 計画したアセットを DL し、リリースの `SHA256SUMS-*.txt` と SHA-256 を照合し、展開して
/// 実行中のインストールに差し替え、再起動方法を返す。`log` に進捗の人間可読行が届く。
///
/// 失敗時は実行中のインストールを温存（検証は差し替え前に行い、差し替えはステージング
/// コピーを先に書く）。
pub fn install(
    plan: &UpdatePlan,
    release: &ReleaseInfo,
    log: &dyn Fn(&str),
) -> Result<Relaunch, String> {
    log(&format!(
        "ダウンロード中 {} ({:.1} MB)…",
        plan.asset.name,
        plan.asset.size as f64 / 1_048_576.0
    ));
    let bytes = http_bytes(&plan.asset.url)?;

    log("チェックサム検証中…");
    let expected = expected_checksum(release, &plan.asset.name)?;
    let actual = sha256_hex(&bytes);
    if actual != expected {
        return Err(format!(
            "checksum mismatch for {} (expected {}…, got {}…) — install aborted, current version untouched",
            plan.asset.name,
            &expected[..8.min(expected.len())],
            &actual[..8.min(actual.len())],
        ));
    }

    // DL をテンポラリにステージ（展開完了まで生存）。
    let staging = tempfile::Builder::new()
        .prefix("prowl-update-")
        .tempdir()
        .map_err(|e| format!("tempdir: {e}"))?;
    let archive = staging.path().join(&plan.asset.name);
    std::fs::write(&archive, &bytes).map_err(|e| format!("write archive: {e}"))?;

    log("インストール中…");
    let target = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
    install_platform(&archive, staging.path(), &target, log)
}

/// リリースの `SHA256SUMS-*.txt` を取得して `asset_name` の期待 SHA-256 を探す。
fn expected_checksum(release: &ReleaseInfo, asset_name: &str) -> Result<String, String> {
    let sums: Vec<&Asset> = release
        .assets
        .iter()
        .filter(|a| a.name.starts_with("SHA256SUMS"))
        .collect();
    if sums.is_empty() {
        return Err("release has no SHA256SUMS file to verify against".to_string());
    }
    for a in sums {
        if let Ok(text) = http_text(&a.url) {
            if let Some(hex) = domain::find_checksum(&text, asset_name) {
                return Ok(hex);
            }
        }
    }
    Err(format!("no checksum found for {asset_name}"))
}

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

// ── プラットフォーム別 展開＋アトミック差し替え ──────────────

#[cfg(target_os = "linux")]
fn install_platform(
    archive: &Path,
    staging: &Path,
    target: &Path,
    log: &dyn Fn(&str),
) -> Result<Relaunch, String> {
    // tar.gz（裸のバイナリ）。レイアウトは prowl-<v>-<arch>/bin/prowl。
    run(std::process::Command::new("tar").args([
        "-xzf",
        archive.to_str().ok_or("non-utf8 archive path")?,
        "-C",
        staging.to_str().ok_or("non-utf8 staging path")?,
    ]))?;
    let new_bin = find_file(staging, "prowl").ok_or("prowl binary not found in tarball")?;
    swap_file(&new_bin, target, log)?;
    Ok(Relaunch {
        program: target.to_string_lossy().into_owned(),
        args: vec![],
    })
}

#[cfg(target_os = "macos")]
fn install_platform(
    archive: &Path,
    staging: &Path,
    target: &Path,
    log: &dyn Fn(&str),
) -> Result<Relaunch, String> {
    // target = …/prowl.app/Contents/MacOS/prowl → アプリルートは3階層上。
    let app_root = target
        .ancestors()
        .nth(3)
        .filter(|p| p.extension().map(|e| e == "app").unwrap_or(false))
        .ok_or("not running from an installed prowl.app — download the .dmg manually")?
        .to_path_buf();

    let mnt = staging.join("mnt");
    std::fs::create_dir_all(&mnt).map_err(|e| format!("mkdir mnt: {e}"))?;
    run(std::process::Command::new("hdiutil").args([
        "attach",
        "-nobrowse",
        "-quiet",
        "-mountpoint",
        mnt.to_str().ok_or("non-utf8 mnt")?,
        archive.to_str().ok_or("non-utf8 archive")?,
    ]))?;
    let detach = || {
        let _ = std::process::Command::new("hdiutil")
            .args(["detach", "-quiet", &mnt.to_string_lossy()])
            .status();
    };
    let new_app = mnt.join("prowl.app");
    if !new_app.exists() {
        detach();
        return Err("prowl.app not found in the .dmg".to_string());
    }
    // 読み取り専用 DMG から新アプリをステージへコピーしてから差し替え。
    let staged_app = staging.join("prowl.app");
    let copy = run(std::process::Command::new("cp").args([
        "-R",
        new_app.to_str().ok_or("non-utf8 new app")?,
        staged_app.to_str().ok_or("non-utf8 staged app")?,
    ]));
    detach();
    copy?;
    swap_dir(&staged_app, &app_root, log)?;
    Ok(Relaunch {
        program: "open".to_string(),
        args: vec![app_root.to_string_lossy().into_owned()],
    })
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn install_platform(
    _archive: &Path,
    _staging: &Path,
    _target: &Path,
    _log: &dyn Fn(&str),
) -> Result<Relaunch, String> {
    Err("self-update is only supported on macOS and Linux".to_string())
}

/// `target` のファイルを `new` でアトミックに置換（兄弟へコピー → unix なら +x → rename）。
/// 実行中バイナリの開いた inode は旧コードを保持するので、Linux/macOS で実行中でも安全。
#[cfg(any(target_os = "linux", target_os = "macos"))]
#[allow(dead_code)] // Linux のインストール経路でのみ呼ばれる（unix なら test で叩く）
fn swap_file(new: &Path, target: &Path, _log: &dyn Fn(&str)) -> Result<(), String> {
    let staged = target.with_extension("new");
    std::fs::copy(new, &staged).map_err(|e| format!("stage new binary: {e}"))?;
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&staged)
            .map_err(|e| format!("stat staged: {e}"))?
            .permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&staged, perms).map_err(|e| format!("chmod staged: {e}"))?;
    }
    std::fs::rename(&staged, target).map_err(|e| {
        let _ = std::fs::remove_file(&staged);
        format!("swap into place: {e}")
    })
}

/// `target` ディレクトリを `new` で置換: 旧を退避 → 新を移動 → 旧を best-effort 削除。
/// macOS の `.app` バンドル差し替えに使う。
#[cfg(target_os = "macos")]
fn swap_dir(new: &Path, target: &Path, _log: &dyn Fn(&str)) -> Result<(), String> {
    let backup = target.with_extension("app.old");
    let _ = std::fs::remove_dir_all(&backup);
    std::fs::rename(target, &backup).map_err(|e| format!("move old app aside: {e}"))?;
    if let Err(e) = std::fs::rename(new, target) {
        let _ = std::fs::rename(&backup, target); // ロールバック
        return Err(format!("move new app into place: {e}"));
    }
    let _ = std::fs::remove_dir_all(&backup);
    Ok(())
}

/// `root` 配下で名前が `name` の最初のファイルを再帰探索。
#[cfg(target_os = "linux")]
fn find_file(root: &Path, name: &str) -> Option<std::path::PathBuf> {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).ok()?;
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.file_name().and_then(|n| n.to_str()) == Some(name) {
                return Some(p);
            }
        }
    }
    None
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn run(cmd: &mut std::process::Command) -> Result<(), String> {
    let status = cmd.status().map_err(|e| format!("spawn: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("command failed: {status}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_known_vector() {
        // SHA-256("abc")
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn swap_file_replaces_target_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("prowl");
        let new = dir.path().join("new-prowl");
        std::fs::write(&target, b"OLD").unwrap();
        std::fs::write(&new, b"NEWCONTENT").unwrap();
        swap_file(&new, &target, &|_| {}).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"NEWCONTENT");
        // ステージング兄弟が掃除されている
        assert!(!target.with_extension("new").exists());
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&target).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111, "target is executable");
    }

    #[test]
    fn current_version_honors_override() {
        std::env::set_var("PROWL_UPDATE_FORCE_CURRENT", "0.0.1");
        assert_eq!(current_version(), Version::parse("0.0.1").unwrap());
        std::env::remove_var("PROWL_UPDATE_FORCE_CURRENT");
    }
}
