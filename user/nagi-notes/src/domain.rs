use nagi_model::{ObjectId, WorkspaceId as NagiWorkspaceId};

pub type NoteId = ObjectId;
pub type WorkspaceId = NagiWorkspaceId;
pub type NoteRevision = u64;

#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct Timestamp(pub u64);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NoteDocument {
    pub id: NoteId,
    pub title: String,
    pub blocks: Vec<Block>,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    /// Revision zero means the note has not been persisted yet.
    pub revision: NoteRevision,
    /// Current workspace context for activity and presentation.
    pub workspace_id: Option<WorkspaceId>,
    /// Notes may participate in more than one semantic workspace.
    pub workspace_ids: Vec<WorkspaceId>,
    pub tags: Vec<String>,
    pub deleted: bool,
}

impl NoteDocument {
    pub fn new(id: NoteId, title: impl Into<String>, now: Timestamp) -> Self {
        Self {
            id,
            title: title.into(),
            blocks: Vec::new(),
            created_at: now,
            updated_at: now,
            revision: 0,
            workspace_id: None,
            workspace_ids: Vec::new(),
            tags: Vec::new(),
            deleted: false,
        }
    }

    pub fn search_text(&self) -> String {
        self.blocks
            .iter()
            .map(Block::search_text)
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn block(&self, id: ObjectId) -> Option<&Block> {
        self.blocks.iter().find(|block| block.id == id)
    }

    pub fn block_mut(&mut self, id: ObjectId) -> Option<&mut Block> {
        self.blocks.iter_mut().find(|block| block.id == id)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Block {
    pub id: ObjectId,
    pub kind: BlockKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AlbertReference {
    pub page_id: ObjectId,
    pub title: String,
    pub url: String,
    pub selection: Option<String>,
}

impl Block {
    pub fn new(id: ObjectId, kind: BlockKind) -> Self {
        Self { id, kind }
    }

    pub fn search_text(&self) -> String {
        match &self.kind {
            BlockKind::Heading { text, .. }
            | BlockKind::Paragraph(text)
            | BlockKind::Quote(text)
            | BlockKind::Callout { text, .. } => text.clone(),
            BlockKind::Checklist { text, .. } => text.clone(),
            BlockKind::Code { text, .. } => text.clone(),
            BlockKind::ImageReference { alt, target } => format!("{alt} {target}"),
            BlockKind::FileReference { label, object_id } => {
                format!("{label} {:016x}", object_id.0)
            }
            BlockKind::WebReference { title, url } => format!("{title} {url}"),
            BlockKind::AlbertReference(reference) => format!(
                "{} {} {}",
                reference.title,
                reference.url,
                reference.selection.as_deref().unwrap_or_default()
            ),
            BlockKind::Table { rows } => rows
                .iter()
                .flat_map(|row| row.iter())
                .cloned()
                .collect::<Vec<_>>()
                .join(" "),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BlockKind {
    Heading {
        level: u8,
        text: String,
    },
    Paragraph(String),
    Checklist {
        text: String,
        completed: bool,
    },
    Code {
        language: Option<String>,
        text: String,
    },
    Quote(String),
    ImageReference {
        alt: String,
        target: String,
    },
    FileReference {
        label: String,
        object_id: ObjectId,
    },
    WebReference {
        title: String,
        url: String,
    },
    AlbertReference(AlbertReference),
    Table {
        rows: Vec<Vec<String>>,
    },
    Callout {
        kind: String,
        text: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NoteSummary {
    pub id: NoteId,
    pub title: String,
    pub updated_at: Timestamp,
    pub revision: NoteRevision,
    pub workspace_id: Option<WorkspaceId>,
    pub workspace_ids: Vec<WorkspaceId>,
    pub tags: Vec<String>,
    pub deleted: bool,
}
