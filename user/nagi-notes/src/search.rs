use std::fmt;
use std::sync::Arc;

use nagi_model::{ObjectId, WorkspaceId};

use crate::app::{AppError, NotesApp};
use crate::domain::{NoteSummary, Timestamp};
use crate::store::{NoteStore, StoreError};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchRecord {
    pub object_id: ObjectId,
    pub title: String,
    pub text: String,
    pub modified_at: Timestamp,
    pub workspace_id: Option<WorkspaceId>,
    pub workspace_ids: Vec<WorkspaceId>,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchHit {
    pub object_id: ObjectId,
    pub block_id: Option<ObjectId>,
    pub title: String,
    pub snippet: String,
    pub modified_at: Timestamp,
    pub workspace_id: Option<WorkspaceId>,
    pub workspace_ids: Vec<WorkspaceId>,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SearchError {
    Store(StoreError),
    App(AppError),
}

impl fmt::Display for SearchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Store(error) => error.fmt(f),
            Self::App(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for SearchError {}

pub trait SearchProvider: Send + Sync {
    fn records(&self) -> Result<Vec<SearchRecord>, SearchError>;
    fn search(&self, query: &str) -> Result<Vec<SearchHit>, SearchError>;
}

/// Search adapter for the shared Nagi Search service. It emits only stable
/// object/block identities, title, text, modification time, and workspace
/// metadata. It does not own or embed a search index.
pub struct NotesSearchProvider {
    store: Option<Arc<dyn NoteStore>>,
    app: Option<Arc<NotesApp>>,
}

impl NotesSearchProvider {
    pub fn new(store: Arc<dyn NoteStore>) -> Self {
        Self {
            store: Some(store),
            app: None,
        }
    }

    /// Include dirty open documents immediately; the shared search service
    /// remains responsible for indexing and ranking provider records.
    pub fn for_app(app: Arc<NotesApp>) -> Self {
        Self {
            store: None,
            app: Some(app),
        }
    }

    fn list_notes(&self) -> Result<Vec<NoteSummary>, SearchError> {
        if let Some(app) = &self.app {
            app.list_notes().map_err(SearchError::App)
        } else {
            self.store
                .as_ref()
                .expect("store-backed provider has a store")
                .list(false)
                .map_err(SearchError::Store)
        }
    }

    fn load_note(&self, id: ObjectId) -> Result<Option<crate::domain::NoteDocument>, SearchError> {
        if let Some(app) = &self.app {
            match app.get_note(id) {
                Ok(note) => Ok(Some(note)),
                Err(AppError::NotFound(_)) => Ok(None),
                Err(error) => Err(SearchError::App(error)),
            }
        } else {
            self.store
                .as_ref()
                .expect("store-backed provider has a store")
                .load(id)
                .map_err(SearchError::Store)
        }
    }
}

impl SearchProvider for NotesSearchProvider {
    fn records(&self) -> Result<Vec<SearchRecord>, SearchError> {
        self.list_notes()?
            .into_iter()
            .map(|summary: NoteSummary| {
                self.load_note(summary.id)?
                    .map(|note| {
                        let text = note.search_text();
                        SearchRecord {
                            object_id: note.id,
                            title: note.title,
                            text,
                            modified_at: note.updated_at,
                            workspace_id: note.workspace_id,
                            workspace_ids: note.workspace_ids,
                            tags: note.tags,
                        }
                    })
                    .ok_or({
                        SearchError::Store(StoreError::InvalidDocument(
                            "listed note disappeared while building search record",
                        ))
                    })
            })
            .collect()
    }

    fn search(&self, query: &str) -> Result<Vec<SearchHit>, SearchError> {
        let terms = query
            .split_whitespace()
            .map(str::to_lowercase)
            .collect::<Vec<_>>();
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        let mut hits = Vec::new();
        for summary in self.list_notes()? {
            let note = self.load_note(summary.id)?.ok_or({
                SearchError::Store(StoreError::InvalidDocument(
                    "listed note disappeared while searching",
                ))
            })?;
            let title_matches = contains_terms(&note.title, &terms);
            let matching_tags = note
                .tags
                .iter()
                .filter(|tag| contains_terms(tag, &terms))
                .cloned()
                .collect::<Vec<_>>();
            if title_matches || !matching_tags.is_empty() {
                hits.push(SearchHit {
                    object_id: note.id,
                    block_id: None,
                    title: note.title.clone(),
                    snippet: if title_matches {
                        note.title.clone()
                    } else {
                        matching_tags.join(", ")
                    },
                    modified_at: note.updated_at,
                    workspace_id: note.workspace_id,
                    workspace_ids: note.workspace_ids.clone(),
                    tags: note.tags.clone(),
                });
            }
            for block in &note.blocks {
                let text = block.search_text();
                if contains_terms(&text, &terms) {
                    hits.push(SearchHit {
                        object_id: note.id,
                        block_id: Some(block.id),
                        title: note.title.clone(),
                        snippet: bounded_snippet(&text, 160),
                        modified_at: note.updated_at,
                        workspace_id: note.workspace_id,
                        workspace_ids: note.workspace_ids.clone(),
                        tags: note.tags.clone(),
                    });
                }
            }
        }
        hits.sort_by(|left, right| {
            right
                .modified_at
                .cmp(&left.modified_at)
                .then_with(|| left.object_id.0.cmp(&right.object_id.0))
                .then_with(|| {
                    left.block_id
                        .map(|id| id.0)
                        .cmp(&right.block_id.map(|id| id.0))
                })
        });
        Ok(hits)
    }
}

fn contains_terms(text: &str, terms: &[String]) -> bool {
    let text = text.to_lowercase();
    terms.iter().all(|term| text.contains(term))
}

fn bounded_snippet(text: &str, limit: usize) -> String {
    let mut snippet = text.chars().take(limit).collect::<String>();
    if text.chars().count() > limit {
        snippet.push('…');
    }
    snippet
}
