//! prowl-update — GitHub Releases 経由のセルフアップデート。
//!
//! 2層構成:
//! - [`domain`]（純粋）: バージョン解析/比較・release JSON パース・アセット選定・
//!   チェックサム照合。I/O 無しで単体テスト可能。
//! - [`io`]（`ureq` + fs）: 最新リリース取得・DL・SHA-256 検証・アトミック差し替え・再起動。
//!
//! チェックは best-effort（失敗は静か）、インストールは常に確認済み・検証付き・
//! アトミックで破壊的コマンドを伴わない。CLI と GPUI フロントの双方から使う。

mod domain;
pub mod io;

pub use domain::{
    find_checksum, parse_release_json, pick_asset, plan_update, Asset, ReleaseInfo, UpdatePlan,
    Version,
};
pub use io::{check_for_update, check_latest, current_version, install, Relaunch};
