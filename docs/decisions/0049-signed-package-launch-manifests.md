# ADR 0049: Launch declarations come from signed M16 packages

Status: accepted
Date: 2026-10-03
Builds on: ADR 0046 (Supervisor launch registry) and the M16 package format

## Context

ADR 0046's launch manifests were plain text files compiled into init, and
the isolated ELFs were embedded separately. Nothing bound an application's
declared identity and grants to the code that was actually launched, and the
M16 Ed25519 package signature played no part in launching.

## Decision

1. **Package manifests carry grants.**
   - The M16 `PackageManifest` accepts optional `grant=` lines: at most 8,
     unique, each lowercase `[a-z][a-z0-9._-]*`.
   - `MAX_PACKAGE_BYTES` rises from 8 KiB to 64 KiB so isolated ELFs fit.
     The M16 sample keeps its own 8 KiB check.
2. **Packaging.**
   - `nagi-pkg build-signed <manifest> <elf> <output>` packages a static
     ELF, appends `entry=<elf name>` when the manifest has no `entry=` line,
     and signs the package with the pinned Developer Preview key.
   - `./nagi` builds seven acceptance packages into
     `out/artifacts/acceptance-packages/`. One application may ship several
     packages, one per ELF, with identical declarations.
   - init embeds the packages through `NAGI_ACCEPTANCE_PACKAGES`.
3. **The Supervisor launches only from verified packages.**
   `supervisor::launch(package, app_id, placement)` does the following in
   order:
   1. parse the package;
   2. require `is_signed()`, Ed25519 over the whole signed region against
      `TRUSTED_SIGNING_PUBLIC_KEY`;
   3. build the `AppManifest` from the signed id and grants only;
   4. refuse a package that declares a different application;
   5. `register_or_match` the declaration. Identical declarations are
      accepted, and a conflicting declaration of the same `AppId` is
      refused;
   6. check the launch;
   7. spawn the signed executable section.

   The text manifests in `user/nagi-init/manifests/` are inputs to signing
   only; init no longer embeds them.

## Verification

- Host tests:
  - `nagi-package`: grant parsing, uniqueness, and bounds;
  - `libnagi::launch`: declarations built from packages, `register_or_match`
    agreement, and conflict refusal.
- `./nagi isolated-process` on QEMU/OVMF printed `Nagi Supervisor signed
  package verification PASS`:
  - the Supervisor refused a copy with one flipped executable byte
    (`UnsignedPackage`);
  - it refused a valid package requested as another application
    (`WrongApplication`);
  - it refused a truncated package (`InvalidPackage`);
  - it then launched the genuine packages.

## Bounds

- The trust key is still the RFC 8032 test vector pinned for the Developer
  Preview (M16). Production key provisioning and user consent for grants are
  later work.
- Packages are embedded in the init image rather than installed through the
  Package Service store and VFS.
