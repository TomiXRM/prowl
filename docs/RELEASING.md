# リリース手順（prowl）

`v*` タグを push すると `.github/workflows/release.yml` が macOS(.app/.dmg) と
Linux(tar.gz) をビルドし、チェックサムと共に **ドラフト** リリースへアップロードします。
公開（ドラフト解除）は手動レビュー後に行います。

```sh
# 例: v0.1.1 を切る
git tag -a v0.1.1 -m "prowl v0.1.1"
git push origin v0.1.1
# ビルド完走後、ドラフトを確認して公開
gh release edit v0.1.1 --draft=false
```

> バージョンは root `Cargo.toml` の `[workspace.package] version` が単一の真実源。
> タグと一致させること（xtask はここを読んでファイル名を決める）。

---

## macOS の署名・公証（初回起動の警告を消す）

ダウンロードした `.dmg` の初回起動で Gatekeeper が警告するのは、ファイルに付く
`com.apple.quarantine` 属性が原因です。これを消せるのは **Developer ID 署名＋公証
(notarize)** だけで、**Apple Developer Program（年 $99）** が必要です。

CI はシークレット駆動の **two-mode** です:

- シークレットが揃っていれば → **Developer ID 署名＋hardened runtime＋entitlements**
  で署名し、`.dmg` を **公証＋staple**。→ **初回警告ゼロ**。
- 無ければ → **ad-hoc 署名**（現状の挙動）。初回 `.dmg` は右クリック→「開く」で回避。

> 補足: アプリ自身の **セルフアップデート**（`prowl --update` / GUI バナー）は、`ureq` で
> 直接DLして差し替えるため quarantine が付かず、**ad-hoc 署名でも無警告**で更新できます。
> 警告が出るのは「ブラウザで初回 `.dmg` を落としたとき」だけです。

### 必要な GitHub Actions シークレット

| シークレット | 中身 | 取得方法 |
|---|---|---|
| `MACOS_CERTIFICATE` | Developer ID Application 証明書(.p12) の **base64** | Keychain から書き出し → `base64 -i cert.p12 \| pbcopy` |
| `MACOS_CERTIFICATE_PWD` | 上記 .p12 のパスワード | 書き出し時に設定した値 |
| `MACOS_SIGN_IDENTITY` | 署名 ID 文字列 | 例: `Developer ID Application: Your Name (TEAMID)` |
| `MACOS_NOTARY_APPLE_ID` | 公証に使う Apple ID（メール） | Apple Developer アカウント |
| `MACOS_NOTARY_PASSWORD` | **App用パスワード** | appleid.apple.com →「App用パスワード」で生成 |
| `MACOS_NOTARY_TEAM_ID` | Team ID（10桁） | developer.apple.com → Membership |

すべて未設定でも CI は通り、**ad-hoc** で `.dmg` を作ります（公証ステップはスキップ）。

### 証明書(.p12)の作り方（概要）

1. Apple Developer で **Developer ID Application** 証明書を作成・ダウンロード。
2. Keychain Access に取り込み、秘密鍵ごと **.p12 で書き出す**（パスワード設定）。
3. `base64 -i DeveloperID.p12 | pbcopy` で base64 化し、`MACOS_CERTIFICATE` に登録。
4. `security find-identity -v -p codesigning` で出る文字列を `MACOS_SIGN_IDENTITY` に。

### ローカルで署名付きビルドを試す

```sh
export MACOS_SIGN_IDENTITY="Developer ID Application: Your Name (TEAMID)"
cargo run -p xtask -- bundle-macos     # Developer ID＋hardened runtime で署名
cargo run -p xtask -- dmg-macos
export MACOS_NOTARY_APPLE_ID=you@example.com
export MACOS_NOTARY_PASSWORD=xxxx-xxxx-xxxx-xxxx   # App用パスワード
export MACOS_NOTARY_TEAM_ID=ABCDE12345
cargo run -p xtask -- notarize-macos   # .dmg を公証＋staple
```

環境変数を設定しなければ `bundle-macos` は **ad-hoc**、`notarize-macos` は **スキップ** します。

---

## セルフアップデート（バイナリ差し替え）

公開済みリリースに対し、アプリが自分自身を最新版へ差し替えます（`crates/prowl-update`）。

```sh
prowl --check-update   # 最新を確認するだけ
prowl --update         # 確認 → DL → SHA-256 検証 → 差し替え → 再起動
```

GUI(GPUI) では起動時に背景で確認し、新版があればヘッダに「⬆ vX.Y.Z に更新」バナーが出ます。

検証の多層防御:
1. GitHub への **HTTPS**（rustls）。
2. ダウンロード物の **SHA-256** をリリースの `SHA256SUMS-*.txt` と照合（不一致なら中止・現状温存）。
3. （署名済みなら）OS の**コード署名/公証**を再起動時に検証。

差し替えはアトミック（ステージングに書いてから rename / 旧を退避してから移動）で、
失敗時は現在のインストールを温存します。macOS は `.dmg`→`prowl.app` を入れ替え、
Linux は `tar.gz` 内の `bin/prowl` を入れ替えます。
