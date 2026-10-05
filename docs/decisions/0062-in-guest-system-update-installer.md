# ADR 0062: In-guest system update installer and staging request

Status: accepted
Date: 2026-10-06
Builds on: ADR 0061 (authenticated slot manifests), ADR-0011, ADR-0013

## Context

Spec §21 describes the update flow: running A, write the update to B,
verify, boot B, mark success after readiness, and return to A on repeated
failure.

ADR 0061 made the loader verify signed slot manifests. The remaining M30
item was to produce and install an authenticated update *from a running
system*. Two things were missing:

- user space could not write a system slot;
- nothing could ask the loader to trial one.

## Decision

1. **Boot context.** `BootControlInfo` (BootInfo v5) gains `flags`. On a
   confirmed-slot boot with no pending trial, the loader passes
   `BOOT_CONTROL_UPDATE_STAGEABLE` with:
   - the confirmed slot;
   - the journal generation;
   - the runtime `SetVariable` entry point, which the kernel validates as
     runtime code exactly as for trials.

   Trials and Recovery never carry the flag.
2. **Update capability.** On such a boot, the kernel exposes the
   *inactive* system slot as a separate block capability.
   - **Discovery.** The kernel finds the slot by its unique GPT type, and
     an ambiguous type exposes nothing.
   - **Claim.** `SYS_UPDATE_SLOT_CLAIM` hands the capability to init
     (PID 1) only, once per boot, together with the slot's sector count.
   - **Scope.** The capability reads, writes, and flushes only that
     partition, using partition-relative sectors. User Data and Model
     Store capabilities are unchanged.
3. **Staging request.** `SYS_UPDATE_SLOT_STAGE` requires the claimed
   capability and is one-shot. It writes the `NagiBootStage` UEFI variable
   (`BootStageRecord`: slot and journal generation, CRC-protected).
   - Every coordinate comes from BootInfo; none comes from user space.
   - The target is always the slot that is not confirmed.
4. **Loader handling.** On the next boot, the loader consumes and deletes
   `NagiBootStage`. It calls `stage_update` only when all of these hold:
   - the generation still matches;
   - nothing is pending;
   - the slot is not the confirmed one;
   - the candidate's signed manifest verifies;
   - the candidate does not lower the confirmed slot's rollback index.

   Otherwise it prints `update stage request REFUSED reason=<…>`. The
   existing three-attempt trial and readiness promotion then apply
   unchanged.
5. **Installer.** The installer is `user/nagi-init/src/system_update.rs`,
   feature `m30-update-install`. It runs these steps in order:
   1. Read the update bundle `NAGIUPD.BIN` from update media. For the
      Developer Preview, that is the read-only Model Store volume.
   2. Verify the bundle (`nagi_slot_manifest::UpdateBundle`): the header
      and the signed `SLOT.MAN`, then both payload digests. This happens
      before any write.
   3. Format the inactive slot as FAT32 holding `KERNEL.ELF`, `INIT.ELF`
      and `SLOT.MAN` (`crates/nagi-fat32`), then flush.
   4. Read each payload back and check it against the manifest again.
   5. Request staging.

   If the inactive slot already holds the identical manifest, the
   installer neither rewrites nor restages it, so an update that was
   rolled back cannot loop.
6. **FAT32 crate.** `crates/nagi-fat32` is a `no_std` FAT32 root-directory
   formatter and reader. It writes in crash-safe order:
   1. invalidate both boot sectors;
   2. write file data;
   3. write the FATs;
   4. write the root directory and FSInfo;
   5. write the backup boot sector, then the primary one last.

   It uses 32 KiB clusters, so a 4 GiB slot needs about 1,900 sector
   writes. Its host tests cross-check the result with the independent
   M20 guest FAT32 reader.

## Acceptance

`./nagi m30-update` passed on the arm64 macOS host (evidence
`out/evidence/m30-update-1791242245330653000`). It builds two 64 GiB GPT
images. Each one carries a host-signed bundle in Model Store:
- the release kernel;
- a desktop init;
- rollback index 2.

**Signed bundle.**
1. **First boot, System A.** A verified its own manifest, claimed slot B,
   verified the bundle, wrote B, verified the read-back, persisted the
   staging request, and printed `Nagi update install PASS slot=B`.
2. **Second boot.** The loader printed `update stage request accepted
   slot=B PASS` and `trial attempt=1 slot=B`. Firmware read the
   guest-written FAT32 volume, the loader verified `slot=B
   rollback-index=2`, and B persisted readiness.
3. **Third boot.** The readiness record was consumed and the decision was
   `confirmed slot=B`.

**Tampered bundle** (last init byte flipped).
1. **First boot.** The installer printed `bundle REJECTED` and wrote
   nothing.
2. **Next boot.** A stayed confirmed with no staging.

`./nagi m27` and the clean-worktree `./nagi m30` still pass (recorded in
`docs/implementation_status.md`).

## Bounds and non-goals

- **Update media.** Update media is a file in the Model Store volume.
  Fetching updates over the network (`nagi-net`), an update UI, and user
  consent for system updates are later work.
- **Signer.** The signer is the published RFC 8032 test key (ADR 0061).
- **Capability form.** The update capability follows the existing kernel
  block-capability form (an opaque u64 checked per syscall). It is
  additionally bound to PID 1 and to a one-shot claim.
- **Root directory only.** The installer writes only the three root
  files. Larger system payloads beyond the 4 MiB bundle buffer need a
  streaming writer.
