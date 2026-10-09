//! UI-independent, host-only Writer engine. No filesystem, clock, network or
//! authority is created here. IDs and provenance are supplied by the caller.
pub mod adapters;
pub mod formats;
pub mod model;
pub mod operations;
pub use model::*;
pub use nagi_history::activity::{Actor, RevisionId};
pub use nagi_model::ObjectId;
pub use operations::*;
