//! Host tests cover queue orchestration, never native inference. Target checks
//! compile the exact guest worker and actual model-service leaves together.
#![no_std]
extern crate alloc;
#[cfg(test)]
extern crate std;

#[cfg(target_os = "nagi")]
#[path = "../../../../user/nagi-init/src/model_service.rs"]
pub mod model_service;
#[path = "../../../../user/nagi-init/src/session_services.rs"]
pub mod session_services;
