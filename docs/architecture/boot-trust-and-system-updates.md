# Boot Trust and System Updates

This document describes how Nagi 0.1 decides what it boots and how a running
system installs an update. The decisions are
[ADR-0011](../decisions/ADR-0011-m27-boot-readiness.md),
[ADR-0012](../decisions/ADR-0012-m27-recovery-boot-selection.md),
[ADR-0013](../decisions/ADR-0013-m30-reference-disk-layout.md),
[ADR 0061](../decisions/0061-authenticated-slot-manifests.md),
[ADR 0062](../decisions/0062-in-guest-system-update-installer.md) and
[ADR 0063](../decisions/0063-desktop-owner-login.md).

## Disk and slots

The reference disk is a GPT image with six partitions:

- the ESP;
- System A;
- System B;
- User Data;
- Recovery;
- Model Store.

The ESP holds only the UEFI loader. System A, System B and Recovery are FAT32
volumes. Each holds `KERNEL.ELF`, `INIT.ELF` and a signed `SLOT.MAN`.

The kernel exposes storage to user space only through bounded extent
capabilities, never as a raw disk:

| Capability | Who holds it | Access |
| --- | --- | --- |
| User Data | init, every boot | read/write, first 8 MiB of the partition |
| Model Store | init, every boot | read-only |
| Inactive system slot | init, once, on an update-stageable boot | read/write, that partition only |

## Which slot boots

The loader keeps a checksummed two-record **boot-control journal** in UEFI
variables. The journal records:

- the confirmed slot;
- an optional pending slot;
- an attempt count;
- a generation number.

On each boot, the loader handles the journal in this order:

1. It consumes a **readiness record** (`NagiBootReady`) left by the kernel
   of a successful trial. If it matches the pending trial, the trial
   becomes the confirmed slot.
2. It consumes a **staging request** (`NagiBootStage`) left by an update
   installer (see below).
3. It applies the operator's boot-menu choice, if any. Recovery never
   changes the journal.
4. It boots the pending slot for up to three attempts, then rolls back to
   the confirmed slot.

## What the loader trusts

Before the loader uses any payload byte, it verifies the selected slot's
`SLOT.MAN`:

- **Format.** The text manifest pins a version label, a rollback index,
  and the SHA-256 and size of `KERNEL.ELF` and `INIT.ELF`.
- **Signature.** The manifest is followed by an Ed25519 signature over
  `"nagi-slot-manifest-v1\0" || text`. The domain separator keeps slot
  signatures distinct from package signatures.
- **Signer.** The signature is checked against the pinned Developer
  Preview signer, the published RFC 8032 test key. Production key
  provisioning is open.
- **Rollback.** A trial slot must not lower the confirmed slot's rollback
  index.
- **Payloads.** The kernel and init are checked against the manifest
  before the ELF is parsed and before init is handed to the kernel.

Any failure prints `Nagi slot manifest REJECTED slot=<S> reason=<…>`. It
then takes the trial-rejection path, so a bad update consumes attempts and
the boot rolls back. A confirmed slot that fails verification fails closed.

## Installing an update

A boot of the confirmed slot with nothing pending is **update-stageable**.
The loader marks it so in `BootInfo` and passes the journal generation and
the runtime `SetVariable` entry point. The kernel validates that entry
point as UEFI runtime code. Installation then proceeds:

1. **Claim.** init claims the inactive-slot capability
   (`SYS_UPDATE_SLOT_CLAIM`). Only init may claim it, and only once per
   boot.
2. **Verify first.** The installer reads an update bundle. The bundle is
   the signed `SLOT.MAN`, the kernel and the init, from update media; for
   the Developer Preview, that is `NAGIUPD.BIN` in Model Store. The
   installer verifies the signature and both digests before writing
   anything.
3. **Write.** The installer formats the inactive slot as FAT32
   (`crates/nagi-fat32`) in crash-safe order: it invalidates the boot
   sectors first and writes the primary boot sector last. It then
   flushes.
4. **Read back.** It reads every payload back and checks it against the
   manifest again.
5. **Stage.** It calls `SYS_UPDATE_SLOT_STAGE`, which writes
   `NagiBootStage` (slot and generation, CRC-protected). Every coordinate
   comes from `BootInfo`; none comes from user space.

On the next boot, the loader honors the request only when all of these
hold:

- the generation still matches;
- nothing is pending;
- the target slot is the unconfirmed one;
- the candidate's manifest verifies;
- the rollback index is not lowered.

If the slot already holds the identical manifest, the installer neither
rewrites nor restages it, so an update that rolled back cannot loop.

## When a trial is "ready"

A trial is confirmed only when the guest reports readiness through
`SYS_BOOT_READY`. With `desktop-login`, the desktop reports readiness only
after the owner has signed in. A trial is therefore confirmed by a
signed-in desktop, not just by a drawn frame.

## Acceptance

| Command | What it proves |
| --- | --- |
| `./nagi m27` | Three rejected trials and rollback, including a bootable but untrusted System B; a healthy B confirmed after sign-in; Recovery leaves the journal unchanged |
| `./nagi m30-update` | A installs a signed update into B; the loader trials B, B signs in and is confirmed; a tampered bundle is refused before any write |
| `./nagi m30` | The release image boots System A and Recovery through manifest verification |

## Open work

- Network delivery of update bundles.
- An update UI, and user consent for system updates.
- Production signing keys and rotation.
- A tamper-resistant rollback counter. Today the rollback index is
  compared only against the confirmed slot's manifest.
- Streaming writes for payloads larger than the 4 MiB installer buffer.
