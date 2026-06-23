# prowl

[![CI](https://github.com/TomiXRM/prowl/actions/workflows/ci.yml/badge.svg)](https://github.com/TomiXRM/prowl/actions/workflows/ci.yml)

**A no-sudo TUI LAN scanner, written in Rust.**
ネットワークに今なにが繋がっているかを、権限なしでサッと一覧する TUI ツール。

`prowl` はローカルLANの端末を発見し、**IP / MAC / ベンダー / ホスト名 / 開放ポート** を一画面で見せ、
さらに**死活を継続監視**して新規/離脱端末を色で知らせます。`nmap` や `arp-scan` のように root を要求しません。

> ⚠️ **自分が管理している、または許可を得たネットワークだけ**をスキャンしてください。

---

## 特長

- 🔓 **権限不要** — UDP誘発＋OSの近隣テーブル読みでホスト/MACを取得（raw socket不使用）
- 🏷 **フルOUIベンダーDB** — MACから製造元を解決（MA-L/M/S対応・DB同梱）
- 🔎 **名前解決チェーン** — OSリゾルバ(getnameinfo) → mDNS → NetBIOS の best-effort
- 🚪 **ポート/サービススキャン** — 選択ホストを TCP connect でスキャン、サービス名＋バナー取得
- 📡 **継続モニタ** — 一定間隔で自動再スキャン、**新規=緑 / 離脱=赤** でハイライト
- 🖥 **TUI** — `ratatui` 製、非同期でUIが固まらない
- 🧩 **拡張しやすい設計** — 発見/付与/スキャン/フロントをすべてトレイトで差し替え可能

## 操作

| キー | 動作 |
|---|---|
| `↑` `↓` / `j` `k` | ホスト選択 |
| `Enter` / `s` | 選択ホストをポートスキャン |
| `m` | 継続モニタ ON/OFF |
| `r` | 手動で再スキャン |
| `/` | 絞り込み（IP/MAC/ベンダー/名前） |
| `q` | 終了 |

凡例: ` ` 稼働 / `+` 新規(緑) / `×` 離脱(赤)

## ビルド & 実行

```sh
git clone https://github.com/TomiXRM/prowl
cd prowl
cargo run                 # TUI 起動（root 不要）
cargo run -- --web        # Web UI 起動 → http://127.0.0.1:7878
cargo run -- --web --mock # 実NW非依存の決定論モード（デモ/テスト用）
```

```sh
cargo build --release   # 単一バイナリ → target/release/prowl
cargo test --workspace  # Rust テスト
```

対応環境: macOS (Apple Silicon / Intel) + Linux (x86_64 / arm64)、Rust 1.96+。

### Web UI ＆ AI が回せる e2e テスト

`--web` で同じエンジンを **ブラウザ(DOM)** に映す（TUI と同一の `Command`/`AppState` 契約）。
DOM なので **Playwright** で end-to-end に検証でき、`--mock` と組み合わせると実LAN無しで決定論的に回せる：

```sh
cd e2e && npm ci && npx playwright install chromium && npx playwright test
```

## しくみ（ざっくり）

```
 提示の軸(外側)  TUI(ratatui) / Web(DOM, WebSocket) / GPUI(将来)
        │  Command(操作)↓ / AppState(状態)↑   ← UI非依存の契約 (prowl-app)
 prowl-core (エンジン)
        │  Discovery / Enricher / PortScanner トレイト
 データの軸(内側)  発見(近隣テーブル/ARP)・名前(DNS/mDNS/NetBIOS)・OUI・ポートスキャン
```

| クレート | 役割 |
|---|---|
| `prowl-app` | UI非依存の契約（`Command`/`AppState`/`Frontend`） |
| `prowl-core` | スキャンエンジン＋各トレイト実装（UI非依存・単体テスト可能） |
| `prowl-tui` | `ratatui` フロントエンド |
| `prowl-web` | Web(DOM) フロントエンド（axum + WebSocket、Playwright で検証可） |
| `prowl` | 配線してフロントを起動する薄いバイナリ |

拡張ポイントの例:
- IPv6 発見を足す → `Discovery` を実装して登録（既存ARPは無改変）
- OS推定/サービス検出を足す → `Enricher` / `PortScanner` を追加
- GUI を足す → `Frontend` を実装（各フロントが自前ランタイムを持つ方針）

設計の詳細・要件は [`REQUIREMENTS.md`](./REQUIREMENTS.md) を参照。

## ロードマップ

- [x] Web(DOM) フロント＋ Playwright e2e
- [ ] GPUI ネイティブフロント（longbridge gpui-component）
- [ ] OS 推定（TTL/開放ポート指紋）
- [ ] JSON / CSV エクスポート
- [ ] mDNS サービスブラウズ（名前と機種情報の強化）
- [ ] sudo モード（ARP/SYN スキャンによる高速・高精度化）
- [ ] IPv6 対応

## ライセンス

MIT — see [LICENSE](./LICENSE).
