# Nagi OS 0.2 — Writer Document Core Host Foundation
**Document ID:** NAGI-0.2-PARALLEL-WRITER-20261008  
**担当:** Claude ②  
**対象:** `RT-NISH/NagiOS`  
**作業区分:** Phase 2 Writerの **Host-only / UI-independent** 文書エンジン  
**対象仕様:** `docs/NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md` §57、0.2のResource/Document/Object/Revision/Activity/Wayback公共契約  
**実行状態:** 新規Workstream提案。Owner登録/host-only activation前にproductionへ接続しない。

> **目標：** Nagi WriterをAIがなくても通常の文書エディタとして成立させるため、Document/Section/Block/Style/Revisionの共通コアをRustで実装する。将来のAI局所編集とWaybackのために安定Object IDと差分を残すが、AIやゲストUIは実装しない。

## 1. 実装前に読む正規仕様

- `AGENTS.md`（パスローカルを含む）。
- `docs/Nagi_OS_0.1_Codex_Implementation_Spec.md` と `docs/implementation_status.md`。
- ユーザー提供 `Nagi_OS_0.2_Codex_Implementation_Spec.md` の §23/24（Resource/Document/Object/Revision/Activity/Wayback）、M17/M18/M21/M22、Phase 2並行ポリシー。
- `docs/NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md` §57（Writer）。
- `.dev/workstreams.json` / `docs/0.2/WORKSTREAMS.md` / 既存`nagi-model`・`nagi-notes`・Wayback・SDK契約。
- 既存の文書やRevision等の実装有無。既存Notesやpublic contractが使えるなら再利用を優先。

**特に重要：** Track Changes（文書内改訂）とOS Activity（誰が何をしたかのLedger）を別責務とし、混同しない。

## 2. 所有権と分離

| 項目 | 新規提案 |
|---|---|
| Workstream ID | `writer-core-01` |
| owner branch | `claude/0.2-writer-core-01` |
| worktree | `../NagiOS-writer-core-01` |
| ソース | `crates/nagi-writer-core/**` |
| テスト | `tests/writer-core/**`（承認時のみ） |
| 設計書 | `docs/workstreams/NagiOS_0.2_Writer_Document_Core_Workstream.md` |
| State | `.dev/workstreams/writer-core-01/state.json` |

- **全て案であり現時点でregistry登録済みではない。** Integration Ownerへproposal提出、branch/path重複解消、host-only checkpoint明示後に該当作業を開始。
- 新規crateをroot Cargo workspaceへ無断登録しない。単独manifestとしてformat/test/Clippyが実行できるようにする。
- 既存shared `DocumentId`/`ObjectId`/`RevisionId`があるなら共有定義を使う。未完成ならcrate内の可換adapter型を明示し、将来platform移行用compatibility fixtureを用意する。
- 外部ツール/フォント/マシン固有ファイル/ブラウザを隠れた必須依存にしない。

### 変更禁止領域

`main`、Harkの`hark/**`、`kernel/**`、`loader/**`、`user/nagi-init/**`、`third_party/**`、0.1 milestone acceptance、共有root Cargo.toml/Cargo.lock、共有IDL/ABI、CI、`.dev/workstreams.json`、他Workstreamのstate、Notes/Wayback本体の無断変更。NagiOS checkout外のプライベートファイルを探索しない。

## 3. モデル設計

```text
DocumentId                  // stable logical identity
RevisionId                  // immutable version identity
ObjectId                    // stable block/paragraph/table identity
Document
  metadata / settings
  ordered Sections / Blocks
    Heading
    Paragraph
    List
    Table
    Quote
    CodeBlock
    Citation / LinkedReference (source metadata)
    Optional Figure Placeholder
  Styles (Title/Subtitle/Heading/Body/Quote/Caption/Code/Custom)
  Comments (thread/reply/resolve)
  Revision history / ChangeSet
  External references (resource + revision when known)
```

### 不変条件

- 文章の挿入/並べ替え/スタイル変更後も、変更対象でない既存ObjectのIDは変わらない。
- 表示位置（何段落目か）やファイル名をObject IDの代用にしない。
- 操作は期待Revisionを受け取り、stale revisionなら明示的Conflictを返す。
- ユーザーの元データを失う可能性のあるimport/exportは事前警告、unsupported featureは黙って破棄しない。
- すべての構造化変更はtyped `ChangeSet`/Diffへ変換できるようにする。
- undo/restoreに必要な逆操作やcheckpointは**可能性として記述**し、実Store/Waybackなしに「復元済み」と返さない。
- 外部Citationはuntrusted data。ネットワーク取得や埋め込みコード実行をしない。
- 日本語/英語テキストをUTF-8で保持。文字位置はbyte indexと可視文字indexを混同しない（カーソル扱いを仕様化）。

## 4. 段階的Deliverables

### W1 — Document tree / typed operations
- Document/Section/Heading/Paragraph/List/Table/Quote/CodeBlock/Citationの最小構造。
- `insert_*`、`replace_text`、`move_object`、`delete_object`、`get_outline`相当の純粋/副作用限定コアAPI。
- `ObjectId`の一意性と安定性、親子循環を拒否。
- `DocumentId`や`RevisionId`等の現行共有contractの参照を明記。

### W2 — Styles / Outline / multi-language
- Title/Subtitle/Heading1-3/Body/Quote/Caption/Codeのスタイル定義と継承・適用。
- 直書き装飾よりstyle-based formattingを優先。
- HeadingからOutline生成。見出し並べ替え・段落挿入後も安定して参照可能。
- 日本語文字列、改行、全角/半角、リストや引用を含むfixture。

### W3 — Revision / Track Changes / Comments
- Immutable revision、ChangeSet、add/delete/move/formatの差分区分。
- 各ChangeSetにActor/Source/Timeを付けられるmetadataのboundary。ただしOS Activity Ledger本体は編集しない。
- Edit concurrency：期待Revision不一致時には拒否または明示的なrebase plan。勝手にlast-write-winsにしない。
- Comment / Reply / ResolveをObjectIdに関連づける。
- 元文書を壊さず変更をpreviewできる純粋関数を追加できれば優先。

### W4 — Markdown/plain textの安全な入出力
- MarkdownからHeading/Paragraph/List/Quote/Codeの**対応済みsubset**をimport。
- 現時点で未対応の表/画像/HTML等は実装範囲を明示し、UnsupportedWarningで保持・保留・拒否のいずれかを返す。重要内容のsilent dropは禁止。
- Markdown/plain textのexportとroundtrip fixtureを用意。
- 64-bit OS・host font/layoutに依存せず、文字化け・改行正規化・大きな入力のbounded解析を検証。
- PDF/DOCXの出力は本Workstreamの完成条件にしない。別I/O providerと安全なformat互換テストが必要。

### W5 — Platform adapter seam（実接続なし）
- 将来`writer.create/open/save/get_document/get_outline/insert_paragraph/apply_style/export`等へ接続できるtrait/schemaを提案。
- SearchはDocument title/headings/paragraphsから安全にIndex itemへ投影可能なinterfaceのみ。
- ActivityとWaybackは注入するsink/checkpoint adapterで表現。機能が存在しない場合はUnavailableを正しく返す。
- `ObjectId`を介したAlbert Citation / Notes → Writer / Sheets-linked-referenceは参照型のみを用意。実Cross-App採用は0.2正式activation後。
- User/Profile/Permissionは境界で注入できるように設計し、Writerコアが直接権限を作り出さない。

## 5. Host Acceptance

| ID | 判定基準 |
|---|---|
| WRITER-H01 | AIなしで文書作成・追加・移動・削除・閲覧が成立 |
| WRITER-H02 | Heading/Paragraph/List/Table/Quote/Code/Citationの基本構造を保持 |
| WRITER-H03 | 変更対象外Object IDは編集・再順序化・読込後も維持（可能範囲） |
| WRITER-H04 | Style変更が構造と分離され、Outline生成が正しい |
| WRITER-H05 | Revision履歴が不変で、stale Revision更新を拒否 |
| WRITER-H06 | add/delete/move/formatが型付きChangeSetに表現される |
| WRITER-H07 | Comment/Reply/Resolveがstable Objectに結びつく |
| WRITER-H08 | 対応Markdown subsetとPlain textが正しくimport/export/roundtrip |
| WRITER-H09 | 未対応要素が警告または明示エラーとなり、無言の内容喪失なし |
| WRITER-H10 | 日本語/英語混在、Unicode境界、不正長入力を安全に処理 |
| WRITER-H11 | Wayback/Activity adapter不在時に復元・記録済みを偽称しない |
| WRITER-H12 | format/Clippy/Tests、State更新、Commit・PushがSHAと対応 |

このHost-only PASSは最終`WRITER-001`〜`020`のフルAcceptanceではない。`WRITER-006` PDF出力、`WRITER-008` DOCX、`WRITER-009` Albert integration、`WRITER-014` actual Wayback、native UI、target persistenceは別gate。

## 6. 実行コマンド例

```bash
git status --short --branch
./nagi dev status
./nagi dev resume
./nagi dev verify
cargo fmt --manifest-path crates/nagi-writer-core/Cargo.toml --all -- --check
cargo test --manifest-path crates/nagi-writer-core/Cargo.toml --offline
cargo clippy --manifest-path crates/nagi-writer-core/Cargo.toml --all-targets --offline -- -D warnings
git diff --check
```

`--locked`を適用できる場合は適用。依存やtarget buildが未承認で失敗するなら、shared rootを書き換えず既存許可範囲のhost検証に限定。出力したMarkdownを実測テストせずに対応済みと言わない。

## 7. 作業継続と状態記録

1. AGENTS/M30状態とworkstream承認・許可パスを確認。
2. W1→W2→W3→W4→W5を小さく実装する。既存コアが同等なら重複作成しない。
3. 各単位で実装→テスト→修正→再テスト→owned Stateの証拠更新→commit→own branch push→CI確認。
4. 失敗は再現、Root cause、試した修正、次の別仮説を記録。同じ失敗コマンドの無意味な繰り返し禁止。
5. 書き換えが許されないPlatform契約の変更が必要なら、修正案・期待consumer・compatibility fixturesをIntegration Ownerへ提出する。
6. 承認範囲内の項目が残っていれば次を実装。ゲートで止まる場合はゲートを回避しない。
7. 最終報告は、Host PASS/PARTIAL/BLOCKED、実装範囲、未対応Format/UI/Wayback、Tests、SHA、Push先、CI evidence、統合に必要なcheckpoint。

**非目標：** Writer UI、DOCX/PDF engineの本格実装、フォント/レイアウトエンジン、AI自動編集、Wayback target runtime、Nagi Agent、root workspace/CIの無断変更、Hark branchの改修。
