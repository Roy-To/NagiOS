use crate::model::ClipboardGeneration;

/// Stable diagnostic event codes. No payload, media type, metadata, origin
/// label, or caller identity is ever attached.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClipboardEventCode {
    /// Content replaced.
    Written,
    /// Content cleared.
    Cleared,
    /// A representation was read.
    RepresentationRead,
    /// An operation was denied by the authorizer.
    Denied,
    /// The authorizer was unavailable; failed closed.
    AuthorizationUnavailable,
    /// Content failed validation.
    Rejected,
    /// A stale expected generation was supplied.
    StaleGeneration,
    /// An unsupported contract version was requested.
    UnsupportedVersion,
}

/// One bounded metadata-only diagnostic event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClipboardEvent {
    /// Stable code.
    pub code: ClipboardEventCode,
    /// Clipboard generation after the operation.
    pub generation: ClipboardGeneration,
}

/// Optional failure-tolerant diagnostics sink.
///
/// No sink is installed by default: the reference service emits nothing
/// unless an owner explicitly wires one. A sink error never changes the
/// outcome of a clipboard operation.
pub trait ClipboardDiagnosticsSink {
    /// Record one event.
    fn record(&mut self, event: ClipboardEvent) -> Result<(), SinkError>;
}

/// The sink could not record an event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SinkError;
