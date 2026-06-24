//! gpui-component が参照するアイコン(SVG)を供給する [`AssetSource`]。
//!
//! gpui-component 0.5.1 は SVG を同梱しない（`IconName::ChevronDown` 等は
//! `icons/<name>.svg` を**アプリのアセット**から読む）。登録が無いと Select の矢印や
//! 通知アイコンが空になる。ここで使うアイコンだけ Lucide の SVG を埋め込んで返す
//! （未登録パスは `None` ＝ 描画スキップ）。

use std::borrow::Cow;

use anyhow::Result;
use gpui::{AssetSource, SharedString};

/// Lucide 風 SVG（24x24・stroke=currentColor）。gpui はアルファマスクとして描画し、
/// 要素の `text_color` で着色するので stroke ベースで問題ない。
macro_rules! lucide {
    ($inner:literal) => {
        concat!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">"#,
            $inner,
            "</svg>"
        )
    };
}

/// `icons/<name>.svg` → 埋め込み SVG。使うものだけ列挙する。
const ICONS: &[(&str, &str)] = &[
    (
        "icons/chevron-down.svg",
        lucide!(r#"<path d="m6 9 6 6 6-6"/>"#),
    ),
    ("icons/check.svg", lucide!(r#"<path d="M20 6 9 17l-5-5"/>"#)),
    (
        "icons/circle-check.svg",
        lucide!(r#"<circle cx="12" cy="12" r="10"/><path d="m9 12 2 2 4-4"/>"#),
    ),
    (
        "icons/circle-x.svg",
        lucide!(r#"<circle cx="12" cy="12" r="10"/><path d="m15 9-6 6"/><path d="m9 9 6 6"/>"#),
    ),
    (
        "icons/close.svg",
        lucide!(r#"<path d="M18 6 6 18"/><path d="m6 6 12 12"/>"#),
    ),
    (
        "icons/info.svg",
        lucide!(r#"<circle cx="12" cy="12" r="10"/><path d="M12 16v-4"/><path d="M12 8h.01"/>"#),
    ),
    (
        "icons/triangle-alert.svg",
        lucide!(
            r#"<path d="m21.73 18-8-14a2 2 0 0 0-3.48 0l-8 14A2 2 0 0 0 4 21h16a2 2 0 0 0 1.73-3"/><path d="M12 9v4"/><path d="M12 17h.01"/>"#
        ),
    ),
    (
        "icons/search.svg",
        lucide!(r#"<circle cx="11" cy="11" r="8"/><path d="m21 21-4.3-4.3"/>"#),
    ),
    (
        "icons/inbox.svg",
        lucide!(
            r#"<polyline points="22 12 16 12 14 15 10 15 8 12 2 12"/><path d="M5.45 5.11 2 12v6a2 2 0 0 0 2 2h16a2 2 0 0 0 2-2v-6l-3.45-6.89A2 2 0 0 0 16.76 4H7.24a2 2 0 0 0-1.79 1.11z"/>"#
        ),
    ),
];

/// gpui-component のアイコンを供給するアセット源（`Application::with_assets` に渡す）。
pub struct Assets;

impl AssetSource for Assets {
    fn load(&self, path: &str) -> Result<Option<Cow<'static, [u8]>>> {
        Ok(ICONS
            .iter()
            .find(|(p, _)| *p == path)
            .map(|(_, svg)| Cow::Borrowed(svg.as_bytes())))
    }

    fn list(&self, _path: &str) -> Result<Vec<SharedString>> {
        Ok(ICONS.iter().map(|(p, _)| SharedString::from(*p)).collect())
    }
}
