# hark-sheets-functions — IFS, ROUNDUP, ROUNDDOWN (host-only)

Status: PARTIAL — host-only. No guest/runtime/UI claim. Not an owner
registration; `.dev/workstreams.json` is unchanged. Registered owner of
`crates/nagi-sheets-core/**` remains `sheets-calc-01`.

Base: `origin/main` 69fe92d35568649638dbb1964fd680f1b32aa432.
Branch: `hark/sheets-ifs-roundup-rounddown`.

## Spec basis

`docs/NAGI_FIRST_PARTY_SOFTWARE_IMPLEMENTATION_SPEC.md`

- 58.5 (L2013-2021): parser -> AST -> dependency graph -> engine, incremental recalculation.
- 58.6 (L2023-2044): English canonical internal function IDs (L2025);
  initial set lists `IF/IFS/AND/OR/NOT` (L2033) and `ROUND variants` (L2034);
  extensible function registry (L2044).
- 58.7 (L2046-2057): structured errors incl. `NOT_AVAILABLE`, `VALUE_ERROR`.
- `docs/workstreams/NagiOS_0.2_Sheets_Calculation_Core_Workstream.md` S3
  (L65-70): unneeded branches are not evaluated; rounding/NaN/Infinity/overflow
  behaviour must be documented.

The spec names the functions but not their argument/error semantics. Where it is
silent, this change follows the existing engine conventions documented in
`crates/nagi-sheets-core/README.md` (IF/ROUND behaviour). README is not edited
here (outside this lane's allowed paths); the semantics are recorded below.

## Semantics

IFS(cond1, value1, [cond2, value2], ...)
- Arity: at least 2 and even, else VALUE_ERROR (existing arity convention: arity
  errors are VALUE_ERROR at evaluation, formula still commits).
- Conditions evaluated left to right with IF's condition conversion (Empty→false,
  Boolean, Number≠0; Text/Date/Range→VALUE_ERROR; errors propagate).
- Only the first true condition's value is evaluated; later conditions and all
  other values are never evaluated.
- No true condition → NOT_AVAILABLE (spec 58.7 structured error).
- Dependency edges include every syntactic reference (same conservative policy as
  IF): edits in a dead branch trigger recalculation without changing the value, and
  a dead-branch self reference is CIRCULAR_REFERENCE.

ROUNDUP(number, digits) / ROUNDDOWN(number, digits)
- ROUNDUP rounds away from zero; ROUNDDOWN toward zero.
- Arity exactly 2, else VALUE_ERROR. Argument conversion as arithmetic/ROUND
  (Empty→0, Boolean→0/1, Text/Date→VALUE_ERROR, errors propagate).
- digits must be an integer in [-308, 308] (same rule as ROUND); fractional or
  out-of-range digits → NUMERIC_ERROR. (Engine convention; differs from Excel,
  which truncates fractional digits.)
- Rounding operates on the shortest round-trip decimal representation of the f64
  input, so `ROUNDUP(1.1,2)` is 1.1 (not 1.11 from 110.00000000000001). Note
  `ROUNDUP(0.1+0.2,1)` is 0.4 because the value is 0.30000000000000004.
- Result is parsed back with correct rounding; overflow (e.g. ROUNDUP(MAX,-308))
  → NUMERIC_ERROR. NaN/Infinity are never produced. -0 is normalised to 0.

## Data / serialization contracts

Snapshot format unchanged (stores formula source + bindings; ASTs are re-parsed).
`FunctionId` gains three variants appended at the end (`Ifs`, `RoundUp`,
`RoundDown`); the enum is not `#[non_exhaustive]`, so an external exhaustive
`match` would need new arms (no in-repo consumer exists outside the crate).

## Out of scope / not changed

ROUND itself (still scales in binary f64), README, Cargo manifests/lock, workflows,
registry, other formulas.

See `evidence.json` for commands, counts and CI run IDs.
