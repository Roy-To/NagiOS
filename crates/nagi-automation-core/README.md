# Automation draft host contract

Status: PARTIAL, host/reference only. First-party specification §62,
AUTO-003/005/006/007/008/010/011/012 and 0.2 specification §38 authorize
this independent Phase 2 foundation; no milestone/release gate is promoted.

## Contract and public boundary

`Draft<Action, Scope>` retains existing Platform Action inputs and scope types
unchanged. `ActionBoundary::inspect` is a read-only injected adapter to the
existing ActionRegistry/Validator/capability policy. It must reject unknown
Actions, invalid parameters, denied resources/permissions and invalid scopes;
it returns authoritative effect, permission and affected-resource descriptions.
Scope restriction is asserted by that adapter, never by draft content. This
crate does not recreate NagiPlan, ActionRegistry, Validator, Executor,
CapabilityScope::AutomationTarget or Job. No shared registry is introduced.

Preview borrows the complete draft (including Trigger, Conditions, Scope and
ErrorPolicy) and lists inspected Actions/effects/resources/permissions. No
executor or dispatcher is accepted by any API. Preview is available even when
disabled. A disabled run-once request short-circuits without inspecting Actions.
Enabled requests yield only a descriptive admission state. Even the ordinary
state explicitly requires fresh Platform validation; it is not an executable
plan or authority token. External/destructive requests always remain pending
confirmation; restricted scope is mandatory and no boolean in the draft can
waive confirmation. The adapter must obtain authenticated identity externally
and must never trust permission/effect declarations from Action payloads.

Host admission limits: 16 Actions, 16 Conditions, 64 resources and 32 permission
labels per inspected Action, 128-byte identifiers, and 3 retries (4 attempts).
These are conservative local limits, not a new Platform schema. All metadata
is bounded before being copied into a preview. Trigger values are structured
contracts only: scheduled Unix seconds, positive interval seconds, platform
event, resource change, device state or manual. This crate does not interpret
timezones, subscribe to events or evaluate conditions. Condition keys/values
are opaque adapter inputs and their meaning must be validated by the eventual
condition evaluator. Unknown keys must not be treated as satisfied.

Run outcome summarization takes an approval-run reference, the ordered
expected step IDs, and results that each retain both the approval-run reference
and step ID. It returns a summary only for the same run and exact expected
sequence. Duplicate expected IDs, duplicate results, wrong-run results,
unregistered steps and out-of-order results return a `SummaryError`.
`RecoveryRequired` is a run-level safety barrier: any such result takes
precedence over every other outcome, including `Succeeded`, `Failed`, `Cancelled`,
`TimedOut` and `Pending`, and remains visible if another result is missing. Without
a recovery barrier, a trailing missing result or any `Pending` result keeps the
run `Pending`. A non-empty run is successful only when every expected step has
exactly one ordered `Succeeded` result. When no `RecoveryRequired` is present, any completed mix containing both
`Succeeded` and another outcome is `Partial`, including `Succeeded` plus `Skipped`.
Without `Succeeded`, one terminal failure state (`Failed`, `Cancelled` or `TimedOut`)
is preserved; mixed terminal failure states collapse to `Failed`, and all-skipped
or empty work is `Skipped`. This is a public API change from the old count-only
summary: callers now provide typed run and ordered step IDs, and `summarize_run` returns `Result<RunStatus, SummaryError>`. `StepOutcome` and `RunStatus` add explicit
`Cancelled`, `TimedOut` and `RecoveryRequired` variants.

`ApprovalRunRef` and `StepId` preserve the core's existing 128-byte identifier
grammar, including slash. They stay separate; an automation ID alone is not a
run/step identity. `SchedulerJobId::new` is an explicit pure validation
boundary for the standalone Job scheduler's 96-byte, alphanumeric-first
`[A-Za-z0-9._:-]` grammar. It does not compose, normalize or truncate values.
A future adapter must preserve approval-run plus step identity when it derives
scheduler request/deduplication IDs.

When a future adapter observes the Job scheduler, it must inspect the
`JobRecord` state after `DispatchResult::Completed`; that result only says an
attempt was applied and is not itself success. `DispatchResult::Superseded`
must emit no step result. A cancellation request is not a final cancellation
and may settle as succeeded; only the resulting state is summarized. Other
nonterminal Job states remain Pending; RecoveryRequired stays distinct
and does not claim the Job is terminal or safe to retry. These core types do not
connect to Job or implement durable deduplication, an outbox, notifications,
or exactly-once execution.

K1 (binding approval to the exact draft/effect), K2 (fail-closed external
effect errors), and K4 (notification publication separate from completion
CAS) remain runtime HOLDs. A future adapter must use `no_retry`,
`NonIdempotent`, and `pause=false`; a verified checkpoint does not make an
external effect safe to retry. This crate does not choose scheduled/event
caller authority. It also does not execute retry/continue/notify policies;
it only validates and describes the draft.

Non-goals: timers, event subscriptions, Job dispatch, natural-language
generation, OS sandbox, persistent history/Activity, production UI and guest
integration. The runtime must revalidate after preview, enforce conditions,
acquire current capabilities and confirmation, and execute through the existing
Platform.

## Verification

Negative integration tests precede the implementation (initially failed because
the library did not yet exist). Run only this standalone
manifest with a dedicated SSD target and `CARGO_BUILD_JOBS=1`: `cargo test
--manifest-path crates/nagi-automation-core/Cargo.toml --offline`, then
manifest-scoped fmt and warning-denied clippy. No dependencies or shared lock
changes are needed. Guest/production acceptance is unverified.

The initial implementation was verified on an arm64 macOS host using the
existing Rust 1.97.1 toolchain and explicit `aarch64-apple-darwin` target: 13
integration tests passed, fmt check passed, and all-target warning-denied Clippy
passed. This includes an exhaustive
small-state outcome matrix, shape/metadata bounds, unknown/denied Actions,
restricted scopes and all external/destructive effect combinations. Preview
has no executor interface; disabled admission makes zero adapter calls.
The initial default-toolchain run could not link because PATH selected an
x86_64 Homebrew compiler against arm64 developer tools. Verification explicitly
selected matching cargo/rustc/rustdoc/clippy tools, without installing anything.
The model-free workflow `.github/workflows/nagi-automation-core.yml` runs the
standalone fmt, all-target Clippy and contract tests when this crate, workflow
or pinned toolchain changes. Local checks do not replace remote CI or production
and guest acceptance.

The earlier compile/API RED established that the typed run/step API was
absent; it did not exercise the old count-only summary behavior. A separate
fixture built from the original PR-head library ran duplicate-step and
unregistered-step counterexamples: the ordinary distinct-step control passed,
while both regression assertions failed because the old API returned `Success`.
For the `RecoveryRequired` precedence correction, the standalone contract suite
covers all 49 ordered pairs of `Succeeded`, `Failed`, `Skipped`, `Pending`, `Cancelled`,
`TimedOut` and `RecoveryRequired` outcomes, plus empty and missing-result cases.
Local verification uses pinned `nightly-2025-08-01`, the
`aarch64-apple-darwin` target and one Cargo build job. Command/time/exit metadata
and raw logs are under
`private-evidence/pr77-automation-core-recovery-p2-20261103/` on the development
SSD. The workflow YAML is syntax-checked locally; its GitHub Actions run and
all guest/runtime acceptance remain unverified.
