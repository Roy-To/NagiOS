//! In-guest system update installer (ADR 0055).
//!
//! On a confirmed-slot boot with nothing pending, init may claim the kernel's
//! one-shot capability for the inactive system slot. The installer then:
//!
//! 1. reads the update bundle from update media (`NAGIUPD.BIN` in the Model
//!    Store volume for the Developer Preview);
//! 2. verifies the signed slot manifest and both payload digests before
//!    writing anything;
//! 3. formats the inactive slot as FAT32 holding `KERNEL.ELF`, `INIT.ELF`
//!    and `SLOT.MAN`, and flushes it;
//! 4. reads every file back and verifies the bundle's digests again;
//! 5. asks the loader to trial that slot on the next boot.
//!
//! The loader re-verifies the slot's manifest before staging, and the M27
//! journal rolls back after three failed trials. If the inactive slot
//! already holds this exact manifest, the installer neither rewrites nor
//! restages it, so a rejected update cannot loop.

use libnagi::BLOCK_SECTOR_SIZE;
use nagi_fat32::{FatError, RootFile, RootFileReader, SectorDevice};
use nagi_slot_manifest::{UpdateBundle, TRUSTED_SLOT_SIGNING_PUBLIC_KEY};

const BUNDLE_NAME: [u8; 11] = *b"NAGIUPD BIN";
const KERNEL_NAME: [u8; 11] = *b"KERNEL  ELF";
const INIT_NAME: [u8; 11] = *b"INIT    ELF";
const MANIFEST_NAME: [u8; 11] = *b"SLOT    MAN";
/// The M30 reference Model Store partition (ADR-0013).
const MODEL_STORE_SECTORS: u64 = 67_108_864;
/// 32 KiB clusters keep the slot's FATs small (ADR 0055).
const SLOT_SECTORS_PER_CLUSTER: u32 = 64;
const MAX_BUNDLE_BYTES: usize = 4 * 1024 * 1024;

static mut BUNDLE: [u8; MAX_BUNDLE_BYTES] = [0; MAX_BUNDLE_BYTES];
static mut READBACK: [u8; MAX_BUNDLE_BYTES] = [0; MAX_BUNDLE_BYTES];

struct CapabilityDevice {
    capability: u64,
    writable: bool,
}

impl SectorDevice for CapabilityDevice {
    fn read_sector(&mut self, sector: u64, buffer: &mut [u8; 512]) -> Result<(), FatError> {
        let buffer: &mut [u8; BLOCK_SECTOR_SIZE] = buffer;
        libnagi::block_read(self.capability, sector, buffer)
            .then_some(())
            .ok_or(FatError::Device)
    }

    fn write_sector(&mut self, sector: u64, buffer: &[u8; 512]) -> Result<(), FatError> {
        if !self.writable {
            return Err(FatError::Device);
        }
        libnagi::block_write(self.capability, sector, buffer)
            .then_some(())
            .ok_or(FatError::Device)
    }
}

fn say(parts: &[&[u8]]) {
    for part in parts {
        libnagi::console_write(part);
    }
    libnagi::console_write(b"\r\n");
}

fn fail(reason: &[u8]) -> bool {
    say(&[b"Nagi update install FAIL reason=", reason]);
    false
}

/// Install and stage the update from update media. Returns `false` only on
/// an installer failure; "no update available" is not a failure.
pub fn run(model_store_capability: u64) -> bool {
    let Some(slot) = libnagi::update_slot_claim() else {
        say(&[b"Nagi update install skipped: no inactive slot is writable on this boot"]);
        return true;
    };
    let slot_name: &[u8] = if slot.slot == 0 { b"A" } else { b"B" };
    let Ok(volume_sectors) = u32::try_from(slot.sector_count) else {
        return fail(b"slot-size");
    };

    // 1. Read the bundle from update media.
    let bundle = unsafe { &mut *core::ptr::addr_of_mut!(BUNDLE) };
    let mut media = CapabilityDevice {
        capability: model_store_capability,
        writable: false,
    };
    let length = match RootFileReader::open(&mut media, MODEL_STORE_SECTORS, &BUNDLE_NAME) {
        Ok(mut reader) => match reader.read_all(bundle) {
            Ok(length) => length,
            Err(_) => return fail(b"media-read"),
        },
        Err(FatError::NotFound) => {
            say(&[b"Nagi update install skipped: no update bundle"]);
            return true;
        }
        Err(_) => return fail(b"media"),
    };

    // 2. Verify before writing anything.
    let update = match UpdateBundle::verify(&bundle[..length], &TRUSTED_SLOT_SIGNING_PUBLIC_KEY) {
        Ok(update) => update,
        Err(_) => {
            say(&[b"Nagi update bundle REJECTED: signature or digest mismatch"]);
            return true;
        }
    };
    say(&[b"Nagi update bundle verified PASS"]);

    let mut target = CapabilityDevice {
        capability: slot.capability,
        writable: true,
    };
    let readback = unsafe { &mut *core::ptr::addr_of_mut!(READBACK) };
    if let Ok(mut reader) = RootFileReader::open(&mut target, slot.sector_count, &MANIFEST_NAME) {
        if reader.len() as usize == update.manifest_file.len()
            && reader.read_all(readback).is_ok()
            && readback[..update.manifest_file.len()] == *update.manifest_file
        {
            say(&[
                b"Nagi update already present in slot=",
                slot_name,
                b"; not restaging",
            ]);
            return true;
        }
    }

    // 3. Write the inactive slot.
    let files = [
        RootFile {
            name: KERNEL_NAME,
            contents: update.kernel,
        },
        RootFile {
            name: INIT_NAME,
            contents: update.init,
        },
        RootFile {
            name: MANIFEST_NAME,
            contents: update.manifest_file,
        },
    ];
    let label = if slot.slot == 0 {
        b"NAGI SYS A "
    } else {
        b"NAGI SYS B "
    };
    if nagi_fat32::format(
        &mut target,
        volume_sectors,
        SLOT_SECTORS_PER_CLUSTER,
        label,
        &files,
    )
    .is_err()
    {
        return fail(b"write");
    }
    if !libnagi::block_flush(slot.capability) {
        return fail(b"flush");
    }
    say(&[b"Nagi update written slot=", slot_name, b" PASS"]);

    // 4. Read back and verify against the signed manifest again.
    for (name, expected) in [
        (KERNEL_NAME, update.manifest.kernel),
        (INIT_NAME, update.manifest.init),
    ] {
        let Ok(mut reader) = RootFileReader::open(&mut target, slot.sector_count, &name) else {
            return fail(b"readback-open");
        };
        let Ok(read) = reader.read_all(readback) else {
            return fail(b"readback");
        };
        if expected.check(&readback[..read]).is_err() {
            return fail(b"readback-digest");
        }
    }
    say(&[b"Nagi update readback verified slot=", slot_name, b" PASS"]);

    // 5. Ask the loader to trial it on the next boot.
    if !libnagi::update_slot_stage(slot.capability) {
        return fail(b"stage");
    }
    say(&[b"Nagi update install PASS slot=", slot_name]);
    true
}
