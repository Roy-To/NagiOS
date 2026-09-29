# Nagi OS 0.1 Developer Preview — 開発者向けガイド

このガイドは、Nagi OS の開発ホスト準備、ゲストの起動、受け入れ確認、
失敗時の診断方法を案内します。Nagi は QEMU 上で動く独立 OS です。
ホストのファイルシステムやブラウザーをゲスト機能の代わりには使いません。

## 先に確認する資料

- [リポジトリの作業規約](../../AGENTS.md)
- [実装仕様](../Nagi_OS_0.1_Codex_Implementation_Spec.md)
- [現在の実装状態と受け入れ証拠](../implementation_status.md)
- [言語アーキテクチャ](../architecture/language-architecture.md)
- [サードパーティ通知](../../THIRD_PARTY_NOTICES.md)

マイルストーンの判定は `docs/implementation_status.md` と各
`docs/workstreams/` の記録を参照してください。コマンドやライブラリが
存在することだけでは、ゲスト統合やマイルストーンの `PASS` を意味しません。

## ホストの準備と基本操作

Rust ツールチェーンは [`rust-toolchain.toml`](../../rust-toolchain.toml)、
ホスト依存関係と OVMF の候補は [`nagi.toml`](../../nagi.toml) に記録されています。
まずリポジトリのルートで実行します。

macOS で Homebrew 版 Rust も使っている場合は、リポジトリの
`rust-toolchain.toml` を選ぶ rustup shim が優先されるよう PATH を確認し、
`rustup show active-toolchain` がこのリポジトリの pinned toolchain を示すことを
確認してください。

POSIX ホスト:

```sh
./nagi --help
./nagi doctor
./nagi fetch
./nagi build
./nagi test
./nagi fmt
./nagi lint
```

Windows PowerShell:

```powershell
.\nagi.ps1 --help
.\nagi.ps1 doctor
.\nagi.ps1 fetch
.\nagi.ps1 build
.\nagi.ps1 test
.\nagi.ps1 fmt
.\nagi.ps1 lint
```

`doctor` は開発ホストのツールを調べます。`doctor --allow-missing` は
不足依存の診断用で、通常の `doctor` やビルド／受け入れ確認の成功を
代替しません。`fetch` はロックされた外部ソースを取得・検証するため、
ネットワーク接続と大きな空き容量が必要になる場合があります。

`build`、`test`、`lint` はホスト互換 Cargo workspace を対象にし、
`nagi-kernel` を除外します。Nagi ターゲットのカーネル・init・UEFI を
含む既定イメージは `image` が作り、`run` はそのイメージを QEMU で起動します。

```sh
./nagi image
./nagi run
```

公式の参照構成は QEMU x86-64、UEFI/OVMF、q35、4 vCPU、8 GiB RAM、
VirtIO Block/Network/GPU/Sound/RNG です。物理 PC での起動は 0.1 の
受け入れ条件ではありません。

## 対象を絞ったゲスト受け入れ

`./nagi --help` が現在の CLI コマンド一覧です。特に以下は QEMU ゲストを
起動し、リポジトリ内の `out/logs/` にシリアルログを残します。

| コマンド | 確認対象 | 判定上の注意 |
| --- | --- | --- |
| `./nagi m17` | Servo が生成した最初の実フレームを Nagi Surface へ提示 | M17 の表示受け入れであり、Albert 全機能の完成を意味しません。 |
| `./nagi m18` | Albert の QEMU 経由 HTTPS ページ描画 | クリップボード、ダウンロード／アップロード、IME テキスト合成、信頼済みサイト権限 UI などの未接続プロバイダーは別の不足として記録されています。 |
| `./nagi m19` | Search スナップショットの VFS 再マウント／QEMU 再起動後の保持 | 現状のゲスト確認は固定 fixture です。認証済み検索サービスや実 Files/page producer の受け入れではありません。 |
| `./nagi m22` | NH16 History archive、VFS のグループ移動と undo の再起動復旧 fixture | M21 の本番認可済み AI 操作や本番 Activity Ledger との統合を証明しません。 |

現在の実装状態と blocker は上の表だけから判断せず、必ず
[`implementation_status.md`](../implementation_status.md) と対応する
workstream を確認してください。QEMU の受け入れコマンドはビルドを含み、
Servo/Mesa の取得・生成時には時間とディスク容量が必要です。

## SDK サンプル

`samples/hello-nagi` は Rust SDK の小さなサンプルです。CI と同じ手順で
`.napp` と `.xapp` のパッケージ成果物を作れます。

```sh
cargo build --manifest-path samples/hello-nagi/Cargo.toml --offline --locked
cargo run --manifest-path samples/hello-nagi/Cargo.toml --bin hello-nagi-package \
  --offline --locked -- out/artifacts/hello-nagi.napp
cargo run --manifest-path tools/nagi-pkg/Cargo.toml --offline --locked -- \
  build-hello out/artifacts/hello-nagi.napp out/artifacts/hello-nagi.xapp
```

この手順はサンプルのパッケージ成果物を作成します。これだけでは、
任意の外部アプリのインストール、権限付与、実行までを受け入れたことにはなりません。

## 現在の Preview UX と未提供の部分

- 初回起動は QEMU の参照構成を対象とします。`image` で参照イメージを作り、
  `run` で起動できます。現在、利用者向けの初回設定ウィザードや物理 PC 用
  インストーラーの Acceptance はありません。
- `user/nagi-init/src/desktop.rs` の M10 desktop は Calculator、Notes、Files、
  Terminal の固定パネルを描く受け入れ用サーフェスです。完成したアプリランチャー、
  既定アプリ選択、統合 Settings UI の完成を意味しません。
- 言語アーキテクチャは `en-US` と `ja-JP` を同等のユーザー言語として定義します。
  ただし現在の desktop サーフェスは英語の固定ラベルと日本語サンプル文字列を含む
  受け入れ用表示であり、システム言語・地域・入力言語を切り替える完成した Settings
  画面や全画面の翻訳完了は確認されていません。
- Albert の view model には状態を示す localization key がありますが、画面全体の
  アクセシビリティツリーや支援技術との Acceptance はありません。利用可能なキーや
  マウス操作の存在から、アクセシビリティ完成を推定しないでください。
- ブート障害の共通ダイアログ、Recovery Environment の画面、起動時間の測定値は
  まだありません。回復機能は [M27 の記録](../workstreams/NagiOS_M27_AB_Recovery_Workstream.md)
  と実際の acceptance 状態を確認してください。
- この Developer Preview 文書には Nagi の画面キャプチャを含めていません。
  実画面を確認する場合は QEMU を起動してください。シリアルログや静的な受け入れ用
  描画を実際のスクリーンショットとして扱わないでください。

## 未接続プロバイダーを切り分ける

「機能が利用できない」場合は、まずホスト環境の失敗とゲスト側の
プロバイダー未実装を分けてください。

- Model Manager はモデル manifest、取得元 pin、成果物ハッシュ検証、
  provider-neutral runtime 契約を持ちます。現在の実装状態では Nagi 内の
  llama.cpp backend、モデル保管／ロードサービス、Granite 推論は未接続です。
  `fetch` による llama.cpp ソース取得は、ゲスト推論の実装やモデル配布ではありません。
- M21 の AI サービスは検証・実行境界と `file.search` adapter を持ちますが、
  production guest service と認証済み caller policy は未接続です。
  `app.launch`、`file.copy`、`file.move`、`system.volume.set` の本番 handler が
  あるとは扱わないでください。
- M22 の archive fixture が成功しても、それは M21 が許可した AI の
  ファイル変更や本番 Undo ではありません。現在の制約は
  [M22 workstream](../workstreams/NagiOS_M22_AI_Safety_Undo_Integration_Workstream.md)
  を参照してください。
- 起動障害時の A/B rollback と Recovery Environment は、
  [M27 workstream](../workstreams/NagiOS_M27_AB_Recovery_Workstream.md) の
  broken-slot QEMU acceptance と Recovery Environment の完了が確認できるまで、
  利用可能な復旧手段として案内しないでください。
- M23 以降の UI、音声、意味検索、モデル routing、A/B recovery の挙動は、
  該当 workstream と実装状態の受け入れ記録を確認するまで利用可能と案内しないでください。

安定した内部診断名・CLI 名は英語です。ユーザー向け UI 文字列の言語と
フォールバック規則は [言語アーキテクチャ](../architecture/language-architecture.md)
に従います。診断ログの文言を UI 翻訳の代わりに使わないでください。

## 失敗時の診断とログ保全

1. コマンド全体、終了コード、ホスト OS/CPU、`rustc --version`、
   `./nagi doctor` の結果を保存します。
2. CLI が表示した `out/logs/...` のパスを開き、最初の `FAIL` または
   compiler error とその前後のトレースを確認します。
3. 受け入れコマンドで確認する場合は、同じ QEMU 実行のシリアルログと
   `Nagi ... PASS` marker の有無を記録します。PASS marker がない場合は、
   コマンドが終了しただけで受け入れ成功としません。
4. CI で再現した場合は workflow run URL と commit SHA を添えます。
   ローカルの `out/logs/` は CI に自動添付されるとは限りません。

主なログ名は次のとおりです（コマンドやソースにより追加ログもあります）。

- M17: `out/logs/m17-first-boot.log`、`out/logs/m17-servo.log`
- M18: `out/logs/m18-first-boot.log`、`out/logs/m18-albert.log`
- M19: `out/logs/m19-search-bootstrap.log`、`m19-search-initial.log`、
  `m19-search-restart.log`（いずれも `out/logs/` 内）
- M22: `out/logs/m22-history-bootstrap.log` と
  `out/logs/m22-history-boot-*.log`

現在の CLI には仕様書で定義された `nagi diagnose bundle` コマンドは
ありません。利用可能なホスト診断は `./nagi doctor`、ゲスト診断は各受け入れ
コマンドのシリアルログです。診断束が実装済みであるかのように案内しないでください。

`./nagi clean` はリポジトリ内の生成物 `target/` と `out/` を削除します。
ログ、QEMU の永続ディスク、生成した package/image が必要な場合は、
`clean` の前に必要なファイルを別の場所へ保存してください。ソース変更や
`third_party/` の取得済みソースを消すコマンドではありません。

## 公開・共有する前に

Developer Preview は実験用です。パスワード、Cookie、AI 会話、文書本文、
秘密鍵、個人情報を public issue やログ抜粋へ含めないでください。
未完成の機能と再現手順を報告する際は、権限やセキュリティを緩めず、
状態表に記録された blocker をそのまま明示してください。
