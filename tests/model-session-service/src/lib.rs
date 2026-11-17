//! Compile the exact guest leaves without modifying init's shared feature graph.
//! No host inference backend is linked or used by this harness.
#![no_std]
extern crate alloc;

#[cfg(all(target_os = "nagi", feature = "m20-model-service"))]
#[path = "../../../user/nagi-init/src/model_service.rs"]
pub mod model_service;

// Preserve compile coverage for the old real-inference acceptance after its
// backend extraction. The normal-session service uses its own read-only reader.
#[cfg(target_os = "nagi")]
pub struct SyscallModelStoreReader(pub u64);
#[cfg(target_os = "nagi")]
impl nagi_model_manager::ModelStoreSectorReader for SyscallModelStoreReader {
    fn read_sector(
        &mut self,
        sector: u64,
        destination: &mut [u8; 512],
    ) -> Result<(), nagi_model_manager::ArtifactReadError> {
        libnagi::block_read(self.0, sector, destination)
            .then_some(())
            .ok_or(nagi_model_manager::ArtifactReadError::Unavailable)
    }
}
#[cfg(all(target_os = "nagi", feature = "granite-acceptance"))]
#[path = "../../../user/nagi-init/src/m20_granite.rs"]
pub mod granite_acceptance;

#[cfg(not(target_os = "nagi"))]
#[path = "../../../user/nagi-init/src/model_session_access.rs"]
pub mod model_session_access;
