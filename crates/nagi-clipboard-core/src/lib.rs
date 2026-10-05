//! Host-only clipboard / data-transfer contract and bounded in-memory reference
//! (CLIP-01).
//!
//! This crate does not own compositor or window-manager clipboard ownership,
//! keyboard shortcuts, primary selection, drag-and-drop plumbing, a production
//! IPC transport, a target service, durable history, or capability policy.
//! Caller identity comes only from a [`CallerContext`] supplied by the trusted
//! transport boundary; origin fields inside clipboard content are untrusted
//! claims and never grant authority.
//!
//! Canonical identities (`AppId`, `AppSessionId`, `ExecutionInstanceId`,
//! `ObjectId`) are re-used from `nagi-model`; this crate defines none of its own.

mod authority;
mod codec;
mod diagnostics;
mod limits;
mod media_type;
mod model;
mod service;

pub use authority::*;
pub use codec::*;
pub use diagnostics::*;
pub use limits::*;
pub use media_type::*;
pub use model::*;
pub use service::*;

/// Re-exported canonical identities used by this contract.
pub use nagi_model::{AppId, AppSessionId, ExecutionInstanceId, ObjectId};
