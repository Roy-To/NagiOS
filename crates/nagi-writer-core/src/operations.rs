use crate::model::{block_bytes, valid_revision};
use crate::*;
use std::ops::Range;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Operation {
    InsertSection {
        id: ObjectId,
        index: usize,
        title: String,
    },
    InsertBlock {
        section: ObjectId,
        index: usize,
        block: Block,
    },
    /// Offsets are UTF-8 bytes, must be scalar boundaries; not grapheme indices.
    ReplaceText {
        object: ObjectId,
        range: Range<usize>,
        text: String,
    },
    ReplaceTable {
        object: ObjectId,
        table: Table,
    },
    MoveObject {
        object: ObjectId,
        section: Option<ObjectId>,
        index: usize,
    },
    DeleteObject {
        object: ObjectId,
    },
    ApplyStyle {
        object: ObjectId,
        style: String,
    },
    DefineStyle {
        name: String,
        style: Style,
    },
    SetMetadata(Metadata),
    SetSettings(Settings),
    AddComment {
        id: ObjectId,
        object: ObjectId,
        message: Message,
    },
    Reply {
        comment: ObjectId,
        message: Message,
    },
    Resolve {
        comment: ObjectId,
        resolved: bool,
    },
    Suggest {
        id: ObjectId,
        operations: Vec<Operation>,
    },
    Accept {
        change: ObjectId,
    },
    Reject {
        change: ObjectId,
    },
}
impl Operation {
    pub fn is_content(&self) -> bool {
        matches!(
            self,
            Self::InsertSection { .. }
                | Self::InsertBlock { .. }
                | Self::ReplaceText { .. }
                | Self::ReplaceTable { .. }
                | Self::MoveObject { .. }
                | Self::DeleteObject { .. }
                | Self::ApplyStyle { .. }
                | Self::DefineStyle { .. }
                | Self::SetMetadata(_)
                | Self::SetSettings(_)
        )
    }
    pub(crate) fn payload_bytes(&self) -> usize {
        match self {
            Self::InsertSection { title, .. } => title.len(),
            Self::InsertBlock { block, .. } => {
                block.style.len().saturating_add(block_bytes(&block.kind))
            }
            Self::ReplaceText { text, .. } => text.len(),
            Self::ReplaceTable { table, .. } => table
                .rows
                .iter()
                .flatten()
                .fold(0usize, |n, s| n.saturating_add(s.len())),
            Self::ApplyStyle { style, .. } => style.len(),
            Self::DefineStyle { name, style } => name
                .len()
                .saturating_add(style.parent.as_ref().map_or(0, String::len)),
            Self::SetMetadata(m) => m.title.len().saturating_add(m.language.len()),
            Self::AddComment { message, .. } | Self::Reply { message, .. } => message.text.len(),
            // The validation path rejects nested Suggest before traversing it.
            _ => 0,
        }
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChangeKind {
    Add,
    Delete,
    Move,
    Format,
    Text,
    Table,
    Metadata,
    Comment,
    Review,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Change {
    pub kind: ChangeKind,
    pub object: Option<ObjectId>,
    pub operation: Operation,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChangeSet {
    pub document: DocumentId,
    pub from: RevisionId,
    pub to: RevisionId,
    pub provenance: Provenance,
    pub changes: Vec<Change>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Revision {
    pub document: Document,
    pub changes: Option<ChangeSet>,
}
/// Owned in-memory history; returned revisions are immutable borrows. A preview
/// has no effects. Saving/reopening history is the injected provider's duty.
pub struct Engine {
    revisions: Vec<Revision>,
    limits: Limits,
}
impl Engine {
    pub fn open(document: Document, limits: Limits) -> Result<Self, Error> {
        document.validate(limits)?;
        if limits.max_revisions == 0 {
            return Err(Error::LimitExceeded);
        }
        Ok(Self {
            revisions: vec![Revision {
                document,
                changes: None,
            }],
            limits,
        })
    }
    pub fn document(&self) -> &Document {
        &self.revisions.last().expect("initial revision").document
    }
    pub fn history(&self) -> &[Revision] {
        &self.revisions
    }
    pub fn preview(
        &self,
        expected: RevisionId,
        operations: &[Operation],
        provenance: &Provenance,
    ) -> Result<Revision, Error> {
        let current = self.document();
        if expected != current.revision {
            return Err(Error::Conflict {
                expected,
                actual: current.revision,
            });
        }
        if operations.is_empty()
            || operations.len() > self.limits.max_operations
            || self.revisions.len() >= self.limits.max_revisions
            || provenance.source.len() > self.limits.max_bytes
        {
            return Err(Error::LimitExceeded);
        }
        if operations.len() > 1
            && operations.iter().any(|o| {
                matches!(
                    o,
                    Operation::Suggest { .. } | Operation::Accept { .. } | Operation::Reject { .. }
                )
            })
        {
            return Err(Error::InvalidReview);
        }
        let next = RevisionId(expected.0.checked_add(1).ok_or(Error::LimitExceeded)?);
        valid_revision(next)?;
        let mut document = current.clone();
        let mut changes = vec![];
        for op in operations {
            apply(
                &mut document,
                op,
                provenance,
                next,
                self.limits,
                &mut changes,
            )?;
            document.validate(self.limits)?;
        }
        document.revision = next;
        Ok(Revision {
            document,
            changes: Some(ChangeSet {
                document: current.id,
                from: expected,
                to: next,
                provenance: provenance.clone(),
                changes,
            }),
        })
    }
    pub fn apply(
        &mut self,
        expected: RevisionId,
        operations: &[Operation],
        provenance: Provenance,
    ) -> Result<&ChangeSet, Error> {
        let revision = self.preview(expected, operations, &provenance)?;
        self.revisions.push(revision);
        Ok(self
            .revisions
            .last()
            .expect("committed revision")
            .changes
            .as_ref()
            .expect("edit changes"))
    }
    pub fn preview_suggestion(&self, change: ObjectId) -> Result<Document, Error> {
        let t = self
            .document()
            .tracked_changes
            .iter()
            .find(|t| t.id == change)
            .ok_or(Error::MissingObject(change))?;
        if t.status != ReviewStatus::Pending {
            return Err(Error::InvalidReview);
        }
        Ok(self.preview(t.base, &t.operations, &t.provenance)?.document)
    }
}
fn position(document: &Document, id: ObjectId) -> Result<(usize, usize), Error> {
    document
        .sections
        .iter()
        .enumerate()
        .find_map(|(s, section)| {
            section
                .blocks
                .iter()
                .position(|b| b.id == id)
                .map(|b| (s, b))
        })
        .ok_or(Error::MissingObject(id))
}
fn comment_mut(document: &mut Document, id: ObjectId) -> Result<&mut Comment, Error> {
    document
        .comments
        .iter_mut()
        .find(|c| c.id == id)
        .ok_or(Error::MissingObject(id))
}
fn apply(
    document: &mut Document,
    op: &Operation,
    provenance: &Provenance,
    next: RevisionId,
    limits: Limits,
    changes: &mut Vec<Change>,
) -> Result<(), Error> {
    if op.payload_bytes() > limits.max_bytes || changes.len() >= limits.max_operations {
        return Err(Error::LimitExceeded);
    }
    let (kind, object) = match op {
        Operation::InsertSection { id, index, title } => {
            if *index > document.sections.len() {
                return Err(Error::InvalidIndex);
            }
            document.issue(*id)?;
            document.sections.insert(
                *index,
                Section {
                    id: *id,
                    title: title.clone(),
                    blocks: vec![],
                },
            );
            (ChangeKind::Add, Some(*id))
        }
        Operation::InsertBlock {
            section,
            index,
            block,
        } => {
            let s = document
                .sections
                .iter()
                .position(|s| s.id == *section)
                .ok_or(Error::InvalidParent)?;
            if *index > document.sections[s].blocks.len() {
                return Err(Error::InvalidIndex);
            }
            document.issue(block.id)?;
            document.sections[s].blocks.insert(*index, block.clone());
            (ChangeKind::Add, Some(block.id))
        }
        Operation::ReplaceText {
            object,
            range,
            text,
        } => {
            let original = document
                .block_mut(*object)?
                .kind
                .text_mut()
                .ok_or(Error::InvalidStructure)?;
            if range.start > range.end
                || !original.is_char_boundary(range.start)
                || !original.is_char_boundary(range.end)
            {
                return Err(Error::InvalidTextRange);
            }
            let size = original
                .len()
                .checked_sub(range.len())
                .and_then(|n| n.checked_add(text.len()))
                .ok_or(Error::LimitExceeded)?;
            if size > limits.max_bytes {
                return Err(Error::LimitExceeded);
            }
            original.replace_range(range.clone(), text);
            (ChangeKind::Text, Some(*object))
        }
        Operation::ReplaceTable { object, table } => {
            table.validate(limits.max_table_cells)?;
            let block = document.block_mut(*object)?;
            if !matches!(block.kind, BlockKind::Table(_)) {
                return Err(Error::InvalidStructure);
            }
            block.kind = BlockKind::Table(table.clone());
            (ChangeKind::Table, Some(*object))
        }
        Operation::MoveObject {
            object,
            section,
            index,
        } => {
            if let Some(source) = document.sections.iter().position(|s| s.id == *object) {
                // Sections can only be reordered at the root; blocks are leaves.
                if section.is_some() {
                    return Err(Error::InvalidParent);
                }
                if *index >= document.sections.len() {
                    return Err(Error::InvalidIndex);
                }
                let s = document.sections.remove(source);
                document.sections.insert(*index, s);
            } else {
                let (source, b) = position(document, *object)?;
                let target = section
                    .and_then(|id| document.sections.iter().position(|s| s.id == id))
                    .ok_or(Error::InvalidParent)?;
                let target_len =
                    document.sections[target].blocks.len() - usize::from(source == target);
                if *index > target_len {
                    return Err(Error::InvalidIndex);
                }
                let block = document.sections[source].blocks.remove(b);
                document.sections[target].blocks.insert(*index, block);
            }
            (ChangeKind::Move, Some(*object))
        }
        Operation::DeleteObject { object } => {
            if let Some(s) = document.sections.iter().position(|s| s.id == *object) {
                document.sections.remove(s);
            } else {
                let (s, b) = position(document, *object)?;
                document.sections[s].blocks.remove(b);
            }
            (ChangeKind::Delete, Some(*object))
        }
        Operation::ApplyStyle { object, style } => {
            document.effective_style(style)?;
            document.block_mut(*object)?.style = style.clone();
            (ChangeKind::Format, Some(*object))
        }
        Operation::DefineStyle { name, style } => {
            document.styles.insert(name.clone(), style.clone());
            document.effective_style(name)?;
            (ChangeKind::Format, None)
        }
        Operation::SetMetadata(metadata) => {
            document.metadata = metadata.clone();
            (ChangeKind::Metadata, None)
        }
        Operation::SetSettings(settings) => {
            document.settings = settings.clone();
            (ChangeKind::Format, None)
        }
        Operation::AddComment {
            id,
            object,
            message,
        } => {
            if document.block(*object).is_none()
                && !document.sections.iter().any(|s| s.id == *object)
            {
                return Err(Error::MissingObject(*object));
            }
            document.issue(*id)?;
            document.comments.push(Comment {
                id: *id,
                object: *object,
                messages: vec![message.clone()],
                resolved: false,
            });
            (ChangeKind::Comment, Some(*object))
        }
        Operation::Reply { comment, message } => {
            comment_mut(document, *comment)?
                .messages
                .push(message.clone());
            (ChangeKind::Comment, Some(*comment))
        }
        Operation::Resolve { comment, resolved } => {
            comment_mut(document, *comment)?.resolved = *resolved;
            (ChangeKind::Comment, Some(*comment))
        }
        Operation::Suggest { id, operations } => {
            if operations.is_empty()
                || operations.len() > limits.max_operations
                || operations.iter().any(|o| !o.is_content())
            {
                return Err(Error::InvalidReview);
            }
            let mut trial = document.clone();
            trial.issue(*id)?;
            let mut trial_changes = vec![];
            for operation in operations {
                apply(
                    &mut trial,
                    operation,
                    provenance,
                    next,
                    limits,
                    &mut trial_changes,
                )?;
                trial.validate(limits)?;
            }
            document.issue(*id)?;
            document.tracked_changes.push(TrackedChange {
                id: *id,
                base: next,
                operations: operations.clone(),
                provenance: provenance.clone(),
                status: ReviewStatus::Pending,
            });
            (ChangeKind::Review, Some(*id))
        }
        Operation::Accept { change } | Operation::Reject { change } => {
            let index = document
                .tracked_changes
                .iter()
                .position(|t| t.id == *change)
                .ok_or(Error::MissingObject(*change))?;
            let t = document.tracked_changes[index].clone();
            if t.status != ReviewStatus::Pending {
                return Err(Error::InvalidReview);
            }
            if matches!(op, Operation::Accept { .. }) {
                if t.base != document.revision {
                    return Err(Error::Conflict {
                        expected: t.base,
                        actual: document.revision,
                    });
                }
                for operation in &t.operations {
                    apply(document, operation, &t.provenance, next, limits, changes)?;
                    document.validate(limits)?;
                }
                document.tracked_changes[index].status = ReviewStatus::Accepted;
            } else {
                document.tracked_changes[index].status = ReviewStatus::Rejected;
            }
            (ChangeKind::Review, Some(*change))
        }
    };
    changes.push(Change {
        kind,
        object,
        operation: op.clone(),
    });
    Ok(())
}
