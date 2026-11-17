//! UI-independent, host-only presentation model. Caller-supplied canonical IDs;
//! no rendering, storage, authority, Activity/Wayback or external execution.
mod model;
pub mod native;
mod operations;
pub use model::*;
pub use nagi_history::activity::RevisionId;
pub use nagi_model::ObjectId;
pub use operations::Edit;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error {
    InvalidId,
    DuplicateId(ObjectId),
    MissingObject(ObjectId),
    InvalidIndex,
    InvalidGeometry,
    InvalidText,
    InvalidReference,
    InvalidTheme,
    InvalidRevision,
    Conflict {
        expected: RevisionId,
        actual: RevisionId,
    },
    LimitExceeded,
    MalformedNative,
    UnknownVersion(u16),
    AdapterUnavailable,
}
pub type Result<T> = std::result::Result<T, Error>;

/// Read-only injected source adapter; this does not authenticate or fetch data.
pub trait SourceRevisions {
    fn revision(&self, resource: ObjectId) -> Result<RevisionId>;
}
pub struct MissingSourceAdapter;
impl SourceRevisions for MissingSourceAdapter {
    fn revision(&self, _: ObjectId) -> Result<RevisionId> {
        Err(Error::AdapterUnavailable)
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SourceStatus {
    Current,
    Changed {
        baseline: RevisionId,
        observed: RevisionId,
    },
}
impl SourceReference {
    pub fn status(&self, adapter: &impl SourceRevisions) -> Result<SourceStatus> {
        model::valid_id(self.resource)?;
        model::valid_revision(self.revision)?;
        let observed = adapter.revision(self.resource)?;
        model::valid_revision(observed)?;
        Ok(if observed == self.revision {
            SourceStatus::Current
        } else {
            SourceStatus::Changed {
                baseline: self.revision,
                observed,
            }
        })
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputFormat {
    Native,
    Pptx,
    Pdf,
}
impl Presentation {
    pub fn export(&self, format: OutputFormat, limits: Limits) -> Result<Vec<u8>> {
        match format {
            OutputFormat::Native => native::encode(self, limits),
            OutputFormat::Pptx | OutputFormat::Pdf => Err(Error::AdapterUnavailable),
        }
    }
}
