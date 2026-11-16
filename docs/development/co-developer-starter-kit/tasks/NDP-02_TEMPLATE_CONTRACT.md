# NDP-02 — Level 0アプリ雛形の契約

状態: 担当確定（Core Developer、2026-11-15）/ NOT STARTED。PT-02。開始にはNDP-01、SDK ownerとの接続確認、担当pathの承認が必要。

## 目的

実在するSDKとManifestだけで作れる最小アプリ雛形を定義し、NDP-03が架空の依存やAPIを生成しないようにする。

## 読むもの

DEC-045のPT-02、`docs/porting-toolkit/PT02_SDK_SURFACE_AUDIT_2026-11-13.md`、最新版SDK README、`sdk/rust/Cargo.toml`、`sdk/rust/docs/APP_SDK_CONTRACT.md`、Manifest schemaとvalidator、hello-nagiの現物。

## 作業

SDKの配送・pin方法、repo外の依存解決、現行Manifestの必須field、第三者アプリ用ID・権限・entryの制約を確認する。監査資料の `publish = false` を現在のコードで再確認し、存在を確認していないCrates.io名やGit依存を生成しない。

入力候補はアプリID/表示名、選択SDK版、対応構成、明示出力先。出力候補は既存契約に合うManifest、最小source、build設定、READMEと未対応説明。実際の構成は現行SDKに合わせて小さく決める。UI、ネットワーク、Action登録など未提供のものをLevel 0の名目で追加しない。

## 成果

承認済み担当pathの短い雛形contractとfixture候補。既存SDK/Manifestのコピーを新しい正本にせず、元のschema/validatorを参照する。正規SDK参照が未確定なら、その箇所だけ未確定としてNDP-03の依存生成を止める。

## 確認項目案

- 現行validatorが受理する最小Manifestと、拒否する不正例を特定した。
- SDK参照・バージョン・相対依存を実コードで説明できる。
- Manifestの宣言を権限付与や正式署名と扱っていない。
- 生成対象が機能を提供しない場合、明示的に未対応である。

検証の実行は承認されたhost範囲だけ。コマンドは実際のtoolから確認する。契約確認をNagi上のアプリ起動成功にしない。

## 次

契約が利用可能になったらNDP-03へ渡す。SDK本体改修が必要ならownerへ最小提案を返し、別の承認済みtaskを進める。

共通: [規則](../03_COMMON_RULES.md)、[権限](../04_SCOPE_AND_AUTHORITY.md)。
