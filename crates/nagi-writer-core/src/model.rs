use crate::{Actor, ObjectId, RevisionId};
use std::collections::{BTreeMap, BTreeSet};

/// Temporary document-role adapter over canonical ObjectId, matching Notes'
/// logical identity. A future platform DocumentId migration preserves the u64.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DocumentId(pub ObjectId);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidId,
    DuplicateId(ObjectId),
    MissingObject(ObjectId),
    InvalidParent,
    InvalidIndex,
    InvalidTextRange,
    InvalidStructure,
    InvalidStyle,
    StyleCycle,
    Conflict {
        expected: RevisionId,
        actual: RevisionId,
    },
    LimitExceeded,
    InvalidReview,
    Unsupported,
    Unavailable,
    PermissionDenied,
}

/// Bounds include snapshots, issued IDs and revision count; there is no
/// automatic eviction of history or tombstones that could silently lose data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Limits {
    pub max_bytes: usize,
    pub max_objects: usize,
    pub max_revisions: usize,
    pub max_operations: usize,
    pub max_table_cells: usize,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_bytes: 1_048_576,
            max_objects: 4096,
            max_revisions: 256,
            max_operations: 1024,
            max_table_cells: 16384,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Metadata {
    pub title: String,
    pub language: String,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Settings {
    pub columns: u16,
    pub landscape: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            columns: 1,
            landscape: false,
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Reference {
    /// Existing platform resource identity; never a path or URL as identity.
    pub resource: ObjectId,
    pub revision: Option<RevisionId>,
    pub object: Option<ObjectId>,
    pub label: String,
    /// Untrusted locator; never fetched or executed by the core.
    pub locator: Option<String>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BlockKind {
    Paragraph(String),
    Heading {
        level: u8,
        text: String,
    },
    List {
        ordered: bool,
        items: Vec<String>,
    },
    Table(Table),
    Quote(String),
    Code {
        language: String,
        text: String,
    },
    Citation(Reference),
    LinkedReference(Reference),
    Figure {
        alt: String,
        source: Option<Reference>,
    },
}
impl BlockKind {
    pub fn text(&self) -> Option<&str> {
        match self {
            Self::Paragraph(t)
            | Self::Heading { text: t, .. }
            | Self::Quote(t)
            | Self::Code { text: t, .. } => Some(t),
            _ => None,
        }
    }
    pub(crate) fn text_mut(&mut self) -> Option<&mut String> {
        match self {
            Self::Paragraph(t)
            | Self::Heading { text: t, .. }
            | Self::Quote(t)
            | Self::Code { text: t, .. } => Some(t),
            _ => None,
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Table {
    pub rows: Vec<Vec<String>>,
}
impl Table {
    pub fn new(rows: Vec<Vec<String>>) -> Result<Self, Error> {
        let table = Self { rows };
        table.validate(usize::MAX)?;
        Ok(table)
    }
    pub fn validate(&self, max: usize) -> Result<(), Error> {
        let width = self.rows.first().map_or(0, Vec::len);
        if width == 0 || self.rows.iter().any(|r| r.len() != width) {
            return Err(Error::InvalidStructure);
        }
        if self
            .rows
            .len()
            .checked_mul(width)
            .ok_or(Error::LimitExceeded)?
            > max
        {
            return Err(Error::LimitExceeded);
        }
        Ok(())
    }
    pub fn insert_row(&mut self, index: usize, cells: Vec<String>) -> Result<(), Error> {
        self.validate(usize::MAX)?;
        if index > self.rows.len() || cells.len() != self.rows[0].len() {
            return Err(Error::InvalidIndex);
        }
        self.rows.insert(index, cells);
        Ok(())
    }
    pub fn delete_row(&mut self, index: usize) -> Result<(), Error> {
        self.validate(usize::MAX)?;
        if self.rows.len() <= 1 || index >= self.rows.len() {
            return Err(Error::InvalidIndex);
        }
        self.rows.remove(index);
        Ok(())
    }
    pub fn insert_column(&mut self, index: usize) -> Result<(), Error> {
        self.validate(usize::MAX)?;
        if index > self.rows[0].len() {
            return Err(Error::InvalidIndex);
        }
        for row in &mut self.rows {
            row.insert(index, String::new());
        }
        Ok(())
    }
    pub fn delete_column(&mut self, index: usize) -> Result<(), Error> {
        self.validate(usize::MAX)?;
        if self.rows[0].len() <= 1 || index >= self.rows[0].len() {
            return Err(Error::InvalidIndex);
        }
        for row in &mut self.rows {
            row.remove(index);
        }
        Ok(())
    }
    pub fn set_cell(&mut self, row: usize, column: usize, text: String) -> Result<(), Error> {
        self.validate(usize::MAX)?;
        *self
            .rows
            .get_mut(row)
            .and_then(|r| r.get_mut(column))
            .ok_or(Error::InvalidIndex)? = text;
        Ok(())
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Block {
    pub id: ObjectId,
    pub style: String,
    pub kind: BlockKind,
}
impl Block {
    pub fn new(id: ObjectId, kind: BlockKind) -> Self {
        let style = match &kind {
            BlockKind::Heading { level, .. } => format!("Heading{}", (*level).min(3)),
            BlockKind::Quote(_) => "Quote".into(),
            BlockKind::Code { .. } => "Code".into(),
            _ => "Body".into(),
        };
        Self { id, style, kind }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Section {
    pub id: ObjectId,
    pub title: String,
    pub blocks: Vec<Block>,
}
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Formatting {
    pub bold: Option<bool>,
    pub italic: Option<bool>,
    pub size_points: Option<u16>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Style {
    pub parent: Option<String>,
    pub formatting: Formatting,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Message {
    pub author: Actor,
    pub time: u64,
    pub text: String,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Comment {
    pub id: ObjectId,
    pub object: ObjectId,
    pub messages: Vec<Message>,
    pub resolved: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReviewStatus {
    Pending,
    Accepted,
    Rejected,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrackedChange {
    pub id: ObjectId,
    pub base: RevisionId,
    pub operations: Vec<crate::Operation>,
    pub provenance: Provenance,
    pub status: ReviewStatus,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Provenance {
    pub actor: Actor,
    pub source: String,
    pub time: u64,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Document {
    pub id: DocumentId,
    pub revision: RevisionId,
    pub metadata: Metadata,
    pub settings: Settings,
    pub sections: Vec<Section>,
    pub styles: BTreeMap<String, Style>,
    pub comments: Vec<Comment>,
    pub tracked_changes: Vec<TrackedChange>,
    /// Includes deleted identities. Save/open adapters must preserve this set.
    pub issued_ids: BTreeSet<u64>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutlineItem {
    pub id: ObjectId,
    pub section: ObjectId,
    pub level: u8,
    pub text: String,
}
impl Document {
    pub fn new(id: DocumentId, revision: RevisionId, title: String) -> Result<Self, Error> {
        valid_id(id.0)?;
        valid_revision(revision)?;
        let mut styles = BTreeMap::new();
        styles.insert(
            "Body".into(),
            Style {
                parent: None,
                formatting: Formatting::default(),
            },
        );
        for name in [
            "Title", "Subtitle", "Heading1", "Heading2", "Heading3", "Quote", "Caption", "Code",
        ] {
            styles.insert(
                name.into(),
                Style {
                    parent: Some("Body".into()),
                    formatting: Formatting {
                        bold: matches!(name, "Title" | "Heading1" | "Heading2" | "Heading3")
                            .then_some(true),
                        italic: (name == "Quote").then_some(true),
                        size_points: match name {
                            "Title" => Some(28),
                            "Subtitle" => Some(18),
                            "Heading1" => Some(22),
                            "Heading2" => Some(18),
                            "Heading3" => Some(14),
                            "Caption" => Some(10),
                            _ => None,
                        },
                    },
                },
            );
        }
        Ok(Self {
            id,
            revision,
            metadata: Metadata {
                title,
                language: "en-US".into(),
            },
            settings: Settings::default(),
            sections: vec![],
            styles,
            comments: vec![],
            tracked_changes: vec![],
            issued_ids: BTreeSet::from([id.0 .0]),
        })
    }
    pub fn block(&self, id: ObjectId) -> Option<&Block> {
        self.sections
            .iter()
            .flat_map(|s| &s.blocks)
            .find(|b| b.id == id)
    }
    pub(crate) fn block_mut(&mut self, id: ObjectId) -> Result<&mut Block, Error> {
        self.sections
            .iter_mut()
            .flat_map(|s| &mut s.blocks)
            .find(|b| b.id == id)
            .ok_or(Error::MissingObject(id))
    }
    pub fn outline(&self) -> Vec<OutlineItem> {
        self.sections
            .iter()
            .flat_map(|s| {
                s.blocks.iter().filter_map(|b| {
                    if let BlockKind::Heading { level, text } = &b.kind {
                        Some(OutlineItem {
                            id: b.id,
                            section: s.id,
                            level: *level,
                            text: text.clone(),
                        })
                    } else {
                        None
                    }
                })
            })
            .collect()
    }
    pub fn effective_style(&self, name: &str) -> Result<Formatting, Error> {
        let mut chain = vec![];
        let mut next = Some(name);
        let mut seen = BTreeSet::new();
        while let Some(n) = next {
            if !seen.insert(n) {
                return Err(Error::StyleCycle);
            }
            let s = self.styles.get(n).ok_or(Error::InvalidStyle)?;
            chain.push(&s.formatting);
            next = s.parent.as_deref();
        }
        let mut result = Formatting::default();
        for f in chain.into_iter().rev() {
            if f.bold.is_some() {
                result.bold = f.bold;
            }
            if f.italic.is_some() {
                result.italic = f.italic;
            }
            if f.size_points.is_some() {
                result.size_points = f.size_points;
            }
        }
        Ok(result)
    }
    pub(crate) fn issue(&mut self, id: ObjectId) -> Result<(), Error> {
        valid_id(id)?;
        if !self.issued_ids.insert(id.0) {
            return Err(Error::DuplicateId(id));
        }
        Ok(())
    }
    pub fn validate(&self, limits: Limits) -> Result<(), Error> {
        valid_id(self.id.0)?;
        valid_revision(self.revision)?;
        if self.settings.columns == 0
            || self.styles.len() > limits.max_objects
            || self.issued_ids.len() > limits.max_objects
        {
            return Err(Error::LimitExceeded);
        }
        for id in &self.issued_ids {
            valid_id(ObjectId(*id))?;
        }
        let mut ids = BTreeSet::new();
        let mut bytes = self
            .metadata
            .title
            .len()
            .saturating_add(self.metadata.language.len());
        let mut check = |id| -> Result<(), Error> {
            valid_id(id)?;
            if !ids.insert(id.0) {
                return Err(Error::DuplicateId(id));
            }
            if !self.issued_ids.contains(&id.0) {
                return Err(Error::InvalidId);
            }
            Ok(())
        };
        check(self.id.0)?;
        for (name, style) in &self.styles {
            if name.is_empty() || style.formatting.size_points == Some(0) {
                return Err(Error::InvalidStyle);
            }
            self.effective_style(name)?;
            bytes = bytes
                .saturating_add(name.len())
                .saturating_add(style.parent.as_ref().map_or(0, String::len));
        }
        for s in &self.sections {
            check(s.id)?;
            bytes = bytes.saturating_add(s.title.len());
            for b in &s.blocks {
                check(b.id)?;
                self.effective_style(&b.style)?;
                bytes = bytes.saturating_add(b.style.len());
                match &b.kind {
                    BlockKind::Heading { level, .. } if !(1..=6).contains(level) => {
                        return Err(Error::InvalidStructure)
                    }
                    BlockKind::Table(t) => t.validate(limits.max_table_cells)?,
                    BlockKind::List { items, .. } if items.len() > limits.max_objects => {
                        return Err(Error::LimitExceeded)
                    }
                    BlockKind::Citation(r) | BlockKind::LinkedReference(r) => {
                        validate_reference(r)?
                    }
                    BlockKind::Figure {
                        source: Some(r), ..
                    } => validate_reference(r)?,
                    _ => (),
                }
                bytes = bytes.saturating_add(block_bytes(&b.kind));
                let nodes = match &b.kind {
                    BlockKind::Table(t) => t.rows.len().saturating_mul(t.rows[0].len()),
                    BlockKind::List { items, .. } => items.len(),
                    _ => 0,
                };
                bytes = bytes.saturating_add(nodes.saturating_mul(24));
            }
        }
        for c in &self.comments {
            check(c.id)?;
            if c.messages.len() > limits.max_objects {
                return Err(Error::LimitExceeded);
            }
            // Deletion keeps an anchored, inspectable orphan thread, never reanchors it.
            if !self.issued_ids.contains(&c.object.0)
                || c.object == self.id.0
                || c.messages.is_empty()
            {
                return Err(Error::InvalidStructure);
            }
            for m in &c.messages {
                bytes = bytes.saturating_add(m.text.len()).saturating_add(32);
            }
        }
        for t in &self.tracked_changes {
            check(t.id)?;
            valid_revision(t.base)?;
            if t.operations.is_empty()
                || t.operations.len() > limits.max_operations
                || t.operations.iter().any(|o| !o.is_content())
            {
                return Err(Error::InvalidReview);
            }
            bytes = bytes.saturating_add(t.provenance.source.len());
            for op in &t.operations {
                bytes = bytes.saturating_add(op.payload_bytes());
            }
        }
        if bytes > limits.max_bytes {
            return Err(Error::LimitExceeded);
        }
        Ok(())
    }
}
pub(crate) fn valid_id(id: ObjectId) -> Result<(), Error> {
    if id.0 == 0 || id.0 == u64::MAX {
        Err(Error::InvalidId)
    } else {
        Ok(())
    }
}
pub(crate) fn valid_revision(id: RevisionId) -> Result<(), Error> {
    RevisionId::new(id.0)
        .map(|_| ())
        .map_err(|_| Error::InvalidId)
}
pub(crate) fn block_bytes(kind: &BlockKind) -> usize {
    match kind {
        BlockKind::Paragraph(t) | BlockKind::Heading { text: t, .. } | BlockKind::Quote(t) => {
            t.len()
        }
        BlockKind::Code { language, text } => language.len().saturating_add(text.len()),
        BlockKind::List { items, .. } => {
            items.iter().fold(0usize, |n, s| n.saturating_add(s.len()))
        }
        BlockKind::Table(t) => t
            .rows
            .iter()
            .flatten()
            .fold(0usize, |n, s| n.saturating_add(s.len())),
        BlockKind::Citation(r) | BlockKind::LinkedReference(r) => r
            .label
            .len()
            .saturating_add(r.locator.as_ref().map_or(0, String::len)),
        BlockKind::Figure { alt, source } => {
            alt.len().saturating_add(source.as_ref().map_or(0, |r| {
                r.label
                    .len()
                    .saturating_add(r.locator.as_ref().map_or(0, String::len))
            }))
        }
    }
}

fn validate_reference(r: &Reference) -> Result<(), Error> {
    valid_id(r.resource)?;
    if let Some(object) = r.object {
        valid_id(object)?;
    }
    if let Some(revision) = r.revision {
        valid_revision(revision)?;
    }
    Ok(())
}
