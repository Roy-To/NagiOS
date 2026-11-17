# NDP-02 自動レビュー記録

## Round 1 — claude review of 5c5626f0e7cc

P1/P2 の指摘はありません。変更はすべて担当 path (`tools/nagi-developer-platform/generator/contract/**`) 内で、`sdk/**` と `tools/nagi-pkg/**` は読み取りのみでした。

私は diff の主要ファイル（`contract.py`、`level0.json`、`test_contract.py`、`lib.rs.tmpl`、`Cargo.toml.tmpl`）を読みました。テストの再実行はしていません。`README.md`、`README.md.tmpl`、`UNSUPPORTED_APIS.md.tmpl` は開いていません。

## 良い点
- **目標との整合**: 契約 JSON が生成物 5 点と入力パラメータを定めています。禁止事項は `prohibited` と `writePolicy` で、PROVISIONAL の扱いは README の「仮定（PROVISIONAL）」節と JSON の `status` で表されています。
- **差し替え 1 か所**: SDK の参照方式は `sdk_dependency()` と `sdkReference` に集約されています。テスト `test_dependency_adapter_handles_depth_and_single_location_replacement` が、`repoPath` を変えると `Cargo.toml` だけが変わることを確認しています。
- **架空 API の検証**: テストが実ファイルを読んで照合しています。
  - SDK 側では、`AppIdentity` / `AppOrigin` / `AppError` の宣言と export、バージョン定数、`AppIdentity::new` のシグネチャを確認しています。
  - schema 側では、`required` を再帰的に突き合わせています。
  - `samples/hello-nagi/Cargo.toml` とは path 依存の一致を確認しています。
  - loader (`app_manifest.rs`) とは、必須フィールドと manifest の上限バイト数を確認しています。
  - 必須フィールドを 1 つずつ欠落させて検出できることも確認しており、決め打ちや弱い assertion ではありません。
- **安全面**: `preview()` は書込みも外部取得も実行もせず、副作用のテストがあります。`output_dir` は絶対パス、`..`、バックスラッシュ、制御文字を拒否します。書込み側の責務（symlink、既存 destination の拒否、排他的作成）は NDP-03 に委ねると明記されています。
- **host と target の区別**: `targetAcceptance` は `not-run`、`entrypointArtifact` は `not-generated-or-verified` と明示されています。

## P3（軽微）
- `level0.json:71` と `contract.py` の manifest: entrypoint の `bin/${package_name}` は、Cargo の `[lib]` では実在しない成果物を指します。`entrypointArtifact: not-generated-or-verified` で明示済みなので許容範囲です。README と `UNSUPPORTED_APIS.md` に同趣旨の記載があるか、NDP-03 以降で再確認してください。
- `test_contract.py` の `canonical_errors` はテスト専用の簡易 walker です。docstring に明記されており、未知キーワードで失敗するので弱体化はしにくい作りです。typed validator での検証は未実施で、それは目標外です。

## 疑問
- `pub struct AppIdentity<'a>` のようなソース文字列一致は SDK の整形変更に脆いです。ただし、変更を検知するというテストの意図には合っています。

NDP-REVIEW: PASS

