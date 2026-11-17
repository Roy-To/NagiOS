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
