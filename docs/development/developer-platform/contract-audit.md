# NDP-02 / NDP-08 接続調査 — 2026-11-15

状態: PARTIAL。実コードの読取結果であり、SDK/CLI ownerとの合意や新しい公開契約ではない。実装言語・内部構成を担当内で選べることと、正規SDKの配送方式を新設できることは分ける。

## NDP-02: 生成前に確定できること

- [SDK定義](../../../sdk/rust/Cargo.toml)はnagi-sdk 0.1.0、publish=false、nagi-modelへの相対path依存。[hello-nagi](../../../samples/hello-nagi/Cargo.toml)もrepo内のpath参照。公開package名や外部app向けGit pin方式は生成しない。
- [App Contract v1](../../../sdk/rust/docs/APP_SDK_CONTRACT.md)はhostで検査できる型であり、process起動・IPC接続・Capability付与を提供するruntimeではない。既存のM16 .xapp ManifestとJSON v1を混同しない。
- 正本は[JSON schema](../../../sdk/rust/schemas/app-manifest.schema.json)と[実装済みloader](../../../tools/nagi-pkg/src/app_manifest.rs)。schemaVersion=1、SDK contractはmajor=1/minor=0。入力上限は1 MiB。未知のroot field、危険なpackage相対entry、互換性のない版、重複宣言等を拒否する。
- 必須fieldはschemaVersion、sdkContractVersion、id、version、origin、displayName、supportedLocales、entrypoint、resources、intents、requestedCapabilities、stateCompatibility。これは調査時の要約であり、正本schema/typed validationを置換しない。
- 初回第三者appの候補はorigin=third-party、空のintents/resources/requestedCapabilities、portableまたはnativeの明示entry、en-US/ja-JPの表示名、state版1。最小Manifestは現行validatorで未検証なので「受理済みfixture」として出力しない。宣言は権限を付与しない。

NDP-03へ渡す前に必要なのは、SDK ownerが確認したout-of-tree参照・固定方法と、実validatorが受理する最小例。SDK/Manifest/IDLを二重実装して不足を埋めない。現環境にはCargoも既存nagi-pkg binaryもなく、Rust依存導入範囲は未指定のため、この部分は未実行。

## NDP-08: 正規toolの利用表

照合対象は新しい main の上に作り直した `codex/ndp-host-integration-v2`（NDP-08c の作業 branch で照合）。下表の終了値はコードから確認したもので、今回の照合で各コマンドは実行していない（NDP-08c でコードを読み取り照合し直した）。

`./nagi` の各行に共通して、shell・Cargo・Rust toolchain と、読める Nagi checkout（`nagi.toml`、`rust-toolchain.toml`、`third_party/sources.lock`）が必要。wrapper は `cargo run --locked --offline` で CLI をビルド・実行する。`--offline` のため wrapper 自身は依存を取得せず、キャッシュ未準備の Cargo 失敗は CLI の終了値に到達しない（値は未確認）。`fetch`・`dev`・`legal` は `tools/nagi-bootstrap/Cargo.toml`（`nagi-cli` と同じソースを別 lockfile でビルド）、それ以外は `-p nagi-cli` 経由。Cargo が見つからない場合は wrapper が `4` で終了する。CLI は引数不正を `2`、プロジェクト設定エラーを `4` で返し、`NAGI_RUN_RECEIPT_DIR` 等で有効化した場合は実行記録も書く。根拠は `nagi` (`--offline`, `nagi-bootstrap`)、`tools/nagi-cli/src/config.rs` (`validate_project`)、`tools/nagi-cli/src/main.rs` (`take_from_env_at_startup`)、`tools/nagi-cli/src/run_receipt.rs` (`NAGI_RUN_RECEIPT_DIR`)。

Stage 1 は静的検査の段階。この表で使うのは準備済みの信頼できる `nagi-pkg` を固定引数で呼ぶ Manifest 検証のみで、入力の build script・entrypoint・任意コマンドの実行、Cargo 経由の起動、依存取得は行わない。Stage 2 は許可された host コマンドと環境に限る。Stage 3 は実在する対象、個別の実行許可、既存の release Gate が揃った場合に限る。現行 NDP-09 の接続は Manifest 検証と承認済みの自作 C fixture の host テストまでで、Stage 3 は未接続。[PT-05 の Stage 契約](../../product-decisions/DEC-045-nagi-porting-toolkit.md)・[NDP-09 adapter](../../../tools/nagi-developer-platform/validation/README.md)

**NDP-08 は PARTIAL。CLI / Package owner の確認待ち。**

- CLI owner — NDP-09 が依存できる CLI の版・終了値・出力契約、承認対象とする host コマンド、Linux target で使う起動・受入 entry と条件（未確認）。
- Package owner — 準備済み validator の固定・提供方法、Manifest の終了値・出力の互換性、`.xapp` の構造・署名検査に使える入口、正式署名・install の手順と権限（未確認）。

**NDP-08c の変更理由（新しい main への追従）:** `./nagi image` は撤去済み（新しい main で `image` subcommand・`image.rs`・`registry_source.rs` を削除）のため、行は消さず「撤去済み」として残す。`./nagi fetch` は Servo checkout・Cargo fetch・patch 適用をやめ、smoltcp 固定情報の検査と font 取得だけになった。`./nagi run` は QEMU を直接組み立てず、準備済み Linux 開発 bundle を `tools/linux-boot/run_dev_image.py` で起動する。新しい subcommand のうち Stage 判断に関係する `verify`・`smoke`・`diagnostics`・`dev`・`clean` を追加した。

| 正規entry | 入力 | 出力 | 終了状態 | 副作用 | 依存・実行許可 | Stage | 根拠（repo 相対パスと識別子・flag） |
|---|---|---|---|---|---|---|---|
| `./nagi doctor [--allow-missing]` | host の `PATH`・既定探索先、OVMF、`nagi.toml` の要件 | 各依存の PASS / WARN / FAIL と件数 | `0` 失敗なし、`10` 検査失敗、`4` 設定不備、`2` 引数不正。`--allow-missing` は未検出を警告にする | CLI はツールの `--version` を実行して OVMF ファイルを探す。wrapper のビルド副作用あり | 共通条件と、信頼済み host ツールを探して起動する許可 | 2・3 の環境準備。1 では使用しない | `nagi` (`--locked`)<br>`tools/nagi-cli/src/commands.rs` (`execute_doctor`, `--allow-missing`)<br>`tools/nagi-cli/src/doctor.rs` (`run_doctor_with_requirements`, `run_candidate`) |
| `./nagi fetch` | `third_party/sources.lock`（smoltcp の固定 version・revision・source_hash・license 行を検査）と `third_party/fonts.lock` | フェーズ進捗（stderr）、成功時は stdout に `PASS fetch: verified pinned smoltcp metadata and prepared fonts` | `0` 成功。sources.lock の読込不可・smoltcp 固定項目の不足・font の取得／検証エラーはすべて `4`。引数追加は `2`。Cargo の終了値は引き継がない | smoltcp 自体・Servo・Cargo 依存は取得せず patch も適用しない。`out/cache/fonts/` の各固定 font をサイズと SHA-256 で検証し、欠落または検証不一致のものは `curl`（https のみ）で `.partial` に再取得、検証後に配置（既存の不一致ファイルは置換）して `manifest.tsv` を書く。キャッシュが揃っていても破損していれば通信と置換が起こる。`manifest.tsv` は全 font が検証済みでも毎回 `fs::write` で上書きし、シンボリックリンクを辿るため、実行前から書込み可能な別ファイルへのリンクならリンク先を上書きしたまま `0` で終わる（`out`・`out/cache`・`out/cache/fonts` がリンクの場合も、作成・取得・書込みはリンク先のディレクトリで起こる） | 共通条件、font の欠落または検証不一致時の curl と取得先への通信、キャッシュの書込み・置換の許可。実行前に `out`・`out/cache`・`out/cache/fonts` が実ディレクトリで、`manifest.tsv` が存在しないか通常ファイルである（シンボリックリンクでない）ことを確認する | 2・3 用の font 準備を別途許可された場合のみ。1 では通信・書込みのため使用しない | `nagi` (`nagi-bootstrap`)<br>`tools/nagi-bootstrap/Cargo.toml` (`nagi-bootstrap`)<br>`tools/nagi-cli/src/commands.rs` (`execute_fetch`, `ensure_font_cache`, `EXIT_CONFIG_ERROR`)<br>`tools/nagi-cli/src/fetch_progress.rs` (`phase`)<br>`tools/nagi-cli/src/fonts.rs` (`ensure_font_cache`, `verify_font_file`, `FONT_LOCK`, `FONT_MANIFEST`, `create_dir_all`, `fs::write`, `curl`, `partial`) |
| `./nagi build` | Nagi checkout の host package、Cargo 設定と lockfile | ビルド成果物、PASS 要約または Cargo の失敗診断 | `0` 成功、Cargo の非ゼロ値を引き継ぐ（起動不能や終了値なしは `4`） | `cargo build --locked` でビルドスクリプト等を実行し得る。ビルド先やキャッシュに書込み。依存未準備なら取得が起こり得る | 共通条件と対象 package の依存、承認済み checkout・Cargo 実行と書込みの許可 | 2。1 では使用しない | `tools/nagi-cli/src/commands.rs` (`Command::Build`, `host_workspace_args_for_arch`, `--locked`, `finish_tool`) |
| `./nagi image` | 撤去済み（新しい main で削除）。subcommand 一覧に `image` はなく、実行すると `unknown command` | stdout に `FAIL unknown command` 行のみ | `2`（未知 command）。成功する経路はない | なし。FAT12 イメージ生成・target ビルドはこの CLI から行わない | 依存として使わない。Linux 開発イメージの準備は `./nagi` の外（`tools/linux-boot/make_dev_bundle.py`。内容は未照合） | Stage 判断に使用しない。再追加の有無は CLI owner に確認 | `tools/nagi-cli/src/commands.rs` (`parse_command`, `EXIT_USAGE`)<br>`tools/linux-boot/run_dev_image.py` (`make_dev_bundle.py`) |
| `./nagi run` | 準備済み bundle ディレクトリ `out/linux-dev`（`bundle.json`、kernel、initramfs、読取専用 System image、書込可 Data image）、`tools/linux-boot/run_dev_image.py`、Python インタプリタ（Windows は `python`、その他の OS は `python3`）、QEMU（既定 `qemu-system-x86_64`、`NAGI_QEMU` で変更） | QEMU ウィンドウとシリアルを stdio へ継承。成功時 stdout に `run: QEMU exited normally`、失敗時は `FAIL run: ...` | `0` スクリプトが `0` で終了。スクリプトの欠落・起動不能・非ゼロ終了（bundle 不備や QEMU 未検出のスクリプト終了値 `2` を含む）は一律 `4`。引数追加は `2` | bundle の SHA-256（kernel・initramfs・System image）を検証し、QEMU を network なし（`-nic none`）で起動。System image は読取専用、Data image は書込み可で実行間の内容が残る。bundle の作成・署名・ビルドはしない。`load_bundle` は末端の `linux-dev` と各ファイルがシンボリックリンクでないことしか確かめないため、`out` がリンクならリンク先の bundle を受理する。Data image が bundle 外のファイルへのハードリンクでも拒否されず（同一ファイル検査の相手は bundle 内の読取専用入力3件のみ）、どちらの場合もゲストの書込みで承認範囲外のファイルが変わり得る | 共通条件、Python インタプリタ（Windows は `python`、その他は `python3`。見つからなければ起動不能で `4`）、QEMU x86-64、bundle の事前準備、VM 実行・Data image 変更の許可。実行前に `out` が実ディレクトリで、Data image が承認済み bundle 内の、ほかと共有されていない（リンク数 1 の）通常ファイルであることを確認する | 3 の Linux 開発イメージの対話起動。移植対象 app の受入 entry・自動判定は未確認（ゲスト受入マーカーの検査はない） | `tools/nagi-cli/src/commands.rs` (`execute_run`, `run_dev_image.py`, `python`, `python3`, `EXIT_CONFIG_ERROR`)<br>`tools/linux-boot/run_dev_image.py` (`load_bundle`, `is_symlink`, `samefile`, `qemu_argv`, `qemu-system-x86_64`, `-nic`) |
| `./nagi test` | Nagi checkout の host package とテスト、Cargo 設定と lockfile | PASS 要約または Cargo の失敗診断 | `0` 成功、Cargo の非ゼロ値を引き継ぐ（起動不能や終了値なしは `4`） | `cargo test --locked` でビルドしたテストと該当する doctest を実行。ビルド先やキャッシュに書込み。依存未準備なら取得が起こり得る | 共通条件と対象 package の依存、承認済み checkout・ビルド・テスト実行と書込みの許可 | 2。1 では使用しない | `tools/nagi-cli/src/commands.rs` (`Command::Test`, `host_workspace_args_for_arch`, `--locked`, `finish_tool`) |
| `./nagi test --acceptance [options]` | `tests/acceptance/registry.tsv` と登録済みケース。`--host-only` / `--target-only` 等で選択 | ケース結果・集計、`--ci` / `--json` で JSON、`--list` で一覧 | `0` 選択ケースに FAIL / SKIP / BLOCKED なし（一覧・ヘルプ成功も `0`）、`11` いずれかあり、`2` 引数不正・選択なし、`4` registry・報告等のエラー | 登録済みスクリプトを実行しログを書込む（`--list` は一覧のみ）。追加の副作用はケースに依存し、この表では未確認 | 共通条件、登録済みスクリプトの実行・レポート書込み許可と選択ケースの依存 | 承認済み host ケースは 2、target ケースは 3。1 では使用しない | `tools/nagi-cli/src/commands.rs` (`--acceptance`, `Command::Acceptance`)<br>`tools/nagi-cli/src/acceptance.rs` (`REGISTRY_PATH`, `run_case`, `--host-only`, `--target-only`, `EXIT_ACCEPTANCE_FAILURE`) |
| `./nagi verify` / `./nagi smoke [--host-only]` / `./nagi diagnostics`（共通 option: `--json`、`--format text` または `--format json`、`--output PATH`。`verify`・`diagnostics` は `--scope ID`） | repo 内の必須ファイル、`docs/testing/diagnostic-report.schema.json`、`.dev/workstreams.json`（ある場合）、host ツールの `--version` 探索、`git rev-parse HEAD` | 検査ごとの PASS / FAIL / SKIPPED。text または JSON を stdout へ。`diagnostics` は安全化したイベントを加えた bundle | `0` 全体 PASS、`11` PASS 以外（FAIL・NOT RUN を含む）、`2` option 不正、`4` 設定・レポート書込みエラー | 既定は読取のみ。`--output PATH` 指定時はそのファイルを書込む（既存は上書き）。書込みは結果が不合格（終了値 `11`）でも行われ、指定先が書込み可能な別ファイルへのシンボリックリンクならリンク先を切り詰めて書く。host の探索で外部ツールを起動する | 共通条件、host ツール探索・git 実行の許可、`--output` を使う場合は書込み許可と、実行前に出力先が存在しないか通常ファイルであり、親ディレクトリが承認済みの実ディレクトリであることの確認 | 2 の host 事前検査。ツールと git を起動するため 1 では使用しない | `tools/nagi-cli/src/commands.rs` (`execute_verify`, `execute_smoke`, `execute_diagnostics`, `parse_report_options`, `report_command_result`, `EXIT_VERIFY_FAILURE`, `--host-only`) |
| `./nagi dev status` / `dev resume` / `dev verify` / `dev diagnose --stage ID --exit-code N --log PATH …` / `dev fingerprint …` | `.dev/workstreams.json` と各 state ファイル、git の状態、`diagnose` は指定ログ・artifact、`fingerprint` は指定入力 | status / resume / verify は行テキスト。`diagnose` は JSON レポート、`fingerprint` は digest または JSON | `0` 成功、`2` usage 不正、`4` 登録・state・git・書込みの不備（`fingerprint` は専用の終了値で、この表では未確認） | `git` を起動。`diagnose --output` と `fingerprint --output` は既存を上書きしない新規ファイル作成。ほかは読取のみ | 共通条件、git 実行の許可、出力先書込み許可。`nagi-bootstrap` 経由 | 2 の host 状態確認と記録。ビルド・実行は行わない | `nagi` (`nagi-bootstrap`)<br>`tools/nagi-cli/src/commands.rs` (`execute_dev`, `fingerprint`)<br>`tools/nagi-cli/src/development.rs` (`execute`, `diagnose`, `REGISTRY_PATH`) |
| `./nagi clean` | Nagi checkout | stdout に `PASS clean: removed N repository-owned output path(s)` | `0` 成功、`4` 失敗（シンボリックリンクや非ディレクトリは削除を拒否）、引数追加は `2` | checkout 直下の `target/` と `out/`（`out/linux-dev` の bundle・font cache を含む）を削除 | 共通条件と、生成物を失う削除の許可 | 2 でも既定では使用しない。必要な場合は個別に許可 | `tools/nagi-cli/src/commands.rs` (`execute_clean`)<br>`tools/nagi-cli/src/paths.rs` (`clean_owned_outputs`) |
| `nagi-pkg manifest validate <manifest.json>` | JSON Manifest v1（1 MiB 以下、SDK contract 1.0） | stdout に PASS・入力パス・AppId・SDK contract 版、拒否理由は stderr | `0` メタデータ受理、`2` 引数不正・入力読込／検証失敗 | 検証器は JSON を読み取る。アプリ起動・ビルド・書込み・取得なし | 準備済み `nagi-pkg` の実行と指定 Manifest の読取許可。現行 adapter は入力を限定して一時コピーを渡す | 1。validator 自体の Cargo ビルドは事前準備 | `tools/nagi-pkg/src/main.rs` (`manifest_command`, `usage`)<br>`tools/nagi-pkg/src/app_manifest.rs` (`load_manifest`, `MAX_APP_MANIFEST_BYTES`)<br>`tools/nagi-developer-platform/validation/validate.py` (`VALIDATOR`) |
| `nagi-pkg info <package>` | 既存 `.xapp` ファイル | stdout に Manifest の ID・版・実行部のバイト数 | `0` 解析成功、`2` パス引数なし。読込・解析失敗は panic（数値は未確認） | ファイル全体を読み取って構造を解析。署名の信頼判定や install はしない | 準備済み `nagi-pkg` の実行と Package の読取許可 | 2 の Package 構造検査候補。1 の adapter 対象外 | `tools/nagi-pkg/src/main.rs` (`info`, `PackageView::parse`)<br>`user/nagi-package/src/lib.rs` (`PackageView`, `is_signed`) |
| `nagi-pkg build-hello <napp> [output]` | NAPP サンプル、組み込み SDK Manifest、任意の出力パス | 未署名 `.xapp`（既定 `out/artifacts/hello-nagi.xapp`）、PASS と AppId | `0` 成功、`2` 引数不正・NAPP 不適合・生成検証失敗。I/O 等の panic の数値は未確認 | 親ディレクトリを作成して Package を書込み、その後に SDK identity を確認する | 準備済み `nagi-pkg` の実行、サンプル読取と出力先書込みの許可 | 2 のサンプル Package 生成候補 | `tools/nagi-pkg/src/main.rs` (`build-hello`, `build_hello`, `HELLO_MANIFEST`) |
| `nagi-pkg build-signed-hello <napp> [output]` | NAPP サンプル、組み込み SDK Manifest、任意の出力パス | Developer Preview テスト署名付き `.xapp`（既定 `out/artifacts/hello-nagi.xapp`）、PASS と AppId | `0` 成功、`2` 引数不正・NAPP 不適合・生成検証失敗。I/O 等の panic の数値は未確認 | 組み込みテスト鍵で署名し、書込み後に SDK identity と署名を確認する | `build-hello` の依存と、テスト署名付き成果物を作る許可 | 2 のサンプル Package 生成候補 | `tools/nagi-pkg/src/main.rs` (`build-signed-hello`, `build_hello`, `DEVELOPER_PREVIEW_SIGNING_SECRET`) |
| `nagi-pkg build-signed <manifest> <elf> <output>` | `.xapp` 用の行形式 Manifest（`grant=` 等）、Nagi ELF、出力パス | Developer Preview テスト署名付き `.xapp`、PASS・AppId・grant 件数 | `0` 成功、`2` 引数不正・ELF マジック不一致・生成検証失敗。I/O 等の panic の数値は未確認 | 必要なら `entry=` をメモリ上で補完し、Package を生成・テスト署名して書込む | 準備済み `nagi-pkg` の実行、入力読取、出力先書込みとテスト署名の許可 | 2 の Package 生成候補。正式署名と install の手順は未確認 | `tools/nagi-pkg/src/main.rs` (`build-signed`, `build_signed`, `DEVELOPER_PREVIEW_SIGNING_SECRET`) |

`build` と通常の `test` は x86-64 で `--workspace`、その他の host ではコードに列挙された package を対象とする。`./nagi fmt`・`lint`・`legal` は今回の照合対象外（表に含めない）。準備済みバイナリを使う Stage 1 とそのビルド手順は [adapter README](../../../tools/nagi-developer-platform/validation/README.md) を参照。Package 生成や host テストの成功から、正式署名・install・権限付与・Nagi target 受入は判定しない。

## 残る接続

- PT-01: 別案とPR #91の正規interface選択が未確定。NDP-04/05で別scanner/report正本を作らない。
- UI: NDP-06は構造事実の抽出まで実装済み。NDP-07はUI ownerとの接続・mappingの根拠確認が残る。
- 環境: Cargo/Rustと標準host Python 3.10以上の導入範囲、共有pathの登録・統合先、push先は未指定。
- レビュー: 現在の[DEC-051方式A](../../product-decisions/DEC-051-review-methods.md)による別担当レビューと修正後の再レビューを実施済み。[記録](../../../reviews/ndp-host-pilots.md)を参照。旧DEC-049の全レーンDeepSeek必須を新しい停止条件にしない。

## NDP-11: UI-01の限定run

入力は自作の `tests/developer-platform/evaluation/ui/task_panel.ui`（715d308）、toolはNDP-06解析器（cb37e91）。上記READMEの実CLIで明示入力だけを解析し、3部品・1レイアウト・2 tabstop・未実行の接続1件を検出、unknown/unresolvedは空、入力は不変だった。これはToolkit非AIの構造解析runで、tool開発にはCodexを使用した。手動のみの移植比較ではない。

手動baseline、ローカルLLM補助、外部開発AIとの比較、能動作業時間・速度比・再利用率は未実施/未測定。renderer、mapping、実イベント、Nagi targetも未実施。比較不能な項目に数値を付けず、PT-06評価票の将来runに引き継ぐ。


## 2026-11-17 実接続の追記

上記「Cargoなし／実validator未実行」は11月15日の観測。現在は既存専用worktree内の
無視対象 `.nagi-host-tools/` に固定nightlyを隔離導入し、lockfile不変のnagi-pkg buildと
正規7テストが成功。SDK-owned notesとNDP-owned最小第三者Manifestを実validatorが受理した。
NDP-09 Stage 1 adapterの実接続・失敗系11テストも成功。入力不変、出力上限、
timeout/cancel、拒否、未接続Stage 2/3を区別する。[実行手順](../../../tools/nagi-developer-platform/validation/README.md)。

SDKの外部配送契約・owner合意、PT-01正規入力の選択、Stage 2/3、generator、targetは
この実行では解決していない。package宣言の受理をartifact存在・install/launch・権限付与とはしない。

## 2026-11-17 NDP-11b: GTK入力を含む繰返し評価

`run_host_checks.py` に `gtk3_task_panel.ui` と `gtk4_task_panel.ui` を追加し、Qtと同様に
stdout一致、部品・layout・signal・binding・unknown・要求versionの照合、入力不変確認を行う
（期待値は固定XMLから記述。GTKの実行・描画・handler実行はしない）。runnerのfailure-path
テストはGTK分を含め14件。

実測（Darwin arm64、Apple clang 21、Python 3.9.6、各解析はsubprocess壁時計でインタプリタ起動を含む）:
10回で全入力のstdout一致、入力不変。中央値は Qt解析 約29.8 ms、GTK 3 約30.3 ms、
GTK 4 約30.5 ms（portable C実行は約2.1 ms、初回のみ約38 ms）。3回の既定runでも
PASS（各約29〜31 ms）。差は数ms以内で起動時間が支配的であり、構文間の性能差や速度比は主張しない。
測定は1台・1セッションのみで、ウォームアップ除外や分散評価はしていない。

比較限界・未実施: 人手のみの移植、ローカルLLM補助、外部開発AIとの比較、能動作業時間、
GTKの実インスタンス化・描画・入力、Windows/Linux入力の実行、SDK validator、Nagi targetは
未実施。自作fixtureのhost結果であり、移植成功やtarget受入を意味しない。
Python 3.9.6では標準のPython 3.10+ gateを満たさない。

## 2026-11-18 NDP-11c: API rules・generator・Stage 2を含む繰返し評価

`run_host_checks.py` に3種を追加した（各 `--repeats` 回、既定3）。いずれも各toolの実CLIをsubprocessで実行する。
- NDP-04/05 API rules: 手書きPT-01 report v1 fixture 5件（Windows read_prefix・text_only、Linux watch_changes・text_only・混在）を標準入力で渡し、行・rule id・検出種別・一致aliasと「compatibility=not_assessed／target_acceptance=not_run／approval=PROVISIONAL」を照合。negative controlが既知APIを主張しないことも確認。scanner出力ではない。
- NDP-03 generator: 一時checkout（SDK `Cargo.toml` はstand-in）へLevel 0雛形を毎回新規に生成。stdout・生成tree一致、5ファイルの過不足、出力先外に何も作らないこと、Manifestの内容（id・表示名・権限なし・third-party）を確認。生成crateのbuild・Manifestの実validator検査はしない。
- NDP-09s2 Stage 2: 自作portable C fixtureのみ（固定clang引数）。compile/run_testの両step成功とstdout一致を確認。

入力のtool・fixture・templateもsha256で固定し、失敗時を含め不変を確認。runnerのfailure-path/期待値テストは14→40件（clang実行を含み、skipなし）。誤ったが安定した出力、必須要素の欠落、nondeterministic出力、timeout/nonzero、不正JSON、入力改変の各検知を実CLI出力に注入した回帰で確認。

実測（Darwin arm64、Apple clang 21、Python 3.9.6、10回、全PASS・入力不変）: 各API診断の中央値は約41〜43 ms、generatorは約32 ms、Stage 2（clang compile+実行含む）は約300 ms、Qt解析は約30 ms。いずれもsubprocess壁時計でインタプリタ/clang起動を含み、1台・1セッションのみ。

比較限界: 速度比、移植成功、Nagi target成功は主張しない。手動baseline・ローカルLLM・外部開発AIとの比較、能動作業時間は未実施。API rulesの入力は手書きfixtureで本物のscanner接続ではない。Stage 2は自作fixtureのみで第三者/実アプリのbuildではない。Stage 3は未接続。host結果であり、Python 3.9.6は標準の3.10+ gateを満たさない。

## 2026-11-18 NDP-11d: 実nagi-pkgによる雛形Manifest受理とXAML入力の繰返し評価

`run_host_checks.py` に2種を追加した（各 `--repeats` 回、既定3）。
- XAML（NDP-06d）: `xaml_minimal`・WPF・WinUIの自作fixture 3件を解析器CLIで評価。stdout一致、部品/親、layout子とGrid.Row、tab順、宣言ハンドラ・binding（未実行のまま）、resources、unknown、未解決resource keyを、XAML本文から書いた表と照合。実行・描画・読込はしない。
- Manifest受理: generatorで作った雛形のmanifestを既存Stage 1 adapter経由で準備済みの実 `nagi-pkg manifest validate` に通す（build・変更はしない。バイナリのsha256も不変確認）。受理報告の形（SDK contract 1.0等）・stdoutと生成treeの一致・validatorがmanifestを書換えないことを確認し、`id` を不正値にしたnegative controlが拒否されることで「何でも受理」するvalidator/adapterを検知する。バイナリが無い/symlink/実行不可なら、プロセスを起動せず `not_run`（理由 `validator_unavailable`）を出し、成功にしない。

runnerのテストは40→63件（skipなし）。XAMLの誤った安定出力・空出力・不正JSON・不一致・timeout/nonzero・入力改変、validatorの全受理・報告改変・実行間不一致・manifest書換え・不正出力・timeout/想定外exit・binary差替え・未準備時の `not_run` を、実CLI出力に注入して検知を確認。

実測（Darwin arm64、Python 3.9.6、10回、全PASS・入力不変、stdout一致）: XAML解析の中央値は約32〜33 ms（minimal 31.9／WPF 32.9／WinUI 32.6）、雛形生成＋実validatorの受理は約28 ms（生成を含まないvalidator呼出単体の分離はしていない）。subprocess壁時計でインタプリタ起動を含み、1台・1セッション。

計測・競合の限界: `median_ms` はadapter呼出（accept側）の時間のみで、negative control・生成・fingerprintは含まない。実行直前にvalidatorが差し替えられる競合はfingerprintでは検知できず、脅威モデル外。

比較限界: 実validatorが受理するのはpackage宣言のメタデータのみ（artifact存在・install・権限付与・build成否ではない）。雛形crateのbuildは未実施。XAMLは静的構造のみで実行・描画・WPF/WinUI互換は未検証。negative controlは1種類（不正id）のみで、validatorの拒否網羅ではない。速度比・手動/LLM比較・Stage 3・Nagi targetは未実施で、host結果をtarget受入とは扱わない。
