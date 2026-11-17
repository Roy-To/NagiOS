# NDP-01 調査結果と初回開始記録

確認日: 2026-11-15。**創設者確認済み（2026-11-15）。**
NDP-01の読取照合結果と、創設者が確認した初回開始条件の記録です。workstreamのregistry登録や製品仕様の決定を示すものではありません。

## 創設者の確認結果（2026-11-15）

- **CONFIRMED:** 本書の初回開始案を、内容を変えずに承認。
- **CONFIRMED:** NDP-01〜12を共同開発者（Core Developer）の担当範囲とする（[DEC-046](../../product-decisions/DEC-046-co-developer-governance.md)）。担当範囲内のtask着手・設計・実装・作業順は、taskごとの創設者承認を前提にしない。PT-01・SDK・Package・UI等の既存ownerとの接続、共有path・registry、release Gateの扱いは変えない。
- **NDP-01:** 本書の照合をもって完了。
- **NDP-10:** 下表の条件で開始できる。
- **未指定:** Integration Owner、workstream登録担当、push先（本記録のPRは本体repositoryの担当branchから提出）、Python・Rustの導入範囲、DeepSeekレビューの受渡し経路。下表のとおり、NDP-10の入力・README作成はこれらの指定なしで進められる。PRのmergeとregistry登録の前に、Integration Ownerと登録担当の指定が必要。

## 確認した基準と実行結果

- 作業branch: `codex/ndp-bootstrap`、基準commit: `3b22141a56c95f6d47c5d8465bdb796014e6e43f`。調査開始時の差分なし。
- 公開mainのregistryとDEC-049を再取得し、checkoutと同一内容であることを確認。公開mainにはNDP専用登録なし。開始承認はテンプレート以外に確認できず。
- host: macOS / arm64。Git、Clang、Python 3.9.6は利用可能。Cargo / Rust / Rustup、CMake、Meson、Ninja、QEMUはPATH上で確認できず。Pythonは `nagi.toml` の最低3.10.0に未達。
- `./nagi doctor` → exit 4、`Cargo was not found`。依存導入、build、製品テスト、VM実行は未実施。
- GitHub connectorのrepository権限はpull/pushともtrue。ローカルGitのpush認証は未検証。権限の存在を担当割当や操作承認とは扱わない。

## 既存契約との接続

| 接続先 | 実物で確認した内容 | 残る判断・担当 |
|---|---|---|
| PT-01 | [担当記録](../../workstreams/PT-01-readonly-host-assignment-2026-11-13.md)では `tools/nagi-porting/**`、Python標準ライブラリによる別案。記録末尾でも統合調整未完。このcheckoutに実装・report schemaなし | 既存PT-01担当・Integration Ownerが採用経路を決める。NDP側でscannerを新造しない |
| PR #91 | [open Draft](https://github.com/Roy-To/NagiOS/pull/91)、head `0bd3af462abd33c6173eba222e931e88470c314c`。`tools/nagi-cli/src/porting.rs` の `analyze(&Path) -> Result<Value, String>` を確認。main未統合 | PT-01案との正規接続・採用は未確定。以下の形式はPR内の現状であり、採用決定ではない |
| #91 report | `schema_version: 1`、`detected_languages`、`manifests`、`dependencies`、`analysis`、`findings`、`limitations`。findingは `category/path/line/evidence/review_status`。互換保証ではなく要確認の診断 | NDP-04/05のadapter入力は正規interface合意後に固定 |
| SDK | `sdk/rust/Cargo.toml` は `nagi-sdk` 0.1.0、`publish=false`、`nagi-model` の相対path依存。`hello-nagi` もrepo内SDKへのpath依存 | SDK ownerがout-of-tree参照・版固定方法を確認。未確認の配布packageを生成しない |
| App Contract | [Contract v1](../../../sdk/rust/docs/APP_SDK_CONTRACT.md)と `sdk/rust/src/app_contract/` にidentity、lifecycle、state、IPC envelope、capability request等の型あり | 型の存在をruntimeの起動・IPC接続・権限付与とみなさない。SDK本体は既存ownerが保持 |
| Manifest / Package | schemaは `sdk/rust/schemas/app-manifest.schema.json`。`tools/nagi-pkg/src/app_manifest.rs` が1 MiB制限、SDK版・厳格なフィールド・型・意味検証を実装。CLIは `manifest validate <file>` | 検証器を再実装しない。JSON v1と既存M16 `.xapp` Manifestは別契約。メタデータ検証はinstall/launch成功ではない |
| UI | [UI契約](../../architecture/ui-design-system.md)、`user/nagi-ui/**` は描画backendに依存しないデータ・操作契約。registryのownerは既存Integration Owner | NDP-06/07の最初のUI形式・対応範囲は未合意。rendererを新造しない |
| PT-06 | [既存評価票](../../porting-toolkit/PT06_MIGRATION_EVALUATION_TEMPLATE_2026-11-13.md)はTEMPLATE ONLY。`samples/hello-nagi` とSDK-owned Manifest fixturesは既存成果 | 自作の評価入力のみを初回候補にする。既存sampleを新規成果として複製しない |

## 初回開始条件 — NDP-10を先行（2026-11-15 承認）

以下は2026-11-15に創設者が確認した内容です。「指定」「確認」とある項目のうち指定を受けていないものは、上記の未指定事項に残します。依存するPT-01・SDK・UIの将来契約を、NDP-10の開始条件として追加しません。

| 項目 | 内容 |
|---|---|
| Core Developer | この作業の共同開発者（確定） |
| Founder / Integration Owner | 創設者が統合受取担当を指定（未指定）。既存ownerの引受けは未確認 |
| 初回task | NDP-10。自作のportable core、Windows/Linux APIを含む小さな入力、UI定義候補と期待診断・未検証事項のREADME |
| 作業環境 | 現在の専用checkoutを再利用、書込みAIは1セッション。新規clone/worktree、QEMU、モデル取得、全体buildなし |
| branch / PR base | 実装branch `codex/ndp-10-fixtures`、PR base `main`。指定ownerのreview・統合。mainへの直接push・mergeを含めない |
| 編集path | `tests/developer-platform/evaluation/**`。担当内説明も同pathのREADMEに置く |
| 競合照合 | 候補pathは不存在、専用登録なし。ただし `integration-next-phase` が `tests/**` 等を広く所有。Integration Ownerによる切出し確認が必要。未pushの他担当作業は確認不能 |
| 共通contract | PT-01 / SDK / Package / UIの既存ownerが保持。共有registry・Cargo・CIの更新は指定Integration Ownerが担当 |
| host-only checkpoint | NDP-10限定のM30前host準備として明示。workstream ID候補 `ndp-evaluation-host`、上記pathのみを登録する案 |
| Git操作 | 割当branchへの編集・host確認・commit・push・PR作成を一括で確認。push先は本体の担当branchか本人forkかを創設者が指定 |
| 外部入力・資源 | 公開Nagi資料と自作合成入力のみ。第三者アプリの取得・実行、新しい課金、追加の外部AI送信は含めない |
| 依存導入 | 初回入力・README作成は既存環境で可能。標準host条件を満たすPython 3.10以上、Rustが必要な工程では固定 `nightly-2025-08-01` の導入方法・範囲を確認。全依存は取得しない |
| レビュー | Core Developerが差分確認。DeepSeekレビューは既存認可済み担当への受渡し経路を創設者が指定。credentialを共有しない。経路未指定を新しい全面停止条件にはしない |

初回成果の確認は、入力ごとの目的・期待診断・未対応・利用範囲、元入力と生成物の分離、再実行時の入力不変を対象にします。未実装scannerの診断成功、実アプリ移植成功、Nagi target PASSは主張しません。

## 創設者へ送った確認内容（記録）

> NDP-01の読取照合を実施しました。初回はNDP-10を担当し、現在のcheckoutで `tests/developer-platform/evaluation/**` に自作の小さな評価入力とREADMEを作る案です。この限定host作業の担当・path切出し・M30前checkpointを確定し、統合担当とworkstream登録担当を指定してください。実装branchは `codex/ndp-10-fixtures`、PR baseは `main` を候補とし、push先、編集・検証・commit/push・PR作成、必要な依存導入の範囲をまとめて確認したいです。PT-01/SDK/UI接続未決のtaskは今回の実装対象に含めません。既存の確定記録があればその参照で足ります。

## 後続の並行候補

| Task | 編集path候補 | 開始前に残る接続 |
|---|---|---|
| NDP-04 | `tools/nagi-developer-platform/api/windows/**` | PT-01正規interface・owner合意 |
| NDP-05 | `tools/nagi-developer-platform/api/linux/**` | 同上 |
| NDP-06 | `tools/nagi-developer-platform/ui/**` | 最初のUI形式・UI owner合意 |
| NDP-10 | `tests/developer-platform/evaluation/**` | 開始承認済み（registry登録は未実施） |

初回はNDP-10の1セッションで開始します。NDP-02〜09・11・12も担当範囲に含まれるため、表と各taskファイルの接続条件が揃えば、創設者の追加割当なしで着手できます。編集path候補は着手時に確定します。並行セッションの起動・独立作業場所の作成は未実施。後続taskの依頼には、そのtask MD、本記録、固定base、独立したbranch/pathを [実装開始文](prompts/02_CODEX_IMPLEMENT.md)と一緒に渡します。

この記録をもってNDP-10の実装へ進みます。registry登録は指定された登録担当が行います。
