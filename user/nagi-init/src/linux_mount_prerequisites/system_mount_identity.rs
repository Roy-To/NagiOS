//! Fixed development System fixture. This does not select a product filesystem.

#[cfg(all(
    target_os = "linux",
    target_arch = "x86_64",
    linux_dev_system_mount,
    not(any(linux_dev_preview_smoke, nagi_release_build))
))]
pub(super) const SERIAL: &str = "NAGI-SYS-212B8AB4";
const UUID: [u8; 16] = [
    0x3c, 0xf4, 0x72, 0xed, 0xd2, 0xdb, 0x42, 0x2a, 0x80, 0x84, 0x6a, 0x61, 0x79, 0xd0, 0x6a, 0x36,
];

pub(super) fn clean_fixed_system(superblock: &[u8; 1024]) -> bool {
    // ext4 s_state, s_feature_incompat, s_feature_ro_compat, s_uuid and s_volume_name.
    const VALID_FS: u16 = 1;
    const NEEDS_RECOVERY_OR_EXTERNAL_JOURNAL: u32 = 0x04 | 0x08;
    const EXTENTS: u32 = 0x40;
    const ORPHAN_PRESENT: u32 = 0x10000;
    let incompat = u32::from_le_bytes(superblock[0x60..0x64].try_into().unwrap());
    let ro_compat = u32::from_le_bytes(superblock[0x64..0x68].try_into().unwrap());
    superblock[0x38..0x3a] == [0x53, 0xef]
        && superblock[0x3a..0x3c] == VALID_FS.to_le_bytes()
        && incompat & EXTENTS != 0
        && incompat & NEEDS_RECOVERY_OR_EXTERNAL_JOURNAL == 0
        && ro_compat & ORPHAN_PRESENT == 0
        && superblock[0x68..0x78] == UUID
        && superblock[0x78..0x88] == *b"nagi-system\0\0\0\0\0"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed_superblock() -> [u8; 1024] {
        let mut block = [0; 1024];
        block[0x38..0x3c].copy_from_slice(&[0x53, 0xef, 1, 0]);
        block[0x60] = 0x40;
        block[0x68..0x78].copy_from_slice(&UUID);
        block[0x78..0x88].copy_from_slice(b"nagi-system\0\0\0\0\0");
        block
    }

    #[test]
    fn accepts_the_clean_system_fixture_identity() {
        assert!(clean_fixed_system(&fixed_superblock()));
    }

    #[test]
    fn rejects_wrong_identity_or_any_pending_recovery() {
        for (offset, value) in [
            (0x68, 0xaa),
            (0x7d, b'd'),
            (0x3a, 0),
            (0x60, 0x44),
            (0x66, 1),
        ] {
            let mut block = fixed_superblock();
            block[offset] = value;
            assert!(!clean_fixed_system(&block), "offset {offset:#x}");
        }
    }
}
