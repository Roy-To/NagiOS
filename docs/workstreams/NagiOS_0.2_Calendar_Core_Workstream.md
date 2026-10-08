# Nagi OS 0.2 — Calendar Scheduling Core Host Foundation
**Document ID:** NAGI-0.2-PARALLEL-CALENDAR-20261008\
**担当:** Codex ②\
**対象:** `RT-NISH/NagiOS`\
**作業区分:** Phase 2 Calendarの **Host-only / Provider-independent** 予定管理基盤\
**対象仕様:** `docs/NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md` §61、0.2のPhase 2並行方針\
**実行状態:** BP-SBOM-HOST-20261008 / cd871f9で登録・host-only開始承認済み。C1–C5実装済み。検証・CIの正確な結果はown State参照。

> **目標：** Nagi Calendarの共通Event/Recurrence/TimeZone/Availabilityモデルを、クラウド接続・GUI・LLMに依存しないRustライブラリとして整備する。Microsoft Graph、外部invite、Nagi runtimeへの接続は今回行わない。

## 1. 正規仕様と起動手順

開始時に必読：

1. `AGENTS.md` とパスローカル指示。
2. `docs/Nagi_OS_0.1_Codex_Implementation_Spec.md` と `docs/implementation_status.md`。
3. ユーザー提供 `Nagi_OS_0.2_Codex_Implementation_Spec.md` のM17/M21/M22、Release line gate、Phase 2並行作業。
4. `docs/NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md` §61 Calendar（特にTimezone、Event、Availability、external policy）。
5. `.dev/workstreams.json` と `docs/0.2/WORKSTREAMS.md`、現行のSDK/Identity/Permission/Job関連contracts。
6. 既存 `calendar`/`recurrence`/`time` 型の有無。既存のaccepted public契約があれば再利用し、無断の別系統を作らない。

GitHubでの事実確認を先に行い、作業開始時のbase SHAを記録する。会話の過去の進捗メモだけを根拠にしない。

## 2. 独立Workstreamの提案

| 項目 | 提案 |
|---|---|
| Workstream ID | `calendar-core-01` |
| owner branch | `codex/0.2-calendar-core-01` |
| worktree | `../NagiOS-calendar-core-01` |
| ソース | `crates/nagi-calendar-core/**` |
| テスト | `tests/calendar-core/**`（承認時のみ） |
| 設計資料 | `docs/workstreams/NagiOS_0.2_Calendar_Core_Workstream.md` |
| State | `.dev/workstreams/calendar-core-01/state.json` |

新規ID・branch・パスは**未登録の提案名**。Integration Ownerにproposalを提出して、activation gateの明示承認後に限り上記の専有範囲を作成・変更する。登録と許可がない場合は仕様・契約・テスト計画等の許可済み準備だけに留める。

### 不可侵範囲

- `main` と Harkの `hark/**`、0.1 release acceptance。
- `kernel/**`、`loader/**`、`user/nagi-init/**`、`third_party/**`。
- 共有root `Cargo.toml` / `Cargo.lock`、`.github/workflows/**`、共有IDL/ABI、`.dev/workstreams.json`、他stream state。
- 実際のMail/Graph/ネットワークプロバイダ、System Job Scheduler/Notification runtime。
- 個人ホームの走査や対象repo外のファイル取得。

Standalone crateでビルド・テスト可能にし、必要な依存は固定。root workspaceへの登録はIntegration Ownerに提案するだけとする。

## 3. データモデル

最低限の型（名称は採用済みのNagi public contractsに合わせる）：

```text
CalendarId
EventId
EventRevision
Calendar
Event
  - title / description / location
  - organizer / participants[]
  - schedule: Timed(Instant or LocalTime + TimeZoneId)
            | FloatingLocalTime
            | AllDay(LocalDate range)
  - recurrence + exception/override
  - reminders[]
  - resource/workspace references[]
  - invitation_state (local draft only)
  - provenance/owner metadata (adapter boundary)
AvailabilityWindow
TimeZoneResolver (dependency injection)
Clock (dependency injection)
EventStore (in-memory reference interface)
```

### 3.1 時刻を曖昧にしない

- **Instant:** UTC上の一意の時点。
- **LocalDateTime + TimeZoneId:** 地域ルールを伴う壁時計時刻。
- **Floating time:** 地域を固定しない壁時計時刻（例：移動先の現地9:00）。
- **All-day:** UTC 00:00固定ではなく、LocalDateの半開区間で扱う。
- **TimeZoneResolver:** `ZoneId`からOffset/DST変換を提供する抽象化。host標準timezone/local設定への暗黙依存は禁止。
- DSTで存在しない時刻は黙ってずらさず、policy errorもしくは明示的なresolve-choiceが必要。
- DSTで2通り存在する時刻は、first/secondなど明示的なambiguity policyを要求。曖昧なまま勝手に予定を送信しない。
- `end <= start`、overflow、日付範囲上限、マイクロ秒精度の扱いを明文化する。

### 3.2 繰り返し

初期のbounded recurrence subset：

- `DAILY` / `WEEKLY` / `MONTHLY`、`INTERVAL`、`COUNT`または`UNTIL`、曜日指定の基本。
- `EXDATE`/単発例外、期間を指定した展開、上限件数。
- 繰り返し展開で無限ループしない。日付計算で月末(1/31→2月)のルールは仕様で明示。
- local-time-based recurrenceとinstant interval recurrenceを区別する。DSTをまたぐ毎朝9:00は意図した現地9:00を維持できる構造にする。
- 完全なRFC 5545の実装を本Host Foundationの完了条件にはしない。

### 3.3 Availability / reminder / draft

- タイムゾーンが異なるイベントを同じinstant axisへ変換し、busy intervalsを正規化。
- 指定期間・所要時間から候補slotを列挙する純粋関数。バッファ/営業時間/参加者条件は設定可能な入力として扱う。
- Reminderは相対時刻/絶対時刻を**定義だけ**可能。通知配信やJob実行は行わない。
- Event draftには状態を持たせる（Draft / Validated / RequiresConfirmation）。
- 予定のローカルCRUDと**外部invite送信・invite応答は別Action**。外部送信は今回未実装。外部送信成功の偽装禁止。

## 4. 実装段階

### C1 — Time/Calendar/Event基本モデル
- Stable CalendarId/EventId、イベント作成/取得/変更/削除のin-memory reference。
- EventRevisionによるstale update拒否。optional owner metadataを持ち込めるが、認証serviceを自作しない。
- Timed/Floating/All-dayと表示用情報を混ぜない。UTF-8日本語/英語文字列を検証。
- `created_at`等の時刻はClockを注入して再現可能にする。

### C2 — Timezone/edge-case
- 基本UTC変換、DST gap/foldの扱い、跨日/年越し、leap day。
- Timezone databaseを本体へ直接ベタ書きせず、fixture resolverで境界を先に完成。
- 実際のtimezone provider導入はライセンス・容量・オフライン提供方式の承認を得てから。

### C3 — Recurrence
- bounded展開と例外、日付範囲フィルタ。
- 週開始・月末規則・DSTの誤変換をテスト。

### C4 — Availability
- 複数予定の重なり統合、空き時間抽出、最小所要時間でのcandidate生成。
- 壊れたEventや無効Windowを静かに飲み込まない。

### C5 — Platform adapterの提案（実接続なし）
- `calendar.list` / `search` / `get_event` / `create_event` / `update_event` / `delete_event` / `find_availability` / `add_reminder` の型境界を記載。
- `calendar.invite`、`calendar.respond_invite` は `EXTERNAL` side effectとして分離。Runtime permissionとユーザー確認が必要と明記。
- Local stateのsnapshot serializerを追加する場合はschema version + migrations/validationを付け、in-memory snapshotとdurable guest storageを混同しない。
- Jobs/Notifications/People/Mail/Workspaceのconsumer adapterはtrait/mockに限定。

## 5. Host Acceptance

| ID | 内容 |
|---|---|
| CAL-H01 | 異なるCalendar/Eventのstable IDsが作成/更新をまたいで保たれる |
| CAL-H02 | Timed、Floating、All-dayの時刻意味論を混同しない |
| CAL-H03 | UTCと時差を跨ぐ変換を、注入Resolverで再現可能に確認 |
| CAL-H04 | DST nonexistent/ambiguous local timeを意図せず正規化しない |
| CAL-H05 | DAILY/WEEKLY/MONTHLY、COUNT/UNTIL/例外がboundedに展開する |
| CAL-H06 | 月末/年越し/うるう日/跨日イベントで期待通り振る舞う |
| CAL-H07 | 重複busy intervalsと複数イベントから正しいfree slotsを算出 |
| CAL-H08 | stale EventRevision/不正期間/overflow/多すぎるrecurrenceを拒否 |
| CAL-H09 | Clockを固定すれば同一fixtureが毎回同一結果になる |
| CAL-H10 | 外部inviteは実行せず、構造化draftと確認ポリシーを分離 |
| CAL-H11 | 日本語/英語内容、長い件名、empty fieldsでpanicしない |
| CAL-H12 | Test/Clippy/format・own State・commit/pushが証拠に紐づく |

本Host FoundationをPASSにしても最終`CAL-001`〜`CAL-011`のすべては満たさない。特に実UI、provider、offline durable cache、本物のSearch integration、招待配信、Nagi targetでの動作は別gate。

## 6. 検証コマンド例

```bash
git status --short --branch
./nagi dev status
./nagi dev resume
./nagi dev verify
cargo fmt --manifest-path crates/nagi-calendar-core/Cargo.toml --all -- --check
cargo test --manifest-path crates/nagi-calendar-core/Cargo.toml --offline
cargo clippy --manifest-path crates/nagi-calendar-core/Cargo.toml --all-targets --offline -- -D warnings
git diff --check
```

Manifestが存在することを先に確認。`--locked`を使える場合は追加。`cargo test`がparent workspace登録を要求する場合、共通rootを変更せず、独立manifestや隔離fixtureで解決する。UnsupportedなNagi guest targetのPASSを主張しない。

## 7. 実装→検証→Pushの継続ループ

1. まず承認済みbase SHA/owned paths/activationを確定。
2. C1→C2→C3→C4→C5の順に小さく実装し、各段階でnegative testsを含める。
3. テスト失敗は分類して修正、fmt/lint/focused testsを再実行。
4. `state.json`（登録・所有済みの場合）にexact SHA、テスト数、failure class、次アクションを記録。
5. 論理単位でcommit、own branchにpush、CIをSHA単位で確認して次のin-scope acceptanceへ。
6. 権限外のshared fileやM30 gateで詰まったら、提案と証拠を残し、承認済み範囲にほかの作業があれば続ける。
7. 最終的に **PASS（ホスト受入のみ）/ PARTIAL / BLOCKED** と未対応のguest/provider接続を分けて報告する。

**非目標：** Calendar GUIの完成、メール送信、外部invite、Microsoft Graph、CalDAV、実Notification、System Jobs、本番認証、クラウド同期、0.3 Agent実行、root workspace/CIの無断変更。

---

# Calendar Core host foundation — calendar-core-01

Status: host-only C1–C5 implemented; verification/CI evidence lives in
`.dev/workstreams/calendar-core-01/state.json`. This is not product Calendar PASS.

## Activation and ownership

Host-only activation is registered by Integration Owner checkpoint
BP-SBOM-HOST-20261008 at cd871f905bd06ef406ccaa9f80c484f19158209f.
The implementation began from 80ea6246dd394deb5158c44aa7f0012d76a9c409 after
explicit user approval; the owner's published registration/CI checkpoint then
superseded the provisional branch-local row. That official row is retained verbatim.
The final feature diff is against cd871f9 and contains only Calendar-owned paths.

Owned: `crates/nagi-calendar-core/**`, this document and
`.dev/workstreams/calendar-core-01/**`. Tests live inside the standalone crate.
The Calendar implementation changes no root Cargo/lock, shared CI/IDL/SDK, other stream State, 0.1 acceptance, Hark,
Kernel/Loader, guest service or personal Mac file was modified.
Guest/runtime/provider delivery requires Nagi 0.1 M30 PASS and a separate
Integration Owner checkpoint. Current main at
`9e06967640776298dfdcbceb55085eb9d2b717c5` records M30 PARTIAL.

## Adopted contracts

The standalone crate depends only on canonical `nagi-model` 0.1.0 pinned by Git revision
`cd871f905bd06ef406ccaa9f80c484f19158209f`, reusing AppId/UserId/ObjectId/WorkspaceId. It adds domain CalendarId
and CalendarEventId (EventId is a crate-local alias), not a global identity service.
Owner metadata never grants authority. Public constructors validate IDs; fixture
SequentialIds are deterministic, not persistent/global allocators.

RealtimeSource is the Calendar adapter seam for PAL's fallible realtime_ns;
conversion rejects sub-microsecond precision loss. It never reads host time.
JOB-01's virtual monotonic millisecond clock is not event UTC; reminders are data
only. IDENT-01/Capability mapping stays a future authenticated adapter; no new
PrincipalId, profile identity, authentication or permission evaluator is introduced.
SDK identity/intent/state/IPC and accepted action@1 caller binding are preserved.
LocalAdapter is a host interface, not a registered public runtime Action service.

## C1: local model and CRUD

Calendar/event IDs remain stable across local updates. Calendar deletion rejects
nonempty calendars. EventRevision is nonzero u64; expected-revision updates/deletes
reject stale callers, and revision overflow leaves records unchanged. All failed
record validations and clock errors preserve stored records. ID allocation may
consume a fixture ID on a failed call; no allocation transaction is promised.

Store capacity (1..65536) includes ID tombstones, bounding memory while preventing
ID reuse during that store lifetime. Search is bounded, deterministic literal
substring search of local text/participants, not a platform Search provider.
Clock rollback rejects updates; fixed clocks may produce identical timestamps
while revisions advance. No durable snapshots or serialization are claimed.

Text uses UTF-8: nonblank title up to 4096 bytes, description up to 65536, location
up to 4096, optional empty fields permitted, NUL rejected; collection caps are
validated. Contact labels are adapter inputs, not authenticated People identities.

## C2: explicit temporal domains

Instant is signed Unix microseconds without leap-second representation. Date is
proleptic Gregorian 1..9999; LocalDateTime validates wall time to microseconds.
Schedule distinguishes Instant, ZonedLocal, FloatingLocal and half-open AllDay.
Floating/AllDay require an explicit zone context to become busy intervals.
Local positive ranges also require a positive resolved instant interval.

TimeZoneResolver is injected. Gap always rejects. Fold requires Reject/Earlier/
Later at each endpoint; unique/fold outputs are checked for chronological order,
offset bounds and local/instant roundtrip consistency. Provider errors propagate.
FixedOffsetResolver is explicitly configured for one zone, not a timezone DB.
The NewYork test fixture covers only two 2026 transitions and is not a provider.
All-day midnight conversion produces 23/25-hour DST days when appropriate.
Wall and elapsed durations differ explicitly when local recurrences cross DST.
Stored drafts receive structural validation; timezone validity is checked at
resolution/expansion. DraftState does not claim delivery or authenticated authority.

## C3: bounded recurrence subset

Daily, Weekly, Monthly apply to Local/Floating/AllDay domains; InstantEvery applies
only to Instant. Interval must be positive. Bound is exactly one Count or inclusive
same-domain Until by construction. Count counts valid scheduled occurrences before
EXDATE/query filtering; exclusions never add replacement occurrences.

Weekly uses weekdays Monday=0..Sunday=6 and Monday week anchors. Monthly preserves
the original day and skips invalid months (January 31 → March 31), never clamps or
drifts. Local recurrence preserves wall start; InstantEvery uses elapsed intervals.
Both scanned candidates and emitted results have explicit limits (1..65536).
Skipped weekdays/months, EXDATEs and out-of-window events cannot create infinite
expansion. Limit exhaustion returns an error, never a complete partial list.

Exceptions use original domain recurrence keys. Duplicate keys are structurally
impossible in maps/sets; EXDATE/override conflicts, wrong domains and keys outside
the bounded series are rejected. Exception membership validation has an additional
fixed 65536-probe bound. Expansion resolves the entire bounded series and filters
by overlap, so overnight events and moved-in/out overrides are handled correctly.
This deliberately favors bounded correctness over large-series seek optimization.
Positive advancement beyond the representable domain finishes an in-domain UNTIL;
COUNT that needs unrepresentable occurrences still fails. Full RFC5545 is excluded.

## C4: availability

Pure functions resolve events onto one instant axis, validate all inputs (including
transparent events), buffer/clip/sort/union busy intervals, compute the complement,
intersect required participant working windows, and enumerate ascending slots on
a window-anchored grid. Unknown availability is an error; empty working windows
mean no work time. Durations/steps are positive; arithmetic, input counts, scans
and output counts are bounded. Adjacent half-open intervals share no busy time.
No remote participant lookup or scheduling service is invoked.

## C5: adapter boundary

Action v1 classifies local/read operations and exposes CRUD, draft, availability,
reminder and resource-link methods. `calendar.invite` and `calendar.respond_invite`
are EXTERNAL and require confirmation. ExternalDraft::execute always returns
ExternalActionUnavailable, even after confirmation; no Sent or successful delivery
value exists. No sender, Graph/CalDAV, Jobs or Notification executor is connected.
Runtime must authenticate the caller and apply the shared Validator/Policy/Permission
boundary before any real side effect. LocalAdapter implements none of that policy.

## Host verification and later gates

The focused suite maps CAL-H01..11 to stable IDs, temporal types, offsets, DST,
bounded recurrence/exceptions, month/leap/year boundaries, availability, negative
ranges/overflow/revision conflicts, fixed clocks, unavailable invites and UTF-8.
It includes month-boundary date roundtrips across every supported year.
CAL-H12 additionally requires state, commit/push and exact CI evidence; the owner-published 0.2-host-integration workflow runs standalone Calendar
fmt/Clippy/tests on Linux and Windows. The exact SHA/run result is recorded in State.

Run with the pinned nightly-2025-08-01 toolchain:

```sh
cargo fmt --manifest-path crates/nagi-calendar-core/Cargo.toml --all -- --check
cargo test --manifest-path crates/nagi-calendar-core/Cargo.toml --locked --offline
cargo clippy --manifest-path crates/nagi-calendar-core/Cargo.toml --all-targets --locked --offline -- -D warnings
./nagi dev status
./nagi dev resume
./nagi dev verify
git diff --check
```

Only the Integration Owner may add a shared host CI step, root workspace registration
or runtime wiring. Full CAL-001..011 still includes UI, real provider delivery,
offline durable cache, Search/People/Mail/Workspace adoption and target acceptance.

## Host portability correction

Initial run 37778135726 passed Ubuntu, but Windows failed before Calendar tests:
`cargo fmt --all` followed the local nagi-model path into the shared workspace
and checked third_party CRLF files against Unix newline requirements. A pinned
Git dependency on the exact owner-checkpoint model revision preserves canonical
IDs while preventing cargo-fmt traversal into another owner’s path workspace.
Calendar source is unchanged by this dependency relocation. The first
`cargo fetch --locked` needs network; subsequent focused checks/tests are offline.
The owner CI runs non-offline Clippy before offline tests, populating that cache.
