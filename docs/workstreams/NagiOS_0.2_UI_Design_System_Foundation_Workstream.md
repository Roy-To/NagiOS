# Nagi OS 0.2 — UI Design System Foundation Workstream

## 1. Workstream

- Proposed ID: `UI-DS-01`
- Name: `Nagi 0.2 UI Design System Foundation`
- Proposed branch: `codex/0.2-ui-design-system`
- Target: Nagi OS 0.2
- Type: Independent foundation workstream
- Priority: High
- Final target state: `PASS`

同等目的のUI Design System workstreamがすでにDF-01またはリポジトリ内に存在する場合、新しいworkstreamや競合branchを重複作成しないこと。

その場合は、

1. 既存workstreamを特定する
2. 既存branch / state / ownership / dependenciesを引き継ぐ
3. 本Markdownを追加・更新された必須実装仕様として扱う
4. 既存の有効な実装を保全する

Base SHAは推測せず、Git、DF-01、workstream state等から実際のapproved baseを確認すること。

---

## 2. Purpose

Nagi 0.2のfirst-party application全体で共有できるUI基盤を構築する。

対象例:

- Albert
- Files
- Notes
- Terminal
- Activity
- Wayback
- Search
- Home

個別アプリのUIを完成させることではなく、以下を共通化することを目的とする。

- visual/design tokens
- spacing
- typography
- layout conventions
- focus behavior
- keyboard interaction
- component states
- accessibility
- localization-aware layout
- reusable UI primitives
- first-party application shell contract

NagiのUIは以下の性格を維持する。

- quiet
- controlled
- clear
- capable
- low-noise
- predictable

不要な装飾や派手なanimationを共通UI contractへ組み込まない。

---

## 3. Relationship to Nagi 0.1

本workstreamはNagi 0.1 M18以降と並行して実行する。

0.2側の都合だけで以下を変更・置換しないこと。

- active M18/M19 implementation
- kernel
- boot flow
- Servo integration
- active runtime interfaces
- 他workstream所有領域

0.1 runtimeへの直接統合が未完成の場合は、

- interface
- adapter
- mock
- test double
- host-testable abstraction

を用いて独立作業を先行する。

Nagi 0.1完了を本workstream開始条件にしない。

---

## 4. Architectural Principles

### 4.1 Platform-neutral core

design semanticsとrenderer implementationを分離する。

可能な限り、

- design tokens
- component state
- interaction contracts

を、

- compositor
- GPU
- renderer
- final runtime implementation

から独立させる。

### 4.2 First-party consistency

first-party appが個別に以下を再定義しないようにする。

- spacing
- typography
- focus
- disabled state
- selected state
- hover state
- error state
- loading state
- control sizing
- application padding

### 4.3 English baseline + Japanese equal-class support

内部engineering基準は英語でよいが、日本語を同格の正式対応言語として扱う。

以下を避ける。

- ASCII前提
- 英語文字列長前提
- 固定幅label
- 日本語で崩れるline height
- 英語専用layout

### 4.4 Accessibility

最低限以下を共通contractへ含める。

- keyboard navigation
- visible focus
- logical focus order
- disabled semantics
- selected semantics
- semantic labels
- input/error relation
- scalable text
- reduced-motion compatibility

---

## 5. Scope

### 5.1 Design Tokens

typed/common tokenとして最低限以下を定義する。

- spacing
- control dimensions
- radius/border semantics
- typography roles
- layer/elevation semantics
- icon sizing
- motion duration
- focus treatment
- state semantics
- density

raw magic numberの分散を避ける。

### 5.2 Typography

semantic roleを定義する。

例:

- body
- secondary body
- caption
- label
- heading
- title
- monospace/code

proprietary fontをcore contractへ強制しない。

Latin / Japanese双方のmetricsを考慮する。

### 5.3 Interaction States

最低限:

- normal
- hover
- pressed
- focused
- selected
- disabled
- loading
- success
- warning
- error

可能であれば無関係なboolean乱立ではなく、構造化されたstate modelとする。

### 5.4 Focus / Keyboard

最低限定義する。

- Tab
- reverse Tab
- Enter / Space activation
- Escape/cancel
- arrow navigation
- focus restore
- modal focus containment

runtime未完成でもhost-side test可能なcontractは先行実装する。

### 5.5 Common Primitives

適切なものから実装する。

候補:

- Button
- IconButton
- TextField
- SearchField
- Checkbox
- Radio
- Switch
- Select
- ListItem
- Menu
- ContextMenu
- Tabs
- Toolbar
- ProgressIndicator
- Dialog
- Panel
- Tooltip
- EmptyState
- ErrorState
- ApplicationShell

数を増やすこと自体を目的にしない。

### 5.6 Application Shell Contract

first-party app共通構造として必要に応じ、

- title region
- navigation
- primary content
- sidebar
- toolbar
- status area
- overlay
- dialog
- loading/error surfaces

を定義する。

### 5.7 Localization-aware Layout

以下を扱う。

- expanding labels
- multiline
- Japanese text
- truncation
- ellipsis
- wrapping
- min/max sizing

critical informationを無言でtruncateしない。

---

## 6. Non-scope

以下は原則対象外。

- kernel redesign
- boot redesign
- M18変更
- Servo replacement
- Albert固有機能
- Files固有機能
- GPU driver
- Model Runtime
- Wayback internals
- unrelated Capability redesign
- first-party app全体rewrite

---

## 7. Repository / Ownership

最初に確認する。

- `AGENTS.md`
- main specs
- `docs/0.2/DEVELOPMENT_ARCHITECTURE.md`
- `.dev/workstreams.json`
- current `state.json`
- current Git state
- existing UI modules
- localization infrastructure
- accessibility infrastructure

既存moduleがある場合は競合する新architectureを作らず拡張する。

Shared registryがIntegration Owner所有なら直接変更せず、validated registration proposalを残す。

---

## 8. Implementation Sequence

### Phase A — Audit

既存UI、tokens、shell、localization、testsを確認する。

調査結果だけで終了しない。

### Phase B — Core Semantics

実装:

- token types
- typography roles
- state model
- focus contracts
- localization-aware sizing

同時にtestsを追加する。

### Phase C — Primitive Components

現architectureで実装可能な高価値primitiveを実装する。

### Phase D — Application Shell

app固有logicを持たない共通shell contractを実装する。

### Phase E — Verification

実行:

- focused unit tests
- module/crate tests
- host tests
- format
- lint
- safe integration tests

担当範囲のfailureは修正する。

### Phase F — State / Git

- docs更新
- state更新
- dependency記録
- acceptance更新
- commit
- push

まで進める。

---

## 9. Required Tests

最低限:

- token invariants
- state transition
- disabled behavior
- focus transition
- keyboard semantics
- Japanese/English text
- localization-sensitive sizing
- invalid state
- component defaults
- accessibility invariants

---

## 10. Acceptance Criteria

- [ ] typed design tokens
- [ ] typography roles
- [ ] spacing/density conventions
- [ ] interaction state model
- [ ] keyboard/focus contracts
- [ ] Japanese/English対応
- [ ] accessibility contract
- [ ] reusable core primitives
- [ ] application shell contract
- [ ] relevant tests PASS
- [ ] format/lint PASS
- [ ] docs updated
- [ ] workstream state updated
- [ ] committed
- [ ] pushed
- [ ] working tree clean

renderer/runtime attachmentのみ別workstream依存で残る場合、interface境界と将来integration pointを明確にし、mock/adapterでfoundationが検証済みなら独立foundationをPASSにできる。

---

## 11. Completion Report

報告する。

- Status
- Workstream ID
- Branch
- Base SHA
- HEAD SHA
- Implemented
- Tests
- CI
- Deferred integration
- Blockers
- Push status
- Working tree status
