//! Host harness for the M25 Whisper provider.
//!
//! `provider_session` (and, with `real-engine`, `provider_ffi`) are the same
//! source files the Nagi guest provider compiles. Host results produced here
//! are host measurements only; they are not guest results.

extern crate alloc;

#[path = "../../../tools/whisper/provider_session.rs"]
pub mod provider_session;

#[cfg(feature = "real-engine")]
#[path = "../../../tools/whisper/provider_ffi.rs"]
pub mod provider_ffi;

#[cfg(feature = "real-engine")]
pub mod host_engine;

pub mod metrics;
