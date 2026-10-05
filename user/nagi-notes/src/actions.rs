use std::fmt;
use std::sync::Arc;

use nagi_model::{AppSessionId, ObjectId, WorkspaceId};

use crate::activity::ActivityOrigin;
use crate::app::{AppError, NotesApp};
use crate::domain::{Block, BlockKind, NoteDocument};
use crate::references::{object_id_from_uri, valid_image_target, valid_web_url};
use crate::search::{SearchError, SearchHit, SearchProvider};
use crate::session::SessionError;
use crate::store::StoreError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ActionPrincipal {
    User,
    Agent(AppSessionId),
}

impl ActionPrincipal {
    fn origin(self) -> ActivityOrigin {
        match self {
            Self::User => ActivityOrigin::User,
            Self::Agent(session_id) => ActivityOrigin::Agent(session_id),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NotesActionKind {
    Create,
    Open,
    Get,
    Search,
    AppendBlock,
    InsertBlock,
    UpdateBlock,
    DeleteBlock,
    MoveBlock,
    AddReference,
    RemoveReference,
    SetTags,
    AddToWorkspace,
    RemoveFromWorkspace,
}

impl NotesActionKind {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Create => "notes.create",
            Self::Open => "notes.open",
            Self::Get => "notes.get",
            Self::Search => "notes.search",
            Self::AppendBlock => "notes.append_block",
            Self::InsertBlock => "notes.insert_block",
            Self::UpdateBlock => "notes.update_block",
            Self::DeleteBlock => "notes.delete_block",
            Self::MoveBlock => "notes.move_block",
            Self::AddReference => "notes.add_reference",
            Self::RemoveReference => "notes.remove_reference",
            Self::SetTags => "notes.set_tags",
            Self::AddToWorkspace => "notes.add_to_workspace",
            Self::RemoveFromWorkspace => "notes.remove_from_workspace",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NotesAction {
    Create {
        title: String,
    },
    Open {
        note_id: ObjectId,
    },
    Get {
        note_id: ObjectId,
    },
    Search {
        query: String,
    },
    AppendBlock {
        note_id: ObjectId,
        block: Block,
    },
    InsertBlock {
        note_id: ObjectId,
        index: usize,
        block: Block,
    },
    UpdateBlock {
        note_id: ObjectId,
        block_id: ObjectId,
        kind: BlockKind,
    },
    DeleteBlock {
        note_id: ObjectId,
        block_id: ObjectId,
    },
    MoveBlock {
        note_id: ObjectId,
        block_id: ObjectId,
        to: usize,
    },
    AddReference {
        note_id: ObjectId,
        block: Block,
    },
    RemoveReference {
        note_id: ObjectId,
        block_id: ObjectId,
    },
    SetTags {
        note_id: ObjectId,
        tags: Vec<String>,
    },
    AddToWorkspace {
        note_id: ObjectId,
        workspace_id: WorkspaceId,
    },
    RemoveFromWorkspace {
        note_id: ObjectId,
        workspace_id: WorkspaceId,
    },
}

impl NotesAction {
    pub fn kind(&self) -> NotesActionKind {
        match self {
            Self::Create { .. } => NotesActionKind::Create,
            Self::Open { .. } => NotesActionKind::Open,
            Self::Get { .. } => NotesActionKind::Get,
            Self::Search { .. } => NotesActionKind::Search,
            Self::AppendBlock { .. } => NotesActionKind::AppendBlock,
            Self::InsertBlock { .. } => NotesActionKind::InsertBlock,
            Self::UpdateBlock { .. } => NotesActionKind::UpdateBlock,
            Self::DeleteBlock { .. } => NotesActionKind::DeleteBlock,
            Self::MoveBlock { .. } => NotesActionKind::MoveBlock,
            Self::AddReference { .. } => NotesActionKind::AddReference,
            Self::RemoveReference { .. } => NotesActionKind::RemoveReference,
            Self::SetTags { .. } => NotesActionKind::SetTags,
            Self::AddToWorkspace { .. } => NotesActionKind::AddToWorkspace,
            Self::RemoveFromWorkspace { .. } => NotesActionKind::RemoveFromWorkspace,
        }
    }

    fn object_id(&self) -> Option<ObjectId> {
        match self {
            Self::Open { note_id }
            | Self::Get { note_id }
            | Self::AppendBlock { note_id, .. }
            | Self::InsertBlock { note_id, .. }
            | Self::UpdateBlock { note_id, .. }
            | Self::DeleteBlock { note_id, .. }
            | Self::MoveBlock { note_id, .. }
            | Self::AddReference { note_id, .. }
            | Self::RemoveReference { note_id, .. }
            | Self::SetTags { note_id, .. }
            | Self::AddToWorkspace { note_id, .. }
            | Self::RemoveFromWorkspace { note_id, .. } => Some(*note_id),
            Self::Create { .. } | Self::Search { .. } => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActionPolicyError {
    Denied,
    ProviderUnavailable(String),
}

impl fmt::Display for ActionPolicyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Denied => f.write_str("Notes action was denied"),
            Self::ProviderUnavailable(message) => {
                write!(f, "Notes authorization provider failed: {message}")
            }
        }
    }
}

impl std::error::Error for ActionPolicyError {}

/// Production callers inject the Nagi Capability/Permission adapter here.
/// There is intentionally no permissive default implementation.
pub trait NotesActionPolicy: Send + Sync {
    fn authorize(
        &self,
        principal: ActionPrincipal,
        action: NotesActionKind,
        object_id: Option<ObjectId>,
    ) -> Result<(), ActionPolicyError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActionError {
    Policy(ActionPolicyError),
    App(AppError),
    Search(SearchError),
    InvalidReference,
}

impl fmt::Display for ActionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Policy(error) => error.fmt(f),
            Self::App(error) => error.fmt(f),
            Self::Search(error) => error.fmt(f),
            Self::InvalidReference => f.write_str("block is not a supported reference"),
        }
    }
}

impl std::error::Error for ActionError {}

impl From<AppError> for ActionError {
    fn from(error: AppError) -> Self {
        Self::App(error)
    }
}

impl From<SessionError> for ActionError {
    fn from(error: SessionError) -> Self {
        Self::App(AppError::Session(error))
    }
}

impl From<SearchError> for ActionError {
    fn from(error: SearchError) -> Self {
        Self::Search(error)
    }
}

impl From<StoreError> for ActionError {
    fn from(error: StoreError) -> Self {
        Self::App(AppError::Store(error))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActionResult {
    Document(NoteDocument),
    SearchResults(Vec<SearchHit>),
}

pub struct NotesActionExecutor {
    app: Arc<NotesApp>,
    search: Arc<dyn SearchProvider>,
    policy: Arc<dyn NotesActionPolicy>,
}

impl NotesActionExecutor {
    pub fn new(
        app: Arc<NotesApp>,
        search: Arc<dyn SearchProvider>,
        policy: Arc<dyn NotesActionPolicy>,
    ) -> Self {
        Self {
            app,
            search,
            policy,
        }
    }

    pub fn execute(
        &self,
        principal: ActionPrincipal,
        action: NotesAction,
    ) -> Result<ActionResult, ActionError> {
        let kind = action.kind();
        self.policy
            .authorize(principal, kind, action.object_id())
            .map_err(ActionError::Policy)?;
        let origin = principal.origin();
        match action {
            NotesAction::Create { title } => {
                let session = self.app.create_note_by(origin, title)?;
                Ok(ActionResult::Document(session.flush()?))
            }
            NotesAction::Open { note_id } => {
                let session = self.app.open_note_by(origin, note_id)?;
                Ok(ActionResult::Document(session.note()))
            }
            NotesAction::Get { note_id } => Ok(ActionResult::Document(self.app.get_note(note_id)?)),
            NotesAction::Search { query } => {
                let hits = self.search.search(&query)?;
                let mut visible = Vec::new();
                for hit in hits {
                    match self.policy.authorize(
                        principal,
                        NotesActionKind::Get,
                        Some(hit.object_id),
                    ) {
                        Ok(()) => visible.push(hit),
                        Err(ActionPolicyError::Denied) => {}
                        Err(error) => return Err(ActionError::Policy(error)),
                    }
                }
                Ok(ActionResult::SearchResults(visible))
            }
            NotesAction::AppendBlock { note_id, block } => {
                let session = self.app.open_note_by(origin, note_id)?;
                session.append_block_by(origin, block)?;
                Ok(ActionResult::Document(session.flush()?))
            }
            NotesAction::InsertBlock {
                note_id,
                index,
                block,
            } => {
                let session = self.app.open_note_by(origin, note_id)?;
                session.insert_block_by(origin, index, block)?;
                Ok(ActionResult::Document(session.flush()?))
            }
            NotesAction::UpdateBlock {
                note_id,
                block_id,
                kind,
            } => {
                let session = self.app.open_note_by(origin, note_id)?;
                session.update_block_by(origin, block_id, kind)?;
                Ok(ActionResult::Document(session.flush()?))
            }
            NotesAction::DeleteBlock { note_id, block_id } => {
                let session = self.app.open_note_by(origin, note_id)?;
                session.delete_block_by(origin, block_id)?;
                Ok(ActionResult::Document(session.flush()?))
            }
            NotesAction::MoveBlock {
                note_id,
                block_id,
                to,
            } => {
                let session = self.app.open_note_by(origin, note_id)?;
                session.move_block_by(origin, block_id, to)?;
                Ok(ActionResult::Document(session.flush()?))
            }
            NotesAction::AddReference { note_id, block } => {
                if !is_reference(&block.kind) {
                    return Err(ActionError::InvalidReference);
                }
                if let Some(referenced_object) = reference_object_id(&block.kind) {
                    self.policy
                        .authorize(principal, NotesActionKind::Get, Some(referenced_object))
                        .map_err(ActionError::Policy)?;
                }
                let session = self.app.open_note_by(origin, note_id)?;
                session.append_block_by(origin, block)?;
                Ok(ActionResult::Document(session.flush()?))
            }
            NotesAction::RemoveReference { note_id, block_id } => {
                let session = self.app.open_note_by(origin, note_id)?;
                let note = session.note();
                let block = note
                    .block(block_id)
                    .ok_or(ActionError::App(AppError::Session(
                        crate::session::SessionError::BlockNotFound(block_id),
                    )))?;
                if !is_reference(&block.kind) {
                    return Err(ActionError::InvalidReference);
                }
                session.delete_block_by(origin, block_id)?;
                Ok(ActionResult::Document(session.flush()?))
            }
            NotesAction::SetTags { note_id, tags } => {
                let session = self.app.open_note_by(origin, note_id)?;
                session.set_tags_by(origin, tags)?;
                Ok(ActionResult::Document(session.flush()?))
            }
            NotesAction::AddToWorkspace {
                note_id,
                workspace_id,
            } => {
                let session = self.app.open_note_by(origin, note_id)?;
                session.add_to_workspace(origin, workspace_id)?;
                Ok(ActionResult::Document(session.flush()?))
            }
            NotesAction::RemoveFromWorkspace {
                note_id,
                workspace_id,
            } => {
                let session = self.app.open_note_by(origin, note_id)?;
                session.remove_from_workspace(origin, workspace_id)?;
                Ok(ActionResult::Document(session.flush()?))
            }
        }
    }
}

fn is_reference(kind: &BlockKind) -> bool {
    match kind {
        BlockKind::FileReference { .. } => true,
        BlockKind::WebReference { url, .. } => valid_web_url(url),
        BlockKind::AlbertReference(reference) => valid_web_url(&reference.url),
        BlockKind::ImageReference { target, .. } => valid_image_target(target),
        _ => false,
    }
}

fn reference_object_id(kind: &BlockKind) -> Option<ObjectId> {
    match kind {
        BlockKind::FileReference { object_id, .. } => Some(*object_id),
        BlockKind::AlbertReference(reference) => Some(reference.page_id),
        BlockKind::ImageReference { target, .. } => object_id_from_uri(target),
        BlockKind::Heading { .. }
        | BlockKind::Paragraph(_)
        | BlockKind::Checklist { .. }
        | BlockKind::Code { .. }
        | BlockKind::Quote(_)
        | BlockKind::WebReference { .. }
        | BlockKind::Table { .. }
        | BlockKind::Callout { .. } => None,
    }
}
