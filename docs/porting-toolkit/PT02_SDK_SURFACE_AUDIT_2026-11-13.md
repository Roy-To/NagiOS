# PT-02 準備資料 — 現行Nagi SDK接続点の監査（2026-11-13）

> **2026-11-15 公開移管:** [DEC-050](../product-decisions/DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

**状態: READ-ONLY AUDIT / PREPARATION. コード実装・ビルド・受入未実施。**  
**タスク:** 非公開リポジトリのIssue #8(移管前。公開側からは参照できない)  
**製品正本:** [DEC-045](../product-decisions/DEC-045-nagi-porting-toolkit.md)（PT-02）  
**公開側基準:** `Roy-To/NagiOS` の `main`、各参照ファイルの以下のblob SHAで確認。新しいcommitやowner変更がある場合は読み直す。

## 1. 実在を確認したSDKとツール

| 公開リポジトリ内の正本 | 確認した内容 | blob SHA |
|---|---|---|
| `sdk/README.md` | 0.1 Developer Preview。Rust `nagi-sdk` とC interface、IDL由来の生成binding。現行SDKの不足機能を明示 | `01736b63bd034e96f7ac1796995168e5f3ca15b9` |
| `sdk/rust/Cargo.toml` | `nagi-sdk` v0.1.0、`publish = false`、`nagi-model` を相対path参照 | `d09a2c3b7e634e96583533f7e3e7ec1cf23922b6` |
| `samples/hello-nagi/Cargo.toml` | `nagi-sdk = { path = "../../sdk/rust" }` の**リポジトリ内**サンプル。外部に配布済みSDKとは言えない | `7cfd8a2721a1753e9133231c48dd1a84ccd4b64b` |
| `samples/hello-nagi/src/lib.rs` | `no_std`、アプリID・Session／Presentation情報・hello package dataの例。実際のGUI起動を示すものではない | `8ff7d4024ba53f96c7e38fc8ae39a0d7` |
| `tools/nagi-pkg/src/main.rs` | `manifest validate`、`build-hello`、`build-signed-hello`、`build-signed`、`info`。固定テスト用のDeveloper Preview署名鍵を利用 | `ecc4573873f010195f566020b050d99f737c3032` |
| `tools/nagi-pkg/src/app_manifest.rs` | Manifestの実装済み検証器、SDK版照合、サイズ・JSON・型／フィールド検査 | `7f6c87d75be5714acf16ac01ddf1080591edafd3` |
| `sdk/rust/fixtures/manifests/files.json` | `schemaVersion:1`、`sdkContractVersion: {major:1,minor:0}` 等を持つ**既存fixture例**。そのまま第三者アプリ用に流用しない | `fc3614743c49d1a06d6e06cd42a080dc025547b9` |
| `.dev/workstreams.json` | 既存Owner／branch／allowed_paths／Gate | `004bd09b6504ab7c5272d04d20640118c24d8b9b` |

## 2. 重要な発見と未解決条件

- **独立したout-of-tree SDK利用の配送経路が未確定:** 現行の `sdk/rust/Cargo.toml` は `publish = false`、`nagi-model` は相対path依存。既存 `hello-nagi` sampleも同じリポジトリ内のSDKを相対pathで参照する。PT-02が外部アプリを生成する際、公開SDKを**どの方法でPin/参照し、依存を解決するか**はSDK Ownerと正式契約を決める必要がある。未確認のCrates.io packageやGit依存、downloadを勝手に生成しない。
- **現行のAPI範囲は限定的:** Rust/CのApplication identity／presentation modelは確認できるが、一般file-handle、IPC、network、audio、clipboard、notification、Action登録、完全なGUI toolkit等は現在のREADME上で未提供。PT-02のLevel 0雛形がこれらを実装済みとして生成するのは禁止。
- **署名と配布:** `nagi-pkg` の固定テスト秘密鍵は製品署名・一般配布の根拠にならない。Manifest生成はPermissionの付与ではない。
- **WorkstreamとGate:** `app-sdk-contract` は既存Ownerがあり `sdk/**`、`tools/nagi-pkg/**` を所有し、host側contract PASSのみ。`sdk` と `developer-tooling` のownerは現行registryで未割当。Developer Toolingのproduct-facing工程はM30 PASS Gateの対象。PT-02用の実装owner/branch/pathは**未割当**。この監査は新規コード書込みやrelease activationを認可しない。

## 3. PT-02初期雛形の検討項目（PROVISIONAL）

- 想定入力：アプリID、表示名、使用する正規SDK版、対象ホスト、選択した出力ディレクトリ。
- 出力候補：`Cargo.toml`、最小 `src/lib.rs`、正規Manifest形式の雛形、README、現行validatorの呼出し案。ファイル構成やCLI契約は未決定。
- 非目標：独自SDK/IDL/Package形式、未実装のGUIやネットワークAPI、隠れた権限、元プロジェクトの無断編集、依存の無断取得、AI必須化。
- 検証候補：Manifest Validator実行、参照されたSDK版と実在API、既存出力先との競合拒否、同一入力の再現性、HostとNagi targetのAcceptance区別。

## 4. 次の担当者アクション

1. Integration OwnerからPT-02の**owner・branch・変更path・既存SDK担当との接続checkpoint・host-only実行範囲**を確認する。既存ワークツリー／cloneを無断追加しない。
2. SDK Ownerとout-of-tree利用方式および有効なManifest最小セットを正式な現行Contractに照らして確認する。
3. 明示的に許可された範囲でのみhost generatorやfixture実装・テストを始める。実施したテスト／未実施・結果不明を証拠で記録する。
4. DotのPT-01作業やMac上のLinux移行、共有QEMU・CI・SDK ownerの作業に干渉しない。

**本資料は読み取りによる現況監査であり、コード生成・実行・GitHub Actionsの成功を主張しない。**
