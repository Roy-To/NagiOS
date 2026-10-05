# ADR 0054: Authenticated system-slot manifests

Status: accepted
Date: 2026-10-06
Builds on: ADR-0011 (M27 boot readiness), ADR-0012 (Recovery selection),
ADR-0013 (M30 reference disk layout)

## Context

M27 selects System A, System B or Recovery through a checksummed UEFI
variable journal. Before this decision the loader trusted any `KERNEL.ELF`
and `INIT.ELF` found on the selected volume: an update candidate was refused
only if its ELF was malformed. The M30 checklist and spec §21 require
updates to be written to the inactive slot and verified before boot, and
the open M30 item is "an authenticated GPT update with authenticated slot
manifests".

## Decision

1. **Slot manifest.** Every System A, System B and Recovery volume carries
   `SLOT.MAN` at its root (or in its `EFI/NAGI/<SLOT>/` directory on the
   legacy FAT12 acceptance layout). It is a strict text manifest followed by
   a 64-byte Ed25519 signature:

   ```text
   nagi-slot-manifest 1
   version=<label>
   rollback-index=<u64>
   kernel-sha256=<hex>
   kernel-size=<bytes>
   init-sha256=<hex>
   init-size=<bytes>
   ```

   The signature covers `"nagi-slot-manifest-v1\0" || text`. The domain
   separator means a slot signature can never be replayed as an M16
   package signature, or the reverse, although the Developer Preview uses
   the same pinned RFC 8032 test key for both.
2. **Shared crate.** `crates/nagi-slot-manifest` is `no_std` and
   allocation-free. It parses, verifies and (with feature `sign`, host
   only) signs manifests. Its tests cover:
   - round trips;
   - untrusted keys;
   - text altered after signing;
   - domain separation;
   - strict parsing;
   - rollback comparison.
3. **Loader checks.** With `m27-ab-slot-boot-control`, which is enabled in
   M27 and M30 release images, the loader runs these checks in order:
   1. It reads and verifies the selected slot's `SLOT.MAN` before reading
      any payload.
   2. For a trial boot (`attempt != 0`), it also verifies the confirmed
      slot's manifest. The candidate's rollback index must not be lower.
   3. It checks `KERNEL.ELF` against the manifest's size and SHA-256
      before parsing the ELF.
   4. It checks `INIT.ELF` the same way before handing it to the kernel.

   **Failures.** Any failure prints `Nagi slot manifest REJECTED slot=<S>
   reason=<missing|size|signature|malformed|confirmed-manifest|rollback|
   kernel-size|kernel-digest|init-size|init-digest>`. It then takes the
   existing `trial payload rejected` path, so a refused System B consumes
   one of the three journal attempts and the journal rolls back to the
   confirmed slot. A refused confirmed slot fails closed, as a malformed
   one already did.

   **Success.** The loader prints `Nagi slot manifest verified slot=<S>
   rollback-index=<n> PASS`.
4. **Soft-float build.** The UEFI target is soft-float, so the repository
   Cargo configuration selects curve25519-dalek's portable `serial`
   backend for `x86_64-unknown-uefi` only. The guest's existing package
   verification is unchanged.
5. **Image builders.** The image builders sign each slot's manifest with
   the Developer Preview key, with rollback index 1 and the CLI version
   label.
   - **FAT12 fixtures.** Their manifests truthfully describe their
     payloads, including deliberately malformed ones. A malformed kernel is
     therefore still refused by ELF validation.
   - **GPT M27 fixture.** It now gives System B the same valid kernel and
     init as System A, but signs B's manifest with an untrusted key. The
     loader must refuse an update whose ELF would boot.

## Acceptance

**`./nagi m27`** passed on the arm64 macOS host (evidence
`out/evidence/m27-ab-rollback-1791241326571375000`).

- **GPT untrusted B.** Three trials each printed `REJECTED slot=B
  reason=signature` and did not reach the kernel. Recovery left the
  journal unchanged, and the next boot rolled back to System A.
- **GPT healthy B.** B verified, persisted readiness, and was promoted.
- **Verified slots.** Every System A, B and Recovery boot printed
  `verified … rollback-index=1 PASS`.
- **FAT12 malformed B.** It was still refused as `invalid ELF`.

**`./nagi m30`** verifies the release image boots System A through the
manifest check (recorded in `docs/implementation_status.md`).

## Bounds and non-goals

- **Signer.** The signer is the published RFC 8032 test key, so this
  establishes the verification path, not production trust. Key
  provisioning and rotation are later work.
- **Rollback index source.** The rollback index is compared only against
  the confirmed slot's manifest. There is no separate tamper-resistant
  counter.
- **Installer.** There is no in-guest update installer yet. Writing a
  signed update into System B and staging the journal from a running
  System A is the next step of the M30 update item.
- **Directory layout.** The legacy FAT12 directory layout is acceptance
  only. Release images use GPT.
