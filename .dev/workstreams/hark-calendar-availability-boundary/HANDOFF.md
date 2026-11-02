# Hark bounded lane: Calendar availability grid boundary

Scope: `crates/nagi-calendar-core/src/availability.rs` (`find_availability` candidate
grid + inline `boundary_tests`) only. Not a registered workstream; no registry, CI,
manifest, lock or other-owner file is changed. Registered owners `calendar-core-01`
and `calendar-core-adoption` both record `status: PASS`; `codex/0.2-calendar-core-01`
(fb00ad7) has an `availability.rs` identical to main 084f219 and
`codex/0.2-adopt-calendar-core` (5cef9f6) is an ancestor of main.

## Defect (reproduced on unfixed 084f219)

Window `[i64::MAX-1, i64::MAX)`, duration 1, step 1 returned `Err(Overflow)` instead
of `[(MAX-1, MAX)]`. After the final valid slot the loop computed the next candidate
end (`MAX + 1`, or the next start for steps > remaining room) with checked
arithmetic and propagated the overflow, discarding all valid slots. The same class
affected the per-free-window grid alignment: a grid point beyond the free window
whose position is unrepresentable (huge step) also errored.

8 of the 12 new inline tests failed before the fix, all with `left: Err(Overflow)`:
`final_candidate_ending_at_max_is_returned`, `grid_stops_when_next_start_overflows`,
`grid_stops_when_next_start_reaches_window_end_at_max`,
`maximal_duration_and_step_fill_widest_window_once`,
`huge_step_yields_only_first_candidate`,
`aligned_first_candidate_beyond_free_window_is_skipped`,
`aligned_first_candidate_unrepresentable_is_skipped`,
`limits_still_enforced_at_max_boundary`.

## Fix

- Grid termination: iterate only while `cursor < window.end()` (durations are
  positive, so no later start can fit); if `cursor + step` overflows, the next grid
  point is past `i64::MAX >= window.end`, so stop and keep results.
- Alignment: if `steps * step` or `window.start + offset` is unrepresentable, the
  grid point lies beyond this free window (request windows span at most `i64::MAX`
  by `Interval::new`), so this free window has no candidate.
- Unchanged: `cursor + duration` overflow for a candidate that starts inside the
  window is still `Err(Overflow)` (existing acceptance contract
  `availability_overflow_and_fully_busy`); InvalidStep / InvalidRange / InvalidLimit,
  scan and output limits are unchanged and still enforced at the boundary.

## Verification (host only, aarch64 Linux sandbox, nightly-2025-08-01)

- `cargo test --manifest-path crates/nagi-calendar-core/Cargo.toml --locked --offline`:
  13 unit (1 existing + 12 new) + 60 acceptance PASS.
- `cargo fmt ... -- --check`, `cargo clippy ... --all-targets --locked --offline -- -D warnings`,
  `crates/nagi-calendar-core/verify.sh`, `./nagi dev verify`, `git diff --check`: PASS.
- Not verified: Windows, guest/Nagi target, remote providers.
- `ci.yml` does not run Calendar tests; `0.2-host-integration.yml` runs them but is
  gated to listed owner branches, so it is expected to skip on this branch.

## Not changed (noted for owner)

`normalize_busy` still errors with `Overflow` when a busy interval's buffered end
(`end + buffer_after`) is unrepresentable even though it would be clipped to the
window; outside this lane's minimal fix.
