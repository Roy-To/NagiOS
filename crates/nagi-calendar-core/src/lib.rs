//! Host-only Calendar contracts and deterministic reference algorithms.
//! No provider delivery, authentication, notification dispatch or guest persistence.
pub mod adapter;
pub mod availability;
pub mod model;
pub mod recurrence;
pub mod store;
pub mod time;

pub use nagi_model::{AppId, ObjectId, UserId, WorkspaceId};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidId,
    InvalidText,
    Capacity,
    NotFound,
    CalendarNotEmpty,
    DuplicateId,
    RevisionConflict,
    Overflow,
    InvalidDate,
    InvalidRange,
    PrecisionLoss,
    ClockUnavailable,
    ClockRollback,
    UnknownZone,
    ResolverUnavailable,
    InvalidResolver,
    NonexistentLocalTime,
    AmbiguousLocalTime,
    ZoneContextRequired,
    InvalidRecurrence,
    InvalidException,
    ScanLimitExceeded,
    OutputLimitExceeded,
    InvalidLimit,
    InvalidStep,
    UnknownAvailability,
    ExternalActionUnavailable,
}
pub type Result<T> = std::result::Result<T, Error>;
