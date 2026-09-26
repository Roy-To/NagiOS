//! Notes domain and host-preview implementation.
//!
//! The core depends on injected document, activity, clock, and identity
//! providers. HostPreviewStore is an explicitly host-only sandbox adapter;
//! it is not the Nagi production filesystem or a substitute for a Nagi
//! Storage capability.

mod actions;
mod activity;
mod app;
mod domain;
mod identity;
mod localization;
mod markdown;
mod references;
mod search;
mod session;
mod store;

pub use actions::{
    ActionError, ActionPolicyError, ActionPrincipal, ActionResult, NotesAction,
    NotesActionExecutor, NotesActionKind, NotesActionPolicy,
};
pub use activity::{
    ActivityError, ActivityEvent, ActivityKind, ActivityOrigin, ActivitySink, NoopActivitySink,
};
pub use app::{AppError, NotesApp, OpenDocument};
pub use domain::{
    AlbertReference, Block, BlockKind, NoteDocument, NoteId, NoteRevision, NoteSummary, Timestamp,
    WorkspaceId,
};
pub use identity::{HostObjectIdSource, ObjectIdSource};
pub use localization::{Locale, Localizer};
pub use markdown::{export_markdown, import_markdown, MarkdownError, StoredNoteError};
pub use search::{NotesSearchProvider, SearchError, SearchHit, SearchProvider, SearchRecord};
pub use session::{Clock, NoteSession, SaveError, SaveStatus, SessionError, SystemClock};
pub use store::{HostPreviewStore, InMemoryNoteStore, NoteStore, SandboxError, StoreError};

pub use nagi_model::{AppId, ObjectId, WorkspaceId as NagiWorkspaceId};

pub const APP_ID: AppId = AppId::from_identifier(b"com.nagi.notes");
