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

照合対象は手元の `codex/ndp-host-integration` (`dd5dca8`)。下表の終了値はコードから確認したもので、今回の照合で各コマンドは実行していない。

`./nagi` の各行に共通して、shell・Cargo・Rust toolchain と、読める Nagi checkout（`nagi.toml`、`rust-toolchain.toml`、`third_party/sources.lock`）が必要。wrapper は `cargo run --locked` で CLI をビルド・実行し、依存取得とビルド出力の書込みが起こり得る。`--locked` はオフライン指定ではない。Cargo が見つからない場合は wrapper が `4` で終了し、Cargo 側で起動に失敗した場合は CLI の終了値に到達しない。CLI は引数不正を `2`、プロジェクト設定エラーを `4` で返し、`NAGI_RUN_RECEIPT_DIR` 等で有効化した場合は実行記録も書く。根拠は `nagi` (`--locked`)、`tools/nagi-cli/src/config.rs` (`validate_project`)、`tools/nagi-cli/src/main.rs` (`take_from_env_at_startup`)、`tools/nagi-cli/src/run_receipt.rs` (`NAGI_RUN_RECEIPT_DIR`)。

Stage 1 は静的検査の段階。この表で使うのは準備済みの信頼できる `nagi-pkg` を固定引数で呼ぶ Manifest 検証のみで、入力の build script・entrypoint・任意コマンドの実行、Cargo 経由の起動、依存取得は行わない。Stage 2 は許可された host コマンドと環境に限る。Stage 3 は実在する対象、個別の実行許可、既存の release Gate が揃った場合に限る。現行 NDP-09 の接続は Manifest 検証と承認済みの自作 C fixture の host テストまでで、Stage 3 は未接続。[PT-05 の Stage 契約](../../product-decisions/DEC-045-nagi-porting-toolkit.md)・[NDP-09 adapter](../../../tools/nagi-developer-platform/validation/README.md)

**NDP-08 は PARTIAL。CLI / Package owner の確認待ち。**

- CLI owner — NDP-09 が依存できる CLI の版・終了値・出力契約、承認対象とする host コマンド、Linux target で使う起動・受入 entry と条件（未確認）。
- Package owner — 準備済み validator の固定・提供方法、Manifest の終了値・出力の互換性、`.xapp` の構造・署名検査に使える入口、正式署名・install の手順と権限（未確認）。

| 正規entry | 入力 | 出力 | 終了状態 | 副作用 | 依存・実行許可 | Stage | 根拠（repo 相対パスと識別子・flag） |
|---|---|---|---|---|---|---|---|
| `./nagi doctor [--allow-missing]` | host の `PATH`・既定探索先、OVMF、`nagi.toml` の要件 | 各依存の PASS / WARN / FAIL と件数 | `0` 失敗なし、`10` 検査失敗、`4` 設定不備、`2` 引数不正。`--allow-missing` は未検出を警告にする | CLI はツールの `--version` を実行して OVMF ファイルを探す。wrapper のビルド副作用あり | 共通条件と、信頼済み host ツールを探して起動する許可 | 2・3 の環境準備。1 では使用しない | `nagi` (`--locked`)<br>`tools/nagi-cli/src/commands.rs` (`execute_doctor`, `--allow-missing`)<br>`tools/nagi-cli/src/doctor.rs` (`run_doctor_with_requirements`, `run_candidate`) |
| `./nagi fetch` | `third_party/sources.lock` と各 source / patch / font の固定情報 | 取得フェーズの進捗（stderr）、検証済み source の場所（stdout） | `0` 成功、読込・取得・検証エラーは `4`、Servo の Cargo fetch 失敗はその終了値を引き継ぐ（起動不能や終了値なしは `4`） | bootstrap 経由で固定 source・Cargo 依存・font を取得／検証し、checkout やキャッシュへ書込み、Nagi patch を適用 | 共通条件、Cargo・Git・必要時 curl。取得先への通信とキャッシュ書込みの許可 | 2・3 用の事前準備を別途許可された場合のみ。1 では依存取得禁止 | `nagi` (`nagi-bootstrap`)<br>`tools/nagi-bootstrap/Cargo.toml` (`nagi-bootstrap`)<br>`tools/nagi-cli/src/commands.rs` (`execute_fetch`, `ensure_servo_checkout`)<br>`tools/nagi-cli/src/registry_source.rs` (`apply_patch`)<br>`tools/nagi-cli/src/fonts.rs` (`curl`) |
| `./nagi build` | Nagi checkout の host package、Cargo 設定と lockfile | ビルド成果物、PASS 要約または Cargo の失敗診断 | `0` 成功、Cargo の非ゼロ値を引き継ぐ（起動不能や終了値なしは `4`） | `cargo build --locked` でビルドスクリプト等を実行し得る。ビルド先やキャッシュに書込み。依存未準備なら取得が起こり得る | 共通条件と対象 package の依存、承認済み checkout・Cargo 実行と書込みの許可 | 2。1 では使用しない | `tools/nagi-cli/src/commands.rs` (`Command::Build`, `host_workspace_args_for_arch`, `--locked`, `finish_tool`) |
| `./nagi image` | checkout の init・kernel・UEFI loader と target 設定 | FAT12 イメージ `out/artifacts/nagi-0.1-m1.img`、サイズと配置の要約 | `0` 生成成功、ビルド失敗はその終了値を引き継ぐ。読込・生成エラーは `4` | init・kernel・loader を target 向けにビルドし、イメージを書込む。ゲストは起動しない | 共通条件、nightly Cargo/Rust・`rust-src`（`-Zbuild-std`）、`x86_64-unknown-uefi`、target ビルド・成果物書込みの許可 | 3 のイメージ準備。1・2 では使用しない | `tools/nagi-cli/src/commands.rs` (`execute_image`, `execute_image_with_init_build_env_using_writer_and_recovery`, `x86_64-unknown-uefi`)<br>`tools/nagi-cli/src/image.rs` (`write_fat12_image`) |
| `./nagi run` | 自身が再生成する既定イメージ、QEMU、許可リストにある OVMF CODE/VARS、永続データディスク | シリアルログ（`out/logs/m1-qemu-boot.log` など）、ゲストマーカーと結果の要約 | `0` ゲスト受入マーカー確認、イメージ失敗はその終了値を引き継ぐ。QEMU／OVMF 不備、タイムアウト、マーカー不足等は `4` | イメージ生成後に QEMU user networking 付きでゲストを起動。VARS コピーとログを書き、永続ディスクを作成・再利用し、旧形式なら移行し得る | `image` の依存、QEMU x86-64・対応 OVMF、VM 実行・永続ディスク変更の許可 | 3 の既定イメージ起動・永続化確認。移植対象 app の受入 entry は未確認 | `tools/nagi-cli/src/commands.rs` (`execute_run`, `qemu-system-x86_64`)<br>`tools/nagi-cli/src/image.rs` (`run_qemu`, `ensure_persistent_disk`, `GUEST_ACCEPTANCE_MARKER`, `-netdev`) |
| `./nagi test` | Nagi checkout の host package とテスト、Cargo 設定と lockfile | PASS 要約または Cargo の失敗診断 | `0` 成功、Cargo の非ゼロ値を引き継ぐ（起動不能や終了値なしは `4`） | `cargo test --locked` でビルドしたテストと該当する doctest を実行。ビルド先やキャッシュに書込み。依存未準備なら取得が起こり得る | 共通条件と対象 package の依存、承認済み checkout・ビルド・テスト実行と書込みの許可 | 2。1 では使用しない | `tools/nagi-cli/src/commands.rs` (`Command::Test`, `host_workspace_args_for_arch`, `--locked`, `finish_tool`) |
| `./nagi test --acceptance [options]` | `tests/acceptance/registry.tsv` と登録済みケース。`--host-only` / `--target-only` 等で選択 | ケース結果・集計、`--ci` / `--json` で JSON、`--list` で一覧 | `0` 選択ケースに FAIL / SKIP / BLOCKED なし（一覧・ヘルプ成功も `0`）、`11` いずれかあり、`2` 引数不正・選択なし、`4` registry・報告等のエラー | 登録済みスクリプトを実行しログを書込む（`--list` は一覧のみ）。追加の副作用はケースに依存し、この表では未確認 | 共通条件、登録済みスクリプトの実行・レポート書込み許可と選択ケースの依存 | 承認済み host ケースは 2、target ケースは 3。1 では使用しない | `tools/nagi-cli/src/commands.rs` (`--acceptance`, `Command::Acceptance`)<br>`tools/nagi-cli/src/acceptance.rs` (`REGISTRY_PATH`, `run_case`, `--host-only`, `--target-only`, `EXIT_ACCEPTANCE_FAILURE`) |
| `nagi-pkg manifest validate <manifest.json>` | JSON Manifest v1（1 MiB 以下、SDK contract 1.0） | stdout に PASS・入力パス・AppId・SDK contract 版、拒否理由は stderr | `0` メタデータ受理、`2` 引数不正・入力読込／検証失敗 | 検証器は JSON を読み取る。アプリ起動・ビルド・書込み・取得なし | 準備済み `nagi-pkg` の実行と指定 Manifest の読取許可。現行 adapter は入力を限定して一時コピーを渡す | 1。validator 自体の Cargo ビルドは事前準備 | `tools/nagi-pkg/src/main.rs` (`manifest_command`, `usage`)<br>`tools/nagi-pkg/src/app_manifest.rs` (`load_manifest`, `MAX_APP_MANIFEST_BYTES`)<br>`tools/nagi-developer-platform/validation/validate.py` (`VALIDATOR`) |
| `nagi-pkg info <package>` | 既存 `.xapp` ファイル | stdout に Manifest の ID・版・実行部のバイト数 | `0` 解析成功、`2` パス引数なし。読込・解析失敗は panic（数値は未確認） | ファイル全体を読み取って構造を解析。署名の信頼判定や install はしない | 準備済み `nagi-pkg` の実行と Package の読取許可 | 2 の Package 構造検査候補。1 の adapter 対象外 | `tools/nagi-pkg/src/main.rs` (`info`, `PackageView::parse`)<br>`user/nagi-package/src/lib.rs` (`PackageView`, `is_signed`) |
| `nagi-pkg build-hello <napp> [output]` | NAPP サンプル、組み込み SDK Manifest、任意の出力パス | 未署名 `.xapp`（既定 `out/artifacts/hello-nagi.xapp`）、PASS と AppId | `0` 成功、`2` 引数不正・NAPP 不適合・生成検証失敗。I/O 等の panic の数値は未確認 | 親ディレクトリを作成して Package を書込み、その後に SDK identity を確認する | 準備済み `nagi-pkg` の実行、サンプル読取と出力先書込みの許可 | 2 のサンプル Package 生成候補 | `tools/nagi-pkg/src/main.rs` (`build-hello`, `build_hello`, `HELLO_MANIFEST`) |
| `nagi-pkg build-signed-hello <napp> [output]` | NAPP サンプル、組み込み SDK Manifest、任意の出力パス | Developer Preview テスト署名付き `.xapp`（既定 `out/artifacts/hello-nagi.xapp`）、PASS と AppId | `0` 成功、`2` 引数不正・NAPP 不適合・生成検証失敗。I/O 等の panic の数値は未確認 | 組み込みテスト鍵で署名し、書込み後に SDK identity と署名を確認する | `build-hello` の依存と、テスト署名付き成果物を作る許可 | 2 のサンプル Package 生成候補 | `tools/nagi-pkg/src/main.rs` (`build-signed-hello`, `build_hello`, `DEVELOPER_PREVIEW_SIGNING_SECRET`) |
| `nagi-pkg build-signed <manifest> <elf> <output>` | `.xapp` 用の行形式 Manifest（`grant=` 等）、Nagi ELF、出力パス | Developer Preview テスト署名付き `.xapp`、PASS・AppId・grant 件数 | `0` 成功、`2` 引数不正・ELF マジック不一致・生成検証失敗。I/O 等の panic の数値は未確認 | 必要なら `entry=` をメモリ上で補完し、Package を生成・テスト署名して書込む | 準備済み `nagi-pkg` の実行、入力読取、出力先書込みとテスト署名の許可 | 2 の Package 生成候補。正式署名と install の手順は未確認 | `tools/nagi-pkg/src/main.rs` (`build-signed`, `build_signed`, `DEVELOPER_PREVIEW_SIGNING_SECRET`) |

`build` と通常の `test` は x86-64 で `nagi-kernel` を除く workspace、その他の host ではコードに列挙された package を対象とする。準備済みバイナリを使う Stage 1 とそのビルド手順は [adapter README](../../../tools/nagi-developer-platform/validation/README.md) を参照。Package 生成や host テストの成功から、正式署名・install・権限付与・Nagi target 受入は判定しない。

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
