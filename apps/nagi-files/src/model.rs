use std::fmt::{Display, Formatter};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub const MAX_PREVIEW_BYTES: usize = 1024 * 1024;

static NEXT_OPERATION_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ResourceId(pub u128);

impl Display for ResourceId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:032x}", self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TransactionId(pub u64);

impl TransactionId {
    pub fn new() -> Self {
        Self(NEXT_OPERATION_ID.fetch_add(1, Ordering::Relaxed))
    }
}

impl Default for TransactionId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Location(String);

impl Location {
    pub fn root() -> Self {
        Self(String::new())
    }

    /// Parses a provider-relative path using `/` as the sole separator.
    /// Empty components are collapsed; `.` and `..`, roots, drive prefixes,
    /// and backslashes are rejected so validation is host-independent.
    pub fn parse(path: &str) -> Result<Self, FilesError> {
        if path.starts_with('/') || path.starts_with('\\') || path.contains('\\') {
            return Err(FilesError::new(FilesErrorKind::InvalidLocation));
        }
        if path.as_bytes().get(1) == Some(&b':') {
            return Err(FilesError::new(FilesErrorKind::InvalidLocation));
        }
        let mut parts = Vec::new();
        for part in path.split('/') {
            if part.is_empty() {
                continue;
            }
            if part == "." || part == ".." {
                return Err(FilesError::new(FilesErrorKind::SandboxEscape));
            }
            if part.chars().any(char::is_control) {
                return Err(FilesError::new(FilesErrorKind::InvalidName));
            }
            parts.push(FileName::parse(part)?.0);
        }
        Ok(Self(parts.join("/")))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn is_root(&self) -> bool {
        self.0.is_empty()
    }

    pub fn parent(&self) -> Option<Self> {
        if self.is_root() {
            return None;
        }
        match self.0.rsplit_once('/') {
            Some((parent, _)) => Some(Self(parent.to_owned())),
            None => Some(Self::root()),
        }
    }

    pub fn file_name(&self) -> Option<&str> {
        self.0.rsplit('/').next().filter(|name| !name.is_empty())
    }

    pub fn join(&self, name: &FileName) -> Self {
        if self.is_root() {
            Self(name.0.clone())
        } else {
            Self(format!("{}/{}", self.0, name.0))
        }
    }

    pub fn is_within(&self, scope: &Self) -> bool {
        scope.is_root()
            || self == scope
            || self
                .0
                .strip_prefix(&scope.0)
                .is_some_and(|suffix| suffix.starts_with('/'))
    }

    pub(crate) fn components(&self) -> impl Iterator<Item = &str> {
        self.0.split('/').filter(|part| !part.is_empty())
    }
}

impl Display for Location {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        if self.is_root() {
            f.write_str("/")
        } else {
            f.write_str(&self.0)
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FileName(String);

impl FileName {
    pub fn parse(name: &str) -> Result<Self, FilesError> {
        if name.is_empty()
            || name == "."
            || name == ".."
            || name.contains('/')
            || name.contains('\\')
            || name.chars().any(|character| ":*?\"<>|".contains(character))
            || name.chars().any(char::is_control)
            || name.ends_with('.')
            || name.ends_with(' ')
        {
            return Err(FilesError::new(FilesErrorKind::InvalidName));
        }
        let device_name = name
            .split('.')
            .next()
            .unwrap_or(name)
            .trim_end_matches(['.', ' '])
            .to_ascii_uppercase();
        if matches!(device_name.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || ["COM", "LPT"].iter().any(|prefix| {
                device_name.strip_prefix(prefix).is_some_and(|number| {
                    number.len() == 1
                        && number
                            .chars()
                            .next()
                            .is_some_and(|digit| ('1'..='9').contains(&digit))
                })
            })
        {
            return Err(FilesError::new(FilesErrorKind::InvalidName));
        }
        Ok(Self(name.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Display for FileName {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EntryKind {
    File,
    Folder,
    Symlink,
    Unsupported,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileEntry {
    pub id: ResourceId,
    pub name: FileName,
    pub location: Location,
    pub kind: EntryKind,
    pub size_bytes: Option<u64>,
    pub created_at: Option<i64>,
    pub modified_at: Option<i64>,
    pub tags: Vec<String>,
    pub availability: EntryAvailability,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilesContextSnapshot {
    pub current_location: Location,
    pub selected_resources: Vec<ResourceId>,
    pub focused_resource: Option<ResourceId>,
    pub active_workspace: Option<String>,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Tag(String);

impl Tag {
    pub fn parse(value: &str) -> Result<Self, FilesError> {
        let value = value.trim();
        if value.is_empty() || value.chars().count() > 64 || value.chars().any(char::is_control) {
            return Err(FilesError::new(FilesErrorKind::InvalidName));
        }
        Ok(Self(value.to_owned()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FileEntry {
    pub fn child_location(&self) -> Location {
        self.location.join(&self.name)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EntryAvailability {
    Available,
    ReadOnly,
    PermissionRequired,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationKind {
    CreateFolder,
    Rename,
    Copy,
    Move,
    Duplicate,
    Trash,
    Restore,
    PermanentDelete,
    Open,
    AddToWorkspace,
    RemoveFromWorkspace,
    SetTags,
}

impl OperationKind {
    pub fn action_id(self) -> &'static str {
        match self {
            Self::CreateFolder => "files.create_folder",
            Self::Rename => "files.rename",
            Self::Copy => "files.copy",
            Self::Move => "files.move",
            Self::Duplicate => "files.duplicate",
            Self::Trash => "files.delete",
            Self::Restore => "files.restore",
            Self::PermanentDelete => "files.delete_permanently",
            Self::Open => "files.open",
            Self::AddToWorkspace => "files.add_to_workspace",
            Self::RemoveFromWorkspace => "files.remove_from_workspace",
            Self::SetTags => "files.set_tags",
        }
    }

    pub fn reversibility(self) -> Reversibility {
        match self {
            Self::CreateFolder
            | Self::Rename
            | Self::Copy
            | Self::Move
            | Self::Duplicate
            | Self::Trash
            | Self::Restore
            | Self::AddToWorkspace
            | Self::RemoveFromWorkspace
            | Self::SetTags => Reversibility::Reversible,
            Self::PermanentDelete => Reversibility::Irreversible,
            Self::Open => Reversibility::NoMutation,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Reversibility {
    Reversible,
    Irreversible,
    NoMutation,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Actor {
    User,
    Agent,
    App,
    System,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CapabilityRight {
    Read,
    Enumerate,
    Create,
    Write,
    Rename,
    Move,
    Delete,
    Restore,
    PermanentDelete,
    WorkspaceReference,
    SetMetadata,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionDecision {
    Allow,
    Ask,
    Deny,
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityRequest {
    pub right: CapabilityRight,
    pub location: Location,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FilesErrorKind {
    InvalidLocation,
    InvalidName,
    SandboxEscape,
    SymlinkNotAllowed,
    NotFound,
    AlreadyExists,
    NotDirectory,
    IsDirectory,
    UnsupportedEntry,
    FileTooLarge,
    PermissionDenied,
    PermissionRequired,
    CapabilityUnavailable,
    Conflict,
    DestinationInsideSource,
    Cancelled,
    ConfirmationRequired,
    InvalidConfirmation,
    ConfirmationLimitReached,
    ProviderUnavailable,
    ProviderFailure,
    ActivityUnavailable,
    ActivityFailure,
    WorkspaceUnavailable,
    PartialFailure,
    CorruptMetadata,
    ReadOnly,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilesError {
    pub kind: FilesErrorKind,
    pub location: Option<Location>,
}

impl FilesError {
    pub fn new(kind: FilesErrorKind) -> Self {
        Self {
            kind,
            location: None,
        }
    }

    pub fn at(mut self, location: Location) -> Self {
        self.location = Some(location);
        self
    }
}

impl Display for FilesError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?}", self.kind)?;
        if let Some(location) = &self.location {
            write!(f, " at {location}")?;
        }
        Ok(())
    }
}

impl std::error::Error for FilesError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrashEntry {
    pub id: ResourceId,
    pub original_location: Location,
    pub kind: EntryKind,
    pub size_bytes: Option<u64>,
    pub deleted_at: i64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OperationPrecondition {
    SourceExists,
    DestinationExists,
    DestinationAbsent,
    UserConfirmed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OperationIntent {
    pub transaction_id: TransactionId,
    pub kind: OperationKind,
    pub actor: Actor,
    pub source: Option<Location>,
    pub destination: Option<Location>,
    pub affected: Vec<ResourceId>,
    pub preconditions: Vec<OperationPrecondition>,
    pub reversibility: Reversibility,
}

impl OperationIntent {
    pub fn new(kind: OperationKind, actor: Actor) -> Self {
        Self {
            transaction_id: TransactionId::new(),
            kind,
            actor,
            source: None,
            destination: None,
            affected: Vec::new(),
            preconditions: Vec::new(),
            reversibility: kind.reversibility(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConfirmationChallenge {
    nonce: u64,
    transaction_id: TransactionId,
    target: ResourceId,
    pub display_name: String,
}

impl ConfirmationChallenge {
    pub(crate) fn new(
        nonce: u64,
        transaction_id: TransactionId,
        target: ResourceId,
        display_name: String,
    ) -> Self {
        Self {
            nonce,
            transaction_id,
            target,
            display_name,
        }
    }

    pub(crate) fn binding(&self) -> (u64, TransactionId, ResourceId) {
        (self.nonce, self.transaction_id, self.target)
    }
}

#[derive(Clone, Debug)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}
