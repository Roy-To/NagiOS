# Host snapshot path allocation follow-up

Status: PARTIAL local correction on published PR #98 head
`3e5a755945547b71ae89155177d27d723f72b12e`. No push, CI rerun or VM execution.

## Failure and scope

[Windows job 127624138653](https://github.com/Roy-To/NagiOS/actions/runs/42135974061/job/127624138653)
in run `42135974061` checked out GitHub merge object
`93b6e61b1aab1e4b408d1719a7b8379b383c2236`. Its tree equals the published
candidate tree `fb7ab4f41ed7c3c2b8ca0b66ab4315e4dad24e6b`.
The 46 Search unit tests passed, followed by 8 passing backend tests and a
failure in `all_backend_load_allocations_are_fallible_read_only_and_retryable`.
Its ten guest-backend cases completed; the first denied allocation in
`host-valid` aborted with `memory allocation of 138 bytes failed`.

The allocator was correctly armed around the actual load call. This was not a
child-process setup/TLS failure: pinned nightly-2025-08-01 std's Windows
`File::open -> maybe_verbatim -> to_u16s` uses infallible
`Vec::with_capacity(s.len() + 1)` for its UTF-16 path. A 68-byte ASCII path
requests 69 u16 elements, or 138 bytes, before reaching our snapshot reservation.
That explains the observed size; no native stack trace was captured by that CI
failure. Linux's short-path optimization hid the same class of issue until an
extended long-path probe caused its CString conversion to allocate and abort.
Earlier Linux-only evidence did not establish Windows host read OOM safety.

## Correction

Only the host reference read-open boundary changes. On Windows, bounded UTF-16
scratch buffers replace std's infallible Vec conversion. The adapter preserves
the pinned std short absolute/verbatim paths and normalizes other paths once
with GetFullPathNameW before applying the drive/UNC/device prefix rules. It
retains native UTF-16 units, including unpaired surrogates, rejects interior NUL,
and checks all buffer and prefix bounds. CreateFileW uses the same read access,
sharing, OPEN_EXISTING, non-inheritable handle and flags as std File::open. The
owned handle immediately becomes File and closes on all subsequent error paths.
Windows scratch is two 32,768-u16 arrays (128 KiB total), confined to the host.

On Linux/macOS, bounded NUL-terminated bytes and read-only libc::open replace
the long-path CString conversion. Native path bytes, close-on-exec, EINTR retry
and large-file handling are retained. Unix scratch is one 32 KiB array. The
target-specific libc dependency reuses existing version 0.2.189; Cargo.lock
adds only the generated nagi-search dependency edge, with no package version
changes. Nagi's target selects neither host adapter nor this dependency.

Snapshot buffer growth remains fallible. The constructor, host writer and
other caller/OS allocations are outside this narrow read audit. No Nagi backend,
serializer, update/recovery latch, format, workflow or gate is changed.

## Reproduction and validation

Executed on Linux x86-64, rustc 1.90.0-nightly
`adcb3d3b4cd3b7c4cde642f3ed537037f293738e` / nightly-2025-08-01:

- The original published crate with the added long-path input reproduces an
  actual HostFileBackend abort at `host-long nth=0 persistent=false`, requesting
  390 bytes. This is an original-source negative control, not a failing test
  removed from the final suite.
- A separate Linux replay of the pinned Windows ASCII conversion step aborts
  on the same 138-byte request. The corrected production preparation succeeds
  with the allocator continuously denied and zero allocation attempts. This
  replay is not native Windows File::open execution.
- Exact-source isolated host workspace: 46 Search unit, 12 backend integration
  and 3 Model tests pass (61 total). All previous cases/assertions remain. Real
  host relative, Unicode and long paths are added to the same subprocess
  one-shot/persistent denial matrix. Native NUL rejection, prefix/surrogate and
  full-buffer/overflow behavior are tested with the allocator denied. Byte
  preservation, modification time, cleanup, callback counts and retry assertions
  remain strict. No test is newly ignored or skipped.
- Linux all-target Clippy with warnings denied passes. Windows/MSVC library
  cross-build and all-target Clippy pass; macOS all-target cross-Clippy passes.
  These check code generation/types, not native Windows/macOS execution.
- Nagi custom-target library check passes. Its dependency tree contains only
  nagi-search -> nagi-model, with no host libc dependency.
- Root no-dependency Cargo metadata accepts the manifest. Actual root package
  tests stop before compilation because `third_party/cc-nagi/Cargo.toml` is not
  materialized. No substitute or unverified third-party source was installed;
  focused tests use isolated verbatim crate sources and pinned registry libc.

The original ignored allocation-child entry point is still explicitly launched
by its parent; the existing opt-in timing test is unchanged. OOM injection is
not disabled around open, restricted to successful calls, or filtered by size.

## Remaining validation

The selected environment has no native Windows/macOS runner. The corrected
Windows process, actual Win32 API behavior and original failing CI job have NOT
been rerun. Native Windows/macOS execution, a bootstrapped root workspace run,
and independent review of this correction remain pending. No new push, CI
rerun, VM/guest, real media or writer-exclusion operation was performed.
This does not establish full startup OOM or guest/media durability. M24 remains
PARTIAL.

## Primary references

- Pinned Rust source: [Windows path allocation](https://github.com/rust-lang/rust/blob/adcb3d3b4cd3b7c4cde642f3ed537037f293738e/library/std/src/sys/pal/windows/mod.rs),
  [path normalization](https://github.com/rust-lang/rust/blob/adcb3d3b4cd3b7c4cde642f3ed537037f293738e/library/std/src/sys/path/windows.rs),
  and [File::open options](https://github.com/rust-lang/rust/blob/adcb3d3b4cd3b7c4cde642f3ed537037f293738e/library/std/src/sys/fs/windows.rs).
- Microsoft: [CreateFileW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-createfilew),
  [GetFullPathNameW](https://learn.microsoft.com/en-us/windows/win32/api/fileapi/nf-fileapi-getfullpathnamew).
