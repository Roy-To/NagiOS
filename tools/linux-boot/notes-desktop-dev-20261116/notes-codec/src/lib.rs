#![allow(dead_code)]

#[path = "../inputs/nagi-notes/src/activity.rs"]
mod activity;
#[path = "../inputs/nagi-notes/src/app.rs"]
mod app;
#[path = "../inputs/nagi-notes/src/domain.rs"]
mod domain;
#[path = "../inputs/nagi-notes/src/identity.rs"]
mod identity;
mod markdown;
#[path = "../inputs/nagi-notes/src/references.rs"]
mod references;
#[path = "../inputs/nagi-notes/src/markdown.rs"]
mod retained_markdown;
#[path = "../inputs/nagi-notes/src/session.rs"]
mod session;
mod store;

pub mod ui_controller;

pub use activity::{
    ActivityError, ActivityEvent, ActivityKind, ActivityOrigin, ActivitySink, NoopActivitySink,
};
pub use app::{AppError, NotesApp, OpenDocument};
pub use domain::{
    AlbertReference, Block, BlockKind, NoteDocument, NoteId, NoteRevision, NoteSummary, Timestamp,
    WorkspaceId,
};
pub use identity::{HostObjectIdSource, ObjectIdSource};
pub use markdown::{export_markdown, import_markdown, MarkdownError, StoredNoteError};
pub use nagi_model::{AppId, ObjectId, WorkspaceId as NagiWorkspaceId};
pub use session::{Clock, NoteSession, SaveError, SaveStatus, SessionError, SystemClock};
/// Path-based host preview persistence for explicit developer-owned test roots only.
/// It is susceptible to concurrent path replacement and is not an authorization or
/// production trust boundary. Linux production callers should use the fd-backed store
/// with a root descriptor supplied by their authorized storage provider.
pub use store::HostPreviewStore;
#[cfg(target_os = "linux")]
pub use store::LinuxDirectoryNoteStore;
pub use store::{
    InMemoryNoteStore, NoteSessionEffectGate, NoteStore, SandboxError, SessionGateError,
    SessionGatedNoteStore, StoreError,
};
#[cfg(target_os = "linux")]
pub use store::{LinuxBlockDevice, SharedVfs, VfsNoteStore};
pub use ui_controller::NotesUiController;

#[cfg(test)]
mod session_gate_tests;
#[cfg(test)]
mod stored_integrity_tests;
#[cfg(test)]
mod tests;
