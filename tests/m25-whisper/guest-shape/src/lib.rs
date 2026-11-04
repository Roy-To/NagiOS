//! Type-checks the guest M25 Whisper module against the real `nagi-audio`
//! and `nagi-model-manager` APIs. Check-only; see Cargo.toml.
#![no_std]

extern crate alloc;

/// Mirrors `user/nagi-init/src/main.rs`'s Model Store reader shape.
pub struct SyscallModelStoreReader(pub u64);

impl nagi_model_manager::ModelStoreSectorReader for SyscallModelStoreReader {
    fn read_sector(
        &mut self,
        _partition_relative_sector: u64,
        _destination: &mut [u8; nagi_model_manager::FAT32_SECTOR_SIZE],
    ) -> Result<(), nagi_model_manager::ArtifactReadError> {
        Err(nagi_model_manager::ArtifactReadError::Unavailable)
    }
}

#[path = "../../../../user/nagi-init/src/m25_whisper.rs"]
pub mod m25_whisper;
