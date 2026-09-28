# Nagi OS 0.2 — Model Runtime Foundation Workstream

## 1. Workstream

- Proposed ID: `MODEL-RT-01`
- Name: `Nagi 0.2 Model Runtime Foundation`
- Proposed branch: `codex/0.2-model-runtime`
- Execution branch: `codex/ws-model-runtime` (existing workstream continuation;
  no new Model Runtime branch was created)
- Target: Nagi OS 0.2
- Type: Independent runtime foundation
- Priority: High
- Final target state: `PASS`

同等workstreamが存在する場合は新規作成せず継続する。

Base SHAは推測しない。

---

## 2. Purpose

Nagiを特定AIモデル、特定vendor、特定inference engineへ固定しないModel Runtime基盤を構築する。

初期model候補:

- Qwen3 4B
- IBM Granite 4.2 3B
- Google Gemma 3 1B

初期default候補:

- IBM Granite 4.2 3B

さらに将来、

- jev
- System One Model
- nontraditional model runtime

等を追加できるarchitectureとする。

本workstreamの目的は**runtime foundation / abstraction**であり、大容量model weightsの実運用統合は必須ではない。

---

## 3. Core Principles

### 3.1 Model-independent Nagi

applicationやOS higher layerが直接、

- Qwen API
- Granite API
- Gemma API
- llama.cpp type
- vendor tokenizer
- vendor tensor type

へ依存しない。

### 3.2 Offline-capable

Nagiのcore AI architectureはcloud必須にしない。

local runtimeを第一級として扱う。

### 3.3 No Model Weights in Git

以下をcommitしない。

- GGUF
- safetensors
- model weights
- large tokenizer assets
- generated caches

testsにはmock / descriptor / fixtureを使う。

### 3.4 Future System One Compatibility

jev固有codeをNagi全体へ拡散しない。

runtime abstractionで、

- conventional generative LLM
- Lite model
- System One
- future alternative runtime

を表現できるようにする。

jevがなくてもNagiは動作すること。

---

## 4. Relationship to Nagi 0.1

active M18以降を壊さない。

以下をModel Runtime都合で変更しない。

- kernel
- boot
- Servo
- active milestone internals

必要なruntime serviceが未完成なら、

- API
- adapter
- mock backend
- host implementation

で先行する。

---

## 5. Model Descriptor

metadataとして最低限表現可能にする。

- stable ID
- display name
- family
- revision/version
- role
- provider
- runtime class
- capabilities
- resource class
- local/remote
- availability
- license/NOTICE reference

---

## 6. Runtime Provider / Backend

provider-neutral interface/traitを実装する。

必要機能:

- discover/enumerate
- validate
- load
- unload
- invoke
- streaming
- cancellation
- health/status
- resource reporting

method nameはrepo conventionへ合わせる。

---

## 7. Model Session / Handle

configured modelとactive invocation/sessionを分離する。

global mutable singletonへ安易に依存しない。

lifecycleを明確化する。

---

## 8. Request / Response Contract

provider-neutral request/resultを実装する。

対応を想定:

- text input
- system/context input
- generation options
- cancellation
- streaming output
- structured errors

provider-specific tensor/tokenizer typeをapplicationへ漏らさない。

---

## 9. Capabilities

例:

- text generation
- streaming
- structured output
- tool invocation
- embeddings
- multimodal

全modelが全capabilityを持つ前提にしない。

---

## 10. Resource Policy

最低限:

- Lite / Standard role
- memory/resource class
- concurrent session policy
- load/unload state
- cancellation
- availability
- fallback

machine-specific memory量を根拠なくhardcodeしない。

---

## 11. Initial Model Roles

### Standard A

Qwen3 4B

### Standard B / Default Candidate

IBM Granite 4.2 3B

### Lite

Google Gemma 3 1B

modelが、

- absent
- incompatible
- disabled
- unavailable
- load failed

でもNagiがcrashしないこと。

---

## 12. Selection Policy

既存仕様がなければ、概念上以下を基本とする。

1. explicit user/admin selection
2. configured compatible default
3. available Standard
4. available Lite
5. unavailable/no-model state

preferred modelが使えない場合、deterministicにfallbackする。

---

## 13. System One / jev Extension Point

future runtime classとして、

- SystemOne
- continuous model
- alternative reasoning runtime
- event-driven runtime

等を追加できるようにする。

jev自体は実装しない。

dummyの実AI実装を作って「対応済み」としない。

---

## 14. Error Model

typed errorを可能な限り採用する。

例:

- ModelUnavailable
- ModelNotInstalled
- RuntimeIncompatible
- LoadFailed
- InferenceFailed
- Cancelled
- Timeout
- InsufficientResources
- UnsupportedCapability
- InvalidConfiguration

machine-readable errorとlocalized UI messageを分離する。

---

## 15. Cancellation / Lifecycle

cancellationをfirst-classにする。

test:

- pre-cancel
- streaming中cancel
- repeated load/unload
- failed load
- unavailable backend
- invalid transition

---

## 16. Mock Backend

deterministicなtest backendを作る。

検証:

- discovery
- selection
- load
- invoke
- streaming
- cancellation
- error
- unload
- health

mockをreal AIとして扱わない。

---

## 17. Licensing / NOTICE

model固有の、

- license
- attribution
- NOTICE
- source/reference

をruntime codeと分離して表現できること。

Gemma等についてNOTICE組込み余地を維持する。

---

## 18. Non-scope

- production model weight download
- weight commit
- complete inference engine
- GPU kernel optimization
- kernel scheduler rewrite
- M18 modification
- complete AI UX
- cloud account system
- jev implementation
- mandatory network dependency

---

## 19. Implementation Sequence

### Phase A — Audit

確認:

- existing AI specs
- model modules
- config
- Capability boundaries
- service abstractions
- error conventions
- DF-01 ownership

調査だけで終了しない。

### Phase B — Core Types

実装:

- model IDs
- descriptors
- runtime classes
- capability sets
- roles
- errors
- lifecycle states

### Phase C — Runtime Abstraction

provider-neutral interfaceを実装。

### Phase D — Registry / Selection

実装:

- registration
- availability
- default
- fallback
- deterministic failure

### Phase E — Mock Backend

host testsを実装。

### Phase F — Future Runtime Extension

System One型backendがAPI redesignなしで追加可能か検証。

### Phase G — Verify / State / Git

- test
- fix
- retest
- format
- lint
- state
- commit
- push

---

## 20. Acceptance Criteria

- [ ] provider-neutral descriptor
- [ ] runtime/provider abstraction
- [ ] lifecycle
- [ ] typed errors
- [ ] capability model
- [ ] Standard/Lite roles
- [ ] Qwen/Granite/Gemmaをvendor branchingなしで表現
- [ ] Graniteをdefault candidateとして設定可能
- [ ] unavailable default fallback
- [ ] mock backend
- [ ] mock inference tests
- [ ] streaming tests
- [ ] cancellation tests
- [ ] load/unload tests
- [ ] failure-path tests
- [ ] System One extension point
- [ ] no model weights committed
- [ ] no mandatory network dependency
- [ ] format/lint PASS
- [ ] docs updated
- [ ] state updated
- [ ] committed
- [ ] pushed
- [ ] clean tree

production inference自体は、他runtime workstream依存ならfoundation PASS必須条件にしない。

---

## 21. Completion Report

- Status
- Workstream ID
- Branch
- Base SHA
- HEAD SHA
- Architecture
- Mock backend
- Tests
- CI
- Production integration boundary
- System One extension
- Blockers
- Push status
