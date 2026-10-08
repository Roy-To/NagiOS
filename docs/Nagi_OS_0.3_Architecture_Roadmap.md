# NAGI OS 0.3 Architecture Roadmap

**Project:** NAGI — Native Agentic General Interface  
**Document Type:** Architecture Roadmap  
**Target Version:** 0.3  
**Status:** Draft / Forward Architecture  
**Purpose:** 0.2実装中に、0.3で必要となる将来互換性と設計境界を明確化する  
**Implementation Status:** 本文書自体は実装開始を意味しない

---

## 1. Overview

NAGI OS 0.3は、NAGIを「ローカルAIを搭載したOS」から、**ユーザーと継続的に関係を構築し、記憶・学習・行動履歴を保持しながら成長する Personal Agent OS** へ進化させるための世代と位置付ける。

NAGIの世代ごとの大きな役割は以下とする。

- **0.1:** NAGIがOSとして起動し、主要機能が動作する基盤を成立させる
- **0.2:** Identity、Permissions、IPC、Jobs、Settings、Model Runtime、Update、Notification等のOS基盤を整備する
- **0.3:** NAGI固有のAI体験である、Profile、Memory、Learning、Model Portability、継続的なPersonal Agent機能を成立させる
- **1.0:** 一般公開可能な品質、安定性、セキュリティ、互換性、開発者体験を備えたNAGI OSとして完成させる

0.3では、特定のLLMそのものをNAGIの人格・記憶・学習状態として扱ってはならない。

NAGIの「ユーザーを理解している状態」は、Granite、ELYZA、Qwen、その他将来のモデルを交換しても維持されなければならない。

---

## 2. Goals

NAGI OS 0.3の主要目標は以下とする。

1. NAGI Profileを導入し、AIの記憶・設定・学習状態をユーザー単位で管理する
2. LLM非依存のUnified AI Contextを確立する
3. 短期・長期・習慣・嗜好・知識を区別したMemory Architectureを構築する
4. 行動の反復・明示指定・時間減衰等を考慮したLearning / Weighting基盤を導入する
5. モデルを交換してもNAGIの記憶と人格的連続性が失われない構造を確立する
6. 個人利用・複数ユーザー利用・チーム利用の基礎を成立させる
7. PC移行時にNAGI Profileを安全に移行できるようにする
8. ローカルファーストかつユーザー所有のデータモデルを維持する
9. 将来的なNagi Account Syncへ拡張可能な境界を用意する
10. 低スペック環境でも成立するAI Context / Memory構造を優先する

---

## 3. Non-Goals

0.3では以下を必須目標としない。

- 完全なクラウド同期サービスの提供
- Nagi Accountの本番運用
- NAGI独自の大規模基盤モデル開発
- 全モデル形式への完全対応
- モデル自体のファインチューニングをユーザー記憶の主要手段とすること
- 常時クラウド接続を前提としたPersonal Agent
- ユーザーの許可なく収集・永続化される行動履歴
- 全アプリケーションへの無制限なMemoryアクセス
- 0.2の主要基盤が不安定な段階での大規模0.3実装開始

---

## 4. 0.3 Design Principles

### 4.1 Model-Independent Memory

ユーザー記憶はLLMから独立したNAGI標準形式で保持する。

**禁止事項:**

- Granite固有形式を唯一の永続形式とする
- QwenのKV Cache等をユーザー記憶そのものとして扱う
- モデル交換によりNAGI Profileが失われる構造
- Prompt全文を唯一のMemory Storeとして永続化する

LLMはMemoryを「利用する実行エンジン」であり、Memoryそのものではない。

---

### 4.2 Local First

NAGIのPersonal Agent機能は、クラウドが存在しなくても成立することを基本とする。

- Profile
- Memory
- Learning state
- Preferences
- Habit
- Context
- Audit history

これらの主要データはローカル環境で完結可能でなければならない。

---

### 4.3 User-Owned Data

NAGI ProfileおよびMemoryは、ユーザーが所有・閲覧・削除・移行できるデータとして扱う。

ユーザーは少なくとも以下を実行できることを目標とする。

- 自分のMemoryを確認する
- 特定Memoryを削除する
- Memoryカテゴリを無効化する
- ProfileをExportする
- ProfileをImportする
- PC移行後に継続利用する

---

### 4.4 Explicit Identity Boundary

AI Contextは明示的なIdentity境界を持つ。

最低限、将来的に以下へ紐づけ可能な構造とする。

- UserId
- ProfileId
- SessionId
- DeviceId
- TeamId
- ApplicationId

0.3では必ずしも全IDを本番運用しないが、単一ユーザー固定の構造にはしない。

---

### 4.5 Small-System Friendly

NAGIは高性能GPU搭載PCだけを前提としない。

Memory検索、Context構築、Profile管理は、低スペック端末でも成立することを重視する。

設計上、以下を避ける。

- 常時巨大Embedding IndexをRAMへ常駐
- 全会話履歴を毎回LLMへ投入
- モデルごとにMemory全体を複製
- AI Context構築のために常時大規模推論を要求
- バックグラウンドで常時高負荷の再学習

---

## 5. High-Level Architecture

```text
+------------------------------------------------------------+
|                        NAGI Profile                        |
|                                                            |
|  Identity / Preferences / Memory / Habits / User Settings |
+------------------------------+-----------------------------+
                               |
                               v
+------------------------------------------------------------+
|                    Unified AI Context                      |
|                                                            |
|  Session Context                                            |
|  Relevant Memory                                            |
|  User Preferences                                           |
|  Application Context                                        |
|  Permission-filtered Data                                   |
+------------------------------+-----------------------------+
                               |
                               v
+------------------------------------------------------------+
|                      Context Adapter                       |
|                                                            |
|  Granite Adapter | ELYZA Adapter | Qwen Adapter | Future   |
+------------------------------+-----------------------------+
                               |
                               v
+------------------------------------------------------------+
|                       Model Runtime                        |
|                                                            |
|  llama.cpp / future runtimes / model-specific execution    |
+------------------------------------------------------------+
```

重要な境界は以下とする。

> **NAGI Profile / Memory → Unified AI Context → Context Adapter → Model Runtime**

Model Runtimeから上位のユーザー記憶を直接所有させない。

---

## 6. NAGI Profile

### 6.1 Purpose

NAGI Profileは、NAGIがユーザーとの継続的な関係を保持するための論理単位とする。

単なるOSユーザー設定とは分離する。

### 6.2 Candidate Structure

```text
NagiProfile
 ├─ ProfileId
 ├─ Owner
 │   ├─ UserId
 │   └─ optional TeamId
 ├─ Preferences
 ├─ Memory
 ├─ Habits
 ├─ Personalization
 ├─ Agent Settings
 ├─ Privacy Policy
 ├─ Model Preferences
 ├─ Learning State
 └─ Migration Metadata
```

### 6.3 Relationship with OS Identity

0.2のIDENT-01等で確立するOS Identityと、NAGI Profileは1:1固定にしない。

将来的に以下を許容する。

- 1 User : 1 Profile
- 1 User : 複数Profile
- Guest Profile
- Temporary Profile
- Shared Team Profile
- Device-local Profile
- Synced Profile

---

## 7. Unified AI Context

### 7.1 Purpose

Unified AI Contextは、NAGI内でLLMへ渡す情報を統一的に構成する中間層とする。

LLMへ直接OS内部データを渡すのではなく、必要な情報のみをPermission評価後にContextへ変換する。

### 7.2 Candidate Context

```text
UnifiedAiContext
 ├─ ProfileId
 ├─ UserId
 ├─ SessionId
 ├─ AppId
 ├─ ActiveTask
 ├─ ConversationContext
 ├─ RelevantMemories[]
 ├─ Preferences[]
 ├─ Habits[]
 ├─ DeviceContext
 ├─ PermissionSnapshot
 └─ ContextMetadata
```

### 7.3 Requirements

- Model Runtimeに依存しない
- JSON等の安定した中間表現を持てる
- Contextサイズを制御できる
- Priority / relevanceによる選択が可能
- Sensitive MemoryをPermissionで除外できる
- モデル別Context Window差をAdapter側で吸収可能

---

## 8. Memory Architecture

NAGI Memoryは単一テーブルではなく、用途に応じて論理的に分類する。

### 8.1 Working Memory

現在のタスクや会話に必要な一時的Memory。

例:

- 現在編集中の文書
- 直前の指示
- 今開いているアプリ
- 現在処理中の計画

特徴:

- 短寿命
- 高速
- SessionIdとの関連が強い
- 原則として長期保存不要

---

### 8.2 Episodic Memory

ユーザーとNAGIの過去の出来事を記録する。

例:

- 過去に行った作業
- 以前選択した設定
- プロジェクト上の意思決定
- 特定のタスクの結果

重要なのは全文ログ保存ではなく、後から再利用可能な出来事として整理すること。

---

### 8.3 Semantic Memory

ユーザーに関する比較的安定した知識や、NAGIが継続利用する事実を保持する。

例:

- 利用環境
- プロジェクト構成
- よく使用するツール
- 長期的な設定

Semantic Memoryは、会話ログそのものとは分離する。

---

### 8.4 Preference Memory

ユーザーの好みを管理する。

例:

- UI表示
- 回答スタイル
- 使用するモデル
- よく利用するアプリ
- 動作確認の粒度
- 自動化に対する許容範囲

Preferenceは暗黙推定だけでなく、ユーザーが明示的に設定・解除できること。

---

### 8.5 Procedure / Habit Memory

繰り返し実行される手順や習慣を保持する。

例:

```text
毎回:
  Source更新
    ↓
  Test
    ↓
  Fix
    ↓
  Verify
    ↓
  Commit
    ↓
  Push
```

単に「過去に一度行った」だけではHabitとみなさず、繰り返しや明示指定によりConfidenceを上げる。

---

## 9. Learning and Weighting

0.3では、Memoryに単純な保存/非保存だけでなく重み付けを導入する。

候補パラメータ:

```text
importance
confidence
frequency
recency
explicitness
stability
sensitivity
last_used_at
created_at
```

### 9.1 Weight Increase

以下で重要度を上げる。

- ユーザーが明示的に「覚えて」と指定
- 同じ行動を複数回繰り返す
- 複数の異なるSessionで同じPreferenceが観測される
- NAGIがMemoryを再利用し、その結果をユーザーが採用

### 9.2 Weight Decay

以下で重要度を徐々に下げる。

- 長期間利用されない
- 新しいPreferenceと矛盾
- 一時的Projectの終了
- ユーザーによる修正

### 9.3 Explicit User Override

ユーザーの明示指定は、暗黙学習より常に優先する。

```text
Explicit User Setting
    >
Confirmed Learned Preference
    >
Repeated Behavior
    >
Single Observation
```

---

## 10. Forgetting Model

人間の記憶と同様、全データを永久保存することを前提としない。

ただし、削除と自動減衰は区別する。

### 10.1 Forget

ユーザーが明示的に忘却を要求した場合。

対象Memoryを利用対象から除外し、必要に応じて完全削除する。

### 10.2 Decay

重要度の低いMemoryの重みを下げる。

### 10.3 Archive

現在のContext構築対象から外すが、履歴として保持する。

### 10.4 Protected Memory

ユーザーが固定したPreference等は自動減衰させない。

---

## 11. Model Portability

0.3では、LLMの交換を通常操作として扱える設計を目標とする。

候補モデル:

- IBM Granite
- ELYZA
- Qwen
- その他将来のローカルLLM

### Required Principle

> モデル交換はNAGI Profile、Memory、Preference、Habitを破壊してはならない。

### Example

```text
Granite
   |
   v
NAGI Unified Context
   |
   +---- Memory Store
   +---- Preferences
   +---- Habits
   +---- Profile

モデル交換

ELYZA
   |
   v
同じNAGI Unified Context
```

---

## 12. Context Adapter

モデル間には以下の差が存在する。

- Context Window
- System Prompt形式
- Chat Template
- Tokenizer
- Tool Calling形式
- Function Calling能力
- JSON出力安定性
- Multimodal能力
- 言語特性

これらをNAGI Coreへ持ち込まず、Context Adapterで吸収する。

候補:

```text
ModelAdapter
 ├─ build_prompt()
 ├─ build_system_context()
 ├─ trim_context()
 ├─ map_tools()
 ├─ parse_tool_calls()
 ├─ map_multimodal_context()
 └─ normalize_output()
```

---

## 13. Model Hot-Swap

0.3では完全な無停止切替を必須とはしないが、ユーザーがモデルを容易に変更できる設計を目標とする。

モデル変更時に保持するもの:

- NAGI Profile
- Memory
- Preferences
- Habits
- Task metadata
- User settings

モデル変更時に破棄可能なもの:

- KV Cache
- Model-specific temporary state
- Runtime-specific memory buffers
- Session-local inference cache

---

## 14. Profile Portability

NAGI Profileは、PCそのものに固定しない。

### 14.1 Export

Profile Export Packageの候補:

```text
nagi-profile/
 ├─ manifest.json
 ├─ profile.json
 ├─ preferences/
 ├─ memory/
 ├─ habits/
 ├─ settings/
 ├─ audit/
 └─ integrity/
```

### 14.2 Import

別NAGI端末にProfileをImportした場合でも、以下を維持できること。

- User preferences
- Learned habits
- Long-term memory
- Personalization
- Agent configuration

---

## 15. Multi-User

同一PCを複数ユーザーで利用するケースを考慮する。

最低条件:

- Memoryのユーザー分離
- Profileのユーザー分離
- Permission分離
- Search Index分離
- Learning state分離

ユーザーAのMemoryがユーザーBのContextへ混入してはならない。

---

## 16. Team Profile

将来的なチーム利用を想定する。

Team Profileは個人Profileと分離する。

```text
Personal Profile
   |
   +---- Personal Memory
   +---- Personal Preference

Team Profile
   |
   +---- Shared Knowledge
   +---- Shared Procedures
   +---- Team Preferences
```

Personal MemoryをTeam Profileへ自動共有してはならない。

共有は明示的なPermissionを要求する。

---

## 17. Privacy and Permission

MemoryはOS内の他データと同様にPermission管理対象とする。

候補Permission:

```text
memory.read
memory.write
memory.delete
memory.search
profile.read
profile.update
habit.read
habit.write
context.request
```

ApplicationIdごとにアクセス範囲を評価可能にする。

---

## 18. Memory Audit

NAGIがMemoryを利用した際、将来的に以下を追跡可能にする。

```text
MemoryId
Requester
ApplicationId
Purpose
AccessedAt
Action
```

ユーザーは「なぜNAGIがこの情報を知っているのか」を確認できることを目標とする。

---

## 19. Backup and Restore

Profile BackupはOS全体Backupとは分離できる構造とする。

### Requirements

- Profile単位Backup
- Integrity check
- Version metadata
- Schema migration
- Restore preview
- Sensitive data protection

---

## 20. Future Nagi Account Sync

0.3ではNagi Account Syncの本番実装を必須としない。

ただしProfileは、将来的に以下の構造へ拡張可能とする。

```text
Local Profile
      |
      v
Encrypted Sync Package
      |
      v
Nagi Account
      |
      +---- Device A
      +---- Device B
      +---- Device C
```

クラウドサービスを前提にせず、Sync Adapterを後付け可能な設計とする。

---

## 21. 0.2 Forward Compatibility Requirements

0.3で大規模な手戻りを発生させないため、0.2実装時点で以下を守る。

### FC-01 — Model Runtime must not own user memory

Model RuntimeはユーザーMemoryの永続的Source of Truthになってはならない。

### FC-02 — Context must support identity binding

AI Contextは将来的に少なくとも以下へ紐づけ可能にする。

- UserId
- ProfileId
- SessionId
- ApplicationId

### FC-03 — Identity must not assume one user forever

Identity実装は単一ユーザー固定にしない。

### FC-04 — Settings must support scopes

Settingsは将来的に以下を区別可能にする。

```text
system
device
user
profile
application
team
```

### FC-05 — Storage must support ownership metadata

AI関連データにOwner / Scope / Permission metadataを付与可能にする。

### FC-06 — Memory and Model Runtime must be separate

Memory APIとInference APIを同一責務へ統合しない。

### FC-07 — IPC must carry identity context

AI関連IPCで、必要に応じてCaller / User / Session / App情報を伝達可能にする。

### FC-08 — Permission must be enforceable at context construction

LLMへデータを投入した後ではなく、Context構築前にPermission評価可能にする。

### FC-09 — Jobs must support profile-scoped background work

将来的なMemory整理・Index更新・Backup等をProfile単位で実行可能にする。

### FC-10 — Avoid model-specific persistent state

モデル固有状態をユーザーの長期データ形式として固定しない。

---

## 22. Relationship with 0.2 Workstreams

### IDENT-01

0.3依存度: **High**

必要事項:

- UserId
- ProfileとIdentityの関連付け
- Guest / Ephemeral Session
- Multi-user foundation

0.3側でIdentityそのものを再実装しない。

---

### Settings

0.3依存度: **High**

必要事項:

- User scope
- Profile scope
- App scope
- Schema migration
- Secure settings

---

### IPC

0.3依存度: **High**

必要事項:

- Identity-aware request
- Permission-aware data flow
- AI service communication
- Memory service communication

---

### Model Runtime

0.3依存度: **Very High**

必要事項:

- Model lifecycle
- Model selection
- Inference abstraction
- Runtime isolation
- Model metadata

ただしMemoryはModel Runtimeから独立させる。

---

### Permissions / Capability

0.3依存度: **Very High**

必要事項:

- App → Memory access
- App → Context request
- User consent
- Sensitive data boundary

---

### Jobs

0.3依存度: **Medium / High**

将来用途:

- Memory compaction
- Index rebuild
- Preference weighting
- Backup
- Migration
- Sync

---

### Storage / VFS

0.3依存度: **Very High**

必要事項:

- Profile data storage
- Transaction
- Integrity
- Migration
- Backup
- Encryption boundary

---

### Search

0.3依存度: **High**

Memory Retrievalに既存Search基盤を再利用できる可能性が高い。

ただし一般ファイル検索とMemory Retrievalの責務を完全に同一化するかは0.3詳細設計時に決定する。

---

### Wayback / Activity Ledger

0.3依存度: **Medium / High**

候補用途:

- AI action provenance
- Memory creation provenance
- Undo
- User-visible explanation
- Audit correlation

---

## 23. Designs to Avoid in 0.2

以下は0.3の手戻りを増大させるため避ける。

### Avoid-01

```text
global CURRENT_USER
```

のみで全サービスが動作する構造。

### Avoid-02

Model Runtime DB内にProfile全体を保存する。

### Avoid-03

AI Memoryを単なるChat Historyとして実装する。

### Avoid-04

全Memoryを毎回Promptへ投入する。

### Avoid-05

SettingsをSystem-wide key/valueのみで固定する。

### Avoid-06

ApplicationがMemory DBへ直接アクセスする。

### Avoid-07

Model名をMemory schemaにハードコードする。

```text
granite_memory
qwen_memory
elyza_memory
```

のような構造は避ける。

---

## 24. Proposed 0.3 Workstreams

以下は現時点の候補であり、0.2安定後に正式化する。

### PROFILE-01 — NAGI Profile Foundation

- Profile model
- Ownership
- Lifecycle
- Export / Import foundation

### MEMORY-01 — Unified Memory Store

- Common memory model
- CRUD
- Scope
- Metadata
- Permission hooks

### MEMORY-02 — Memory Retrieval

- Relevance
- Ranking
- Search integration
- Context selection

### LEARN-01 — Preference & Habit Learning

- Frequency
- Confidence
- Decay
- Explicit override

### CONTEXT-01 — Unified AI Context

- Context schema
- Context builder
- Permission filtering
- Size budget

### MODEL-ADAPTER-01 — Model Context Adapter

- Granite
- ELYZA
- Qwen
- Extensible adapter interface

### PROFILE-PORT-01 — Profile Backup / Migration

- Export
- Import
- Integrity
- Version migration

### TEAM-01 — Shared Profile Foundation

- Team Profile
- Shared memory
- Permission boundary

### AI-AUDIT-01 — AI Memory Audit

- Memory access audit
- Provenance
- User explanation

---

## 25. Proposed 0.3 Phases

### Phase 0 — Architecture Validation

開始条件:

- 0.2主要インターフェースが安定し始めている
- Identity / IPC / Settings / Model Runtime / Permissionの境界が見えている

成果物:

- Architecture review
- Data ownership map
- Dependency map
- Threat model
- Migration model

---

### Phase 1 — Profile + Memory Foundation

対象:

- PROFILE-01
- MEMORY-01
- CONTEXT-01

ここでModel RuntimeとMemoryの分離を確立する。

---

### Phase 2 — Retrieval + Learning

対象:

- MEMORY-02
- LEARN-01
- Search integration
- Weighting
- Decay

---

### Phase 3 — Model Portability

対象:

- MODEL-ADAPTER-01
- Granite Adapter
- ELYZA Adapter
- Qwen Adapter
- Model swap acceptance

---

### Phase 4 — Portability + Multi-User

対象:

- PROFILE-PORT-01
- Multi-user isolation
- Backup / Restore
- Migration

---

### Phase 5 — Team / Audit / Future Sync Boundary

対象:

- TEAM-01
- AI-AUDIT-01
- Sync adapter boundary

---

## 26. Gate for 0.3 Implementation Spec

本Architecture Roadmapから正式な

`Nagi_OS_0.3_Codex_Implementation_Spec.md`

へ移行する目安は、0.2が概ね **50〜70%程度安定**し、以下の条件が満たされた段階とする。

### Required

- IDENT-01のIdentity modelが安定
- Settings scope設計が確定
- IPC contractの主要部分が確定
- Model Runtime APIが安定
- Permission / Capability modelが利用可能
- Storage ownership / persistence方針が確定
- Jobs foundationが利用可能

### Strongly Preferred

- Search persistenceが安定
- Wayback / Activity Ledgerとの統合境界が見えている
- Multi-user前提の設計レビューが完了
- QEMU上で0.2主要サービスが動作

0.2の全Workstream PASSを待つ必要はない。

---

## 27. Implementation Spec作成時に決定する事項

本Roadmapでは固定しない。

- Memory Storeの物理DB
- SQLite / custom store / hybrid
- Embedding engine
- Embedding model
- Vector index形式
- Context serialization format
- Memory retention期間
- Weighting係数
- Exact schema
- IPC wire format
- Profile package format
- Encryption implementation
- Sync protocol
- Team Profile conflict resolution

これらは0.2の実装結果を踏まえて決定する。

---

## 28. Acceptance / Exit Criteria for 0.3

0.3完了時、最低限以下を成立させることを目標とする。

### Profile

- UserごとにNAGI Profileを保持できる
- ProfileをExport / Importできる

### Memory

- NAGIが長期Memoryを保存できる
- Memoryを検索できる
- Memoryを削除できる
- Memoryがユーザー間で混入しない

### Learning

- Preferenceを継続利用できる
- Habitを反復から学習できる
- 古いMemoryを減衰可能

### Model Portability

- 少なくとも2種類以上のLLMで同一Profileを利用できる
- モデル切替後もMemoryが維持される

### Context

- Permission評価後にContextを生成する
- Context budgetを制御できる

### Privacy

- Memory accessを監査できる
- Forget / Deleteを実行できる

### Reliability

- Profile schema migrationが可能
- Backup / Restoreが成立する

---

## 29. Example Acceptance Scenario

```text
1. User A がGraniteを使用
2. NAGIへPreferenceとHabitが蓄積
3. PCを再起動
4. Profileが復元
5. ModelをELYZAへ変更
6. Unified AI Contextが同じProfileを利用
7. ELYZAでも過去のPreferenceを反映
8. User Aが特定Memoryを削除
9. 削除Memoryは以後Contextへ投入されない
10. ProfileをExport
11. 別NAGI端末へImport
12. Profile、Preference、Habitが継続する
```

この一連が成立すれば、0.3の中核思想が実現したと判断できる。

---

## 30. Open Questions

0.3 Implementation Spec作成時に検討する。

1. Memory DBを既存Search DBと統合するか
2. Embeddingを必須とするか、軽量全文検索を第一段階とするか
3. Memory compactionをいつ実行するか
4. AIによるMemory生成にユーザー確認を要求する範囲
5. Team ProfileのOwner model
6. Guest ProfileからPermanent Profileへの昇格
7. Profile Exportの暗号化方式
8. Memory schema versioning
9. Personalizationと人格設定の境界
10. Device固有Memoryとユーザー共通Memoryの分離
11. ELYZA等の日本語特化モデル向けAdapter最適化
12. 1B〜4Bクラスモデルでも十分なContext選択を可能にする方法
13. Raspberry Pi級環境でのMemory Retrieval性能
14. Nagi Account Syncを1.0以前に試験導入するか
15. AI Activity LedgerとMemory provenanceをどこまで統合するか

---

## 31. Immediate Actions for 0.2

0.3のコード実装を開始する必要はない。

0.2実装中は以下のみ確認する。

1. Identityが単一ユーザーに固定されていないか
2. Settingsがscope拡張可能か
3. IPCがIdentity情報を運べるか
4. PermissionsがContext構築前に評価可能か
5. Model RuntimeがUser Memoryを所有していないか
6. StorageへOwner metadataを追加可能か
7. JobsがProfile単位の処理へ拡張可能か
8. Searchが将来Memory Retrievalから利用可能か

問題が見つかった場合のみ、0.2側へForward Compatibility修正を入れる。

---

## 32. Roadmap Policy

本書の目的は0.3の詳細実装を今すぐ固定することではない。

目的は、

> **0.2の実装によって0.3の可能性を閉じないこと**

である。

0.2の実装結果を尊重し、0.3 Implementation Specは実際のAPI・Storage・Identity・IPC・Permission構造が安定してから作成する。

したがって、本書に記載されたWorkstream、schema、API候補は、Architecture Intentとして扱い、現時点では実装契約とはみなさない。

---

## 33. Vision

NAGI OS 0.3で目指すのは、単にLLMをOSへ組み込むことではない。

モデルは交換できる。

ハードウェアも交換できる。

PCも交換できる。

しかし、

**ユーザーとNAGIの間に蓄積された記憶、設定、習慣、文脈は継続する。**

それがNAGIにおけるPersonal Agentの基盤となる。

```text
Hardware changes.
Models change.
Applications change.

NAGI Profile remains.
```

---

**End of Document**
