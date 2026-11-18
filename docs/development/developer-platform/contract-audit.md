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

根拠は[nagi wrapper](../../../nagi)、[CLI dispatch](../../../tools/nagi-cli/src/commands.rs)、[Package CLI](../../../tools/nagi-pkg/src/main.rs)。下記は呼出し候補の調査で、対象アプリを自動実行する許可ではない。

| 正規entry | 入力と結果 | 副作用・前提 / Stage |
|---|---|---|
| ./nagi doctor | host依存とnagi.toml。CLIの終了値は成功0、依存不足10、設定不備4 | wrapperはCargoでtrusted CLIをbuild/runする。Cargoがなければwrapper自体が4で終了。入力appを実行する検査ではない |
| ./nagi fetch | pinned source/dependency設定、取得結果 | bootstrapへ委譲。network/cache書込みがあり、Stage 1の既定処理にしない |
| ./nagi build / test | host workspaceのbuild/test結果 | Cargo呼出し。arm64ではhost互換packageを明示選択する。任意の入力appに向けた汎用build APIではない。実行はStage 2相当の許可範囲だけ |
| ./nagi image | kernel/init等のtarget buildとイメージ作成結果 | target toolchainと出力書込みが必要。入力appの静的確認として起動しない |
| ./nagi run | QEMU等の起動結果 | VM・イメージ・資源が必要。Stage 3は別のtarget条件が揃ってから。起動だけで受入成功としない |
| nagi-pkg manifest validate <manifest.json> | JSON v1メタデータ、成功メッセージ。成功0、usage/validation失敗2 | 検証器はentrypointを起動しない。Cargoで起動する場合はtrusted toolのbuildが先に必要。appのbuild scriptをStage 1から実行しない |
| nagi-pkg info <package> | 既存packageの解析結果 | JSON Manifest v1の検証とM16 package解析は別。署名の信頼やinstall成功まで含めない |

Manifest検証の正規開発入口は次のとおり。対象入力はSDK-owned fixtureで、第三者appではない。実行はしていない。

```sh
cargo run --locked --manifest-path tools/nagi-pkg/Cargo.toml -- \
  manifest validate sdk/rust/fixtures/manifests/notes.json
```

NDP-09はこの既存validatorへの1本のadapterから始める候補。任意shell文字列、入力appのbuild script、network取得、Stage 2/3への自動昇格は行わない。timeout/cancel/nonzero/未実行を別に保持する。実validatorの利用とCLI/Package owner確認ができるまで接続済みとはしない。

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
