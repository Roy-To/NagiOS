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

Run outcome summarization accepts explicit per-step outcomes and never reports
success for empty, incomplete, failed, skipped or pending runs. It does not
execute retry/continue/notify policies; it only validates and displays them.

Non-goals: timers, event subscriptions, Job dispatch, natural-language generation,
OS sandbox, persistent history/Activity, production UI and guest integration.
The runtime must revalidate after preview, enforce conditions, acquire current
capabilities and confirmation, and execute through the existing Platform.

## Verification

Negative integration tests precede the implementation (initially failed because
the library did not yet exist). Run only this standalone
manifest with a dedicated SSD target and `CARGO_BUILD_JOBS=1`: `cargo test
--manifest-path crates/nagi-automation-core/Cargo.toml --offline`, then
manifest-scoped fmt and warning-denied clippy. No dependencies or shared lock
changes are needed. Guest/production acceptance is unverified.

Verified on an arm64 macOS host using the existing Rust 1.97.1 toolchain and
explicit `aarch64-apple-darwin` target: 13 integration tests pass, fmt check
passes, and all-target warning-denied clippy passes. This includes an exhaustive
small-state outcome matrix, shape/metadata bounds, unknown/denied Actions,
restricted scopes and all external/destructive effect combinations. Preview
has no executor interface; disabled admission makes zero adapter calls.
The initial default-toolchain run could not link because PATH selected an
x86_64 Homebrew compiler against arm64 developer tools. Verification explicitly
selected matching cargo/rustc/rustdoc/clippy tools, without installing anything.
Existing repository CI may not select this standalone manifest; these local
checks do not replace production or guest acceptance.
