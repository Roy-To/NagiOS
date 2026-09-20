# NAGI First-Party Software Suite — Final Implementation Specification

- Document ID: `NAGI-FIRST-PARTY-SUITE-SPEC`
- Status: Implementation directive
- Target: Nagi OS / Nagi Platform
- Primary implementation surface for Nagi 0.x: Desktop
- Architecture target: Desktop / Tablet / Mobile / future surfaces
- Product name: **NAGI — Native Agentic General Interface**
- Working application names in this document are provisional unless an existing Nagi specification already fixes the name.

---

## 0. この文書の目的

この文書は、Nagi OS に搭載する純正アプリケーション群と、それらを成立させる Nagi Platform 共通基盤を実装するための最終実装指示書である。

本書はアイデア集ではない。Codex は本書を、既存リポジトリの `AGENTS.md`、Nagi OS 主仕様、既存の共通基盤仕様、既存コードと突合し、矛盾を解消したうえで実装計画・コード・テストへ落とし込むこと。

この文書の中心目標は次の4点である。

1. Nagi 純正アプリを「Nagi Core / Nagi Platform の能力を最大限利用したリファレンス実装」として成立させる。
2. 純正だけが秘密の内部 API を利用する閉鎖構造を避け、将来サードパーティも同じ公開 API で深い統合へ到達できる構造にする。
3. Nagi 0.x の実装は PC / Desktop first とする一方、論理モデル・Action・Document・Workspace・Continuity を端末非依存にし、Tablet / Mobile へ後から水平展開できる構造にする。
4. 個々のアプリをバラバラに実装せず、Resource / Document / Object / Action / Intent / Context / Workspace / Activity / Wayback / Search / Continuity を共通基盤として再利用する。

---

# Part I — Codex 実行規則

## 1. 仕様の優先順位

実装開始前に必ず以下を確認すること。

1. リポジトリ直下および対象ディレクトリ配下の `AGENTS.md`
2. Nagi OS の主仕様・共通基盤仕様
3. 既存の architecture / design / security / packaging / localization / device / model 関連仕様
4. 本書
5. 既存コードとテスト

矛盾が存在する場合は、上位仕様を優先する。ただし、本書が既存仕様より新しい追加要件として明示的に成立する場合は、既存仕様を破壊せず統合すること。

本書と既存仕様の食い違いを見つけた場合、場当たり的に片方を無視してはならない。整合案をコードとドキュメントの双方へ反映し、`implementation_status.md` に判断を記録すること。

## 2. 既存実装を捨てない

特に Albert は既存 0.1 実装を基礎として発展させる。

Codex は既存コードを監査せず、同等機能を新規実装して置き換えてはならない。

既存実装は最低限、次の分類を行うこと。

- `KEEP`: そのまま維持
- `ADAPT`: Nagi Platform に接続して再利用
- `REPLACE`: 技術的理由を明記したうえで置換
- `DEFER`: 今回対象外として残す

## 3. 実装中断を最小化する

Codex は、軽微な不明点・命名・内部構造の選択でユーザー確認を要求して実装を止めないこと。

既存仕様、本書、テスト可能性、保守性から合理的な選択を行い、判断を `implementation_status.md` へ記録する。

外部資格情報、ライセンス同意、実機固有情報など、コードだけでは解決不能なブロッカーを除き、可能な範囲を先に最後まで進める。

同一障害については、原因を変えながら最大10回程度の合理的な修正・再試行を行ってから `BLOCKED` と判定する。無限ループは禁止する。

## 4. Fake 成功禁止

以下は禁止。

- 未実装なのに成功レスポンスを返す
- TODO を通すためだけの常時 true テスト
- ダミー保存で「永続化済み」と扱う
- 外部送信に失敗したのに送信済み表示
- Restore できない処理に見せかけの Undo を表示
- AI が変更しているのに Activity を記録しない
- Import 時に未対応データを黙って破棄する
- Tablet / Mobile 対応として単に Desktop UI を縮小しただけで完了扱いする

## 5. 実装ステータス

リポジトリに既存運用がない場合、`docs/first_party/implementation_status.md` を作成し、少なくとも以下を管理する。

- Milestone
- Status: `NOT_STARTED / IN_PROGRESS / PASS / BLOCKED / DEFERRED`
- Implemented components
- Tests run
- Acceptance tests passed
- Known limitations
- Migration notes
- Decisions made
- Next action

各 Milestone は Acceptance Test が通るまで `PASS` にしてはならない。

---

# Part II — 製品原則

## 6. 第一原則: 参入は簡単、深い統合は強力

Nagi 向けアプリ開発は、Nagi 独自言語や純正専用 IDE を必須にしてはならない。

理想は次である。

- 既存コードを Nagi target 向けにビルドしやすい
- Nagi の特殊機能を一切使わないアプリも動作できる
- 必要な API だけ段階的に採用できる
- 深く統合すれば、Context / Agent / Activity / Wayback / Workspace / Search / Continuity を利用できる
- 純正アプリはその最上位の実例となる

## 7. 純正アプリの優位性は秘密 API ではなく統合深度

原則として、純正アプリとサードパーティアプリは同じ Nagi Platform Public API を利用する。

純正アプリが強力である理由は、Nagi Platform の機能を最大限使うからであり、一般開発者が利用できない裏 API を持つからではない。

例外は、OS 自身を管理するために明示的な `System Capability` が必要な操作のみとする。

## 8. Nagi Core と Nagi Platform

用語を次のように区別する。

### Nagi Core

OS 内部で稼働する共通サービス群。

例:

- App Registry
- Resource Service
- Action Registry
- Intent Router
- Context Broker
- Workspace Service
- Activity Ledger
- Checkpoint / Revision Service
- Search Service
- Permission / Capability Service
- Continuity Service
- Device Service

### Nagi Platform

アプリから Nagi Core を利用するための公開契約・SDK・ABI・API 群。

純正も第三者も原則として Nagi Platform 経由で Core を利用する。

## 9. Integration Level

開発者にフル統合を強制しない。

### Level 0 — Portable

- 通常のアプリとして起動可能
- 標準ウィンドウ・入力・ファイル選択等
- Nagi AI 連携不要

### Level 1 — Platform Aware

- App Manifest
- Resource ID
- Context Publish
- Search Provider の一部

### Level 2 — Agent Ready

- Structured Action
- Intent 対応
- Activity
- Permission-aware Agent 操作

### Level 3 — Deep Native Integration

- Workspace
- Wayback / Checkpoint
- Cross-App Object Reference
- Provenance
- Continuity
- Multi-surface

純正アプリは原則 Level 3 を目標とする。

第三者も同一公開 API を利用して Level 3 に到達できなければならない。

---

# Part III — Nagi Platform Core Model

## 10. App

`App` は Nagi 上で動作するソフトウェア単位。

最低限、以下を持つ。

- stable app ID
- display name
- version
- executable / entry point
- supported surfaces
- permissions
- capabilities
- action declarations
- search provider declarations
- resource/document types

表示名と ID は分離する。

例:

```text
com.nagi.albert
com.nagi.files
com.nagi.writer
com.example.thirdparty
```

本書内の ID は作業用。既存 Nagi package/app ID 規約が存在する場合はそちらへ合わせる。

## 11. App Package

既存 package format がある場合はそれを優先する。

存在しない場合の論理要件は次の通り。

```text
app bundle
├─ manifest
├─ executable / runtime payload
├─ assets
├─ localization
├─ action schemas
├─ optional search/resource provider metadata
└─ signature metadata
```

配布用拡張子として `.napp` を採用する場合でも、中身を完全ブラックボックスにしない。

Nagi Store は唯一のインストール経路にしない。署名・権限・互換性確認を満たす package はサイドロード可能であること。

## 12. Permission / Capability / System Capability

### Permission

ユーザーデータやデバイス機能へのアクセス許可。

例:

- `files.read`
- `files.write`
- `contacts.read`
- `camera`
- `microphone`
- `location`

### Capability

Nagi Platform への参加能力。

例:

- `context.publish`
- `context.read`
- `actions.register`
- `search.provider`
- `workspace.participate`
- `wayback.checkpoint`

### System Capability

OS 自体に影響する強い権限。

例:

- `system.settings.write`
- `system.packages.install`
- `system.credentials.access`
- `system.devices.manage`

純正アプリだから自動許可、というロジックは禁止する。

## 13. Resource

`Resource` は Nagi が識別・参照できるデータ対象の共通概念。

例:

- Local file
- Folder
- Trash item
- App document
- Web page
- Cloud file
- External device file
- Future remote resource

Resource は可能な限り path とは別の stable opaque ID を持つ。

移動や rename 後も同一 Resource と追跡できる場合は ID を維持する。

## 14. Document

`Document` はユーザーが所有・保存・再度開ける永続的成果物。

Document と File は同一概念ではない。

Document はローカルファイル、DB、将来のクラウド等に保存され得る。

最低限:

- document ID
- type
- provider app ID
- title
- revision
- timestamps
- permissions
- resource backing reference

## 15. Object

`Object` はアプリ内に存在し、Nagi から意味を持って参照できる単位。

例:

- Writer paragraph / heading / table / citation
- Sheets table / range / chart / named range
- Slides slide / text box / chart
- Notes block
- Albert page / selection / image / link

Object は安定 ID を持ち、単なる UI 座標や配列 index だけで識別しない。

## 16. Resource Reference

Cross-App 参照は raw path やアプリ内部 ID の直結ではなく、Nagi Resource Service で解決する。

概念例:

```text
nagi://<app>/<resource>/<id>/object/<object-id>
```

文字列形式は既存規約があれば従う。

アプリは URI を勝手に解析せず Resource Service を通す。

## 17. Revision

Document / Object / Resource は必要な範囲で revision を持つ。

参照元は source revision の更新を検知できること。

Last Writer Wins を競合解決の基本戦略にしない。

## 18. Provenance

AI が生成・引用・変換した Object は可能な限り provenance を保持する。

最低限:

- source resource/object
- creator / actor
- timestamp
- transformation/action
- source revision

Nagi が内容だけをコピーし、出典を消す実装を避ける。

## 19. Action

`Action` は App が外部へ公開する、入力・出力・副作用が定義された操作。

例:

```text
files.rename
writer.replace_text
sheets.set_range_values
albert.open_url
```

Action は schema を持つ。

最低限:

- action ID
- input schema
- output schema
- side effect classification
- reversible flag
- required permissions/capabilities
- risk metadata

GUI 座標をクリックする操作を標準 Action にしてはならない。

## 20. Action Risk

最低限:

- `READ`
- `MODIFY`
- `EXTERNAL`
- `DESTRUCTIVE`
- `UNKNOWN`

`UNKNOWN` を安全扱いしない。

外部送信や破壊的操作は、明示的な許可ポリシーなしに自動実行してはならない。

## 21. Intent

`Intent` は「ユーザーが何を達成したいか」を表す高レベル要求。

Action と Intent を混同しない。

例:

```text
Intent: create_presentation_from_dataset
Actions:
  files.open
  sheets.import_csv
  sheets.analyze
  slides.create
  slides.insert_chart
```

Intent は特定純正アプリへ不要に固定しない。

## 22. Context

Context は「今・直前の作業状態」。長期 Memory と分離する。

Context 例:

- active app
- active document
- selected objects
- selected text
- current workspace
- current tab
- current range
- recent resources

App は Context Broker へ明示的に Publish する。

Core が勝手にアプリメモリを読み取る設計は禁止する。

Context には provenance を保持する。

## 23. Workspace

Workspace は、ひとつの目的に関係する複数 App / Document / Resource / Object / Session を束ねる意味的作業空間。

フォルダではない。

同じ Resource が複数 Workspace に属してよい。

最低限:

- workspace ID
- title
- resource references
- document references
- app session references
- active/recent state
- continuity metadata

## 24. Activity

Activity は人間が追跡可能な意味のある操作履歴。

単なる debug log ではない。

Actor は最低限:

- `USER`
- `AGENT`
- `APP`
- `SYSTEM`
- `AUTOMATION`
- `REMOTE_DEVICE`

AI がユーザー成果物を変更した場合、Activity なしの silent mutation を禁止する。

## 25. Transaction

複数 Action をひとつのユーザー目的として grouping できること。

複数 App をまたぐ Transaction も同一 transaction ID で追跡する。

完全 ACID を外部世界へ要求しない。

- local reversible operation → checkpoint / undo
- external operation → compensating action if available
- irreversible operation → explicit confirmation/policy

## 26. Checkpoint / Wayback

Checkpoint は復元可能地点。

最低限:

- Action Undo
- Document Checkpoint
- Workspace Checkpoint

System Snapshot は後段でもよいが API 設計は拡張可能にする。

復元不能操作に fake Undo を表示しない。

## 27. Device

Device は物理・仮想端末。

App は `if phone` のような device class 固定ロジックへ依存しすぎず Capability を見る。

## 28. Surface

App UI の表示形態。

標準候補:

- `desktop`
- `tablet`
- `mobile`
- `external-display`
- `compact-overlay`

Surface は width / height / DPI / orientation / input capability と合わせて扱う。

## 29. Device Capability

例:

- keyboard
- pointer
- touch
- pen
- camera
- microphone
- location
- large display
- multi-window
- local GPU
- local AI accelerator
- biometric auth

## 30. Continuity

Continuity は別端末で意味的に作業を継続する仕組み。

共有するもの:

- open document/resource
- active object/section/range
- workspace
- recent task
- app semantic session state

原則共有しないもの:

- pixel position
- mouse cursor position
- Desktop window coordinates
- device-specific absolute path

別端末では同じ Document / Object を、その端末の Surface で再構成する。

---

# Part IV — 共通サービスと Framework

## 31. Activity Ledger Service

Activity は3階層で扱う。

### Technical Event

内部処理単位。

例:

- `writer.insert_text`
- `sheets.set_formula`
- `files.move`

### Action Group

ユーザーが理解できるまとまり。

例:

- 「Executive Summary を修正」
- 「14ファイルをリネーム」

### Transaction

複数アプリをまたぐ仕事単位。

例:

- 「sales.csv から経営会議用プレゼンを作成」

Activity UI は Transaction / Action Group を主に表示し、Technical Event は詳細で展開する。

イベントモデルには最低限以下を持つ。

```text
event_id
timestamp
actor
app_id
action_id
target_resources[]
source_resources[]
workspace_id?
transaction_id?
summary
reason?
reversible
checkpoint_before?
checkpoint_after?
result
metadata
```

Activity は append-oriented とし、過去履歴を都合よく書き換えてはならない。

## 32. Activity Privacy

Activity DB に機密本文を無条件複製しない。

アプリは sensitivity / redaction policy を指定できること。

秘密情報については、アクセスした事実は必要に応じて残しても、credential 本文や API key を記録してはならない。

## 33. Diff Provider

各 Document Provider は意味のある Diff を提供できる。

例:

- Writer: paragraph/text diff
- Sheets: cell/range/formula/chart diff
- Slides: slide/object diff
- Files: rename/move/delete diff
- Notes: block diff

Provider が高度 Diff を持たない場合でも revision change は表示する。

## 34. Wayback / Checkpoint Service

Checkpoint type:

- `AUTO`
- `USER`
- `AGENT`
- `TRANSACTION`
- `SYSTEM`

Agent による大規模変更前は自動 checkpoint を作成できること。

復元 UI は最低限:

- Open this version
- Compare with current
- Restore
- Restore as copy

`Restore as copy` は必須機能として扱う。

## 35. Workspace Checkpoint

Workspace 単位で復元対象を列挙し、部分復元を許可する。

例:

```text
[x] Writer documents
[x] Notes
[ ] Files
[x] Albert session
[ ] Terminal metadata
```

External side effects は Wayback で消去できないことを UI と API で明示する。

## 36. Search Service

Search App と Search Service を分離する。

各 App は Search Provider として参加可能。

最低限の検索:

- lexical
- metadata
- semantic
- temporal/activity

Semantic Search を無効にしても基本検索が成立しなければならない。

Search Record の論理モデル:

```text
record_id
resource_id
object_id?
provider_id
type
title
searchable_text
metadata
provenance
created_at
modified_at
workspace_ids[]
sensitivity
permissions
revision
```

## 37. Search Ranking

少なくとも次を考慮可能にする。

- exact match
- metadata match
- current context relevance
- workspace relevance
- recency
- semantic similarity
- user behavior signal

Exact match が Embedding の都合で不当に下位へ落ちる設計を避ける。

可能な範囲で検索結果の理由を説明できること。

## 38. Search Security

Search index に存在していても権限のない Resource を結果へ返さない。

権限のない Resource の存在そのものを漏らさない。

Index policy:

- `NONE`
- `METADATA_ONLY`
- `CONTENT`

Credential 等は原則 `NONE` または `METADATA_ONLY`。

## 39. Incremental Index

初回 Full Scan 後は Resource event / revision を利用して incremental update を行う。

毎回全ストレージを再走査しない。

## 40. Nagi Document Framework

Writer / Sheets / Slides / Notes 等で重複実装しない共通層を用意する。

最低限:

- Document identity
- Revision
- Autosave
- Checkpoint hooks
- Stable Object IDs
- Resource embedding/reference
- Cross-App reference
- Import/export lifecycle
- Search provider integration
- Activity hooks
- Diff provider contract
- Continuity state
- Permission model
- localization metadata

App 固有ロジックは Framework の上へ載せる。

## 41. Autosave と Activity の分離

Autosave の内部 revision と人間向け Activity は同一ではない。

毎キー入力ごとに Activity Event を生成しない。

ユーザーが理解できる意味のある変更単位へまとめる。

## 42. Cross-App Reference Graph

Nagi Platform は App 間参照を追跡できる共通 Reference Graph を持つ。

例:

```text
Slides chart
  -> Sheets chart
  -> Sheets table/range
```

```text
Writer citation
  -> Albert page/selection
```

```text
Notes file reference
  -> Files resource
```

Source revision が変わった場合、参照側は更新検知できること。

自動更新 / compare / keep current を App が選べる契約にする。

## 43. Language / Localization

Nagi の内部識別子・API・canonical schema は英語基準とする。

ユーザー向けには English / Japanese を同格の正式対応言語として扱う。

日本語を英語 UI の翻訳おまけとして扱わない。

共通要件:

- UI strings をコードへ直書きしない
- UTF-8 を基本
- 日本語 IME を考慮
- mixed-language document を扱える
- date / number / currency display を locale-aware にする
- internal formula/function ID は locale から独立させる

## 44. Device-neutral Core

Nagi 0.x は Desktop Surface を実装必須とする。

Tablet / Mobile は以下を必須とする。

- Core が Desktop UI へ依存しない
- Surface contract を用意
- Continuity state が device-neutral
- pointer/keyboard 必須前提を避ける
- fixed resolution 前提を避ける

Full Tablet/Mobile UI は段階実装でよい。

## 45. Conflict Handling

複数端末で同一 Document を編集した場合、revision mismatch を検知する。

可能なら object-aware merge。

自動マージ不能なら Compare UI を提示する。

Last Writer Wins で黙って上書きしない。

## 46. Offline

対応可能な純正アプリは offline でも基本編集・閲覧を継続できること。

同期可能になったら:

- revision sync
- Activity sync
- conflict detection

External Action が offline で失敗した場合、完了扱いしない。

Decision Provider が不在・失敗・offline の場合は、互換する local
provider、`LlmDecisionAdapter`、Generative/Reasoning path、または
deterministic/manual fallback を policy の範囲で選択する。fallback に
よって危険な操作の policy を弱めてはならない。Jev や cloud decision
service がなくても Nagi 0.1 の通常機能は成立する。

## 47. Accessibility

純正アプリ共通で最低限:

- keyboard navigation
- focus visibility
- semantic accessibility tree
- screen reader labels
- high contrast compatibility
- text scaling
- touch target sizing for touch surfaces

## 48. Performance 共通原則

- UI thread を長時間 I/O で blocking しない
- virtualization を利用
- incremental persistence / indexing を優先
- huge object collection を eager materialize しない
- AI unavailable 時も通常機能を継続
- background work は cancellable / observable にする

---

# Part V — Phase 1: OS Core Experience

Phase 1 の作業名:

- Home
- Albert
- Files
- Notes
- Terminal
- Activity
- Wayback
- Search

名称は暫定。ただし既存 Albert 名は継続前提。

---

# 49. Activity App

## 49.1 Purpose

Nagi 全体で発生した意味のある操作を、人間が理解できる形で追跡・説明・比較・Undo 起点として扱う。

## 49.2 Desktop UI

Timeline を中心にする。

最低限の filter:

- All
- You
- Nagi
- Automations
- System
- Apps
- Workspaces
- Documents
- Devices

各 Event / Transaction から:

- View details
- View changes
- Open target
- Undo if reversible
- Open in Wayback

へ遷移できること。

## 49.3 Detail

表示候補:

- Actor
- Time
- Device
- App
- Workspace
- User request / reason
- Sources
- Changed objects
- Checkpoint before/after
- Result

## 49.4 Non-goals

- mouse move / hover / scroll の完全記録
- key logger
- debug log viewer の代替

## 49.5 Acceptance

- `ACT-001`: USER と AGENT の変更を区別できる
- `ACT-002`: 複数 Action を Transaction でまとめられる
- `ACT-003`: failed/partial transaction を success 扱いしない
- `ACT-004`: target Resource を開ける
- `ACT-005`: reversible event から Undo/Wayback へ遷移できる
- `ACT-006`: secret payload を無条件保存しない
- `ACT-007`: Device / Workspace を追跡できる

---

# 50. Wayback App

## 50.1 Purpose

Document / Workspace 等を過去状態へ復元する人間向け UI。

## 50.2 Levels

- A: Action Undo
- B: Document Checkpoint
- C: Workspace Checkpoint
- D: System Snapshot — deferred/architecture only for 0.x

## 50.3 UI

Timeline + Preview を基本とする。

各 checkpoint で:

- Open this version
- Compare
- Restore
- Restore as copy
- Pin checkpoint

を提供する。

## 50.4 External Side Effects

Mail sent / publish / upload / purchase / API request 等は過去へ戻せない。

compensating action が存在する場合のみ、それを別 Action として提示する。

## 50.5 Acceptance

- `WB-001`: Document checkpoint を復元できる
- `WB-002`: Restore as copy が可能
- `WB-003`: Restore 自体が Activity に残る
- `WB-004`: irreversible action に Undo を出さない
- `WB-005`: Workspace 部分復元が可能
- `WB-006`: pinned checkpoint が自動整理で消えない

---

# 51. Files App

## 51.1 Purpose

ユーザーが所有・参照する Resource を発見・整理・操作し、Context / Workspace / Search / Activity / Wayback へ接続する標準 Resource Manager。

## 51.2 Desktop UI

基本3ペイン:

```text
Sidebar | Resource View | Inspector
```

Sidebar:

- Home
- Recent
- Starred/Pinned
- Workspaces
- Downloads
- Documents
- Locations
- Devices
- Trash

Inspector:

- Preview
- Metadata
- Activity
- Versions
- Provenance

Inspector は隠せること。

## 51.3 Basic operations

- open
- create folder
- copy
- move
- rename
- duplicate
- trash
- restore
- permanent delete
- cut/copy/paste
- drag/drop
- sort/filter/view mode

通常 Delete は Trash へ送る。

Permanent delete は destructive action。

## 51.4 Search

Files 内検索も Search Service を利用する。

- exact filename/path
- metadata
- semantic

を統合する。

## 51.5 Workspace

`Add to Workspace` は移動ではなく Reference 追加。

## 51.6 Tags

System Metadata Store で管理し、不要な sidecar file を大量生成しない。

## 51.7 Agent Actions

最低限:

```text
files.list
files.search
files.open
files.create_folder
files.copy
files.move
files.rename
files.duplicate
files.delete
files.restore
files.get_metadata
files.set_tags
files.add_to_workspace
files.remove_from_workspace
files.delete_permanently
```

## 51.8 Context

- current location
- selected resources
- focused resource
- recent resources
- active workspace

## 51.9 Resource Provider Architecture

v0.1 は LocalFileProvider / TrashProvider から開始可能。

将来 Cloud / NAS / Remote Device を追加できる provider contract を先に用意する。

## 51.10 Acceptance

- `FILES-001`: 基本ファイル操作が GUI で可能
- `FILES-002`: 同等操作が Action API から可能
- `FILES-003`: Agent 変更が Activity に残る
- `FILES-004`: rename/move 後も可能な限り Resource ID 維持
- `FILES-005`: Trash delete を Undo/restore できる
- `FILES-006`: permanent delete に明示確認
- `FILES-007`: selection を Context publish
- `FILES-008`: Workspace 追加で実ファイル位置が変わらない
- `FILES-009`: supported Resource を Wayback で復元
- `FILES-010`: Core test が Desktop UI なしで動作

---

# 52. Search App

## 52.1 Purpose

Files / Documents / Notes / Web / Activity / Workspace 等を横断検索する UI。

Search Service と UI は分離する。

## 52.2 Result categories

- Everything
- Files
- Documents
- Notes
- Web
- Activity
- Workspaces
- Apps

## 52.3 Quick Search

OS 全体から呼べる Quick Search を提供する。

Search と Intent execution は内部的に分離する。

## 52.4 Context-aware ranking

Current Workspace / Context を ranking boost に利用可能。

ただし勝手に検索範囲を狭めず、scope を UI で確認可能にする。

## 52.5 Actions

```text
search.query
search.query_resources
search.query_objects
search.query_activity
search.open_result
search.reveal_source
search.reindex_resource
```

## 52.6 Acceptance

- `SEARCH-001`: exact filename search
- `SEARCH-002`: Files/Notes/Albert/Activity provider 共通 API
- `SEARCH-003`: metadata filter
- `SEARCH-004`: semantic search on/off
- `SEARCH-005`: semantic disabled でも基本機能成立
- `SEARCH-006`: permission filtering
- `SEARCH-007`: result から該当 Object へ直接移動
- `SEARCH-008`: incremental index update
- `SEARCH-009`: deleted resource から Wayback へ遷移
- `SEARCH-010`: UI 非依存 service test

---

# 53. Notes App

## 53.1 Purpose

素早い記録・収集・整理・関連付けを行う軽量 Knowledge Workspace。

Writer と役割を分離する。

Notes:

- Capture
- Collect
- Think
- Organize loosely

Writer:

- Compose
- Structure
- Format
- Review
- Publish

## 53.2 Block Model

最低限:

- Heading
- Paragraph
- Checklist
- Code
- Quote
- Image
- File Reference
- Web Reference
- Table
- Callout

各 Block は Object ID を持つ。

## 53.3 Markdown

Import / Export を提供する。

Nagi 固有 metadata は必要なら別 metadata layer に保持し、本文を不必要に独自形式へ閉じ込めない。

## 53.4 Quick Note

OS 全体から軽量 Overlay / quick capture を開ける。

Current Context / Workspace を source/reference 候補として提示可能。

勝手に Web 本文全体を複製しない。

## 53.5 References

File / Web / Search result / Nagi conversation 等を Reference Object として保持できる。

Reference Graph を共通サービスへ登録する。

## 53.6 Backlinks

参照された Resource 側から、どの Note / Object が参照しているか取得可能にする。

## 53.7 Agent Actions

```text
notes.create
notes.open
notes.get
notes.search
notes.append_block
notes.insert_block
notes.update_block
notes.delete_block
notes.move_block
notes.add_reference
notes.remove_reference
notes.set_tags
notes.add_to_workspace
notes.remove_from_workspace
```

巨大な `notes.make_better` Action は作らない。

## 53.8 Acceptance

- `NOTES-001`: create/edit/delete
- `NOTES-002`: main block types
- `NOTES-003`: stable block Object ID
- `NOTES-004`: File/Web Reference
- `NOTES-005`: Albert selection/page を reference 追加
- `NOTES-006`: Note/Block search
- `NOTES-007`: Agent editing
- `NOTES-008`: Agent Activity
- `NOTES-009`: Wayback restore
- `NOTES-010`: Restore as copy
- `NOTES-011`: Workspace participation
- `NOTES-012`: Markdown import/export
- `NOTES-013`: offline basic edit
- `NOTES-014`: Core test without Desktop UI

---

# 54. Home

## 54.1 Purpose

Nagi における作業の入口・再開地点。

単なる Desktop / Start Menu / Widget board にしない。

## 54.2 Main sections

- Continue
- Recent
- Workspaces
- Nagi status / important results
- Quick actions
- App launcher
- Search / Ask Nagi

## 54.3 Continue

Recent file ではなく、Workspace / Document / App Session / Cross-device Continuity State を扱う。

## 54.4 Nagi cards

表示条件を厳しくする。

優先:

- user-started process result
- failure
- explicit pending task
- clear continuation
- important deadline/status

Nagi が不要な提案を大量表示しない。

## 54.5 App Launcher

Apps 一覧を提供するが Home の主役にしない。

Quick Search から app launch 可能。

## 54.6 Context

- selected workspace
- selected recent item
- selected Nagi card
- current device

## 54.7 Actions

```text
home.open_workspace
home.resume_workspace
home.open_recent
home.launch_app
home.create_workspace
home.continue_from_device
home.dismiss_card
home.pin_workspace
```

## 54.8 Mobile

Mobile では Home の重要度が増す。

Workspace / Continue / Ask Nagi / Capture を中心にし、Desktop dashboard の縮小版にしない。

## 54.9 Acceptance

- `HOME-001`: recent workspace display
- `HOME-002`: workspace resume
- `HOME-003`: recent document -> source app
- `HOME-004`: remote continuity display
- `HOME-005`: Search Service reuse
- `HOME-006`: Intent entry
- `HOME-007`: noise suppression
- `HOME-008`: workspace data duplication 禁止
- `HOME-009`: UI independent model tests

---

# 55. Terminal v0.2

## 55.1 Purpose

従来 Terminal の能力を保持しつつ、一般ユーザーが CLI を知らなくても安全に高度操作へ近づける純正 Terminal。

初心者向け別 Terminal アプリは作らない。

同一 `Terminal Core` 上に3 Surface を載せる。

```text
Terminal Core
├─ Natural Surface
├─ Assisted Surface
└─ Shell Surface
```

## 55.2 Shell Surface

従来型 Terminal を尊重する。

- interactive shell
- stdin/stdout/stderr
- PTY/session
- command history
- cwd
- tabs
- splits
- multiple sessions

Nagi 独自文法を強制しない。

## 55.3 Natural Surface

自然言語で目的を入力できる。

例:

```text
Downloads の PDF を Documents に移したい
```

この要求を即 shell command 化しない。

まず Intent Router に渡し、Platform Action が適切なら `files.move` 等を使う。

Terminal を利用する必要があるときだけ command proposal へ進む。

## 55.4 Assisted Surface

Nagi が command を生成・説明し、ユーザーが review/edit/run できる。

最低限:

- command preview
- explanation
- risk
- cwd
- affected resource estimate where possible
- Run / Edit / Explain

## 55.5 Surface switching

`Natural / Assisted / Shell` を同一 session のまま切り替えられること。

切替で cwd / running job / history を失わない。

## 55.6 Command Risk

最低限:

- READ_ONLY
- DEVELOPMENT
- MODIFY
- EXTERNAL
- DESTRUCTIVE
- UNKNOWN

UNKNOWN を安全扱いしない。

単純文字列 blacklist だけで判定せず、可能なら:

- command
- args
- cwd
- target estimate
- filesystem effect
- network effect
- privilege requirement

を考慮する。

## 55.7 Dry run / impact preview

可能な操作では影響範囲を事前表示する。

例:

```text
143 .log files
482 MB
```

## 55.8 Checkpoint

大規模・高リスクの local modification 前は checkpoint を作成可能。

外部副作用まで戻せると誤認させない。

## 55.9 Explain

既存 command を貼り付けて意味と危険性を説明できる。

Nagi explanation と raw command/output を UI 上で明確に分離する。

## 55.10 Running Job

長時間処理を `RunningJob` として追跡。

最低限:

```text
job_id
session_id
device_id
command
status
started_at
output_reference
```

別 Device から status/log/cancel を参照可能にする。

process 自体を無理に別 Device へ移動しない。

## 55.11 Activity

Agent command と user command を区別する。

raw stdout/stderr 全文を Activity へ無条件保存しない。

## 55.12 Actions

```text
terminal.create_session
terminal.close_session
terminal.run_command
terminal.send_input
terminal.cancel_job
terminal.get_output
terminal.get_job_status
terminal.set_working_directory
terminal.attach_workspace
```

## 55.13 Acceptance

- `TERM-001`: interactive shell
- `TERM-002`: multiple sessions
- `TERM-003`: workspace cwd linkage
- `TERM-004`: Agent command execution
- `TERM-005`: Agent command Activity
- `TERM-006`: long job status
- `TERM-007`: remote job observation
- `TERM-008`: unfinished job is not success
- `TERM-009`: destructive command not low risk
- `TERM-010`: secret output redaction
- `TERM-011`: Natural Surface input
- `TERM-012`: Platform Action preferred when appropriate
- `TERM-013`: command preview
- `TERM-014`: command explanation
- `TERM-015`: surface switch keeps session
- `TERM-016`: UNKNOWN not auto-safe
- `TERM-017`: impact preview where possible
- `TERM-018`: Agent/User Activity distinction
- `TERM-019`: explanation/raw output separation
- `TERM-020`: routing works without forcing shell

---

# 56. Albert — Existing 0.1 to Platform-native

## 56.1 Purpose

既存 Albert 0.1 を破棄せず、Nagi Platform の最初の深いリファレンスアプリへ発展させる。

ブラウザエンジンを本書の都合で作り直さない。

## 56.2 Existing audit

実装前に最低限確認:

- browser engine
- tabs
- navigation
- downloads
- history
- bookmarks
- settings
- existing AI integration
- process model
- persistence

`KEEP / ADAPT / REPLACE / DEFER` を記録する。

既存 Nagi 0.1 仕様で Albert が Servo ベースと定義済みなら、それを尊重する。

## 56.3 Platform stages

### Stage A

- App Registry
- manifest/capabilities
- Context Publish
- Activity hooks

### Stage B

- Action API
- Object model
- Page / Tab / Selection / Article / Link / Image / Download

### Stage C

- Workspace persistence
- Cross-App Reference
- Search Provider
- Continuity
- Agent navigation

## 56.4 Context

最低限:

- current tab
- current page
- selected text
- selected link
- selected image
- recent tabs
- active workspace

## 56.5 Actions

```text
albert.open_url
albert.search_web
albert.new_tab
albert.close_tab
albert.activate_tab
albert.back
albert.forward
albert.reload
albert.get_page_info
albert.get_selection
albert.download
albert.add_to_notes
albert.add_to_workspace
```

将来:

```text
albert.find_element
albert.activate_element
albert.fill_form
albert.submit_form
```

External side effect を伴う form submit / post / upload / purchase は risk policy を通す。

## 56.6 Activity

通常閲覧履歴をすべて Activity に重複保存しない。

Activity へ優先して残す:

- Agent navigation when meaningful
- download
- save/reference
- workspace add
- form submit
- upload
- external side effect

## 56.7 Search

History / Bookmarks / Saved Pages / Captured content を provider 化。

Private Session は persistent index 対象外。

## 56.8 Downloads

Download を Files Resource と接続し、source URL / source page / timestamp / workspace provenance を保持可能にする。

## 56.9 Workspace

Tab group / research session を Workspace へ保存・復元。

## 56.10 Continuity

別 Device へ:

- URLs
- tab grouping
- active tab
- workspace
- semantic scroll anchor where available

を渡せる。

Desktop window geometry は渡さない。

## 56.11 Private Session

最低限:

- no persistent search indexing
- no long-term workspace history
- reduced context persistence
- no normal history persistence

外部送信等の security-relevant event の扱いは security policy に従う。

## 56.12 Acceptance

- `ALBERT-001`: basic browsing regressionなし
- `ALBERT-002`: current page Context
- `ALBERT-003`: selected text Object
- `ALBERT-004`: open URL via Action
- `ALBERT-005`: meaningful Agent Activity
- `ALBERT-006`: Notes reference handoff
- `ALBERT-007`: Download -> Files Resource
- `ALBERT-008`: Search Provider
- `ALBERT-009`: Workspace tab session restore
- `ALBERT-010`: Private Session excluded from normal index
- `ALBERT-011`: no blind rewrite

---

# Part VI — Phase 2: Work Environment

Phase 2:

- Writer
- Sheets
- Slides
- Mail
- Calendar
- Automations

---

# 57. Writer

## 57.1 Purpose

構造化された文章成果物を作成・レビュー・配布する標準 Document Editor。

AI がなくても通常ワープロとして成立すること。

## 57.2 Document model

最低限:

- Document metadata
- Section
- Heading
- Paragraph
- List
- Table
- Image/Figure
- Quote
- Code Block
- Citation
- Embedded Object
- Comments
- Document settings

主要構造 Object は stable Object ID を持つ。

## 57.3 Styles

Style-based formatting を中心とする。

最低限:

- Title
- Subtitle
- Heading 1-3+
- Body
- Quote
- Caption
- Code
- Custom style

Agent による全見出し調整等は direct format の乱立より style update を優先する。

## 57.4 Layout

最低限:

- page size
- margin
- orientation
- header/footer
- page number
- columns
- page/section break

## 57.5 Table

基本行列編集、merge/split、alignment、style。

高度計算は Sheets へ任せる。

## 57.6 Citation / Provenance

Albert selection/page 等を Citation Object として source reference 付きで取り込める。

「この段落の根拠」を元 Resource へ辿れること。

## 57.7 Embedded live references

Sheets Chart/Table/Named Range 等を source-linked Object として埋め込み可能。

source update 時:

- Update
- Compare
- Keep current

を提供可能。

## 57.8 UI

Desktop:

```text
Outline | Document | Inspector
```

Inspector 候補:

- Style
- References
- Comments
- Activity

Focus Mode を用意し、常時 AI chat pane を強制しない。

## 57.9 AI edit

選択範囲への局所編集は直接 Action 化可能。

大規模変更は可能なら plan/preview:

- section reorder
- paragraph split
- heading addition
- duplicate removal

を提示し、checkpoint を作成。

## 57.10 Track Changes

通常文書レビューの Track Changes と OS Activity を分離する。

最低限:

- add
- delete
- move
- format

## 57.11 Comments

Comment / Reply / Resolve。

Agent が comment 対応する場合、どの comment にどう対応したか追跡可能にする。

## 57.12 Search

Document title / headings / paragraphs / permitted comments / references を Search Provider へ登録。

Result から該当 Object 位置へジャンプ。

## 57.13 Actions

```text
writer.create
writer.open
writer.save
writer.get_document
writer.get_outline
writer.insert_paragraph
writer.insert_heading
writer.insert_list
writer.insert_table
writer.insert_image
writer.insert_reference
writer.replace_text
writer.delete_object
writer.move_object
writer.apply_style
writer.add_comment
writer.resolve_comment
writer.export
```

巨大 Action `writer.make_document_better` は作らない。

## 57.14 Formats

目標:

Import:

- Markdown
- Plain text
- DOCX basic

Export:

- PDF
- DOCX basic
- Markdown
- Plain text

DOCX unsupported feature は警告し、黙って破棄しない。

Native format は既存 Document Framework 規約へ従う。新規定義が必要なら structured container + metadata を優先し opaque proprietary blob を避ける。

## 57.15 Continuity

Semantic cursor/section/object position を共有。

Mobile は read/comment/light edit/Ask Nagi/approve を重視してよい。

## 57.16 Acceptance

- `WRITER-001`: normal editing without AI
- `WRITER-002`: core object types
- `WRITER-003`: stable Object ID
- `WRITER-004`: style formatting
- `WRITER-005`: outline
- `WRITER-006`: PDF export
- `WRITER-007`: Markdown import/export
- `WRITER-008`: basic DOCX import/export + unsupported warning
- `WRITER-009`: Albert citation
- `WRITER-010`: Notes -> Writer
- `WRITER-011`: Agent selected-object edit
- `WRITER-012`: Agent Activity
- `WRITER-013`: checkpoint before broad AI edit
- `WRITER-014`: Wayback restore
- `WRITER-015`: Restore as copy
- `WRITER-016`: Search -> paragraph/object
- `WRITER-017`: source revision update detection
- `WRITER-018`: Core UI independent
- `WRITER-019`: multi-surface same Document model
- `WRITER-020`: Japanese/English official support

---

# 58. Sheets

## 58.1 Purpose

表形式データの編集・計算・分析と、結果の Cross-App Object 利用を行う標準 Data Document Editor。

AI がなくても通常 spreadsheet として成立すること。

## 58.2 Workbook model

```text
Workbook
├─ Sheet
│  ├─ Cells
│  ├─ Ranges
│  ├─ Tables
│  ├─ Charts
│  └─ Objects
├─ Named Ranges
├─ Metadata
└─ Calculation Graph
```

## 58.3 Cell model

最低限:

- value
- formula
- display value
- data type
- format
- validation
- comment
- error

Data type:

- EMPTY
- TEXT
- NUMBER
- BOOLEAN
- DATE
- DATETIME
- DURATION
- ERROR

## 58.4 Sparse / Virtualized Grid

未使用セルを全件実体化しない。

論理 Grid は XLSX 相当規模を扱える設計にする。

UI は virtualized。

## 58.5 Calculation Engine

UI から分離し独立 test 可能にする。

```text
Formula Parser -> AST -> Dependency Graph -> Calculation Engine
```

Incremental recalculation を行う。

## 58.6 Formula canonicalization

内部 function ID は英語 canonical。

UI locale に保存形式を依存させない。

初期関数群:

- arithmetic
- SUM/AVERAGE/MIN/MAX/COUNT/COUNTA
- IF/IFS/AND/OR/NOT
- ROUND variants
- LEFT/RIGHT/MID/LEN/TRIM/CONCAT
- DATE/YEAR/MONTH/DAY/TODAY/NOW
- XLOOKUP / VLOOKUP / HLOOKUP
- INDEX / MATCH
- SUMIF/SUMIFS
- COUNTIF/COUNTIFS
- AVERAGEIF
- IFERROR

Function Registry で拡張可能にする。

## 58.7 Circular / Error model

Circular reference を検出。

Error は structured type:

- DIV_BY_ZERO
- INVALID_REFERENCE
- VALUE_ERROR
- NAME_ERROR
- NOT_AVAILABLE
- CIRCULAR_REFERENCE

## 58.8 Structured Table

Table は単なる range と別の stable Object。

Columns に semantic name を持たせる。

Agent は `D:D` より `Sales.Revenue` のような意味参照を優先できる。

## 58.9 Data operations

最低限:

- sort single/multi-column
- filter text/number/date/empty/value list
- freeze panes
- data validation
- conditional formatting
- named ranges

## 58.10 Charts

最低限:

- column
- bar
- line
- pie
- scatter
- area

Chart は source range/table reference を保持する Object。

Writer / Slides へ source link 付きで渡せる。

## 58.11 Formats

CSV:

- UTF-8
- UTF-8 BOM
- CP932/Shift_JIS
- delimiter preview
- quote/header/type preview

TSV import/export。

XLSX basic import/export:

- values
- formulas
- sheets
- basic formatting
- merged cells
- tables
- charts
- freeze panes
- filters

Unsupported VBA / Power Query / external connection / add-ins 等は警告し、黙って削除しない。

## 58.12 Macro

v0.1 で VBA 実行不要。

独自 macro language を急造しない。

Automation は Nagi Automations + Action API を基本とする。

## 58.13 AI analysis

`Analyze` は原則 READ。

「分析してグラフ作成」は MODIFY。

分析のついでに勝手に書式やデータを書き換えない。

大規模変更は change preview + checkpoint。

Formula insert 後は validation を行い error/circular があれば成功扱いしない。

## 58.14 Actions

```text
sheets.create_workbook
sheets.open
sheets.create_sheet
sheets.rename_sheet
sheets.delete_sheet
sheets.get_range
sheets.set_values
sheets.set_formula
sheets.set_range_values
sheets.set_range_formulas
sheets.insert_rows
sheets.insert_columns
sheets.delete_rows
sheets.delete_columns
sheets.sort
sheets.filter
sheets.create_table
sheets.create_named_range
sheets.apply_format
sheets.add_conditional_format
sheets.create_chart
sheets.update_chart
sheets.import_csv
sheets.export
```

## 58.15 Search

Index:

- workbook title
- sheet name
- table name
- named range
- headers
- text cells
- comments
- chart titles

全数値セルを semantic embedding 化しない。

## 58.16 Performance target

少なくとも 100,000+ populated cells 程度で通常操作が破綻しない設計を目標とする。

- virtualized UI
- sparse model
- incremental recalculation
- incremental persistence

## 58.17 Acceptance

- `SHEETS-001` normal editing without AI
- `SHEETS-002` multiple sheets
- `SHEETS-003` typed cells
- `SHEETS-004` formula calculation
- `SHEETS-005` incremental dependency recalculation
- `SHEETS-006` circular detection
- `SHEETS-007` sort/filter
- `SHEETS-008` structured table
- `SHEETS-009` named range
- `SHEETS-010` basic chart
- `SHEETS-011` chart source reference
- `SHEETS-012` CSV import/export
- `SHEETS-013` UTF-8/CP932
- `SHEETS-014` basic XLSX import/export
- `SHEETS-015` unsupported feature warning
- `SHEETS-016` Agent range read
- `SHEETS-017` Agent range write/formula
- `SHEETS-018` large edit preview
- `SHEETS-019` Agent Activity
- `SHEETS-020` Wayback
- `SHEETS-021` Restore as copy
- `SHEETS-022` chart/table cross-app reference
- `SHEETS-023` Search reachability
- `SHEETS-024` calculation core UI independent
- `SHEETS-025` multi-surface same model
- `SHEETS-026` virtualized grid
- `SHEETS-027` structured errors
- `SHEETS-028` Japanese/English content

---

# 59. Slides

## 59.1 Purpose

文章・データ・画像・Cross-App Object を、発表目的に応じた視覚的ストーリーへ構成する標準 Presentation Document Editor。

単なる「PowerPoint の画面コピー」ではなく、Nagi の provenance / source link / Agent planning を活かす。

AI がなくても通常の presentation editor として成立すること。

## 59.2 Presentation model

```text
Presentation
├─ Theme
├─ Slide Master / Layouts
├─ Slides[]
│  ├─ Text Object
│  ├─ Image Object
│  ├─ Shape
│  ├─ Table
│  ├─ Chart
│  ├─ Media
│  └─ Embedded Reference
├─ Speaker Notes
└─ Metadata
```

Slide と主要 Object は stable Object ID を持つ。

## 59.3 Canvas model

位置・サイズ・rotation・z-order を構造化して保持。

Desktop pixel coordinate は presentation logical coordinate と分離する。

## 59.4 Layout / Theme

最低限:

- theme fonts
- theme colors/tokens
- slide size
- title/content layouts
- section header
- title only
- blank
- custom layout

個別 Object へ direct formatting を乱立させず Theme/Layout を活用する。

## 59.5 Editing

最低限:

- create/delete/duplicate/reorder slide
- add/edit text
- images
- shapes
- tables
- charts
- alignment/distribution
- group/ungroup
- z-order
- guides/grid/snap
- notes

## 59.6 Cross-App Data

Sheets Chart/Table を live source reference として挿入可能。

Writer Section/Paragraph から presentation outline/source を生成可能。

Albert / Notes の source/reference を provenance 付きで利用可能。

## 59.7 Source updates

Linked Chart/Table の source revision が変化した場合:

- Update
- Compare
- Keep current

を提示可能。

## 59.8 AI presentation planning

ユーザー要求例:

```text
この売上分析を10分の経営会議用に5枚でまとめて
```

Intent は次の段階へ分解可能。

1. source inspection
2. audience/purpose interpretation
3. outline proposal
4. object/chart selection
5. slide generation
6. validation

大規模生成前に outline preview を表示できること。

## 59.9 Presentation length / audience

Document metadata として:

- intended audience
- target duration
- language
- tone/purpose

を保持可能にする。

これは AI planning の hint であり、必須入力にはしない。

## 59.10 Presenter Mode

最低限:

- current slide
- next slide
- notes
- timer
- slide navigation
- external display mode

Presenter Mode は editing surface と分離する。

## 59.11 Export / Import

Export:

- PDF
- images
- PPTX basic

Import:

- PPTX basic

PPTX 対応目標:

- text
- basic shapes
- images
- tables
- charts where feasible
- common layouts/themes
- notes basic

unsupported animation/macros/add-ins 等は警告する。

## 59.12 Animation / Transition

v0.1 は基本 transition と simple entrance/appear 程度まででよい。

複雑 animation engine は後回し可能。

## 59.13 Actions

```text
slides.create
slides.open
slides.add_slide
slides.delete_slide
slides.duplicate_slide
slides.reorder_slide
slides.set_layout
slides.insert_text
slides.insert_image
slides.insert_shape
slides.insert_table
slides.insert_chart_reference
slides.insert_object_reference
slides.update_reference
slides.add_speaker_note
slides.apply_theme
slides.export
slides.start_presenter
```

## 59.14 Activity / Wayback

Diff は slide/object 単位。

AI 生成で大量変更する場合 Transaction + Checkpoint を必須にする。

## 59.15 Continuity

Desktop: full editor/presenter preparation

Tablet: touch/pen editing + presentation control

Mobile: review/light edit/presenter remote/Ask Nagi を重視

## 59.16 Acceptance

- `SLIDES-001`: normal slide editing without AI
- `SLIDES-002`: slide/object stable IDs
- `SLIDES-003`: theme/layout
- `SLIDES-004`: text/image/shape/table/chart
- `SLIDES-005`: slide reorder
- `SLIDES-006`: Sheets chart linked reference
- `SLIDES-007`: linked source update detection
- `SLIDES-008`: outline generation from Writer/Notes sources
- `SLIDES-009`: AI outline preview before broad generation
- `SLIDES-010`: Agent Activity
- `SLIDES-011`: Wayback
- `SLIDES-012`: PDF export
- `SLIDES-013`: basic PPTX import/export with warnings
- `SLIDES-014`: presenter mode
- `SLIDES-015`: Core model UI independent
- `SLIDES-016`: multi-surface same presentation model

---

# 60. Mail

## 60.1 Purpose

メールを単なる inbox としてではなく、People / Calendar / Files / Search / Activity / Agent と接続する標準 communication client とする。

送信という外部副作用を持つため、安全モデルを厳格に適用する。

## 60.2 Provider architecture

Mail core と provider adapter を分離する。

Provider の例:

- IMAP/SMTP
- provider-specific API
- enterprise connector

既存 Nagi account/auth framework があれば必ず再利用する。

## 60.3 Data model

最低限:

- Account
- Mailbox/Folder/Label abstraction
- Message
- Conversation/Thread
- Address/Participant
- Draft
- Attachment Resource
- Send state

## 60.4 UI

Desktop 基本3ペイン:

```text
Folders/Views | Message List | Conversation/Message
```

必要に応じて People / Calendar / Attachment inspector を開ける。

## 60.5 Draft first

Agent によるメール作成は原則 Draft を生成する。

`mail.send` は EXTERNAL Action。

標準設定ではユーザー確認なしに Agent が初回送信しない。

ユーザーが Automation 等で明示的に scope を事前承認したケースは、その policy の範囲内で送信可能。

## 60.6 Thread understanding

Search/Agent 用に Thread 単位で:

- participants
- subject
- chronological message refs
- attachments
- related calendar/resource links

を扱う。

AI 要約は生成物であり original mail と区別する。

## 60.7 Attachments

Attachment は Files/Resource Service と接続。

保存した場合 provenance と message source を保持。

## 60.8 Cross-App workflows

例:

- Mail -> Calendar event
- Mail -> Notes reference
- Mail attachment -> Files
- Mail attachment -> Writer/Sheets
- People -> related conversations

## 60.9 Search

Search Provider:

- subject
- participants
- message text subject to policy
- attachment metadata
- date
- labels/folders

Sensitive mailbox policy を尊重。

## 60.10 Context

- active account
- current thread/message
- selected messages
- selected attachment
- selected participant

## 60.11 Actions

```text
mail.list_accounts
mail.list_threads
mail.get_thread
mail.search
mail.create_draft
mail.update_draft
mail.attach_resource
mail.remove_attachment
mail.reply_draft
mail.forward_draft
mail.send
mail.archive
mail.move
mail.mark_read
mail.mark_unread
mail.flag
```

## 60.12 Offline

Cache policy に応じて:

- cached mail read
- draft edit
- queue state

を利用可能。

offline send は `PENDING` とし、送信済みにしない。

## 60.13 Activity

Agent の Draft 編集は Activity 対象。

送信は必ず external side effect として記録。

秘密本文全文を Activity へ複製しない。

## 60.14 Acceptance

- `MAIL-001`: provider abstraction
- `MAIL-002`: message/thread browsing
- `MAIL-003`: create/edit draft
- `MAIL-004`: attachments as Resources
- `MAIL-005`: Agent draft creation without auto-send default
- `MAIL-006`: send is EXTERNAL Action
- `MAIL-007`: send failure not success
- `MAIL-008`: Search Provider
- `MAIL-009`: Mail -> Calendar/Notes/Files linkage
- `MAIL-010`: Activity records external send event
- `MAIL-011`: offline draft
- `MAIL-012`: Core UI independent

---

# 61. Calendar

## 61.1 Purpose

予定・会議・時間ブロック・招待を管理し、Mail / People / Workspace / Automations / Nagi Agent と連携する標準 time management client。

## 61.2 Data model

最低限:

- Calendar
- Event
- Recurrence
- Participant
- Organizer
- Location
- Reminder
- Attachment/Resource Reference
- Availability
- Invitation state

## 61.3 Timezone

Event は timezone-aware に扱う。

floating time と fixed instant を区別できる設計にする。

DST/跨日イベントを正しく扱う。

## 61.4 Views

Desktop:

- day
- week
- month
- agenda

Mobile:

- agenda/day を優先

## 61.5 Scheduling intent

例:

```text
来週どこかで田中さんと1時間
```

Intent は:

1. participant resolution
2. availability lookup
3. candidate slot generation
4. user/policy confirmation
5. event creation/invite

へ分解。

勝手に external invite を送らない。

## 61.6 Mail integration

Mail から日時・参加者・添付を元に Event Draft を作成可能。

## 61.7 Workspace

Event に Workspace を関連付け、会議前に関連 Document/Notes を Home へ提示可能。

## 61.8 Actions

```text
calendar.list
calendar.search
calendar.get_event
calendar.create_event
calendar.update_event
calendar.delete_event
calendar.create_event_draft
calendar.invite
calendar.respond_invite
calendar.find_availability
calendar.add_reminder
calendar.attach_resource
```

Invite/response は EXTERNAL Action。

## 61.9 Search

- title
- participant
- location
- date/time
- notes where policy allows
- workspace

## 61.10 Acceptance

- `CAL-001`: day/week/month/agenda
- `CAL-002`: timezone-aware event
- `CAL-003`: recurrence basic
- `CAL-004`: reminders
- `CAL-005`: availability query
- `CAL-006`: event draft from natural language
- `CAL-007`: invite external action policy
- `CAL-008`: Mail/People/Workspace linkage
- `CAL-009`: Search Provider
- `CAL-010`: offline cached calendar where provider permits
- `CAL-011`: multi-surface same event model

---

# 62. Automations

## 62.1 Purpose

ユーザーが「いつ・何が起きたら・何をするか」を自然言語または構造化 UI で定義し、Nagi Actions を安全に繰り返し実行する標準 automation environment。

## 62.2 Automation model

```text
Automation
├─ Trigger
├─ Conditions[]
├─ Actions[]
├─ Permission Scope
├─ Target Device/Workspace?
├─ Error Policy
└─ Activity/Run History
```

## 62.3 Trigger

最低限:

- scheduled time
- interval
- app/platform event
- resource change
- device state
- manual

将来 external webhook/provider trigger を追加可能。

## 62.4 Conditions

例:

- file type
- workspace
- sender
- time window
- device online
- battery/power state

## 62.5 Action composition

既存 Platform Action を呼ぶ。

Automation 専用の裏操作を増やさない。

## 62.6 Natural language builder

例:

```text
毎朝8時にDownloadsの新しいCSVを確認して、あればSheetsで集計してNotesに結果を残す
```

Nagi はこれを構造化 automation draft に変換し、Trigger/Actions/Permissions を表示する。

## 62.7 Safety

Automation は継続的権限を持つため特に厳格にする。

作成時に:

- trigger
- action list
- external side effects
- affected resources
- permission scope

を明示。

EXTERNAL / DESTRUCTIVE Action を含む Automation は追加確認・scope 制限を要求する。

## 62.8 Dry run

可能なら `Run once / Preview` を提供する。

## 62.9 Run history

各 run を Activity と関連付ける。

Status:

- SUCCESS
- PARTIAL
- FAILED
- SKIPPED
- WAITING/PENDING

## 62.10 Error policy

- stop
- continue safe steps
- retry with bounded count
- notify user

を明示できる。

silent infinite retry を禁止。

## 62.11 Actions

```text
automations.create
automations.update
automations.enable
automations.disable
automations.delete
automations.run_once
automations.preview
automations.get_history
automations.get_status
```

## 62.12 Acceptance

- `AUTO-001`: scheduled trigger
- `AUTO-002`: event trigger architecture
- `AUTO-003`: multi-action workflow
- `AUTO-004`: natural language -> structured draft
- `AUTO-005`: permission scope visible
- `AUTO-006`: external side effect requires explicit policy
- `AUTO-007`: dry run/run once
- `AUTO-008`: bounded retry
- `AUTO-009`: Activity/run history
- `AUTO-010`: failed run not success
- `AUTO-011`: enable/disable
- `AUTO-012`: Platform Actions reused

---

# Part VII — Phase 3: Unified Environment

Phase 3:

- Studio
- People
- Devices
- Store
- Models

---

# 63. Studio

## 63.1 Purpose

Writer / Sheets / Slides / Notes / Files / Albert 等をまたぐ仕事を、ひとつの目的・成果物・source graph として扱う統合作業環境。

Studio は巨大万能エディタではない。

既存アプリの機能を再実装せず、複数 Document / Object / Workspace / Agent workflow を束ねる orchestration surface とする。

## 63.2 Core concept

```text
Studio Project
├─ Workspace Reference
├─ Sources
├─ Working Documents
├─ Output Documents
├─ Agent Plan / Transactions
└─ Provenance Graph
```

例:

```text
sales.csv
  -> Sheets analysis
  -> Writer report
  -> Slides presentation
```

## 63.3 Use cases

- 調査 -> レポート -> プレゼン
- データ -> 分析 -> 文書化
- Albert research -> Notes -> Writer
- 複数資料をまとめて成果物 bundle 作成

## 63.4 UI

Desktop では次を基本とする。

- Project/Workspace navigator
- Sources
- Output artifacts
- Central working surface / selected app embedding or handoff
- Provenance / Activity inspector

Studio 内に Writer/Sheets を丸ごと再実装しない。

必要に応じて App Surface を embed するか、対象 App へ遷移する。

## 63.5 Agent workflow

高レベル Intent を複数 App Action へ分解し、Transaction として追跡。

大規模処理前は Plan を表示可能。

例:

```text
Goal: Create executive review deck
Sources:
- FY2026 Sales workbook
- Market notes

Plan:
1. Analyze workbook
2. Extract 5 key findings
3. Draft executive summary
4. Build 6-slide presentation
5. Validate linked charts
```

## 63.6 Source graph

各 output Object から source Object へ辿れること。

「この数字はどこから？」に対して元 Sheets range まで辿れることを目標にする。

## 63.7 Actions

```text
studio.create_project
studio.add_source
studio.remove_source
studio.add_output
studio.create_workflow
studio.preview_plan
studio.execute_plan
studio.open_artifact
studio.export_bundle
```

## 63.8 Acceptance

- `STUDIO-001`: Workspace/Project creation
- `STUDIO-002`: multiple source apps/resources
- `STUDIO-003`: multiple output documents
- `STUDIO-004`: provenance graph
- `STUDIO-005`: cross-app plan preview
- `STUDIO-006`: transaction execution
- `STUDIO-007`: partial failure visible
- `STUDIO-008`: no duplicate editor implementation
- `STUDIO-009`: Activity integration
- `STUDIO-010`: Continuity-ready project model

---

# 64. People

## 64.1 Purpose

連絡先を単なる住所録ではなく、Mail / Calendar / shared resources / Workspace から参照される標準 Person/Organization identity layer として扱う。

People はユーザーについて勝手な人物評価・関係推定を行う social intelligence engine ではない。

## 64.2 Data model

最低限:

- Person
- Organization
- Contact point
- Email address
- Phone
- Account/provider identity
- Avatar/photo ref
- Notes/reference
- Merge/source metadata

## 64.3 Identity resolution

同一人物候補を自動検出してもよいが、根拠が弱い場合に勝手に merge しない。

Merge は reversible / Activity 記録対象。

## 64.4 Related context

権限がある範囲で:

- recent mail threads
- meetings
- shared documents
- notes references

を人物軸でまとめて表示可能。

ただし People DB へ本文を大量複製せず Reference で接続する。

## 64.5 Context

- selected person
- selected organization
- selected contact point

## 64.6 Actions

```text
people.search
people.get
people.create
people.update
people.merge
people.unmerge
people.add_contact_point
people.link_identity
people.open_related
```

## 64.7 Acceptance

- `PEOPLE-001`: person/contact CRUD
- `PEOPLE-002`: organization linkage
- `PEOPLE-003`: provider identity linkage
- `PEOPLE-004`: merge preview/undo
- `PEOPLE-005`: Mail/Calendar linkage
- `PEOPLE-006`: related resources by reference
- `PEOPLE-007`: Search Provider
- `PEOPLE-008`: privacy permissions respected

---

# 65. Devices

## 65.1 Purpose

ユーザーが所有・信頼する Nagi Device を管理し、Continuity / remote job observation / capability / trust / handoff を可視化する標準 Device Manager。

Devices は初期段階で全面的な remote desktop/control を目的としない。

## 65.2 Device model

最低限:

- device ID
- name
- class hint
- OS/version
- capabilities
- trust state
- online/offline
- last seen
- continuity availability
- running jobs summary

## 65.3 Trust

最低限:

- untrusted
- pending
- trusted
- revoked

新 Device が account に現れただけで自動 trusted にしない。

## 65.4 UI

各 Device で:

- online state
- capabilities
- current/recent Workspace
- running jobs
- continuity targets
- trust/security state

を表示。

## 65.5 Handoff

`Continue here` を提供。

App process の binary state を無理に移送せず semantic Continuity State を利用。

## 65.6 Remote job

Terminal/Automation 等の Running Job を別 Device から:

- status
- logs
- cancel if permitted
- Ask Nagi

できる。

## 65.7 Actions

```text
devices.list
devices.get
devices.rename
devices.trust
devices.revoke
devices.list_capabilities
devices.get_continuity
devices.handoff
devices.list_jobs
devices.cancel_job
```

## 65.8 Acceptance

- `DEV-001`: device discovery/registry
- `DEV-002`: trust state
- `DEV-003`: revoke
- `DEV-004`: capability display
- `DEV-005`: continuity handoff
- `DEV-006`: remote job observation
- `DEV-007`: offline state
- `DEV-008`: no implicit remote control

---

# 66. Store

## 66.1 Purpose

Nagi 向け App / Agent extension 等を発見・導入・更新する便利な公式配布面。

Store は Nagi App 配布の唯一の門番にしない。

サイドロード・企業内配布・開発者直接配布を妨げない。

## 66.2 Open ecosystem principle

Nagi target へ build した app package が、署名・compatibility・permission policy を満たせば Store 外からも導入可能であること。

Store 専用 API を使わないと深い Nagi 統合ができない設計は禁止。

## 66.3 Package metadata

最低限:

- app ID
- display name
- version
- publisher
- signature
- architecture/runtime compatibility
- required permissions
- optional capabilities
- supported surfaces
- update metadata

## 66.4 Install review

インストール前に:

- publisher/signature
- permissions
- system capabilities
- device compatibility
- disk size

を表示。

## 66.5 Updates

- check
- download
- stage
- apply
- rollback where package system supports

を分離。

Update failure で既存 working version を破壊しない transactional update を目標とする。

## 66.6 Sideload

Files から `.napp` 等を開いた場合も同じ Package/Permission review を通す。

Store 経由だけ安全審査がある設計にしない。

## 66.7 Agent/extension packages

将来 Agent package を扱う場合も、App と同様に:

- declared actions
- permissions
- external access
- publisher
- update policy

を可視化する。

## 66.8 Actions

```text
store.search
store.get_package
store.install
store.update
store.uninstall
store.open_permissions
store.check_updates
```

`install/update/uninstall` は system capability を要求。

## 66.9 Acceptance

- `STORE-001`: package catalog architecture
- `STORE-002`: install permission review
- `STORE-003`: signature/publisher display
- `STORE-004`: compatibility check
- `STORE-005`: update flow
- `STORE-006`: failure does not fake success
- `STORE-007`: sideload uses same security model
- `STORE-008`: Store not mandatory for app distribution
- `STORE-009`: no first-party secret integration API

---

# 67. Models

## 67.1 Purpose

The Models App is a typed AI capability/provider registry and lifecycle UI,
not an LLM name list. It must represent Decision capabilities and provider
runtime boundaries without coupling applications to a model or vendor.

Nagi で利用する local/cloud-capable model provider を可視化・管理し、用途・性能・権限・ライセンスをユーザーが理解できるようにする標準 Model Manager。

## 67.2 Source of truth

モデル構成・既定モデル・ライセンス条件は既存 Nagi model policy/spec を最優先する。

本書作成時点の既存計画では、Nagi OS 0.1 のローカル LLM セットとして以下が想定されている。

- IBM Granite 4.2 3B — Default Standard
- Qwen3 4B — Standard alternative
- Google Gemma 3 1B — Lite

Gemma は独自 Terms / NOTICE 要件を保持し、Nagi OS 自身のライセンスと混同しない。

既存主仕様が変更されていればそちらへ追従し、本 App にモデル名をハードコードしない。

## 67.3 Model registry

最低限:

- model ID
- provider
- display name
- version
- local/cloud
- size
- hardware requirements
- capability tags
- license/notice refs
- installed state
- integrity state
- preferred roles
- supported provider family/runtime
- supported DecisionKind values
- batch decision support
- confidence support
- provider health/availability

## 67.4 Capability tags

例:

- chat
- text.generate
- structured.generate
- reasoning
- decision.boolean
- decision.choice
- decision.score
- decision.ranking
- decision.classification
- decision.routing
- decision.candidate_pruning
- decision.batch
- embedding
- vision
- speech
- code
- lightweight

正式な registry tag は既存 Nagi naming rules と整合させる。特定 model
名に機能を直結しない。`System 1` は説明上の Decision lane の呼称で
あり、Models App や共通 contract の必須型名ではない。

## 67.5 Routing

Nagi Core の Model Router が capability、role、availability、provider
health、local/offline state、privacy/sensitivity policy、latency、memory
pressure、loaded state、user preference、fallback availability から
provider/model を選択できる。Generative と Decision は別の route family
として表現する。

Models App は routing policy、provider/runtime、capability、role、local /
cloud の区別を可視化・設定する UI を提供するが、各 App が直接特定
model/vendor に密結合しない。

Nagi 0.1 の標準 Generative route は local Granite/Qwen/Gemma policy と
`llama.cpp` / GGUF runtime に基づく。Decision route に専用 model がない
場合は、同じ typed Decision contract を `LlmDecisionAdapter` で既存
local GenerativeProviderへ接続する。llama.cpp/GGUF は全 provider の
唯一 runtime ではない。

## 67.6 Install/remove

Local model package の:

- download/import
- integrity verification
- install
- storage location
- remove
- update

を管理。

## 67.7 Privacy

Cloud model/provider を利用する場合、local-only resource policy を破らない。

App/Resource sensitivity により cloud route を拒否できること。将来の
cloud DecisionProvider も optional provider であり、explicit configuration、
network capability separation、credential protection、Activity/diagnostic
privacy、offline fallback を満たさなければならない。Jev は optional
provider example であり、Jevなしが Nagi 0.1 の標準動作である。

## 67.8 Performance

Model ごとに:

- estimated RAM/VRAM
- current loaded state
- optional benchmark/latency info

を表示可能。

## 67.9 Actions

```text
models.list
models.get
models.install
models.remove
models.update
models.set_default_role
models.get_route
models.set_route_policy
models.verify
```

## 67.10 Acceptance

- `MODEL-001`: registry-driven, no hardcoded app coupling
- `MODEL-002`: local model install/remove
- `MODEL-003`: integrity verification
- `MODEL-004`: license/notice visibility
- `MODEL-005`: role-based routing
- `MODEL-006`: local/cloud distinction
- `MODEL-007`: privacy policy can block cloud route
- `MODEL-008`: hardware requirement check
- `MODEL-009`: existing Nagi model policy remains source of truth
- `MODEL-010`: registry can represent Decision capability/provider metadata
- `MODEL-011`: fallback works without a specialized or cloud provider
- `MODEL-012`: application requests do not hardcode model/vendor names

---

# Part VIII — Cross-App Workflows

## 68. Workflow A: Web research -> Notes -> Writer

User:

```text
このページの内容をメモして、仕様書に反映して
```

Expected flow:

1. Albert current/selected Object resolved by Context
2. Notes reference Block created with source provenance
3. Writer target Document resolved
4. Checkpoint created if broad edit
5. Writer Object Actions applied
6. Citation/reference retained
7. Activity Transaction recorded

Albert と Writer が直接相互依存してはならない。

## 69. Workflow B: CSV -> Sheets -> Slides

User:

```text
このCSVを分析して、役員向け5枚にして
```

Expected flow:

1. Files Resource resolved
2. Sheets import
3. READ analysis
4. findings/object selection
5. presentation outline preview
6. Slides creation
7. Sheets Chart live references
8. Activity Transaction
9. Checkpoint before broad mutations

## 70. Workflow C: Mail -> Calendar -> Workspace

User:

```text
このメールの打ち合わせを予定に入れて、関連資料もまとめて
```

Expected flow:

1. current Mail thread Context
2. event draft creation
3. participant/time extraction
4. user/policy confirmation before external invite
5. Calendar event
6. relevant attachments/resources to Workspace
7. provenance links retained

## 71. Workflow D: PC -> Phone -> PC Continuity

1. PC Workspace active
2. semantic Continuity State saved
3. Phone Home shows `Continue from PC`
4. Mobile Surface reconstructs relevant Docs/Status
5. user reviews/edits/approves
6. revisions sync
7. PC resumes with updated state

No remote framebuffer requirement.

## 72. Workflow E: Natural Terminal

User:

```text
このプロジェクトのテストして
```

1. Intent Router determines Terminal appropriate
2. Assisted proposal `cargo test` etc.
3. risk = DEVELOPMENT
4. execute in Workspace cwd
5. RunningJob tracked
6. result structured
7. raw log available
8. Activity records Agent command

User:

```text
Downloads のPDFをDocumentsへ移して
```

1. Intent Router recognizes Files Action
2. `files.move`
3. Terminal command is not unnecessarily generated

---

# Part IX — UI/UX Common Rules

## 73. AI must not dominate every screen

純正アプリだからといって、全画面右側に AI chat pane を常設しない。

Nagi は必要な時に利用可能であり、通常作業を邪魔しない。

## 74. Structured interaction first

Agent は GUI automation より Action API を優先する。

GUI click/typing は legacy/non-integrated App への fallback。

## 75. Explainability

Agent が成果物を変更した場合、少なくとも:

- what changed
- actor
- reason/request where available
- affected scope
- reversible state

を追跡できること。

## 76. Confirmations

確認ダイアログを乱発しない。

READ / low-risk reversible operations は user policy の範囲で自動化可能。

EXTERNAL / DESTRUCTIVE / UNKNOWN はより厳しい policy を適用。

## 77. Empty / Loading / Error state

すべての App は:

- empty
- loading
- offline
- partial
- failed
- permission denied

を明示的に持つ。

白画面・無反応で表現しない。

## 78. Keyboard / Touch

Desktop shortcuts をサポートしつつ、Core Action を shortcut handler に閉じ込めない。

Touch Surface は pointer hover 前提を避ける。

## 79. Naming

Application display names は現時点で仮称。

暫定一覧:

- Home
- Albert
- Files
- Notes
- Terminal
- Activity
- Wayback
- Search
- Writer
- Sheets
- Slides
- Mail
- Calendar
- Automations
- Studio
- People
- Devices
- Store
- Models

名前変更を理由に Core/Action schema を作り直さない。

---

# Part X — Security / Privacy

## 80. Least privilege

App は必要な Permission/Capability だけ要求する。

## 81. Agent authority

Agent がユーザーより強い権限を持たない。

Agent が App Action を呼ぶときも Permission Service を通す。

Decision capability と Generative/Reasoning output は同じ untrusted input
boundary を通る。confidence、score、probability、provider identity は
routing/escalation には利用できても、Permission、Capability、Policy、
Owner override、または Executor の authority を付与・増強しない。
すべての side effect は deterministic validation、Policy、Permission、
Executor、Activity/Transaction/Wayback を通る。

## 82. External effects

次を EXTERNAL の代表例とする。

- send mail
- calendar invite/response
- publish/post
- upload/share
- remote API write

Activity で追跡し、policy/confirmation を適用。

## 83. Destructive effects

- permanent delete
- package removal with data loss
- destructive disk operation
- credential revoke

等。

checkpoint で戻せる範囲と戻せない範囲を区別する。

## 84. Sensitive indexing

Search/Embedding へ secret を無条件投入しない。

## 85. Logs

Terminal/Mail/Models 等の log に secret が混ざる可能性を考慮し redaction layer を用意する。

## 86. Provenance privacy

Provenance 自体が機密情報になり得るため、Resource Permission を無視して他 App へ露出しない。

---

# Part XI — Data / Persistence / Sync

## 87. Stable IDs

Resource / Document / Object / Workspace / Activity / Transaction / Device 等は collision resistant opaque ID を使用。

表示名や path を primary identity にしない。

## 88. Storage format

既存 Nagi storage conventions を優先。

新規形式は:

- versioned
- migratable
- testable
- corruption detection possible

であること。

## 89. Schema migration

全 persisted schema は version を持ち、upgrade path を用意する。

開発中に schema を壊して user data を捨てる運用を避ける。

## 90. Crash recovery

Document Framework は autosave/journal/recovery を考慮。

正常終了しなかったとき、最後の安全な revision を復元可能にする。

## 91. Sync

Cross-device sync 実装は OS 共通 Sync/Continuity layer を再利用。

各 App が独自 account/sync engine を勝手に作らない。

## 92. Conflict

revision-aware conflict detection。

Object merge が可能なら利用。

不可能なら user-visible conflict。

---

# Part XII — Implementation Plan

以下は推奨依存順。既存 Nagi OS の Milestone 体系がある場合、その番号体系へ統合する。

## M-APP-00 — Repository Audit

- AGENTS.md 読了
- existing specs 読了
- Albert audit
- existing app/platform APIs inventory
- packaging/localization/device/model policy inventory
- current tests/build baseline

Deliverable:

- audit note
- implementation_status initialized
- no behavioral rewrite yet

## M-APP-01 — Core Contracts

Implement/normalize:

- App Registry
- Permission/Capability
- Resource IDs
- Document/Object contracts
- Action Registry
- Intent contract
- Context Broker

Tests first where practical.

## M-APP-02 — Activity / Revision / Checkpoint

- Activity Ledger
- Transaction
- Revision
- Diff contract
- Checkpoint
- Restore/Restore as copy

## M-APP-03 — Workspace / Continuity / Device Surface Contracts

- Workspace Service
- Device model
- Surface contract
- Capability query
- Continuity semantic state

Desktop implementation first.

## M-APP-04 — Search Foundation

- Search Provider contract
- lexical/metadata
- permission filtering
- semantic provider abstraction
- incremental index

## M-APP-05 — Files

- LocalFileProvider
- TrashProvider
- Files Desktop UI
- Resource integration
- Activity/Wayback/Search

## M-APP-06 — Notes

- Block model
- Markdown
- References
- Quick Note
- Search/Activity/Wayback

## M-APP-07 — Home

- Continue
- Recent
- Workspace cards
- Search/Intent entry
- Continuity cards

## M-APP-08 — Terminal v0.2

- Terminal Core
- Shell Surface
- Assisted Surface
- Natural Surface
- command risk
- RunningJob
- Activity/Continuity

## M-APP-09 — Albert Platform Migration

- KEEP/ADAPT implementation
- Context
- Objects
- Actions
- Search
- Files/Notes handoff
- Workspace/Continuity
- Private Session

Regression tests mandatory.

## M-APP-10 — Document Framework

- shared Document identity
- Object/revision
- autosave
- resource refs
- cross-app refs
- import/export lifecycle
- diff provider contract

## M-APP-11 — Writer

Implement spec + acceptance.

## M-APP-12 — Sheets

Implement calculation engine independently before UI-heavy work.

Then Grid/UI/format/import/export/cross-app refs.

## M-APP-13 — Slides

Implement presentation model/theme/layout/editor, then cross-app refs/AI plan/presenter/export.

## M-APP-14 — People Foundation

Implement common identity/contact model before deep Mail/Calendar integration where useful.

## M-APP-15 — Mail

Provider abstraction first, then UI/drafts/search/cross-app/external action policy.

## M-APP-16 — Calendar

Timezone/recurrence model first, then views/scheduling/cross-app/external invite policy.

## M-APP-17 — Automations

Action composition, trigger service, permissions, preview/run history.

## M-APP-18 — Studio

Only after Writer/Sheets/Slides cross-app contracts are working.

## M-APP-19 — Devices

Expose Continuity/Device/Job capabilities through user-facing App.

## M-APP-20 — Models

Connect to existing capability/provider Model Registry and Model Router. Do
not invent conflicting model architecture, hardcode vendor/model names, or
make Jev/cloud availability a prerequisite. Expose Generative and Decision
roles, local/offline privacy policy, provider health, fallback, and the
existing Granite/Qwen/Gemma policy.

## M-APP-21 — Store

Connect to existing package/security model. Preserve side-loading principle.

## M-APP-22 — Multi-surface Hardening

- verify no Core depends on Desktop widgets
- tablet layout prototypes
- mobile layout prototypes
- input capability tests
- semantic Continuity E2E

Full feature parity on Tablet/Mobile is not required for Nagi 0.x unless existing OS milestones say otherwise.

## M-APP-23 — Integration E2E

Run cross-app workflow tests A-E and failure cases.

## M-APP-24 — Performance / Security / Accessibility Hardening

- large file/data tests
- search index permission tests
- activity secret redaction
- crash recovery
- offline/partial failure
- accessibility checks

## M-APP-25 — Documentation / SDK Examples

Document public Platform APIs.

Where licensing/product policy permits, first-party app integration examples should serve as reference for third-party developers.

---

# Part XIII — Common Acceptance Tests

## 93. Platform

- `PLATFORM-001`: third-party test app can register without first-party private API
- `PLATFORM-002`: app without Agent integration can still launch/use basic OS services
- `PLATFORM-003`: Level 1 app can publish Context
- `PLATFORM-004`: Level 2 app can register structured Action
- `PLATFORM-005`: Level 3 test app can participate in Workspace/Activity/Continuity contracts
- `PLATFORM-006`: System Capability is distinct from first-party identity

## 94. Context / Action

- `CORE-CTX-001`: App explicitly publishes Context
- `CORE-CTX-002`: selected Object can resolve `this/これ`
- `CORE-ACT-001`: Action schema validation rejects invalid input
- `CORE-ACT-002`: permission check runs before Action
- `CORE-ACT-003`: UNKNOWN risk is not silently executed under low-risk policy

## 95. Activity / Wayback

- `CORE-ACTIVITY-001`: Agent mutation always records Activity
- `CORE-ACTIVITY-002`: partial Transaction remains partial
- `CORE-WB-001`: reversible local change restores
- `CORE-WB-002`: irreversible external change has no fake undo
- `CORE-WB-003`: restore operation itself is logged

## 96. Search

- `CORE-SEARCH-001`: permission denial removes result entirely
- `CORE-SEARCH-002`: exact search works without AI model
- `CORE-SEARCH-003`: semantic provider can be replaced
- `CORE-SEARCH-004`: index updates incrementally

## 97. Continuity

- `CORE-CONT-001`: Workspace semantic state transfers between simulated devices
- `CORE-CONT-002`: pixel/window coordinates are not required
- `CORE-CONT-003`: device capability change produces valid Surface adaptation
- `CORE-CONT-004`: revision conflict does not silently overwrite

## 98. Localization

- `CORE-L10N-001`: English UI
- `CORE-L10N-002`: Japanese UI
- `CORE-L10N-003`: mixed Japanese/English content survives round-trip
- `CORE-L10N-004`: internal identifiers remain locale-independent

## 99. Security

- `CORE-SEC-001`: Agent cannot bypass app permission
- `CORE-SEC-002`: secret Resource cannot leak through Search
- `CORE-SEC-003`: system operation requires System Capability
- `CORE-SEC-004`: external side effect has policy/confirmation path
- `CORE-SEC-005`: sensitive Activity/log redaction

---

# Part XIV — Explicit Non-goals / Deferred

以下は本仕様の構造を壊さない範囲で後回し可能。

- System-wide full snapshot implementation
- Full Google Docs/Sheets style real-time collaborative editing
- Complete DOCX/XLSX/PPTX feature parity
- VBA execution
- Power Query clone
- Advanced pivot engine in Sheets v0.1
- Full remote desktop/control in Devices
- Cloud-only dependency for local search/editing
- Decision capability that is unavailable without an external provider
- Full Tablet/Mobile feature parity during earliest Desktop-first milestone
- Nagi-specific mandatory programming language
- Store-only app distribution
- first-party-only hidden Agent APIs

Deferred は「設計無視」を意味しない。将来追加できる extension point を維持すること。

---

# Part XV — Prohibited Implementations

Codex は以下を行ってはならない。

1. Albert を既存監査なしに全面書き換え
2. 各 App が独自 Resource ID system を持つ
3. 各 App が独自 Search engine を持つ
4. 各 App が独自 Sync/Device identity を持つ
5. Writer/Sheets/Slides が別々に Document persistence 基盤を再発明
6. Agent が GUI click を標準操作経路にする
7. Activity を debug log で代用
8. Wayback を単なる Ctrl+Z として実装
9. Search index に secret を無条件保存
10. `is_first_party == true` を特権の根拠にする
11. Tablet/Mobile のために App logic を複製
12. Desktop window geometry を Continuity の必須状態にする
13. Terminal Natural Surface が全要求を shell command 化
14. Mail/Calendar external side effect を silent auto-send
15. failed operation を UI 上で success 扱い
16. Office import で unsupported feature を無警告破棄
17. Model 名を各 App へ hardcode
18. Store を唯一の配布経路にする
19. Japanese support を後付け翻訳扱いにする
20. AI unavailable 時に通常の App 機能まで停止させる

---

# Part XVI — Definition of Done

各 Milestone / App は以下を満たして初めて Done。

1. Build が通る
2. Unit tests が通る
3. Integration tests が通る
4. 対象 Acceptance Tests が通る
5. Failure path が実装されている
6. Activity/Permission/Checkpoint が必要な箇所で動作
7. existing behavior regression がない
8. Desktop Surface で実操作可能
9. Core logic が Desktop UI に密結合していない
10. English/Japanese UI resource 対応
11. implementation_status.md 更新
12. known limitation を明記
13. Fake success/TODO stub で test を通していない

---

# Part XVII — Recommended Repository Layout

既存レイアウトがある場合はそれを優先し、無理に変更しない。

新規整理が必要な場合の論理例:

```text
platform/
  app/
  resource/
  document/
  actions/
  intent/
  context/
  workspace/
  activity/
  checkpoint/
  search/
  continuity/
  devices/
  permissions/

frameworks/
  document/
  terminal/

apps/
  home/
  albert/
  files/
  notes/
  terminal/
  activity/
  wayback/
  search/
  writer/
  sheets/
  slides/
  mail/
  calendar/
  automations/
  studio/
  people/
  devices/
  store/
  models/

docs/
  first_party/
    NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md
    implementation_status.md
```

これは責務分離の例であり、既存 monorepo/build system を破壊して採用してはならない。

---

# Part XVIII — Documentation Deliverables

Codex は実装に合わせて最低限次を整備する。

## A. Platform API

- Resource
- Document/Object
- Action schema
- Context publish/read
- Workspace
- Activity
- Checkpoint
- Search Provider
- Continuity
- Permission/Capability

## B. First-party integration examples

最低限:

- Notes Block Action example
- Files Resource Provider example
- Albert Context publish example
- Writer Object/Diff example
- Sheets Chart cross-app reference example

## C. Third-party onboarding

「普通の App」として開始し、段階的に Nagi 統合を追加できる流れを示す。

例:

```text
1. Build/install app
2. Add manifest
3. Publish Context
4. Register Actions
5. Add Activity/Wayback
6. Add Workspace/Continuity
```

Nagi 専用の難解な巨大 SDK を最初から要求しない。

---

# Part XIX — Final Codex Execution Directive

以下を本書と同じリポジトリで Codex に実行させる場合、そのまま開始指示として使用できる。

> この `NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md` を、Nagi 純正ソフトウェア群と共通 Platform 基盤に対する追加の必須実装仕様として読んでください。
>
> 実装開始前に、リポジトリ内の `AGENTS.md`、Nagi OS 主仕様、既存の共通基盤仕様、security / device / localization / package / model 関連仕様、既存コード、既存テストを確認してください。本書単独で既存設計を上書きしないでください。矛盾がある場合は上位仕様を尊重しつつ、本書の目的を満たす形へ整合させてください。
>
> 特に Albert は既存 0.1 実装を土台として発展させます。既存実装を監査せず全面的に作り直すことは禁止します。最初に KEEP / ADAPT / REPLACE / DEFER を整理してください。
>
> 実装は `Part XII — Implementation Plan` の依存順を基本に進めてください。既存リポジトリに別の正式 Milestone 体系がある場合は、その体系へ統合してください。
>
> Nagi 0.x の実 UI は Desktop first で構いませんが、Core / Document / Action / Context / Workspace / Continuity を Desktop UI へ密結合させないでください。Tablet / Mobile Surface を後から追加できる構造を維持してください。
>
> 純正アプリだけが利用できる秘密 API に依存させないでください。純正アプリは公開 Nagi Platform API を最大限利用するリファレンス実装とし、System Capability が必要な OS 管理操作だけを明示的に区別してください。
>
> 実装中に軽微な不明点が出ても、既存仕様・本書・テスト可能性から合理的に判断して作業を止めず、判断を `implementation_status.md` に残してください。外部資格情報等の真のブロッカーを除き、可能な範囲を先に進めてください。同一障害は原因を変えながら最大10回程度の合理的な修正・再試行を行い、それでも解消しない場合のみ BLOCKED としてください。
>
> 各 Milestone では Build、Unit Test、Integration Test、該当 Acceptance Test を実行してください。テストを通すためだけの fake implementation、常時成功 stub、未実装機能の成功レスポンスは禁止します。失敗・partial・offline・permission denied を正しく表現してください。
>
> Agent によるユーザー成果物の変更は Activity を残し、復元可能な大規模変更では Checkpoint/Wayback を利用してください。外部副作用や不可逆操作に fake Undo を表示しないでください。
>
> 実装・テスト・ドキュメント更新を継続し、各 Milestone の Acceptance Test が通ったものだけを PASS としてください。作業終了時に、完了した Milestone、テスト結果、未完了項目、既知の制約、次に実行すべき作業を `implementation_status.md` に具体的に記録してください。

---

# Part XX — Completion Checklist

実装完了判定時に以下を確認する。

## Platform

- [ ] Public Platform API と Core internal service が分離されている
- [ ] first-party flag が権限根拠になっていない
- [ ] Resource / Document / Object ID が共通化されている
- [ ] Action / Intent / Context が共通化されている
- [ ] Activity / Checkpoint / Wayback が共通化されている
- [ ] Workspace / Continuity が共通化されている
- [ ] Search Provider が共通化されている

## Phase 1

- [ ] Activity
- [ ] Wayback
- [ ] Files
- [ ] Search
- [ ] Notes
- [ ] Home
- [ ] Terminal Natural/Assisted/Shell
- [ ] Albert 0.1 migration

## Phase 2

- [ ] Writer
- [ ] Sheets
- [ ] Slides
- [ ] Mail
- [ ] Calendar
- [ ] Automations

## Phase 3

- [ ] Studio
- [ ] People
- [ ] Devices
- [ ] Store
- [ ] Models

## Multi-device

- [ ] Desktop implementation usable
- [ ] Core logic surface-neutral
- [ ] Tablet Surface contract
- [ ] Mobile Surface contract
- [ ] Continuity semantic state
- [ ] conflict handling

## Quality

- [ ] English/Japanese
- [ ] permission tests
- [ ] Activity redaction
- [ ] Search permission filtering
- [ ] crash recovery
- [ ] failure/partial states
- [ ] accessibility basics
- [ ] performance tests
- [ ] no fake success

---

# End of Specification

この文書で定義した Working Name は将来変更可能である。

変更してはならないのは、以下の製品思想である。

1. **Nagi は PC 専用のアプリ群ではなく、同じ意味モデルを複数 Device / Surface に展開する。**
2. **純正アプリは Nagi Platform の最深統合例だが、第三者を排除する秘密 API で成立させない。**
3. **Agent は UI を力技でクリックするのではなく、意味を持つ Action / Object / Context を通じて操作する。**
4. **AI の操作は追跡可能であり、可能な変更は Wayback で戻せる。**
5. **Nagi が存在しなくても通常アプリとして使え、Nagi を使うと大きく能力が増幅される。**
6. **Terminal を含め、上級者の能力を削らずに初心者の入口を広げる。**
7. **Desktop first で実装しても Desktop only の設計にはしない。**
8. **アプリ同士を直接密結合せず、Nagi Platform を介してひとつの環境として連携させる。**
