# NDP-02 — Level 0雛形契約

[level0.json](level0.json) はNDP-03向けの機械可読契約。
この担当内の契約・実装案はPROVISIONALであり、SDK／Manifestの新しい正本ではない。
正本はrepo rootからの次の参照先で、SDK・Package側は変更しない。

- `sdk/README.md`、`sdk/rust/Cargo.toml`
- `sdk/rust/docs/APP_SDK_CONTRACT.md`
- `sdk/rust/schemas/app-manifest.schema.json`
- `tools/nagi-pkg/src/app_manifest.rs`
- `samples/hello-nagi/Cargo.toml`

## 入力と生成物

| 入力 | 契約 |
| --- | --- |
| `app_id` | 必須。正本schemaのreverse-domain ASCII ID制約を参照、最大255文字 |
| `package_name` | 必須。小文字英字開始、英数字・区切りハイフン、最大64文字。JSONのRust予約語一覧は拒否 |
| `display_name` | 必須。`en-US` と `ja-JP` の非空文字列、各256文字以内。両言語を同等に扱う |
| `output_dir` | 必須。明示指定したrepo相対の新規directory。絶対path、`.`、`..`、空segment、backslash、drive指定、制御文字は拒否 |
| `app_version` | 既定 `0.1.0`。正本schemaのsemantic version制約を参照、最大256文字 |
| `sdk_version` | 既定 `0.1.0`。選択checkoutのSDK Cargo package版のみを許容 |
| `profile` | 既定・唯一の構成 `host-contract-library` |

未知入力は拒否する。権限、任意依存、任意template／shell command、force上書きの入力はない。
表示名の改行・制御文字・不正Unicodeは拒否し、引用符などはJSONとしてescapeする。
SDK package版 `0.1.0`、SDK contract版 `1.0`、manifest schema版 `1` は別の値。

| 生成path | 内容 |
| --- | --- |
| `Cargo.toml` | `publish=false`、edition 2021、独立した `[workspace]`、既存SDKへのpath依存 |
| `manifest.json` | JSON v1の必須root fieldだけ。第三者origin、両言語、空resources／intents／requestedCapabilities、state版1 |
| `src/lib.rs` | `no_std`、実在する `AppIdentity::new` によるfallibleな第三者identity構築のみ |
| `README.md` | 仮定、手動検査の候補、非提供artifactとhost／targetの区別 |
| `UNSUPPORTED_APIS.md` | GUI・runtime I/O／IPC・Action・AI・署名／導入／起動などの未対応表示 |

JSONにコメントfieldを追加しない。他の生成物にはgenerated表示を入れる。
従来M16のhello専用 `HELLO_MANIFEST`／`HELLO_EXECUTABLE` はコピーしない。
`manifest.json` は署名packageではなく、宣言は権限付与を意味しない。

## 仮定（PROVISIONAL）

- SDKの外部配送・pin方式は未確定。本roundは依頼された仮定として、hello-nagiと同じ
  **選択済みrepo内SDKへのpath依存**で進める。以前のtask資料の「SDK参照待ちで生成を止める」
  は今回の明示指示により、この仮定の契約作成を妨げない。正式なowner合意とは扱わない。
- SDK参照情報は `level0.json.sdkReference` に集約する。`repoPath` から出力先までの
  相対pathを `contract.py::sdk_dependency` だけで計算する。templateにSDKの場所を埋め込まない。
  配送方式変更時はこのadapterとdataを差し替える。registry／Git依存へ自動fallbackしない。
- repo全体の選択checkoutを保持する。SDKは `nagi-model` をrepo相対で参照するため、
  `sdk/rust` 単独コピーでは足りない。path依存自体はrevisionをpinしない。
  revision選択は呼出側の責務で、未確認の公開crateや外部配布済みSDKを仮定しない。
- 最小構成はhostで検査できる契約libraryとする。`entrypoint.target=bin/<package_name>` は
  将来package内のlocatorであり、生成される実行ファイルではない。native runtime adapter・
  executable・package接続は後続作業。metadataの受理と起動可能性を同一視しない。
- 両言語の表示名を必須入力とし、実行時UIは生成しない。locale解決の正本は既存SDKで、
  この契約は翻訳基盤や別のruntime実装を追加しない。
- 入力の長さ上限とpackage予約語の拒否は、この最小構成に限定した制約。
  SDK/schema全体の許容範囲を変更せず、manifestサイズを実loaderの1 MiB上限より小さく保つ。

## NDP-03への接続と保全

`contract.py::preview(inputs)` は `{出力相対path: UTF-8 text}` を返す参照実装。
file書込み、出力先の存在検査、command実行、依存取得は行わない。
入力例（repo rootは読取元、`output_dir` は意図された出力先）：

```json
{
  "app_id": "com.example.level-zero",
  "package_name": "level-zero",
  "display_name": {"en-US": "Level Zero", "ja-JP": "レベルゼロ"},
  "output_dir": "samples/level-zero"
}
```

NDP-03のwriterは `writePolicy` を実装すること。既存directoryは空でも拒否し、
symlinkとancestorのsymlinkを拒否する。作成時にもexclusive create／no-followなどで
競合を検出し、既存fileへの上書き・mergeは行わない。失敗時は自分が作成した物だけを
片付け、元project・SDK・Package・共有Cargo workspace／lockを変更しない。
書込み保全・race検証と実生成CLIはNDP-03の作業で、このpreviewのテストでは受入しない。
API／権限の捏造、build script実行、download、署名鍵の埋込み、host処理でのguest代替は禁止。

## 検証

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest discover -s tools/nagi-developer-platform/generator/contract -p 'test_*.py'
```

Python 3.9標準libraryのみ。実SDKのCargo定義、sample依存、public module／re-export／
使用型・constructor signature、SDK版定数、正本schema、実loaderの必須fieldを読み、
契約と突合する。正本schemaを使ったtest専用の構造検査は、欠落必須field・未知field・
不正ID／entry／版も拒否する。これはSDKやsemantic validatorの代替ではない。
生成previewの再現性、入力拒否、JSON escape、SDK参照差替えも検査する。

正規semantic検証は、別途準備済みのtrusted `nagi-pkg manifest validate manifest.json`
を使う。自動build／install／fetchはしない。このroundの必須テストはsource照合と
in-memory previewのhost検査であり、Rust compile・実validator実行・Nagi target起動は
受入に含めない。Nagi上の動作成功やSDKの正式配布方式の確定とは報告しない。
