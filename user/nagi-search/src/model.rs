use alloc::{collections::BTreeMap, string::String, vec::Vec};

use nagi_model::{AppId, AppSessionId, ObjectId, WorkspaceId};

pub const MAX_TITLE_BYTES: usize = 1024;
pub const MAX_LOCATION_BYTES: usize = 4096;
pub const MAX_TAG_BYTES: usize = 256;
pub const MAX_TAGS: usize = 128;
pub const MAX_ATTRIBUTE_BYTES: usize = 4096;
pub const MAX_ATTRIBUTES: usize = 128;
pub const MAX_WORKSPACE_SESSIONS: usize = 4096;
pub const MAX_WORKSPACE_OBJECTS: usize = 65_536;
pub const MAX_SEARCH_RESULTS: usize = 1000;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ModelError {
    EmptyTitle,
    TitleTooLong,
    LocationTooLong,
    EmptyTag,
    TagTooLong,
    TooManyTags,
    EmptyAttributeKey,
    AttributeTooLong,
    TooManyAttributes,
    TooManySessions,
    TooManyWorkspaceObjects,
    InvalidTimeRange,
    EmptyTextQuery,
    SearchLimitExceeded,
    InvalidTraversalBound,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum ObjectKind {
    File = 1,
    Page = 2,
    Note = 3,
    ApplicationData = 4,
    Other = 255,
}

impl ObjectKind {
    pub(crate) fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::File),
            2 => Some(Self::Page),
            3 => Some(Self::Note),
            4 => Some(Self::ApplicationData),
            255 => Some(Self::Other),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum VisibilityScope {
    /// The producer asserts that ordinary discovery is allowed. Runtime policy
    /// may still deny access through the injected visibility filter.
    Public = 1,
    /// Only a filter that recognizes the source application should allow this.
    SourceApplication = 2,
    /// Hidden unless an injected policy explicitly grants access.
    Private = 3,
}

impl VisibilityScope {
    pub(crate) fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Public),
            2 => Some(Self::SourceApplication),
            3 => Some(Self::Private),
            _ => None,
        }
    }
}

/// A producer-owned descriptive record. `ObjectId` is its only object
/// identity; `location` is an optional source reference and is never an ID.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetadataRecord {
    pub object_id: ObjectId,
    pub kind: ObjectKind,
    pub title: String,
    pub location: Option<String>,
    pub source_app: Option<AppId>,
    pub source_session: Option<AppSessionId>,
    pub created_at: Option<i64>,
    pub modified_at: Option<i64>,
    pub observed_at: Option<i64>,
    pub tags: Vec<String>,
    pub attributes: BTreeMap<String, String>,
    pub visibility: VisibilityScope,
    /// Removal is logical and durable. Re-indexing the same stable ID clears
    /// this marker; relations and Workspace membership are removed at delete.
    pub tombstoned_at: Option<i64>,
}

impl MetadataRecord {
    pub fn new(object_id: ObjectId, kind: ObjectKind, title: impl Into<String>) -> Self {
        Self {
            object_id,
            kind,
            title: title.into(),
            location: None,
            source_app: None,
            source_session: None,
            created_at: None,
            modified_at: None,
            observed_at: None,
            tags: Vec::new(),
            attributes: BTreeMap::new(),
            visibility: VisibilityScope::Private,
            tombstoned_at: None,
        }
    }

    pub fn validate(&self) -> Result<(), ModelError> {
        validate_title(&self.title)?;
        if self
            .location
            .as_ref()
            .is_some_and(|value| value.len() > MAX_LOCATION_BYTES)
        {
            return Err(ModelError::LocationTooLong);
        }
        if self.tags.len() > MAX_TAGS {
            return Err(ModelError::TooManyTags);
        }
        for tag in &self.tags {
            if tag.trim().is_empty() {
                return Err(ModelError::EmptyTag);
            }
            if tag.len() > MAX_TAG_BYTES {
                return Err(ModelError::TagTooLong);
            }
        }
        validate_attributes(&self.attributes)
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum RelationKind {
    RelatedTo = 1,
    Contains = 2,
    DerivedFrom = 3,
    References = 4,
}

impl RelationKind {
    pub(crate) fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::RelatedTo),
            2 => Some(Self::Contains),
            3 => Some(Self::DerivedFrom),
            4 => Some(Self::References),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
#[repr(u8)]
pub enum RelationProvenance {
    User = 1,
    Application = 2,
    AiSuggestion = 3,
}

impl RelationProvenance {
    pub(crate) fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::User),
            2 => Some(Self::Application),
            3 => Some(Self::AiSuggestion),
            _ => None,
        }
    }
}

/// A directed relation between stable logical objects. AI-suggested relations
/// retain their provenance and are never relabeled as user/app facts.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Relation {
    pub source: ObjectId,
    pub kind: RelationKind,
    pub target: ObjectId,
    pub provenance: RelationProvenance,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkspaceSession {
    pub app_id: AppId,
    pub session_id: AppSessionId,
}

impl Ord for WorkspaceSession {
    fn cmp(&self, other: &Self) -> core::cmp::Ordering {
        (self.app_id, self.session_id).cmp(&(other.app_id, other.session_id))
    }
}

impl PartialOrd for WorkspaceSession {
    fn partial_cmp(&self, other: &Self) -> Option<core::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// A device-independent logical grouping. Window/surface/PID/layout state is
/// deliberately absent. Membership supports many Workspaces per object.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Workspace {
    pub workspace_id: WorkspaceId,
    pub title: String,
    pub owner_app: Option<AppId>,
    pub visibility: VisibilityScope,
    pub created_at: Option<i64>,
    pub modified_at: Option<i64>,
    pub tags: Vec<String>,
    pub attributes: BTreeMap<String, String>,
    pub sessions: Vec<WorkspaceSession>,
    pub objects: Vec<ObjectId>,
}

impl Workspace {
    pub fn new(workspace_id: WorkspaceId, title: impl Into<String>) -> Self {
        Self {
            workspace_id,
            title: title.into(),
            owner_app: None,
            visibility: VisibilityScope::Private,
            created_at: None,
            modified_at: None,
            tags: Vec::new(),
            attributes: BTreeMap::new(),
            sessions: Vec::new(),
            objects: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<(), ModelError> {
        validate_title(&self.title)?;
        if self.tags.len() > MAX_TAGS {
            return Err(ModelError::TooManyTags);
        }
        for tag in &self.tags {
            if tag.trim().is_empty() {
                return Err(ModelError::EmptyTag);
            }
            if tag.len() > MAX_TAG_BYTES {
                return Err(ModelError::TagTooLong);
            }
        }
        if self.sessions.len() > MAX_WORKSPACE_SESSIONS {
            return Err(ModelError::TooManySessions);
        }
        if self.objects.len() > MAX_WORKSPACE_OBJECTS {
            return Err(ModelError::TooManyWorkspaceObjects);
        }
        validate_attributes(&self.attributes)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccessContext {
    pub app_id: Option<AppId>,
    pub app_session_id: Option<AppSessionId>,
}

impl AccessContext {
    pub const fn anonymous() -> Self {
        Self {
            app_id: None,
            app_session_id: None,
        }
    }

    pub const fn for_application(app_id: AppId, app_session_id: AppSessionId) -> Self {
        Self {
            app_id: Some(app_id),
            app_session_id: Some(app_session_id),
        }
    }
}

/// Implementations must apply the caller's real authority rules. Search
/// performs this check before matching fields, grouping, or returning counts.
pub trait VisibilityFilter {
    fn can_read_object(&self, access: AccessContext, record: &MetadataRecord) -> bool;
    fn can_read_workspace(&self, access: AccessContext, workspace: &Workspace) -> bool;

    /// Workspace membership can contain application-session identifiers. A
    /// filter must explicitly grant those references before they are returned.
    fn can_read_workspace_session(
        &self,
        _access: AccessContext,
        _workspace: &Workspace,
        _session: WorkspaceSession,
    ) -> bool {
        false
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SearchTimeRange {
    /// Inclusive Unix timestamp in seconds.
    pub from: Option<i64>,
    /// Inclusive Unix timestamp in seconds.
    pub through: Option<i64>,
}

impl SearchTimeRange {
    pub const fn new(from: Option<i64>, through: Option<i64>) -> Self {
        Self { from, through }
    }

    pub(crate) fn validate(self) -> Result<(), ModelError> {
        if let (Some(from), Some(through)) = (self.from, self.through) {
            if from > through {
                return Err(ModelError::InvalidTimeRange);
            }
        }
        Ok(())
    }

    pub(crate) fn contains(self, value: Option<i64>) -> bool {
        let Some(value) = value else {
            return false;
        };
        self.from.is_none_or(|from| value >= from)
            && self.through.is_none_or(|through| value <= through)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RelationDirection {
    Either,
    Outgoing,
    Incoming,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchSort {
    Relevance,
    ModifiedNewest,
    TitleAscending,
    ObjectIdAscending,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttributeMatch {
    pub key: String,
    pub value: String,
}

/// All supplied filters are conjunctive. `tags_any` is disjunctive within
/// itself. Text is a case-insensitive literal phrase with exact/prefix/
/// substring title matching; it is not an AI query.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchQuery {
    pub text: Option<String>,
    pub kind: Option<ObjectKind>,
    pub source_app: Option<AppId>,
    pub workspace: Option<WorkspaceId>,
    pub tags_any: Vec<String>,
    pub attributes_all: Vec<AttributeMatch>,
    pub created: Option<SearchTimeRange>,
    pub modified: Option<SearchTimeRange>,
    pub observed: Option<SearchTimeRange>,
    pub related_to: Option<ObjectId>,
    pub relation_direction: RelationDirection,
    pub sort: SearchSort,
    pub limit: usize,
}

impl Default for SearchQuery {
    fn default() -> Self {
        Self {
            text: None,
            kind: None,
            source_app: None,
            workspace: None,
            tags_any: Vec::new(),
            attributes_all: Vec::new(),
            created: None,
            modified: None,
            observed: None,
            related_to: None,
            relation_direction: RelationDirection::Either,
            sort: SearchSort::Relevance,
            limit: MAX_SEARCH_RESULTS,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SearchMatch {
    TitleExact,
    TitlePrefix,
    TitleSubstring,
    Tag(String),
    Attribute(String),
    Kind,
    SourceApplication,
    Workspace(WorkspaceId),
    CreatedTime,
    ModifiedTime,
    ObservedTime,
    Relation {
        kind: RelationKind,
        provenance: RelationProvenance,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchHit {
    pub record: MetadataRecord,
    pub rationale: Vec<SearchMatch>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceHit {
    pub workspace_id: WorkspaceId,
    pub title: String,
    pub modified_at: Option<i64>,
    pub rationale: Vec<SearchMatch>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceGroup {
    pub workspace_id: WorkspaceId,
    pub title: String,
    /// Stable ordered IDs only; consumers already have the matching visible
    /// objects in `SearchResponse::objects`.
    pub object_ids: Vec<ObjectId>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SearchResponse {
    /// Unique authorized object hits, in deterministic order.
    pub objects: Vec<SearchHit>,
    /// Authorized workspaces whose own metadata matched the query.
    pub workspaces: Vec<WorkspaceHit>,
    /// Visible memberships only. Hidden Workspace IDs/titles are omitted.
    pub workspace_groups: Vec<WorkspaceGroup>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchError {
    InvalidModel(ModelError),
    Storage,
    InvalidTraversalBound,
}

fn validate_title(title: &str) -> Result<(), ModelError> {
    if title.trim().is_empty() {
        return Err(ModelError::EmptyTitle);
    }
    if title.len() > MAX_TITLE_BYTES {
        return Err(ModelError::TitleTooLong);
    }
    Ok(())
}

fn validate_attributes(attributes: &BTreeMap<String, String>) -> Result<(), ModelError> {
    if attributes.len() > MAX_ATTRIBUTES {
        return Err(ModelError::TooManyAttributes);
    }
    for (key, value) in attributes {
        if key.trim().is_empty() {
            return Err(ModelError::EmptyAttributeKey);
        }
        if key.len() > MAX_ATTRIBUTE_BYTES || value.len() > MAX_ATTRIBUTE_BYTES {
            return Err(ModelError::AttributeTooLong);
        }
    }
    Ok(())
}
