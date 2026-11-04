//! Compile the exact guest leaves without modifying init's shared feature graph.
//! No host inference backend is linked or used by this harness.
#![cfg_attr(any(target_os = "nagi", not(test)), no_std)]
extern crate alloc;

#[cfg(all(any(target_os = "nagi", test), feature = "m20-model-service"))]
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

#[cfg(all(not(target_os = "nagi"), not(test)))]
#[path = "../../../user/nagi-init/src/model_session_access.rs"]
pub mod model_session_access;

#[cfg(all(not(target_os = "nagi"), not(test)))]
#[path = "../../../user/nagi-init/src/session_page_summary.rs"]
#[allow(dead_code)] // The guest service invokes this leaf; the host harness tests it.
pub mod session_page_summary;

// Exercise Albert's actual policy source without bringing its Servo target graph
// into this standalone host boundary harness.
#[cfg(all(test, not(target_os = "nagi")))]
#[path = "../../../user/nagi-albert/src/assistant_context.rs"]
mod albert_context_policy;
