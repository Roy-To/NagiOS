# Nagi OS 0.2 — Sheets Calculation Core Host Foundation
**Document ID:** NAGI-0.2-PARALLEL-SHEETS-CALC-20261008\
**担当:** Claude ①\
**対象:** `RT-NISH/NagiOS`\
**作業区分:** Phase 2 Sheetsの **UI非依存・Host-only** 計算基盤\
**対象仕様:** `docs/NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md` §58、0.2 Master §27.1/§38\
**実行状態:** 新規Workstream提案。Ownerの登録・host-only activation checkpointを確認した範囲のみ実装する。

> **目標：** Nagi標準表計算「Sheets」に必要なWorkbook/Sheet/Cell/Formulaのコアを、Nagi本体UI・Kernel・Storageから独立したテスト可能なRustライブラリとして作る。AI抜きでも計算できる設計にする。

## 1. 開始時に必ず読むもの

1. `AGENTS.md`（対象パスのローカル版も含む）。
2. `docs/Nagi_OS_0.1_Codex_Implementation_Spec.md`、`docs/implementation_status.md`。
3. ユーザー提供 `Nagi_OS_0.2_Codex_Implementation_Spec.md` のM17、M21、M22、Phase 2並行開発、ゲート・所有範囲。
4. `docs/NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md` §58、共通Document/Object/Action設計。
5. `.dev/workstreams.json`、`.dev/workstreams/*/state.json`、`docs/0.2/WORKSTREAMS.md`。
6. 現在のSheets相当コード、既存`nagi-model`/Resource/Object/Action契約、既存SDK、保存モデル・テスト方式を**対象repo内だけ**検索。

**重要：** 既存Sheet/Cell/Formula実装がある場合は再利用を優先し、同じ責務のcrateを重複作成しない。

## 2. 所有権・ブランチ（すべて提案値）

- Workstream ID案：`sheets-calc-01`
- owner branch案：`claude/0.2-sheets-calc-01`
- worktree案：`../NagiOS-sheets-calc-01`
- host coreパス案：`crates/nagi-sheets-core/**`
- 単体テスト案：`tests/sheets-core/**`（ただし所有承認された場合のみ）
- ドキュメント案：`docs/workstreams/NagiOS_0.2_Sheets_Calculation_Core_Workstream.md`
- State案：`.dev/workstreams/sheets-calc-01/state.json`

**この一覧は登録を意味しない。** Integration Ownerへ`registration-proposal`（ID/owner/branch/base SHA/dependencies/allowed・forbidden paths/gate/merge boundary）を提出して、重複・衝突がないことを確認する。実装開始は登録と明示的host-only checkpointで認められた範囲に限る。

- **変更禁止：** `main`、`hark/**`、0.1 acceptance、`kernel/**`、`loader/**`、`user/nagi-init/**`、`third_party/**`、既存Sheets以外の純正アプリ、共通Cargo.toml/Cargo.lock、共有IDL/ABI、`.github/workflows/**`、`.dev/workstreams.json`、他WorkstreamのState。
- Root workspaceへ追加せずに単体ビルド可能な構成を優先。親workspaceとの関係が未承認なら独立したmanifest（必要ならcrate内`[workspace]`）でhost testsを動かす。
- 新規依存crate導入は既存lock/OSS方針と整合を確認し、共有lockに手を入れない。ネットワーク必須の計算エンジンにしない。
- ホームディレクトリや他repoを走査しない。作業対象はNagiOSの専用worktreeと許可済み一時出力だけ。

## 3. 設計原則

- **UI-independent**：レンダラ/デスクトップと計算エンジンを分ける。
- **Sparse Workbook**：未使用セルを大量に実体化しない。100,000+ populated cellsは将来の性能目標として計測可能な設計にする。
- **Opaque identity**：Workbook/Sheet/Range/Object IDはパス、配列添字、可変表示名と混同しない。公的ID contractがある場合はそれを使用。
- **Formula canonicalization**：関数IDは英語canonical。en-US/ja-JPのUI表示と内部式を分離。
- **Deterministic host tests**：時間関数は注入Clock、データ更新は明示操作。外部時刻/ネットワーク/LLMにテストを依存させない。
- **Structured Error**：`DIV_BY_ZERO`、`INVALID_REFERENCE`、`VALUE_ERROR`、`NAME_ERROR`、`NOT_AVAILABLE`、`CIRCULAR_REFERENCE`などを判別可能にする。
- **Untrusted formula**：パーサ入力/ネスト/セル数/範囲をboundedにし、panicや無限ループを避ける。数式を任意のOSコマンドとして実行しない。
- **Security forward-compat**：将来Actionを介した書き込みにpermission・session・Activity・Waybackが付けられるよう、計算core自体に権限を付与しない。

## 4. Host-only Deliverables

### S1 — 型とWorkbookの最小操作
- `Workbook`、`Sheet`、stable `SheetId`、`CellAddress`、`CellValue`、`CellError`。
- Cell型：`EMPTY / TEXT / NUMBER / BOOLEAN / DATE / DATETIME / DURATION / ERROR` の表現境界（初期は未実装型を明示してよい）。
- Sparseな内部保持、シート作成・rename、セル読み取り・typed value設定。
- 空欄・0・`FALSE`・空文字列を区別。明示的な入力エラーを定義。
- 文字列値はUTF-8、日本語も往復可能。

### S2 — Formula Parser / AST
- `=`から始まる式、数値、基本文字列、演算子の優先順位・括弧・単項符号。
- A1形式セル参照、矩形range、シート付き参照の最小形、絶対参照`$`の保持。
- 関数名をcanonical registryへ解決。未知関数、無効参照・不正構文を型付きエラーへ。
- ゲスト/Excel互換を偽称しない。完全なExcel formula languageは本範囲外。

### S3 — 計算エンジンと関数登録
- 最初に `+ - * /`、比較、`SUM`、`AVERAGE`、`MIN`、`MAX`、`COUNT`、`COUNTA`、`IF`、`AND`、`OR`、`NOT`、`ROUND` を実装。
- 余力があれば `IFERROR`、文字列基本関数（`LEFT/RIGHT/MID/LEN/TRIM/CONCAT`）を追加。
- 条件式とIFで不要分岐を評価しないルール、型変換ポリシー、空セル扱いを定義。
- `TODAY/NOW`など時刻依存関数を追加する場合はClock injection必須。日付シリアルとタイムゾーン規則は採用前に仕様を明文化。
- 数値の丸め/NaN/Infinity/Overflow挙動を明文化。

### S4 — Dependency Graph / incremental recalculation
- Cellから参照先の依存を追跡し、変更セルのtransitive dependentsのみ再計算。
- Formulaの上書き/削除/シートrename時に古い辺が残らない。
- DFS等で循環参照を検出。循環に巻き込まれたセルはstructured errorとする。
- 複数sheet/長い依存鎖でstack overflowしない上限、評価順、失敗時のatomicityを定義。
- テストで**再計算対象の数**を検証し、全セル再計算で見かけだけ通す実装を禁止。

### S5 — Host adapter/保存境界（できる範囲）
- 追加する場合は`serialize(snapshot)`/`load(snapshot)`のversionを明示。
- 永続化成功を装わず、単体コアのin-memory snapshotに留める。
- `sheets.get_range`/`sheets.set_values`/`sheets.set_formula`等へ繋ぐfacadeを契約として表現。ただし本物のPermission Serviceを自作しない。
- 将来Search/Wayback/Activityへ通知可能なイベント型を提案。Platform ID/ABIの共有定義は変更しない。

## 5. テスト計画（重点）

| ID | Acceptance |
|---|---|
| SHEETS-H01 | 2以上のsheetで値・名前・stable IDが衝突しない |
| SHEETS-H02 | 値の型とempty/null/zero/false/空文字が区別される |
| SHEETS-H03 | 算術、優先順位、括弧、絶対/相対参照のparse/evalが正しい |
| SHEETS-H04 | 代表的関数、範囲演算、IF分岐の評価が正しい |
| SHEETS-H05 | A1変更時に依存セルだけ再計算され、無関係セルは更新対象外 |
| SHEETS-H06 | 直接/間接/異なるsheetの循環参照はpanicなしで検出 |
| SHEETS-H07 | `DIV_BY_ZERO`、不正構文、未知関数、参照喪失を型付きエラーで返す |
| SHEETS-H08 | Formula削除/置換後に旧dependencyが残らない |
| SHEETS-H09 | 日本語文字列、シート名と英語canonical functionが衝突しない |
| SHEETS-H10 | 大きめのsparse gridと長い依存鎖をbounded memory/stackで処理 |
| SHEETS-H11 | 既存ownerのコード、Hark/main、root共有ファイルを変更していない |
| SHEETS-H12 | Stateとテスト証拠が実装commitに結びつき、push済み・worktree clean |

本Host FoundationのPASSは最終 `SHEETS-001`〜`028`の**全項目PASSを意味しない**。特にXLSX/CSV・GUI・チャート・本物のPersistence・agent編集・Waybackは別受入。

## 6. 実装と検証コマンド

実装が承認されている前提で、manifestの実際の場所に合わせて実行。

```bash
git status --short --branch
./nagi dev status
./nagi dev resume
./nagi dev verify
cargo fmt --manifest-path crates/nagi-sheets-core/Cargo.toml --all -- --check
cargo test --manifest-path crates/nagi-sheets-core/Cargo.toml --offline
cargo clippy --manifest-path crates/nagi-sheets-core/Cargo.toml --all-targets --offline -- -D warnings
git diff --check
```

独立manifestでroot lockの更新が不要な場合は`--locked`も用いる。コードがまだない状態では存在確認後にコマンドを組み立てる。Repo全体のbuild/target CIはこのHost Foundationの必須条件としない（未承認ゲート突破を避ける）。

## 7. 進め方と継続ルール

1. 承認済みowner pathを確定したら最小のS1から作業し、S2→S3→S4→S5へ進む。
2. 各ステップで実装→テスト→修正→再検証→own state更新→commit→own branch push。
3. テストが失敗したら再現コマンド・エラー・仮説・実施修正を記録し、同じ実験を無意味に繰り返さない。
4. blocked時に別streamや共有ファイルを勝手に変更しない。残りの承認済みin-scope項目へ移る。
5. M30 PASS + 0.2 activation checkpoint以前にnative UI、system IPC、本物のNagi guest runtimeへ接続しない。
6. Stateは登録されたschemaに従い、owned stateのみ更新。未登録時はregistration proposalとテスト証拠を残し、正式`PASS`や統合済みを偽称しない。
7. 最終報告：SHA/branch、追加ファイル、テスト数、性能計測、PASS/PARTIAL/BLOCKED、残るfeature gaps、統合に必要な所有者承認。

## 8. 非目標

- Full XLSX/CSV Import/Export、VBA、Power Query、完全なExcel関数互換。
- UI/grid rendering、macOS/Windows native app、Nagi target app起動。
- Model inferenceや自然言語Agentの実行。
- shared Platform APIsの独断変更、外部データ取得、root workspace/CI変更。
