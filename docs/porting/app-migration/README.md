# Migrating an existing application to Nagi OS

This guide is for bringing an existing Rust, C, or C++ application to Nagi OS by reusing code that already works. It does not promise that a Linux executable, POSIX program, or GUI framework can run unchanged.

## 1. Assess the source before changing it

Run the host-only scanner against the application root:

```text
nagi port readiness PATH/TO/PROJECT --format json
```

When using the NagiOS source checkout, you can invoke the CLI from any working directory without installing it first:

```text
cargo +nightly-2025-08-01 run --locked --offline --manifest-path PATH/TO/NagiOS/tools/nagi-bootstrap/Cargo.toml -- port readiness PATH/TO/PROJECT --format json
```

This builds and runs the Nagi host CLI only; the inspected application is not built or executed.

The command reads source files and recognized manifests. It does not invoke the project's build system, build scripts, compiler, package manager, or executable. The versioned JSON report lists detected language, selected manifests and direct dependencies, bounded source evidence, skipped or truncated input, and limitations. Findings are review prompts; absence of a finding does not prove portability.

The first implementation recognizes common Rust, C, and C++ source extensions; Cargo and package JSON dependencies; basic CMake `find_package`/`pkg_check_modules` names; and `-l` libraries in Makefiles. It uses bounded file-count and size limits, skips symbolic links and generated/vendor directories, and applies simple text patterns. Keep the project tree unchanged during a scan: the scanner does not lock paths against concurrent replacement. It does not resolve transitive dependencies, fully parse build languages, understand macros or generated source, determine license compatibility, or inspect runtime behavior. Always review lockfiles, build scripts, native libraries, and framework usage separately.

## 2. Choose a first porting target by boundary cost

Start with a small, testable unit of the application whose source and dependencies can be reused legally. The table is a planning heuristic; actual effort depends on the code and available Nagi public APIs.

| Source shape | Initial porting burden | First review areas |
| --- | --- | --- |
| Rust domain/model code with few dependencies | Often lower | `std` filesystem/process/network calls, build scripts, native crates, target-specific code |
| C library using standard C only | Often lower to moderate | libc assumptions, compiler flags, linking, ownership and error boundaries |
| C code using POSIX or platform calls | Moderate to high | file/process/thread/network APIs, system headers, native libraries, permissions |
| C++ core without a GUI framework | Moderate and toolchain-dependent | standard library/toolchain, exceptions/RTTI, native dependencies, ABI assumptions |
| Qt, GTK, Electron, SDL, or other GUI/framework integration | High or unknown until tested | framework services, window/input/rendering paths, plugins, native dependencies |

The scanner can report evidence such as `std::fs`, `fork`, `pthread`, or `QApplication`; it cannot tell you that a framework or ABI is supported. Do not infer a migration schedule from these categories alone.

## 3. Separate reusable application logic from operating-system adapters

Keep data models, validation, parsing, algorithms, and business rules in a host-independent core when possible. Put file access, process creation, networking, UI rendering, device access, persistence, and platform threading behind explicit adapters. Port one seam at a time and retain the application's tests around the core.

The [`portable-files` sample](../../../samples/porting/portable-files/README.md) adapts `Location` and `FileName` validation from `apps/nagi-files/src/model.rs`. Its catalog is in-memory: it illustrates reuse of domain code, not a Nagi file provider, host filesystem access, or an installable app.

## 4. Bind platform work to public Nagi contracts

Use the public Rust SDK or C boundary and the documented public API levels appropriate to the feature. First-party and third-party apps use the same public API surface. Do not depend on first-party internals, private services, or implicit privileges.

Ask for only the capabilities the app needs, with a clear purpose, and route protected actions through the Nagi permission/capability contract. A migration must not preserve ambient host authority by silently granting it to the app. A zero-capability manifest is appropriate only while the sample has no protected effects.

## 5. Build and validate in stages

For the portable sample, run its host tests and build from the repository root:

```text
cargo test --manifest-path samples/porting/portable-files/Cargo.toml
cargo build --manifest-path samples/porting/portable-files/Cargo.toml
cargo run --manifest-path tools/nagi-pkg/Cargo.toml -- manifest validate samples/porting/portable-files/manifest.json
```

These steps prove only host compilation/tests and host-side manifest validation. The fixture is not an installable Nagi package. A successful host build is not a target build, and package validation does not grant a capability or prove launch behavior.

Guest install, launch, and runtime acceptance must wait until the applicable Nagi release and runtime gates are open. When they are, test the actual target artifact, declared capabilities, permission prompts, API version, and failure/recovery paths. Until then, report those outcomes as not run.

## What this workflow does not claim

- No general Linux binary, POSIX, libc, or ABI compatibility.
- No Qt, GTK, Electron, SDL, or other framework compatibility without a target-specific demonstration.
- No compatibility certification from a clean readiness report.
- No guest install or launch result from host tests or manifest validation.
