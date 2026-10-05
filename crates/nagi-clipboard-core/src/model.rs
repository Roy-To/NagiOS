use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use nagi_model::{AppId, AppSessionId, ObjectId};

use crate::limits::{ClipboardLimits, MAX_METADATA_KEY_BYTES, MAX_ORIGIN_LABEL_BYTES};
use crate::media_type::MediaType;

/// Monotonic clipboard revision. Every successful write and every clear that
/// removes content advances it by exactly one; it never wraps or repeats.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ClipboardGeneration(u64);

impl ClipboardGeneration {
    /// The generation of a never-written clipboard.
    pub const INITIAL: Self = Self(0);

    /// Wraps a raw generation value.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// The raw generation value.
    pub const fn get(self) -> u64 {
        self.0
    }

    pub(crate) fn next(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }
}

/// What the writer intends a paste to mean.
///
/// `Move` (cut) is a non-destructive hint only. This foundation never deletes,
/// hides, or mutates source data; a future transactional owner must perform
/// and authorize any source removal separately.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TransferIntent {
    /// Duplicate the data at the destination.
    #[default]
    Copy,
    /// The writer suggests the source may be removed after a successful paste.
    Move,
}

/// Payload kinds, used in format listings without exposing payload bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PayloadKind {
    /// UTF-8 text.
    Text,
    /// Opaque bytes.
    Binary,
    /// A reference to an existing canonical object; carries no authority.
    ObjectReference,
}

/// One representation's data.
///
/// `Debug` never prints payload contents, so payloads cannot leak through
/// logs, diagnostics, or test failure output by default.
#[derive(Clone, Eq, PartialEq)]
pub enum Payload {
    /// UTF-8 text; required for `text/*` media types.
    Text(String),
    /// Opaque inline bytes; not allowed for `text/*` media types.
    Binary(Vec<u8>),
    /// A canonical object reference. Holding it grants no access to the object.
    ObjectReference(ObjectId),
}

impl Payload {
    /// The payload kind.
    pub fn kind(&self) -> PayloadKind {
        match self {
            Self::Text(_) => PayloadKind::Text,
            Self::Binary(_) => PayloadKind::Binary,
            Self::ObjectReference(_) => PayloadKind::ObjectReference,
        }
    }

    /// Inline bytes counted against payload bounds (zero for references).
    pub fn inline_len(&self) -> usize {
        match self {
            Self::Text(text) => text.len(),
            Self::Binary(bytes) => bytes.len(),
            Self::ObjectReference(_) => 0,
        }
    }
}

impl fmt::Debug for Payload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{:?}(<redacted {} bytes>)",
            self.kind(),
            self.inline_len()
        )
    }
}

/// One media-typed representation of an item.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Representation {
    media_type: MediaType,
    payload: Payload,
}

impl Representation {
    /// Pairs a media type with a payload. `text/*` requires a text payload and
    /// a text payload requires a `text/*` media type.
    pub fn new(media_type: MediaType, payload: Payload) -> Result<Self, ContentError> {
        let text_payload = matches!(payload, Payload::Text(_));
        if media_type.is_text() != text_payload {
            return Err(ContentError::PayloadMediaTypeMismatch);
        }
        Ok(Self {
            media_type,
            payload,
        })
    }

    /// Convenience constructor for `text/plain`.
    pub fn plain_text(text: impl Into<String>) -> Self {
        Self {
            media_type: MediaType::new(MediaType::TEXT_PLAIN).expect("canonical constant"),
            payload: Payload::Text(text.into()),
        }
    }

    /// The representation's media type.
    pub fn media_type(&self) -> &MediaType {
        &self.media_type
    }

    /// The representation's payload.
    pub fn payload(&self) -> &Payload {
        &self.payload
    }
}

/// One clipboard item: alternative representations in writer preference order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClipboardItem {
    representations: Vec<Representation>,
}

impl ClipboardItem {
    /// Creates an item. Bounds and duplicate checks run when content is
    /// validated against [`ClipboardLimits`].
    pub fn new(representations: Vec<Representation>) -> Self {
        Self { representations }
    }

    /// Representations in writer preference order.
    pub fn representations(&self) -> &[Representation] {
        &self.representations
    }

    /// Finds the representation with exactly this media type.
    pub fn representation(&self, media_type: &MediaType) -> Option<&Representation> {
        self.representations
            .iter()
            .find(|representation| &representation.media_type == media_type)
    }
}

/// Origin information supplied by the writing application.
///
/// This is an untrusted claim. It is stored and returned verbatim, labelled
/// as claimed, and is never consulted for authorization or attribution. The
/// trusted origin is [`crate::VerifiedOrigin`], recorded from the caller.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct ClaimedOrigin {
    /// Application the writer claims to be.
    pub app: Option<AppId>,
    /// Application session the writer claims to be.
    pub app_session: Option<AppSessionId>,
    /// Free-form bounded label, e.g. a document title. Treated as private data.
    pub label: Option<String>,
}

impl fmt::Debug for ClaimedOrigin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClaimedOrigin")
            .field("app", &self.app)
            .field("app_session", &self.app_session)
            .field(
                "label",
                &self
                    .label
                    .as_ref()
                    .map(|label| format!("<redacted {} bytes>", label.len())),
            )
            .finish()
    }
}

/// A locale-neutral untrusted metadata key: `[a-z0-9]` segments joined by
/// `.`, `-`, or `_`, at most 64 bytes.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct MetadataKey(String);

impl MetadataKey {
    /// Validates a metadata key.
    pub fn new(value: impl Into<String>) -> Result<Self, ContentError> {
        let value = value.into();
        let valid = !value.is_empty()
            && value.len() <= MAX_METADATA_KEY_BYTES
            && value
                .bytes()
                .next()
                .is_some_and(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            && value.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'.' | b'-' | b'_')
            });
        if !valid {
            return Err(ContentError::InvalidMetadataKey);
        }
        Ok(Self(value))
    }

    /// The key text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Complete clipboard content written as one replacement unit.
///
/// The same content type describes a generic data offer, so a future
/// drag-and-drop owner can reuse it without a second format model.
#[derive(Clone, Default, Eq, PartialEq)]
pub struct ClipboardContent {
    items: Vec<ClipboardItem>,
    intent: TransferIntent,
    claimed_origin: ClaimedOrigin,
    metadata: BTreeMap<MetadataKey, String>,
}

impl ClipboardContent {
    /// Content with the given ordered items, `Copy` intent, and no claims.
    pub fn new(items: Vec<ClipboardItem>) -> Self {
        Self {
            items,
            ..Self::default()
        }
    }

    /// Single-item `text/plain` content.
    pub fn plain_text(text: impl Into<String>) -> Self {
        Self::new(vec![ClipboardItem::new(vec![Representation::plain_text(
            text,
        )])])
    }

    /// Sets the transfer intent hint.
    pub fn with_intent(mut self, intent: TransferIntent) -> Self {
        self.intent = intent;
        self
    }

    /// Attaches an untrusted origin claim.
    pub fn with_claimed_origin(mut self, claimed_origin: ClaimedOrigin) -> Self {
        self.claimed_origin = claimed_origin;
        self
    }

    /// Inserts or replaces one untrusted metadata entry.
    pub fn with_metadata(mut self, key: MetadataKey, value: impl Into<String>) -> Self {
        self.metadata.insert(key, value.into());
        self
    }

    /// Ordered items.
    pub fn items(&self) -> &[ClipboardItem] {
        &self.items
    }

    /// Transfer intent hint.
    pub fn intent(&self) -> TransferIntent {
        self.intent
    }

    /// Untrusted origin claim.
    pub fn claimed_origin(&self) -> &ClaimedOrigin {
        &self.claimed_origin
    }

    /// Untrusted metadata in deterministic key order.
    pub fn metadata(&self) -> &BTreeMap<MetadataKey, String> {
        &self.metadata
    }

    /// Total inline payload bytes, saturating.
    pub fn total_inline_bytes(&self) -> usize {
        self.items
            .iter()
            .flat_map(|item| item.representations.iter())
            .fold(0usize, |total, representation| {
                total.saturating_add(representation.payload.inline_len())
            })
    }

    /// Validates every structural rule and bound. Never truncates.
    pub fn validate(&self, limits: &ClipboardLimits) -> Result<(), ContentError> {
        if self.items.is_empty() {
            return Err(ContentError::NoItems);
        }
        if self.items.len() > limits.max_items {
            return Err(ContentError::TooManyItems);
        }
        let mut total = 0usize;
        for (item_index, item) in self.items.iter().enumerate() {
            if item.representations.is_empty() {
                return Err(ContentError::EmptyItem { item: item_index });
            }
            if item.representations.len() > limits.max_representations_per_item {
                return Err(ContentError::TooManyRepresentations { item: item_index });
            }
            let mut seen = BTreeSet::new();
            for representation in &item.representations {
                if !seen.insert(&representation.media_type) {
                    return Err(ContentError::DuplicateMediaType { item: item_index });
                }
                if representation.media_type.is_text()
                    != matches!(representation.payload, Payload::Text(_))
                {
                    return Err(ContentError::PayloadMediaTypeMismatch);
                }
                let length = representation.payload.inline_len();
                if length > limits.max_representation_bytes {
                    return Err(ContentError::RepresentationTooLarge { item: item_index });
                }
                total = total.saturating_add(length);
                if total > limits.max_total_bytes {
                    return Err(ContentError::TotalTooLarge);
                }
            }
        }
        if self.metadata.len() > limits.max_metadata_entries {
            return Err(ContentError::TooManyMetadataEntries);
        }
        for value in self.metadata.values() {
            if value.len() > limits.max_metadata_value_bytes {
                return Err(ContentError::MetadataValueTooLong);
            }
            if value.chars().any(char::is_control) {
                return Err(ContentError::InvalidMetadataValue);
            }
        }
        if let Some(label) = &self.claimed_origin.label {
            if label.is_empty()
                || label.len() > MAX_ORIGIN_LABEL_BYTES
                || label.chars().any(char::is_control)
            {
                return Err(ContentError::InvalidOriginLabel);
            }
        }
        Ok(())
    }
}

impl fmt::Debug for ClipboardContent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ClipboardContent")
            .field("items", &self.items)
            .field("intent", &self.intent)
            .field("claimed_origin", &self.claimed_origin)
            .field(
                "metadata_keys",
                &self
                    .metadata
                    .keys()
                    .map(MetadataKey::as_str)
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

/// Why clipboard content was rejected. Variants carry only structural
/// positions, never payload or metadata values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentError {
    /// Content has no items; use clear instead.
    NoItems,
    /// More items than the configured limit.
    TooManyItems,
    /// An item has no representations.
    EmptyItem {
        /// Item index.
        item: usize,
    },
    /// An item has more representations than allowed.
    TooManyRepresentations {
        /// Item index.
        item: usize,
    },
    /// An item offers the same media type twice.
    DuplicateMediaType {
        /// Item index.
        item: usize,
    },
    /// `text/*` must carry text, and text must use `text/*`.
    PayloadMediaTypeMismatch,
    /// One representation exceeds the inline byte bound.
    RepresentationTooLarge {
        /// Item index.
        item: usize,
    },
    /// All representations together exceed the inline byte bound.
    TotalTooLarge,
    /// Too many metadata entries.
    TooManyMetadataEntries,
    /// Invalid metadata key.
    InvalidMetadataKey,
    /// A metadata value exceeds its bound.
    MetadataValueTooLong,
    /// A metadata value contains control characters.
    InvalidMetadataValue,
    /// The claimed origin label is empty, too long, or contains control characters.
    InvalidOriginLabel,
}

impl fmt::Display for ContentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "invalid clipboard content: {self:?}")
    }
}

impl std::error::Error for ContentError {}
