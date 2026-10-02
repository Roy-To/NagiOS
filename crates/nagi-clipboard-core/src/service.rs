use std::collections::BTreeMap;
use std::fmt;

use crate::authority::{
    AuthorizationDecision, AuthorizationRequest, CallerContext, ClipboardAuthorizer,
    ClipboardOperation, VerifiedOrigin,
};
use crate::diagnostics::{ClipboardDiagnosticsSink, ClipboardEvent, ClipboardEventCode};
use crate::limits::{ClipboardLimits, LimitsError};
use crate::media_type::MediaType;
use crate::model::{
    ClaimedOrigin, ClipboardContent, ClipboardGeneration, ContentError, MetadataKey, Payload,
    PayloadKind, TransferIntent,
};

/// Version of the clipboard service contract.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ContractVersion(pub u16);

/// The only contract version this foundation implements. Requests carrying
/// any other version are rejected, never reinterpreted.
pub const CLIPBOARD_CONTRACT_VERSION: ContractVersion = ContractVersion(1);

/// Replace the whole clipboard content.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WriteRequest {
    /// Contract version the caller speaks.
    pub version: ContractVersion,
    /// New content; replaces all previous items atomically.
    pub content: ClipboardContent,
    /// If set, the write only succeeds when the current generation matches.
    pub expected_generation: Option<ClipboardGeneration>,
}

impl WriteRequest {
    /// Unconditional replacement at the current contract version.
    pub fn replace(content: ClipboardContent) -> Self {
        Self {
            version: CLIPBOARD_CONTRACT_VERSION,
            content,
            expected_generation: None,
        }
    }
}

/// Read one representation of one item.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadRequest {
    /// Contract version the caller speaks.
    pub version: ContractVersion,
    /// If set, the read fails when the clipboard changed since this generation,
    /// so a paste never mixes formats from two different copies.
    pub expected_generation: Option<ClipboardGeneration>,
    /// Item index.
    pub item: usize,
    /// Exact media type requested.
    pub media_type: MediaType,
}

impl ReadRequest {
    /// Reads `media_type` from item `item` without a generation check.
    pub fn new(item: usize, media_type: MediaType) -> Self {
        Self {
            version: CLIPBOARD_CONTRACT_VERSION,
            expected_generation: None,
            item,
            media_type,
        }
    }

    /// Requires the clipboard to still be at `generation`.
    pub fn at_generation(mut self, generation: ClipboardGeneration) -> Self {
        self.expected_generation = Some(generation);
        self
    }
}

/// Clear the clipboard.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClearRequest {
    /// Contract version the caller speaks.
    pub version: ContractVersion,
    /// If set, the clear only succeeds when the current generation matches.
    pub expected_generation: Option<ClipboardGeneration>,
}

impl ClearRequest {
    /// Unconditional clear at the current contract version.
    pub const fn unconditional() -> Self {
        Self {
            version: CLIPBOARD_CONTRACT_VERSION,
            expected_generation: None,
        }
    }
}

/// One listed format. Carries size and kind, never payload bytes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormatDescriptor {
    /// Media type.
    pub media_type: MediaType,
    /// Payload kind.
    pub kind: PayloadKind,
    /// Inline payload length in bytes.
    pub inline_len: usize,
}

/// Formats of one item that the caller is allowed to read, in writer order.
/// An item whose formats are all restricted is still listed (empty) so item
/// indices stay stable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ItemFormats {
    /// Readable formats.
    pub formats: Vec<FormatDescriptor>,
}

/// Snapshot of what the clipboard currently offers, without payloads.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClipboardOffer {
    /// Current generation.
    pub generation: ClipboardGeneration,
    /// Ordered items; empty when the clipboard is empty.
    pub items: Vec<ItemFormats>,
    /// Transfer intent hint (non-destructive).
    pub intent: TransferIntent,
    /// Writer as authenticated at write time; `None` when empty.
    pub verified_origin: Option<VerifiedOrigin>,
    /// Untrusted writer claim, returned verbatim for display only.
    pub claimed_origin: ClaimedOrigin,
    /// Untrusted writer metadata.
    pub metadata: BTreeMap<MetadataKey, String>,
}

impl ClipboardOffer {
    /// Whether the clipboard has no content.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// A successfully read representation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadResult {
    /// Generation the payload was read from.
    pub generation: ClipboardGeneration,
    /// Media type read.
    pub media_type: MediaType,
    /// Payload copy.
    pub payload: Payload,
}

/// Typed clipboard failures. None of them carry payload data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClipboardError {
    /// Request contract version is not implemented.
    UnsupportedVersion {
        /// Version the caller requested.
        requested: ContractVersion,
    },
    /// The authorizer denied the operation.
    Denied,
    /// The authorizer was unavailable; failed closed.
    AuthorizationUnavailable,
    /// Content failed validation; nothing changed.
    Invalid(ContentError),
    /// The expected generation is not current; nothing changed.
    StaleGeneration {
        /// Current generation.
        current: ClipboardGeneration,
    },
    /// The clipboard holds no content.
    Empty,
    /// No item at the requested index.
    ItemOutOfRange,
    /// The item does not offer the requested media type.
    FormatNotAvailable,
    /// The generation counter cannot advance without repeating.
    GenerationExhausted,
}

impl fmt::Display for ClipboardError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "clipboard operation failed: {self:?}")
    }
}

impl std::error::Error for ClipboardError {}

/// Host-testable clipboard service contract.
///
/// Every operation takes the trusted [`CallerContext`] separately from its
/// request, checks the contract version, then authorizes before inspecting
/// state, so a denied caller learns nothing about current content.
pub trait ClipboardService {
    /// Replace all content. Returns the new generation.
    fn write(
        &mut self,
        caller: &CallerContext,
        request: WriteRequest,
    ) -> Result<ClipboardGeneration, ClipboardError>;

    /// Remove all content. Returns the resulting generation. Clearing an
    /// empty clipboard is an idempotent no-op that does not advance it.
    fn clear(
        &mut self,
        caller: &CallerContext,
        request: ClearRequest,
    ) -> Result<ClipboardGeneration, ClipboardError>;

    /// Current generation.
    fn generation(
        &mut self,
        caller: &CallerContext,
        version: ContractVersion,
    ) -> Result<ClipboardGeneration, ClipboardError>;

    /// Formats and untrusted metadata the caller may see.
    fn formats(
        &mut self,
        caller: &CallerContext,
        version: ContractVersion,
    ) -> Result<ClipboardOffer, ClipboardError>;

    /// Read one representation.
    fn read(
        &mut self,
        caller: &CallerContext,
        request: ReadRequest,
    ) -> Result<ReadResult, ClipboardError>;
}

struct Stored {
    content: ClipboardContent,
    verified_origin: VerifiedOrigin,
}

/// Deterministic in-memory reference implementation.
///
/// This is not production persistence: content lives only in this value and
/// is lost when it is dropped. It keeps no history.
pub struct InMemoryClipboard<A> {
    authorizer: A,
    limits: ClipboardLimits,
    generation: ClipboardGeneration,
    current: Option<Stored>,
    sink: Option<Box<dyn ClipboardDiagnosticsSink>>,
    dropped_diagnostics: u64,
}

impl<A: ClipboardAuthorizer> InMemoryClipboard<A> {
    /// Empty clipboard at [`ClipboardGeneration::INITIAL`].
    pub fn new(authorizer: A, limits: ClipboardLimits) -> Result<Self, LimitsError> {
        Self::starting_at(authorizer, limits, ClipboardGeneration::INITIAL)
    }

    /// Empty clipboard whose generation continues from `generation`, e.g.
    /// after a service restart that must not repeat generations.
    pub fn starting_at(
        authorizer: A,
        limits: ClipboardLimits,
        generation: ClipboardGeneration,
    ) -> Result<Self, LimitsError> {
        limits.validate()?;
        Ok(Self {
            authorizer,
            limits,
            generation,
            current: None,
            sink: None,
            dropped_diagnostics: 0,
        })
    }

    /// Opt in to metadata-only diagnostics.
    pub fn with_diagnostics_sink(mut self, sink: Box<dyn ClipboardDiagnosticsSink>) -> Self {
        self.sink = Some(sink);
        self
    }

    /// Configured limits.
    pub fn limits(&self) -> &ClipboardLimits {
        &self.limits
    }

    /// Diagnostic events the sink failed to record.
    pub fn dropped_diagnostics(&self) -> u64 {
        self.dropped_diagnostics
    }

    fn emit(&mut self, code: ClipboardEventCode) {
        let event = ClipboardEvent {
            code,
            generation: self.generation,
        };
        if let Some(sink) = self.sink.as_mut() {
            if sink.record(event).is_err() {
                self.dropped_diagnostics = self.dropped_diagnostics.saturating_add(1);
            }
        }
    }

    fn check_version(&mut self, version: ContractVersion) -> Result<(), ClipboardError> {
        if version != CLIPBOARD_CONTRACT_VERSION {
            self.emit(ClipboardEventCode::UnsupportedVersion);
            return Err(ClipboardError::UnsupportedVersion { requested: version });
        }
        Ok(())
    }

    fn authorize(
        &mut self,
        caller: &CallerContext,
        operation: ClipboardOperation<'_>,
    ) -> Result<(), ClipboardError> {
        let decision = self
            .authorizer
            .authorize(&AuthorizationRequest { caller, operation });
        match decision {
            AuthorizationDecision::Allow => Ok(()),
            AuthorizationDecision::Deny => {
                self.emit(ClipboardEventCode::Denied);
                Err(ClipboardError::Denied)
            }
            AuthorizationDecision::Unavailable => {
                self.emit(ClipboardEventCode::AuthorizationUnavailable);
                Err(ClipboardError::AuthorizationUnavailable)
            }
        }
    }

    fn check_generation(
        &mut self,
        expected: Option<ClipboardGeneration>,
    ) -> Result<(), ClipboardError> {
        match expected {
            Some(expected) if expected != self.generation => {
                self.emit(ClipboardEventCode::StaleGeneration);
                Err(ClipboardError::StaleGeneration {
                    current: self.generation,
                })
            }
            _ => Ok(()),
        }
    }

    fn advance(&mut self) -> Result<ClipboardGeneration, ClipboardError> {
        self.generation
            .next()
            .ok_or(ClipboardError::GenerationExhausted)
    }
}

impl<A: ClipboardAuthorizer> ClipboardService for InMemoryClipboard<A> {
    fn write(
        &mut self,
        caller: &CallerContext,
        request: WriteRequest,
    ) -> Result<ClipboardGeneration, ClipboardError> {
        self.check_version(request.version)?;
        self.authorize(caller, ClipboardOperation::Write)?;
        if let Err(error) = request.content.validate(&self.limits) {
            self.emit(ClipboardEventCode::Rejected);
            return Err(ClipboardError::Invalid(error));
        }
        self.check_generation(request.expected_generation)?;
        let next = self.advance()?;
        self.generation = next;
        self.current = Some(Stored {
            content: request.content,
            verified_origin: VerifiedOrigin::from(caller),
        });
        self.emit(ClipboardEventCode::Written);
        Ok(next)
    }

    fn clear(
        &mut self,
        caller: &CallerContext,
        request: ClearRequest,
    ) -> Result<ClipboardGeneration, ClipboardError> {
        self.check_version(request.version)?;
        self.authorize(caller, ClipboardOperation::Clear)?;
        self.check_generation(request.expected_generation)?;
        if self.current.is_none() {
            return Ok(self.generation);
        }
        let next = self.advance()?;
        self.generation = next;
        self.current = None;
        self.emit(ClipboardEventCode::Cleared);
        Ok(next)
    }

    fn generation(
        &mut self,
        caller: &CallerContext,
        version: ContractVersion,
    ) -> Result<ClipboardGeneration, ClipboardError> {
        self.check_version(version)?;
        self.authorize(caller, ClipboardOperation::ReadGeneration)?;
        Ok(self.generation)
    }

    fn formats(
        &mut self,
        caller: &CallerContext,
        version: ContractVersion,
    ) -> Result<ClipboardOffer, ClipboardError> {
        self.check_version(version)?;
        self.authorize(caller, ClipboardOperation::ReadFormats)?;
        let Some(stored) = self.current.as_ref() else {
            return Ok(ClipboardOffer {
                generation: self.generation,
                items: Vec::new(),
                intent: TransferIntent::Copy,
                verified_origin: None,
                claimed_origin: ClaimedOrigin::default(),
                metadata: BTreeMap::new(),
            });
        };
        let mut items = Vec::with_capacity(stored.content.items().len());
        for item in stored.content.items() {
            let mut formats = Vec::new();
            for representation in item.representations() {
                let operation = ClipboardOperation::ReadRepresentation {
                    media_type: representation.media_type(),
                };
                match self
                    .authorizer
                    .authorize(&AuthorizationRequest { caller, operation })
                {
                    AuthorizationDecision::Allow => formats.push(FormatDescriptor {
                        media_type: representation.media_type().clone(),
                        kind: representation.payload().kind(),
                        inline_len: representation.payload().inline_len(),
                    }),
                    AuthorizationDecision::Deny => {}
                    AuthorizationDecision::Unavailable => {
                        self.emit(ClipboardEventCode::AuthorizationUnavailable);
                        return Err(ClipboardError::AuthorizationUnavailable);
                    }
                }
            }
            items.push(ItemFormats { formats });
        }
        Ok(ClipboardOffer {
            generation: self.generation,
            items,
            intent: stored.content.intent(),
            verified_origin: Some(stored.verified_origin),
            claimed_origin: stored.content.claimed_origin().clone(),
            metadata: stored.content.metadata().clone(),
        })
    }

    fn read(
        &mut self,
        caller: &CallerContext,
        request: ReadRequest,
    ) -> Result<ReadResult, ClipboardError> {
        self.check_version(request.version)?;
        self.authorize(
            caller,
            ClipboardOperation::ReadRepresentation {
                media_type: &request.media_type,
            },
        )?;
        self.check_generation(request.expected_generation)?;
        let stored = self.current.as_ref().ok_or(ClipboardError::Empty)?;
        let item = stored
            .content
            .items()
            .get(request.item)
            .ok_or(ClipboardError::ItemOutOfRange)?;
        let representation = item
            .representation(&request.media_type)
            .ok_or(ClipboardError::FormatNotAvailable)?;
        let result = ReadResult {
            generation: self.generation,
            media_type: representation.media_type().clone(),
            payload: representation.payload().clone(),
        };
        self.emit(ClipboardEventCode::RepresentationRead);
        Ok(result)
    }
}
