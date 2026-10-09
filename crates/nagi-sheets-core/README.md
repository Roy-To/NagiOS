# Sheets host calculation core v1

Standalone host-only Rust library authorized by BP-SBOM-HOST-20261008.
It uses `nagi-model::ObjectId` via a local path dependency. It does not join
the root workspace and has no third-party dependencies or I/O.

```sh
cargo test --manifest-path crates/nagi-sheets-core/Cargo.toml --locked --offline
cargo fmt --manifest-path crates/nagi-sheets-core/Cargo.toml --all -- --check
cargo clippy --manifest-path crates/nagi-sheets-core/Cargo.toml --all-targets --locked --offline -- -D warnings
cargo run --manifest-path crates/nagi-sheets-core/Cargo.toml --example measure --release --locked --offline
```

Workbook and sheet ObjectIds are caller-supplied identities, never paths or
permissions. Sheet names are exact, case-sensitive UTF-8 display names. SheetId
is opaque; the workbook checks uniqueness and retains deleted IDs. There are at
most 256 sheet identities over a workbook's lifetime, including retired sheets.
A new display name never revives a reference to a deleted sheet.

Cells use one-based coordinates up to 1,048,576 rows and 16,384 columns.
Only non-empty inputs occupy the BTreeMap. Missing cells read as Empty; Number(0),
Boolean(false), Text("") and Error are distinct stored values. The typed boundary
has no implicit null conversion. Dates are signed Unix-epoch days, datetimes UTC
milliseconds and durations signed milliseconds; they roundtrip as typed values
but do not implicitly coerce into numbers or Excel date serials.

The parser produces an immutable flat AST arena. Formula sources begin with `=`.
It accepts decimal/exponent numbers, doubled-quote escaped text, TRUE/FALSE,
`+ - * /`, comparisons `= <> < <= > >=`, unary signs, parentheses,
A1 references with independently preserved `$` markers, rectangular ranges,
`Sheet1!A1` and `'日本語 集計'!$A$1` (doubled single quotes escape sheet names).
Ranges must be ascending rectangles on one sheet (`Sheet!A1:B2`). English
function names resolve case-insensitively to the closed v1 canonical registry.
Unknown functions, malformed syntax and invalid references return distinct errors.
Copy/fill relocation, named ranges, whole columns, Excel grammar and locale-specific
formula translations are outside this API.

Arithmetic converts Empty to zero and booleans to 0/1; text, dates and durations
cause VALUE_ERROR. Conditions convert Empty to false, booleans directly and
numbers by nonzero; text does not convert. Text comparisons are exact Unicode
scalar ordering; numeric comparisons use arithmetic conversion. Aggregates
SUM/AVERAGE/MIN/MAX/COUNT accept numbers and ignore other non-error types, even
for scalar arguments. COUNTA counts all non-Empty values, including empty text
and errors. COUNT ignores errors; other numeric aggregates propagate errors.
Empty SUM/MIN/MAX ranges yield zero; AVERAGE without numbers yields DIV_BY_ZERO.
All functions require at least one argument; IF requires three, NOT/LEN/TRIM one,
ROUND/IFERROR/LEFT/RIGHT two and MID three.

IF, IFERROR, AND and OR evaluate only necessary branches/arguments. Dependency
edges nevertheless include all syntactic references. Cycle detection is
conservative: even a reference in an inactive IF branch can create a circular
formula. Kahn's algorithm assigns CIRCULAR_REFERENCE to cycle members and all
blocked downstream formulas. This policy is explicit; dynamic dependency discovery
and iterative circular calculation are deferred.

Numbers are finite IEEE f64. Nonfinite input and overflow yield NUMERIC_ERROR;
no NaN/Infinity enters a calculated value. ROUND uses half away from zero with
integer digits in [-308, 308]; overflowing scaling yields NUMERIC_ERROR.
LEFT/RIGHT/MID/LEN operate on Unicode scalar values, not bytes or grapheme clusters.
MID is one-based; negative/fractional counts yield VALUE_ERROR. TRIM collapses
Unicode whitespace. CONCAT accepts text/numbers/booleans/Empty and is bounded.
TODAY/NOW are absent: there is no host clock hidden inside the core.

`set_value`, `set_formula` and batch `set_values` validate the entire request
before mutating cells or graph. Duplicate batch addresses are rejected. Invalid
syntax/reference/input/size leaves the prior values and dependencies intact.
Valid formulas producing calculation errors are committed as structured cell
errors. Their ChangeEvent has `is_successful() == false` and enumerates errors;
callers must inspect that result before claiming formula success. Successful
mutation emits an in-memory ChangeEvent with changed addresses and recalculated
addresses. No asynchronous notification delivery or external logging is implied.

Updates replace outgoing edges and walk only transitive reverse dependencies.
Evaluation is iterative across cells, in deterministic topological order (CellKey
breaks ties). The event enumerates each actually evaluated/invalidated formula
once; literal changes and unrelated cells are excluded. AST evaluation is bounded
recursion. Rename retains resolved IDs and never reparses old display names.
Delete invalidates dependent formulas. Formula deletion drops outgoing edges;
remaining references to an empty cell correctly stay tracked.

Bounds: 8,192 formula bytes, 256 AST nodes, depth 64, 100,000 expanded reference
visits per formula (including repeated ranges), 100,000 populated cells,
1,000,000 unique graph edges and 65,536 bytes per text value. Reverse walks and
Kahn traversal use O(affected cells + affected edges) working space. Formula range
materialization is bounded by the per-formula limit. Maps store only populated
cells and referenced addresses, never the logical grid. Resource bounds are v1
host limits, not a claim of full XLSX-size populated workbook support.

`Workbook::snapshot()` / `Workbook::load()` provide version-1 typed in-memory
snapshots. No byte serialization, filesystem write or durable commit is claimed.
They preserve input values, source formulas, resolved binding IDs and retired
identities across rename and deletion. Loading validates version, identities,
duplicates, coordinates, reference binding shape, limits and finite inputs; it
rebuilds the graph and recomputes values rather than trusting a cached result.
The binding IDs are the identity authority inside this local snapshot; textual
sheet names are retained as historical formula source. Formula bindings to a
retired ID load as INVALID_REFERENCE, and IDs unknown to the snapshot are rejected.

The host facade consists of create/rename/delete sheet, get_range, set_values,
set_formula and snapshot/load. A future public `sheets.*` adapter must enforce
session, permissions, revisions and transactions *before* invoking it, then
translate ChangeEvent into the common Activity/Search/Wayback contracts. This
crate defines no competing permission service, SDK Action ABI or production IPC.
