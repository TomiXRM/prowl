//! xtask — prowl build/bundle helper（git-client の方式を簡素化）。
//!
//! stdlib のみ。`cargo run -p xtask -- <subcommand>` で実行。
//!   icon              assets/prowl.png → assets/icon/{AppIcon.icns,icon_512.png}（macOS, sips/iconutil）
//!   bundle-macos      release(--features gpui) → target/dist/prowl.app（ad-hoc 署名）
//!   dmg-macos         hdiutil で prowl.app + /Applications の DMG
//!   bundle-linux [--bin P]  tar.gz レイアウト（bin + .desktop + icon）
//!
//! GPUI フロントを含めるため release ビルドは常に `--features gpui`。

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const BIN: &str = "prowl";
const DISPLAY: &str = "prowl";
const BUNDLE_ID: &str = "com.tomixrm.prowl";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("xtask: error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let root = workspace_root();
    match args.first().map(String::as_str) {
        Some("icon") => icon(&root),
        Some("bundle-macos") => bundle_macos(&root),
        Some("dmg-macos") => dmg_macos(&root),
        Some("bundle-linux") => {
            let mut bin = None;
            let mut it = args.iter().skip(1);
            while let Some(a) = it.next() {
                match a.as_str() {
                    "--bin" => bin = it.next().map(String::as_str),
                    other => return Err(format!("unknown argument: {other}")),
                }
            }
            bundle_linux(&root, bin)
        }
        Some("-h") | Some("--help") | None => {
            println!("usage: cargo run -p xtask -- <icon|bundle-macos|dmg-macos|bundle-linux>");
            Ok(())
        }
        Some(other) => Err(format!("unknown subcommand: {other}")),
    }
}

// ───────────────────────── helpers ─────────────────────────

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask has a parent")
        .to_path_buf()
}

/// root Cargo.toml の `[workspace.package] version` を取り出す（toml クレート不使用）。
fn version(root: &Path) -> Result<String, String> {
    let text = std::fs::read_to_string(root.join("Cargo.toml")).map_err(|e| e.to_string())?;
    let mut in_wp = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('[') {
            in_wp = line == "[workspace.package]";
            continue;
        }
        if in_wp {
            if let Some(rest) = line.strip_prefix("version") {
                if let Some(v) = rest.trim_start().strip_prefix('=') {
                    let v = v.trim().trim_matches('"');
                    if !v.is_empty() {
                        return Ok(v.to_string());
                    }
                }
            }
        }
    }
    Err("could not find [workspace.package] version".into())
}

fn host_arch() -> &'static str {
    if cfg!(target_arch = "aarch64") {
        "arm64"
    } else {
        "x86_64"
    }
}

fn sh(cmd: &mut Command) -> Result<(), String> {
    let rendered = format!(
        "{} {}",
        cmd.get_program().to_string_lossy(),
        cmd.get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ")
    );
    let status = cmd
        .status()
        .map_err(|e| format!("spawn `{rendered}`: {e}"))?;
    if !status.success() {
        return Err(format!("`{rendered}` failed: {status}"));
    }
    Ok(())
}

fn clean_dir(p: &Path) -> Result<(), String> {
    if p.exists() {
        std::fs::remove_dir_all(p).map_err(|e| format!("rm -rf {}: {e}", p.display()))?;
    }
    Ok(())
}

fn dist(root: &Path) -> PathBuf {
    root.join("target").join("dist")
}

/// release ビルド（GPUI フロント込み）。
fn build_release(root: &Path) -> Result<PathBuf, String> {
    println!("xtask: cargo build --release -p prowl --features gpui");
    sh(Command::new("cargo").current_dir(root).args([
        "build",
        "--release",
        "-p",
        "prowl",
        "--features",
        "gpui",
    ]))?;
    let bin = root.join("target/release").join(BIN);
    if !bin.exists() {
        return Err(format!("binary not found: {}", bin.display()));
    }
    Ok(bin)
}

// ───────────────────────── icon ─────────────────────────

/// assets/prowl.png から AppIcon.icns（macOS）と icon_512.png を生成。
fn icon(root: &Path) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("`icon` requires macOS (sips/iconutil)".into());
    }
    let src = root.join("assets/prowl.png");
    if !src.exists() {
        return Err(format!("{} not found", src.display()));
    }
    let out = root.join("assets/icon");
    std::fs::create_dir_all(&out).map_err(|e| e.to_string())?;

    // 512px (Linux / 汎用)
    sh(Command::new("sips").args([
        "-z",
        "512",
        "512",
        src.to_str().unwrap(),
        "--out",
        out.join("icon_512.png").to_str().unwrap(),
    ]))?;

    // .icns via iconset
    let tmp = std::env::temp_dir().join("prowl.iconset");
    clean_dir(&tmp)?;
    std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
    for s in [16, 32, 128, 256, 512] {
        for (suffix, dim) in [(format!("{s}x{s}"), s), (format!("{s}x{s}@2x"), s * 2)] {
            sh(Command::new("sips").args([
                "-z",
                &dim.to_string(),
                &dim.to_string(),
                src.to_str().unwrap(),
                "--out",
                tmp.join(format!("icon_{suffix}.png")).to_str().unwrap(),
            ]))?;
        }
    }
    sh(Command::new("iconutil").args([
        "-c",
        "icns",
        tmp.to_str().unwrap(),
        "-o",
        out.join("AppIcon.icns").to_str().unwrap(),
    ]))?;
    clean_dir(&tmp)?;
    println!("icon: wrote {}", out.join("AppIcon.icns").display());
    Ok(())
}

// ───────────────────────── macOS ─────────────────────────

fn info_plist(v: &str) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>{DISPLAY}</string>
  <key>CFBundleDisplayName</key><string>{DISPLAY}</string>
  <key>CFBundleIdentifier</key><string>{BUNDLE_ID}</string>
  <key>CFBundleExecutable</key><string>{BIN}</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>{v}</string>
  <key>CFBundleVersion</key><string>{v}</string>
  <key>LSMinimumSystemVersion</key><string>13.0</string>
  <key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
"#
    )
}

fn bundle_macos(root: &Path) -> Result<(), String> {
    let v = version(root)?;
    println!("bundle-macos: prowl {v} ({})", host_arch());

    let icns = root.join("assets/icon/AppIcon.icns");
    if !icns.exists() {
        icon(root)?;
    }
    let bin = build_release(root)?;

    let app = dist(root).join("prowl.app");
    clean_dir(&app)?;
    let macos = app.join("Contents/MacOS");
    let res = app.join("Contents/Resources");
    std::fs::create_dir_all(&macos).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&res).map_err(|e| e.to_string())?;
    std::fs::copy(&bin, macos.join(BIN)).map_err(|e| format!("copy bin: {e}"))?;
    std::fs::write(app.join("Contents/Info.plist"), info_plist(&v)).map_err(|e| e.to_string())?;
    std::fs::copy(&icns, res.join("AppIcon.icns")).map_err(|e| e.to_string())?;
    std::fs::write(app.join("Contents/PkgInfo"), "APPL????").map_err(|e| e.to_string())?;

    // ad-hoc 署名（Developer ID / notarization は将来）
    sh(Command::new("codesign").args(["--force", "-s", "-", "--deep", app.to_str().unwrap()]))?;
    println!("bundle-macos: wrote {}", app.display());
    Ok(())
}

fn dmg_macos(root: &Path) -> Result<(), String> {
    let v = version(root)?;
    let app = dist(root).join("prowl.app");
    if !app.exists() {
        return Err("prowl.app not found — run bundle-macos first".into());
    }
    let stage = dist(root).join("dmg-stage");
    clean_dir(&stage)?;
    std::fs::create_dir_all(&stage).map_err(|e| e.to_string())?;
    sh(Command::new("cp").args(["-R", app.to_str().unwrap(), stage.to_str().unwrap()]))?;
    #[cfg(unix)]
    std::os::unix::fs::symlink("/Applications", stage.join("Applications"))
        .map_err(|e| format!("symlink: {e}"))?;

    let out = dist(root).join(format!("prowl-{v}-{}.dmg", host_arch()));
    let _ = std::fs::remove_file(&out);
    sh(Command::new("hdiutil").args([
        "create",
        "-volname",
        DISPLAY,
        "-srcfolder",
        stage.to_str().unwrap(),
        "-ov",
        "-format",
        "UDZO",
        out.to_str().unwrap(),
    ]))?;
    clean_dir(&stage)?;
    println!("dmg-macos: wrote {}", out.display());
    Ok(())
}

// ───────────────────────── Linux ─────────────────────────

fn desktop_entry() -> &'static str {
    "\
[Desktop Entry]
Type=Application
Name=prowl
GenericName=LAN Scanner
Comment=A no-sudo LAN scanner (TUI/GUI)
Exec=prowl --gpui
Icon=prowl
Terminal=false
Categories=Network;System;Utility;
"
}

fn bundle_linux(root: &Path, override_bin: Option<&str>) -> Result<(), String> {
    let v = version(root)?;
    let bin = match override_bin {
        Some(p) => {
            let pb = PathBuf::from(p);
            if !pb.exists() {
                return Err(format!("--bin not found: {p}"));
            }
            pb
        }
        None => {
            let p = root.join("target/release").join(BIN);
            if !p.exists() {
                return Err(format!(
                    "{} not found (build first or pass --bin)",
                    p.display()
                ));
            }
            p
        }
    };
    // Linux 用アイコン（512）。無ければ汎用の prowl.png を使う。
    let icon = {
        let p512 = root.join("assets/icon/icon_512.png");
        if p512.exists() {
            p512
        } else {
            root.join("assets/prowl.png")
        }
    };

    let arch = if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "x86_64"
    };
    let stem = format!("prowl-{v}-{arch}");
    let stage = dist(root).join(&stem);
    clean_dir(&stage)?;
    let bin_dir = stage.join("bin");
    let apps = stage.join("share/applications");
    let icons = stage.join("share/icons/hicolor/512x512/apps");
    for d in [&bin_dir, &apps, &icons] {
        std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    }
    std::fs::copy(&bin, bin_dir.join(BIN)).map_err(|e| format!("copy bin: {e}"))?;
    std::fs::write(apps.join("prowl.desktop"), desktop_entry()).map_err(|e| e.to_string())?;
    std::fs::copy(&icon, icons.join("prowl.png")).map_err(|e| e.to_string())?;

    let tarball = dist(root).join(format!("{stem}.tar.gz"));
    let _ = std::fs::remove_file(&tarball);
    sh(Command::new("tar").args([
        "-czf",
        tarball.to_str().unwrap(),
        "-C",
        dist(root).to_str().unwrap(),
        &stem,
    ]))?;
    clean_dir(&stage)?;
    println!("bundle-linux: wrote {}", tarball.display());
    Ok(())
}
