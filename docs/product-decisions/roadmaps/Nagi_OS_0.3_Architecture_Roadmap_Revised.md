# NAGI OS 0.3 Architecture Roadmap

> **2026-11-16 近距離ファイル共有:** [DEC-064](../DEC-064-nagi-share-nearby-file-transfer.md)で、Nagi Share（仮称）の純正OS機能としての採用を確定した。Local／Offline-firstの単発ファイル直接転送を扱い、Internet・Cloud account・LLMを必須にしない。DEC-014のWorkspace移行・同期と既存Continuity構想は別範囲であり、本機能のHandoff拡張はアイデア段階。技術・UI詳細・対応版／端末・性能は未確定で、0.3の必須機能や実装済みを意味しない。既存の認可・情報区分・所有／Profile境界とGateを維持する。詳細は[ロードマップ索引](README.md#近距離ファイル共有の製品方針2026-11-16)を参照。

> **2026-11-15 公開移管:** [DEC-050](../DEC-050-public-specification-repository.md)により、この文書は非公開リポジトリから公開リポジトリ `Roy-To/NagiOS` へ移した。本文中の「Privateに保存する」「Publicへ転載しない」「非公開の開発範囲」などの記述は移管前のもので、DEC-050が優先する。

## Linux基盤への適用注記（2026-11-13）

[DEC-042](../DEC-042-linux-kernel-transition.md)のLinux kernel選択、[DEC-043](../DEC-043-nagi-userland-independent-os.md)のNagi専用userland／既存distribution基盤非採用／個別OSSの選択的再利用はCONFIRMED。[DEC-044](../DEC-044-linux-migration-development-authorization.md)と[移行ADR](../../decisions/2026-11-12-linux-migration-development-scope.md)はPrivateでの対象限定の移行設計・実装・検証を承認済みである。これは製品の恒久構成・正式切替やrelease activationと別の承認であり、具体的な実開始・統合成功は対象refと証拠で確認する。

割当済みの移行開発では、本文に残る旧独自kernel完成・旧M30順序を一律の着手前提としない。既存release Gate・Workstream owner/path・安全境界・SSD/Git等の個別制限は維持する。具体的Linux構成・互換方式・package・適用版・切替日を本改訂で確定しない。開発用の可逆的選択はDEC-044の委任内で記録・検証でき、全体設計の完成を新たな一律開始Gateにしない。

[仕様・実装影響表](../KERNEL-TRANSITION-ALIGNMENT-2026-11-13.md)と[Private 0.2補足](../../Nagi_OS_0.2_Linux_Migration_Addendum.md)を併読する。Private内容の公開・配布は別承認を要する。

Profile／model-independent Canonical Memory、Context/Event、型付きPublic Action、offline、Permission、単一Personal Agentの方向を維持する。§26のImplementation Spec作成条件、§28の受入、§34のrelease/activation境界を変更しない。§34.1／34.8等の旧M30順序文は上記の限定開発と区別する。正式Memory Engine（DEC-013）をLinux移行だけで0.5へ延期しない。

元のSource Baseline記述は取込時点の由来であり、本日付の注記を含む現Private改訂全体がPublicとbyte-identicalという意味ではない。元の構想本文は保持する。


**Project:** NAGI — Native Agentic General Interface  
**Document Type:** Architecture Roadmap  
**Target Version:** 0.3  
**Status:** Draft / Forward Architecture  
**Purpose:** 0.2実装中に、0.3で必要となる将来互換性と設計境界を明確化する  
**Implementation Status:** 本文書自体は実装開始を意味しない  
**Revision:** Vision addendum — 2026-10-25; Memory Engine design status — 2026-11-04
**Source Baseline:** Read-only `sources/` copy, verified byte-identical to `NagiOS/docs/` at revision time

---

## 製品優先順位の確認（2026-11-16）

[DEC-074](../DEC-074-human-led-agent-organization.md)の後続確認により、当面はAIがOS標準機能として自然に働くAI組み込み体験型OSの確立を優先する。階層型Agent Organizationは非優先の長期ビジョンとする。単一Personal Agent、Profile／Memory、正規のPublic Actionという本書の方向を維持し、複数Supervisor・子Agent組織を0.3へ追加しない。具体的な提供版・実装順序は未確定であり、§26、§28、§34の既存Gate・Acceptanceを変更しない。

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
- Memoryの論理層数・構造（4層構造を含む）
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

## 34. Vision Revision — Personal Agent and Multi-Agent Evolution

### 34.1 適用範囲と優先順位

本節は、既存のGoals、Workstreams、Phases、Gate、Acceptance Criteria、Open Questionsを補足する。0.3のPersonal Agentにローカル音声とOS Actionによる業務実行を加えつつ、既存の0.1 / 0.2 / 0.3の順序と0.3 Implementation Spec作成gateは変更しない。適用されるactivation gateを通過する前の0.2 / 0.3 runtime実装を許可するものでもない。

アプリ操作は型付きPublic Action / service APIを優先する。正規連携に未対応のアプリで明示許可の下に限定GUI／Accessibility fallbackを使う条件は[DEC-027](../DEC-027-public-action-and-scoped-gui-fallback.md)に従う。この補足は0.3への実装割当てや既存Implementation Spec／activation gateを変更しない。

- **0.1を最優先:** 0.1 M30 PASSまでは0.1が権威ある基準であり、0.2は明示的なactivation後に進める。
- **0.2は汎用Platform Foundation:** AIが無効または利用不能でもOSは使え、Agent用の特権経路を持ち込まない。
- **0.3はPersonal Agent:** ローカルで自然に対話し、ユーザー所有のProfileを保ち、正規のPublic Actionで業務を遂行する単一の秘書Agent「Nagi」を成立させる。
- **0.4以降はMulti-Agent Organizationの将来案:** Nagiが開発・実行担当（Builderは仮称、Agent／Role区分はDEC-024により要再検討）、Research、Review等を調整する構想は別紙 `Nagi_OS_0.4+_Multi-Agent_Organization_Roadmap.md` に分ける。別途activationが必要である。

既存の0.3 gateは維持する。Identity、Settings scope、IPC、Model Runtime API、Permission / Capability、Storage ownership、Jobsが安定した段階でImplementation Specを検討する。0.2が概ね50〜70%安定した段階という既存の目安を用い、全WorkstreamのPASSは要求しない。Host-only PASSをTarget RuntimeのPASSへ読み替えない。

### 34.2 0.3 Personal Agentの実行境界

ユーザーとの対話窓口は秘書Agent Nagiひとつとする。Nagiは依頼を理解し、計画を作成・説明し、決定的なPolicyに従って承認を求め、他のPlatform Clientと同じ型付きPublic Action / service APIを通じてアプリ横断の操作を行う。

```text
Local Voice / Text Interface
        |
        v
Personal Agent Runtime (non-privileged)
        |
        v
Planner -> deterministic Action validation
        |
        +---- Permission / Capability evaluation
        +---- policyで必要な場合のユーザー承認
        |
        v
Public Action / service API -> application or system provider
        |
        +---- Activity / Transaction / Checkpoint / Wayback
```

Agent RuntimeもModel Runtimeも非特権とする。モデルはAction候補と型付き引数を提案できるが、呼出元、Session、対象、現在のRevision、Capability、PermissionはOSの決定的なPolicyが実行時に評価する。Agentとして呼び出されたこと、Nagi製であること、同じWorkspaceに属すること、特定Modelを使うことだけで権限は生じない。Raw Kernel API、非公開Service経路、Memory Storeへの直接接続、画面の座標クリックをPublic Actionの代替にしない。

0.3では、単一Personal Agentが複数アプリにまたがるActionを順に実行できる。自律的な子Agentの生成やAgent組織の運用は行わない。業務を実行できるPersonal Agentを0.3で成立させながら、階層構造は後続世代へ分離する。

### 34.3 Identity、Role、Task、将来の委譲

実行を追跡する候補IDを以下に示す。型、発行者、永続化、wire formatはImplementation Specで決める。

| ID | 0.3での用途 | 0.4以降の用途 |
|---|---|---|
| `AgentId` | 単一Personal Agent定義の安定ID | 各専門Agentの安定ID |
| `AgentSessionId` | 音声 / Text / Task実行ごとのSession | Coordinator / Workerごとの独立Session |
| `RoleId` | `Secretary` / `PersonalAssistant`という役割ラベル。Permissionではない | `Build`、`Research`、`Review`、`Verify`等 |
| `TaskId` | 依頼、計画、Action、状態、結果の相関 | Root Task、委譲されたSubtask、依存関係、期限、証拠 |
| `DelegationId` | 任意の系譜metadataのみ。子Agent実行には使わない | 範囲・期限付きで取消可能な親子間の権限委譲 |
| `OrgId` | 任意の予約metadataのみ。組織としては機能しない | 明示されたAgent Teamのnamespaceと運用境界 |

0.3では`ParentTaskId`、`DelegationId`、`OrgId`は省略または不活性metadataでよく、存在しても子Agent実行を有効にしてはならない。認証済みUser / SessionがPrincipalであり、Agentの系譜を監査用に追加する。Roleは役割を表すだけでCapabilityではない。

0.4以降では、子Agentが使える権限を「親が現在委譲可能な権限」「ユーザーが明示的に承認した範囲」「Task / Resource範囲」「許可されたAction種別」「時間制限」「OSの現在Policy」の共通部分に制限する。再委譲は既定で禁止し、明示的に許す場合も範囲を狭め、全系譜を記録する。OS Providerは各Actionの実行時にPermissionを再評価する。

### 34.4 Model方針とHardwareに応じた拡張

ユーザーが示した次の二つのModel構成を、どちらかに固定せず両方扱えるようにする。

1. **単一のローカルModelを複数Role / Task設定で使う:** 低性能端末や逐次作業に適し、Model重複を抑えやすい。一方で応答時間が延びることがあり、同じModelの自己レビューだけでは独立検証にならない。
2. **作業別に異なるローカルModelを使う:** 日本語、Coding品質、Context長、応答時間、メモリ使用量などで選択できる。専門性の向上を期待できるが、モデル保管とSchedulerの負荷が増える。

0.3はModelに依存しないProfile / Memory / Contextと、既存のModel portability受入条件を満たす。複数Agentの同時実行は要求しない。0.4以降では対応環境に限り不変のModel weightsを共有してもよいが、推論Session、Prompt、KV Cache、取消状態、Budget、Permissionは分離する。Model選択は品質と資源消費を変えるだけで、権限を変えない。

Hardware能力は利用可能なModel profileや並行数を制御し、Security policyを変えない。小型端末では一体のAgentが逐次実行し、標準端末ではModelを順次共有または追加Modelをロードし、高性能端末ではBudgetを定めた並列作業を可能にする案を検討する。区分は基準Hardwareで測定してから確定する。

将来の各Agent Taskに、Wall-clock、CPU / GPU時間、RAM / VRAM、Context / Token量、子Task数、保留Action数、Retry数、成果物容量の上限を持たせる。負荷時はShell、Foreground App、音声連続性をBackground Agentより優先する。Jobは待機、逐次化、速度制限、一時停止、取消、Checkpoint、Model unloadができる。Budget超過は成功扱いにせず、状態として報告する。

### 34.5 Memory scopeとContext構築

| Scope | 内容 | 既定のアクセス範囲 |
|---|---|---|
| User / Profile private | 個人の事実、嗜好、習慣 | Ownerが許可したProfile Context |
| Team shared | 明示共有された知識、手順 | 許可されたTeamメンバーのみ |
| Agent / Role scoped | Agent設定、役割ごとの作業知識 | Policyが許す当該Agent / Role |
| Task working | 現在の計画、作業メモ、中間成果物 | 現Task内。保持期間は別途制御 |
| Organization shared | 明示的な組織Policy、手順 | 明示した所属・Grantのみ。0.3後の候補 |

既存の0.3 `Team Profile / Shared Profile Foundation` は、データ所有者とPermissionの境界として維持する。Agent組織とはみなさない。Personal Memoryは既定でTeam / Organization Memoryへ流さない。Workerの成果はprovenance付きの提案とし、User / Team / Organizationの長期Memoryへ昇格するには明示的なPolicyを要し、設定に応じてユーザー確認を求める。

Contextを組み立てる前にPrincipal、Purpose、Task、Resource、Permissionによって絞り込む。AgentとAppはMemory APIを使い、DBへ直接接続しない。子Agentに秘書Nagiの会話全文や全Memoryを渡さず、Taskに必要なContextだけ渡す。Model weightsやKV Cacheを永続Memoryの正本にしない。

### 34.6 ローカル自然音声とフォールバック

対応するLocal providerとHardwareがある場合、0.3はローカル音声認識、ローカルModel Runtime、ローカル音声合成の一連の音声経路を実現する。対応Offline経路にNetwork接続は要求しない。音声収録にはOS Permissionを要求する。Wake Wordや常時待受を本Roadmapで確定しない。

長い処理中も対話を続けられるよう、依頼を受けたことを短く伝え、計画・実行状況を表示し、完了時に音声で結果を返す。ユーザーは音声を割り込み、TaskをPause / Cancelできる。Model、ASR、TTS、Microphone、Speakerの利用可否を個別に表示する。いずれかが利用できない場合は理由を説明し、Task状態を失わずに使えるローカルText / UIへ切り替える。Cloud音声やModelを将来提供する場合は、明示的なOpt-inとデータ開示を要する。

応答時間の数値目標、ASR / TTS Model、録音音声の保持期間、Wake Wordの扱い、Hardware区分は、測定とレビュー後に決める。

### 34.7 Task Log、Activity Ledger、Wayback、失敗処理

業務運用状態のTask Logとユーザー向けの永続Activityを分ける。

- **Task Log:** 再開・調査に必要な状態、依存関係、Budget、Retry、Error、取消、証拠参照を記録する。アクセス制御し、保持・削除・秘匿方針を定める。
- **Activity Ledger:** 意味のあるAction、Actor、承認、Permission判定、provenance、結果をユーザーが後から確認できる形で記録する。
- **Wayback:** 対応ResourceのCheckpointや実在する逆操作を扱う。外部送信済みのメッセージなど不可逆の副作用にUndoを約束しない。

利用可能な場合、`UserId`、`ProfileId`、`SessionId`、`AppId`、`AgentId`、`AgentSessionId`、`RoleId`、`TaskId`、`ParentTaskId`、`DelegationId`、`OrgId`、`ActionId`、`ResourceId`、`RevisionId`、`ActivityId`、`TransactionId`、承認、Model provider / version、結果状態を相関する。Prompt全文を永久保存する前提にせず、明示Retention policyに基づいて最小限の秘匿済み証拠を保持する。

Retryは回数を制限し、証拠に応じて試行内容を変える。冪等性Keyを使い、不確かな非冪等Actionを盲目的に再実行しない。同じResourceを更新する処理はRevisionを検証し、古い成果で新しい編集を上書きせず、Reviewへ回す。Review / Verify Agentの自己申告だけをPASSの証拠としない。StopはInference、Jobs、子Task、待機Action、音声へ伝播する。部分的な副作用と結果不明の状態を明示する。

### 34.8 0.2向けForward Compatibility追加要件

以下は既存の汎用Platform contractを確認する条件であり、0.2でAgent製品機能を実装する許可ではない。

- 0.1 M30 PASSまで0.1を優先し、0.2 runtime activation前に明示的な境界とIntegration Owner checkpointを要求する。
- 0.2はAI無効・利用不能でも使える状態を維持し、Platform serviceをAgent専用化しない。
- Model Runtimeは非特権のまま、Memory、Identity、Permission、Action実行Policyから責務を分離する。
- Context / IPCに必要なUser、Profile、Session、Application、Caller、Request、Correlation情報を載せられるようにする。
- Activityの`AGENT` ActorとTask / Agent系譜metadataを拡張可能にし、Agent RegistryやOrg Serviceを0.2で要求しない。
- Actionを型付き・Versionedにし、入出力、Capability / Permission、side effect、可逆性、Revision、取消、失敗を必要に応じて表現する。
- Model Contextへ情報を入れる前とAction Providerでの実行時にPermissionを評価する。
- Settings / StorageのOwner scopeをUser、Profile、Application、Team、将来のAgent / Taskへ拡張可能にする。
- Profile単位のJob、取消、Resource制限を扱えるようにしても、0.2でMulti-Agent Schedulerを導入しない。
- OS音声入出力とMicrophone PermissionをModelから分離し、0.3でローカル音声に拡張できるようにする。
- Model固有の永続Memory、Agentによる権限発行、Private serviceへのShortcutを作らない。

Host foundationとTarget Runtime PASSを引き続き区別する。既存の0.3 Implementation Spec gateは変更しない。

### 34.9 追加Workstreamsと依存関係

以下の候補をSection 24の0.3 Workstreamsへ追加する。

| Workstream | 成果 | 主な依存先 |
|---|---|---|
| `AGENT-01` — Single Personal Agent Runtime | Agent / Session / Task lifecycle、Planner、Validator、説明、Pause / Cancel | Identity、IPC、Model Runtime、Capability / Permission、Jobs |
| `ACTION-ORCH-01` — Public Action Orchestration | 型付きクロスアプリActionの計画、Policy評価、Revision検証 | Public Action API、Service IPC、App provider、Activity / Wayback |
| `VOICE-01` — Offline-First Local Voice | Local ASR / TTS境界、streaming、割込み、fallback | 音声Device、Microphone Permission、Model Runtime、localization |
| `TASK-LEDGER-01` — Task / Activity Correlation | 状態、系譜、承認、Retry、証拠、Checkpoint参照 | Jobs、Activity / Transaction、Wayback、diagnostics |

すべて0.3 Implementation Specが正式に作成可能になるまで候補とする。既存`TEAM-01`はTeam ProfileとShared Memory境界を指し、自律Agentを含めない。

### 34.10 追加PhaseとMilestone順

既存のPhase 0–3はそのまま維持する。Model Portability後、既存Phase 4（Portability + Multi-User）前に、**Phase 3A**を挿入する。

#### Phase 3A — Personal Agent / Offline Voice / Public Actions

**依存:** 0.3 Profile、Unified Context、Permission-filtered Memory。0.2 Public Action、IPC、Capability、Jobs、Activity / Wayback contractの安定。既存0.3 Implementation Spec gateの通過。

**Deliver:** Profileに紐づくNagi Agent、計画と説明、型付きPublic Action実行、Permission / Approval、ローカル音声とfallback、取消、Activity / Wayback相関。

**Exit Evidence:** 承認済みReference targetでOfflineクロスアプリシナリオが成立する。拒否されたActionは実行されない。Stopが保留処理に伝わる。部分成功・不可逆な結果を正しく表示する。Model / 音声の障害時も状態を失わずfallbackし、Activityから実行内容を再構成できる。

その後、既存Phase 4（Profile Portability + Multi-User）、Phase 5（Team / Audit / Future Sync Boundary）の順に進む。Organization機能は0.3 Acceptanceと別activationの後に着手する。

### 34.11 0.3 Acceptance追加条件

以下をSection 28の受入条件へ追加する。

**Agent / Action**

- Active Personal ProfileにAgentIdが紐づき、各Conversation / TaskにAgentSessionId / TaskIdがある。
- Nagiは型付きAction計画を説明し、通常のPublic Action / service APIからのみ実行する。
- 誤ったUser / Profile、期限切れSession、取消済Permission、古いRevision、不正Payload、Unavailable providerはfail closedとなる。
- Role、Model、Agent Identity、Nagiから呼ばれた事実でCapabilityを付与しない。
- UserのCancelで未実行Actionと実行中Job / Inferenceが止まり、完了済み副作用は正しく報告される。

**Offline音声 / Fallback**

- Network切断時、対応するLocal hardware / model profileで、会話と短いTaskを完了できる。
- ASR、TTS、Model Runtime、Microphone、Speakerのいずれかが利用できない時は状態を説明し、利用可能なローカルText / UI経路へ切り替えられる。
- Inference中もUIが応答し、進捗を示し、割込み / Cancelが可能。
- Microphone利用はOS Permissionで管理され、常時収音を前提にしない。

**Memory / Audit**

- 拒否対象またはScope外のMemoryはContextに含まれず、削除したMemoryは以後のContextに含まれない。
- 再起動後もAgent / Task / Action / Permission / Approval / outcomeの系譜を確認できる。
- Wayback成功はProviderのRestore成功後のみ表示し、不可逆操作を復元済みと誤表示しない。

**信頼性 / Hardware**

- Model変更後もProfile、Memory、Preference、Task metadataを保持する。
- Retryは有限で、非冪等Actionの重複実行を防ぐ。
- 古いConcurrent resultを拒否するか、Reviewへ提示する。
- 対応Hardwareごとに応答性とResource利用を測定する。数値thresholdはImplementation Specで決める。

### 34.12 Acceptanceシナリオ追加

```text
1. 0.3の開始条件と対象Runtimeが成立していることを確認する。
2. Networkを切断し、ユーザーがローカル音声で2つのAppをまたぐ短い作業を依頼する。
3. Nagiが計画、対象、Action、必要なApprovalを説明する。
4. ユーザーが限定範囲を承認し、Nagiが通常の型付きPublic Action APIを使う。
5. 一つのActionを拒否し、Providerで拒否されたことがLedgerに残る。
6. 実行中にUserがStopし、Inferenceと待機Actionが取消される。
7. 部分成果と結果不明の副作用を区別して報告する。
8. Local音声Providerを止め、Task状態を失わずText / UIへfallbackする。
9. 対応Local Modelを切り替えてもProfile / MemoryとPermissionが維持される。
10. Activity / Waybackを確認し、実際に戻せる変更だけをUndo可能と示す。
```

### 34.13 Implementation Specまで保留する事項

1. Local ASR / TTS Model、Provider API、対応言語、品質指標。
2. Push-to-talk / Wake Word / 常時待受、録音音声の保持と削除。
3. 音声Acknowledgement / 割込みUX、数値Latency目標。
4. Compact / Standard / High-capacityのHardware区分と並列実行測定。
5. Task種別ごとの単一Model Role切替と異種Model routingの使い分け。
6. Action risk tier、必須Approval一覧、事前承認できる上限。
7. IDの型、発行者、永続化、IPC wire format、migration。
8. Task Log / Prompt / Artifactの保持、秘匿、Export、削除。
9. Retry上限、Idempotency、外部Actionの結果不明時、競合時のUX。
10. Team / Organization所有者、Offboarding、Shared Memory競合、Multi-device範囲。
11. 0.4+のVersion、activation時期、次世代へ昇格する機能。

### 34.14 ファイル保存とChatGPT Memory

本RoadmapはTask workspaceに保存したMarkdownファイルである。ファイル保存とChatGPTの長期Memoryへの保存は別機能・別操作であり、この作業ではChatGPT Memoryを更新しない。

---

**End of Revised Document**

## PDF Reader / Editorの製品方針（2026-11-16）

[DEC-061](../DEC-061-first-party-pdf-reader-editor.md)で、Nagi純正PDF Reader兼Editorの採用と段階的拡張をCONFIRMEDとして記録した。高速表示・検索・ページ操作・注釈・印刷・フォーム入力などを先に整え、その後、既存本文／画像の高度編集・OCR・元内容の削除と検証を伴う墨消し・署名関連機能を段階的に検討する。通常閲覧・編集はAI非必須とし、Nagi Project / Smart Paste / Writer / Sheets / Slides等と共通Public APIで連携する。AIによる表抽出・要約・比較・引用は利用者の承認と元資料の権限・機密分類の範囲に限る。

具体的なリリース版・高度機能の順序・技術・合格値はPROVISIONAL。本ロードマップの対象版への割当て、Acrobat完全互換、全機能初期実装、実装済み・Acceptance通過を意味せず、既存Gateは変更しない。
