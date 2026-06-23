# prowl — 要件定義書

TUIで扱える prowl 的なネットワークスキャナ（Rust製）。

- ステータス: **P1＋P2＋P3 実装・実機live確認済**。無権限発見＋フルOUI＋名前解決チェーン＋無権限connectポートスキャン(TUI統合)＋**継続モニタ(10s自動再スキャン・死活差分: 新規=緑/離脱=赤、2ミスでDown確定、`m`でON/OFF)**。sudo版ARP/SYNも`Discovery`/`PortScanner`差し替えで将来追加可。さらに **Web(DOM)フロント（`prowl-web`: axum+WebSocket, `--web`）** を追加（方針Aで2フロント目を実証）、`--mock`＋Playwright で e2e を決定論的に検証（CIにe2eジョブあり）。
- ティア: Standard
- 合意形態: 単独依頼者モード（依頼者の確認＝合意）
- 最終更新: 2026-06-24

---

## 1. 目的・前提（合意済み）

| 項目 | 内容 |
|---|---|
| 目的 | 日常使いで「今LANに何が繋がっているか」を即座に把握する実用ツール |
| 利用者 | 依頼者本人 1名（単独依頼者モード） |
| 対象環境 | macOS（Apple Silicon / Intel）+ Linux（x86_64 / arm64） |
| 権限方針 | **デフォルトは無権限**（blink方式: UDP誘発＋OS近隣テーブル読み）。**sudo版ARP**（raw socket）も `Discovery` 差し替えで選択可 |
| 継続モニタ | TUI起動中のみ定期再スキャン（デーモン化なし） |
| 対象プロトコル | **IPv4 のみ**（IPv6 は後フェーズで別Discoveryエンジンとして追加） |
| フロント方針 | **方針(A)**：各フロントが自前ランタイムを持ち、`Command`/`AppState` の契約だけ共有 |

### ガードレール（運用前提）
- スキャン対象は **自分が管理 or 許可を得たネットワークのみ**。第三者NWのスキャンは行わない。
- デフォルトのスキャン対象はローカルサブネットに限定し、起動時に対象範囲を明示する。

---

## 2. 機能要求（FR）

| ID | 要求 | 優先 | フェーズ |
|---|---|---|---|
| FR-01 | 起動時にローカルNICからスキャン対象サブネットを自動検出（手動指定も可） | Must | P1 |
| FR-02 | 生存ホストの IP/MAC を列挙（無権限=`PingNeighborDiscovery` / sudo=`ArpDiscovery` の2バックエンド） | Must | P1 |
| FR-03 | ホスト名解決（逆引きDNS→mDNS→NetBIOS の best-effort チェーン） | Must | P1 |
| FR-04 | MAC OUI からベンダー名を表示（`mac_oui` 同梱フルDB=MA-L/M/S対応、実機live確認済） | Must | P1 |
| FR-05 | TUI一覧：ソート・インクリメンタル絞り込み（IP/MAC/ベンダー/名前） | Must | P1 |
| FR-06 | 選択ホストのポートスキャン（無権限=`ConnectScanner` TCP connect 並列 / 将来sudo SYN） | Should | **P2済** |
| FR-07 | 開放ポートのサービス/バナー検出（port→service表＋SSH/HTTP等のバナー取得） | Should | **P2済** |
| FR-08 | TTL/指紋からのざっくりOS推定 | Could | P2 |
| FR-09 | TUI起動中の定期再スキャン（10秒間隔・`m`でON/OFF） | Could | **P3済** |
| FR-10 | 差分検知：新規(緑+)/離脱(赤×, 2ミス確定)ホストをハイライト | Could | **P3済** |
| FR-11 | 結果を JSON/CSV にエクスポート | Could | P2 |

---

## 3. 非機能要件（NFR・数値）

| ID | 項目 | 目標値 |
|---|---|---|
| NFR-01 | ホスト発見速度 | /24（254アドレス）を **≤ 3秒**（ARP, sudo） |
| NFR-02 | ポートスキャン速度 | 1ホスト top-1000 ポートを **≤ 5秒**（SYN, 並列） |
| NFR-03 | UI応答性 | キー操作応答 **≤ 100ms**、スキャンは非同期でUIをブロックしない |
| NFR-04 | リソース | アイドル時メモリ **≤ 50MB** 目安 |
| NFR-05 | 対応環境 | macOS(Apple Silicon/Intel) + Linux(x86_64/arm64) |
| NFR-06 | 権限の扱い | 非特権起動時は必要権限を明示し、graceful degrade または明確に終了 |
| NFR-07 | 配布 | 単一軽量バイナリ（`cargo install` / バイナリ配布）。デフォルトビルドはTUIのみで軽量 |

---

## 4. 制約・アーキテクチャ方針（C）

| ID | 制約 |
|---|---|
| C-01 | 発見/付与/スキャンを trait（`Discovery`/`Enricher`/`PortScanner`）で抽象化し、コアを **TUI非依存ライブラリ**（`prowl-core`）に分離する |
| C-02 | `Command`（操作）/`AppState`（状態）の **契約層**（`prowl-app`）を介してフロントを差し替え可能にする。**P1からこの境界を必ず作る**（GPUIは作らないが、TUIもこの境界越しに実装する） |
| C-03 | TUIフロントは P1から **`watch<AppState>` 駆動＋コンポーネント(TEA)構造** で実装し、将来のリアクティブ・ランタイム化（tui-realm風 or 自前）を妨げない。**フルUIフレームワーク自作（=両バックエンド共有の単一UIランタイム）はスコープ外** |

### 4.1 拡張軸は2本（どちらもUI非依存コアにぶら下がる）

```
        ┌─────────────── 提示の軸（外側）───────────────┐
        │   TUI(ratatui)   GPUI(将来)   web/--json(将来) │
        └──────────────────┬───────────────────────────┘
              Command（操作）↓ / AppState（状態）↑  ← UI非依存の契約
        ┌──────────────────┴───────────────────────────┐
        │            prowl-core（エンジン）           │
        └──────────────────┬───────────────────────────┘
        ┌─────────────── データの軸（内側）─────────────┐
        │  Discovery: ARP / 将来NDP   Enricher: OUI/DNS │
        │  PortScanner …                                │
        └───────────────────────────────────────────────┘
```

- **内側の軸** = `Discovery`/`Enricher`/`PortScanner` トレイト（IPv6・OS推定などを足す口）
- **外側の軸** = フロントエンド（TUI/GPUI/…を足す口）。方針(A)：各フロントが自前ランタイムを持ち、契約だけ共有
- メッセージパッシング境界により、各フロントの内部実装（素朴ループ／リアクティブ・ランタイム）はコアから完全に隠蔽される

### 4.2 クレート構成（ワークスペース）

```
prowl/                    ← workspace
  crates/
    prowl-core/           ← エンジン本体。UIを一切知らない（pnet等はここに閉じ込め）
      model.rs              ← Host / MacAddr / Vendor / PortState …
      engine.rs             ← Discovery+Enricher を束ね、状態を更新
      discovery/ arp.rs …   ← 内側の軸（trait Discovery）。将来 ipv6_ndp.rs を追加
      enrich/    oui.rs dns.rs   ← 将来 os_guess.rs を追加
      scan/      (P2)
      export/    (P2)
    prowl-app/            ← UI非依存の契約層：Command / Event / AppState / EngineHandle
    prowl-tui/            ← ratatui フロント（trait Frontend 実装）
    prowl-gpui/           ← 【P4/Future】GPUIフロント（feature gate・重い依存はここに隔離）
  src/main.rs (prowl bin) ← ランタイム起動＋フロント選択だけ行う薄い起動口
```

### 4.3 契約スケッチ

```rust
// prowl-app: UI非依存の契約
pub enum Command {            // ユーザの意図（各フロントが自分の入力をこれに翻訳）
    Rescan, SelectHost(HostId), ScanPorts(HostId),
    SetFilter(String), Quit,
}
pub struct AppState {         // 画面に出す“今の全状態”（フロントはこれを描くだけ）
    pub hosts: Vec<HostRow>,
    pub selected: Option<HostId>,
    pub scanning: bool,
    pub filter: String,
}
pub struct EngineHandle {
    pub commands: mpsc::Sender<Command>,      // 操作を投げる
    pub state:    watch::Receiver<AppState>,  // 最新状態を読む
}

// 各フロントは「ハンドルを受け取って自分のループを回す」だけ
pub trait Frontend {
    async fn run(self, engine: EngineHandle) -> anyhow::Result<()>;
}

// 内側の軸（コア側プラグイン）
pub trait Discovery {
    async fn discover(&self, target: Subnet, tx: Sender<Host>) -> anyhow::Result<()>;
}
pub trait Enricher {
    async fn enrich(&self, host: &mut Host);
}
pub trait PortScanner {   // P2
    async fn scan(&self, host: &Host, ports: &PortSet) -> Vec<PortState>;
}
```

---

## 5. 技術スタック案（設計寄り・P1着手時に確定）

| 領域 | 候補 |
|---|---|
| TUI | `ratatui` + `crossterm` |
| 非同期ランタイム | `tokio` |
| raw socket / ARP / SYN | `pnet`（libpnet） |
| 名前解決 | `hickory-resolver`（旧 trust-dns）＋ mDNS |
| ベンダーDB(OUI) | ビルド時埋め込み |
| エラー処理 | `anyhow` / `thiserror` |

---

## 6. フェーズ計画

| フェーズ | 中身 | MoSCoW |
|---|---|---|
| **P1 (MVP)** | ホスト発見（IP/MAC/名前）＋ベンダー＋TUI一覧/絞り込み。`core`/`app`/`tui` の3クレート境界を確立 | Must |
| **P2** | ポート/サービススキャン、OS推定、JSON/CSVエクスポート | Should/Could |
| **P3** | 継続モニタ＆差分（TUI起動中の定期再スキャン・新規/離脱ハイライト） | Could |
| **P4 (Future)** | GPUIフロントエンド（`prowl-gpui` を `Frontend` 実装として追加） | Could |

---

## 7. P1 受入条件（「できた」の定義）

1. `sudo` 付き起動で自分のLANサブネットを自動検出して表示する
2. 数秒以内に生存ホストが **IP / MAC / ホスト名 / ベンダー** で一覧表示される
3. 一覧をキーでソート＆インクリメンタル絞り込みできる
4. 非特権起動時は必要権限が分かるメッセージが出る
5. macOS と Linux 両方でビルド＆起動できる
6. `prowl-core` が TUI に依存せず、単体（ヘッドレス）でビルド・テストできる

---

## 8. 未決事項台帳

| ID | 内容 | 扱い |
|---|---|---|
| U-01 | IPv6 対応 | P1スコープ外。`Discovery` トレイト経由で後フェーズに別発見エンジン（リンクローカル `ff02::1` ping＋近隣キャッシュ）として追加 |
| U-02 | OUIデータの同梱方法（ビルド時埋め込み vs 外部更新） | P1着手時に決定 |
| U-03 | OS推定のアルゴリズム精度方針 | P2で詰める |
| U-04 | フロント選択をコンパイル時feature にするか実行時`--ui`フラグにするか | P1はTUI単独のため実装時に確定 |

いずれも P1 着手をブロックしない。

---

## 9. 実装前ゲート（GO/NO-GO）

| # | 判定 | 状態 | 根拠 |
|---|---|---|---|
| 1 | 目的紐付け（critical） | ✅ | 全FRが「日常的にLAN接続端末を把握」に紐づく |
| 2 | 合意（critical） | ✅ | 単独依頼者モード（依頼者の承認＝合意） |
| 3 | 要件化（critical） | ✅ | 本書で要求一覧・NFR・制約を確定（未カバー要求0／孤立機能0） |
| 4 | 検証・妥当性 | ✅ | P1は受入条件で妥当性確認可。検証は実装中に継続 |
| 5 | 未決事項 | ✅ | 台帳化済み・どれもP1着手をブロックしない |

**判定：GO（P1 着手可）**
