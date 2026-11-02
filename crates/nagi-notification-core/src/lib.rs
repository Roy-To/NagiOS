//! Host-only notification service contract and bounded in-memory reference.
//!
//! This crate does not register a Nagi service, persist to a guest filesystem,
//! deliver notifications, or implement UI. Profile and source identities are
//! supplied by owner adapters and remain opaque here.

mod model;
mod service;
mod store;

pub use model::*;
pub use service::*;
pub use store::*;
