use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use nagi_model::{AppId, ObjectId, WorkspaceId};

use crate::actions::{ActionAvailability, CapabilityContext, CapabilityId, TypedAction};
use crate::localization::Locale;

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct ProviderId(String);

impl ProviderId {
    pub fn new(value: impl Into<String>) -> Result<Self, &'static str> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 80
            || !value.bytes().all(|byte| {
                byte.is_ascii_lowercase()
                    || byte.is_ascii_digit()
                    || matches!(byte, b'.' | b'-' | b'_')
            })
        {
            return Err("provider identifiers must be lowercase ASCII tokens");
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct SearchRequestId(pub u64);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderSelection {
    All,
    Only(BTreeSet<ProviderId>),
}

impl ProviderSelection {
    pub fn only(providers: impl IntoIterator<Item = ProviderId>) -> Self {
        Self::Only(providers.into_iter().collect())
    }

    fn includes(&self, provider: &ProviderId) -> bool {
        match self {
            Self::All => true,
            Self::Only(providers) => providers.contains(provider),
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SearchContext {
    pub current_workspace: Option<WorkspaceId>,
    pub current_app: Option<AppId>,
    pub now_unix_seconds: u64,
}

#[derive(Clone, Debug, Default)]
pub struct CancellationToken {
    cancelled: Arc<AtomicBool>,
    parent: Option<Arc<AtomicBool>>,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
            || self
                .parent
                .as_ref()
                .is_some_and(|parent| parent.load(Ordering::Acquire))
    }

    fn child(&self) -> Self {
        Self {
            cancelled: Arc::new(AtomicBool::new(false)),
            parent: Some(Arc::clone(&self.cancelled)),
        }
    }
}

#[derive(Clone, Debug)]
pub struct SearchQuery {
    pub request_id: SearchRequestId,
    pub raw_query: String,
    pub normalized_query: String,
    pub provider_selection: ProviderSelection,
    pub context: SearchContext,
    pub capabilities: CapabilityContext,
    pub cancellation: CancellationToken,
    pub locale: Locale,
}

impl SearchQuery {
    pub fn new(
        raw_query: impl Into<String>,
        request_id: SearchRequestId,
        context: SearchContext,
        capabilities: CapabilityContext,
        locale: Locale,
    ) -> Self {
        let raw_query = raw_query.into();
        let normalized_query = normalize_search_text(&raw_query);
        Self {
            request_id,
            raw_query,
            normalized_query,
            provider_selection: ProviderSelection::All,
            context,
            capabilities,
            cancellation: CancellationToken::new(),
            locale,
        }
    }

    pub fn with_provider_selection(mut self, selection: ProviderSelection) -> Self {
        self.provider_selection = selection;
        self
    }

    pub fn with_cancellation(mut self, cancellation: CancellationToken) -> Self {
        self.cancellation = cancellation;
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct SearchIdentityKey(String);

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SearchIdentity {
    App(AppId),
    Object(ObjectId),
    Workspace(WorkspaceId),
    Action(String),
}

impl SearchIdentity {
    fn stable_key(&self) -> SearchIdentityKey {
        let key = match self {
            Self::App(app) => format!("app:{:016x}", app.0),
            Self::Object(object) => format!("object:{:016x}", object.0),
            Self::Workspace(workspace) => format!("workspace:{:016x}", workspace.0),
            Self::Action(action) => format!("action:{action}"),
        };
        SearchIdentityKey(key)
    }

    pub fn stable_id(&self) -> u64 {
        stable_hash(self.stable_key().0.as_bytes())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchCategory {
    Apps,
    Files,
    Notes,
    Activity,
    Actions,
    Workspaces,
    Settings,
}

impl SearchCategory {
    pub const fn localization_key(self) -> &'static str {
        match self {
            Self::Apps => "search.category.apps",
            Self::Files => "search.category.files",
            Self::Notes => "search.category.notes",
            Self::Activity => "search.category.activity",
            Self::Actions => "search.category.actions",
            Self::Workspaces => "search.category.workspaces",
            Self::Settings => "search.category.settings",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchText {
    pub title: String,
    pub subtitle: Option<String>,
    pub content: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchAction {
    pub action: TypedAction,
    pub required_capability: Option<CapabilityId>,
    pub availability: ActionAvailability,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchCandidate {
    pub identity: SearchIdentity,
    pub category: SearchCategory,
    pub text: SearchText,
    pub modified_at_unix_seconds: Option<u64>,
    pub workspace_ids: Vec<WorkspaceId>,
    /// Metadata visibility is checked after provider retrieval. A denied
    /// candidate is dropped before any result text is returned to the caller.
    pub visibility_capability: Option<CapabilityId>,
    pub action: Option<SearchAction>,
    pub preview: Option<String>,
    pub is_fixture: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderDescriptor {
    pub id: ProviderId,
    /// Small deterministic tie-break signal; relevance match quality remains
    /// the dominant ranking input.
    pub priority: i16,
    pub required_capability: Option<CapabilityId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderError {
    Unavailable,
    PermissionDenied,
    InvalidData,
    Failed,
    Cancelled,
}

pub trait SearchProvider: Send + Sync {
    fn descriptor(&self) -> ProviderDescriptor;
    fn search(&self, query: &SearchQuery) -> Result<Vec<SearchCandidate>, ProviderError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MatchReason {
    ExactTitle,
    TitlePrefix,
    TitleTokenPrefix,
    TitleSubstring,
    SubtitleMatch,
    MetadataMatch,
    ContentMatch,
    WorkspaceContext,
}

impl MatchReason {
    pub const fn localization_key(self) -> &'static str {
        match self {
            Self::ExactTitle => "search.match_exact",
            Self::TitlePrefix => "search.match_prefix",
            Self::TitleTokenPrefix => "search.match_prefix",
            Self::TitleSubstring => "search.match_substring",
            Self::SubtitleMatch => "search.match_metadata",
            Self::MetadataMatch => "search.match_metadata",
            Self::ContentMatch => "search.match_content",
            Self::WorkspaceContext => "search.match_workspace",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchResult {
    pub result_id: u64,
    pub provider_id: ProviderId,
    pub identity: SearchIdentity,
    pub category: SearchCategory,
    pub title: String,
    pub subtitle: Option<String>,
    pub score: i32,
    pub match_reason: MatchReason,
    pub action: Option<TypedAction>,
    pub action_availability: Option<ActionAvailability>,
    pub preview: Option<String>,
    pub is_fixture: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProviderIssue {
    pub provider_id: ProviderId,
    pub kind: ProviderIssueKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderIssueKind {
    Failed(ProviderError),
    TimedOut,
    StartFailed,
    PermissionDenied,
    DuplicateProvider,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchResponse {
    pub request_id: SearchRequestId,
    pub results: Vec<SearchResult>,
    pub provider_issues: Vec<ProviderIssue>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SearchError {
    StaleRequest,
    Cancelled,
}

#[derive(Default)]
struct ActiveRequest {
    latest: Option<(SearchRequestId, CancellationToken)>,
}

pub struct SearchCoordinator {
    provider_timeout: Duration,
    max_results: usize,
    next_request_id: AtomicU64,
    active: Mutex<ActiveRequest>,
}

impl SearchCoordinator {
    pub fn new(provider_timeout: Duration, max_results: usize) -> Self {
        Self {
            provider_timeout,
            max_results,
            next_request_id: AtomicU64::new(1),
            active: Mutex::new(ActiveRequest::default()),
        }
    }

    pub fn allocate_request_id(&self) -> SearchRequestId {
        SearchRequestId(self.next_request_id.fetch_add(1, Ordering::Relaxed))
    }

    pub fn cancel_active(&self) {
        if let Ok(active) = self.active.lock() {
            if let Some((_, token)) = &active.latest {
                token.cancel();
            }
        }
    }

    pub fn cancel(&self, request_id: SearchRequestId) -> bool {
        let Ok(active) = self.active.lock() else {
            return false;
        };
        if let Some((active_id, token)) = &active.latest {
            if *active_id == request_id {
                token.cancel();
                return true;
            }
        }
        false
    }

    pub fn search(
        &self,
        query: SearchQuery,
        providers: &[Arc<dyn SearchProvider>],
    ) -> Result<SearchResponse, SearchError> {
        self.begin_request(&query)?;
        if query.cancellation.is_cancelled() {
            return Err(SearchError::Cancelled);
        }
        if query.normalized_query.is_empty() || self.max_results == 0 {
            return Ok(SearchResponse {
                request_id: query.request_id,
                results: Vec::new(),
                provider_issues: Vec::new(),
            });
        }

        let mut issues = Vec::new();
        let mut seen_providers = BTreeSet::new();
        let mut eligible = Vec::new();
        for provider in providers {
            let descriptor = provider.descriptor();
            if !query.provider_selection.includes(&descriptor.id) {
                continue;
            }
            if !seen_providers.insert(descriptor.id.clone()) {
                issues.push(ProviderIssue {
                    provider_id: descriptor.id,
                    kind: ProviderIssueKind::DuplicateProvider,
                });
                continue;
            }
            if !query
                .capabilities
                .allows(descriptor.required_capability.as_ref())
            {
                issues.push(ProviderIssue {
                    provider_id: descriptor.id,
                    kind: ProviderIssueKind::PermissionDenied,
                });
                continue;
            }
            eligible.push((Arc::clone(provider), descriptor));
        }

        let (sender, receiver) = mpsc::channel();
        let mut pending = BTreeMap::<ProviderId, (ProviderDescriptor, CancellationToken)>::new();
        for (provider, descriptor) in eligible {
            let sender = sender.clone();
            let mut provider_query = query.clone();
            let child_token = query.cancellation.child();
            let pending_token = child_token.clone();
            provider_query.cancellation = child_token.clone();
            let provider_id = descriptor.id.clone();
            let pending_descriptor = descriptor.clone();
            let thread_name = format!("nagi-search-{}", provider_id.as_str());
            let spawn = thread::Builder::new().name(thread_name).spawn(move || {
                let result = if child_token.is_cancelled() {
                    Err(ProviderError::Cancelled)
                } else {
                    provider.search(&provider_query)
                };
                let _ = sender.send((descriptor, result));
            });
            match spawn {
                Ok(_) => {
                    pending.insert(provider_id, (pending_descriptor, pending_token));
                }
                Err(_) => issues.push(ProviderIssue {
                    provider_id,
                    kind: ProviderIssueKind::StartFailed,
                }),
            }
        }
        drop(sender);

        let deadline = Instant::now() + self.provider_timeout;
        let mut candidates = Vec::new();
        while !pending.is_empty() {
            if query.cancellation.is_cancelled() {
                return self.cancel_or_stale(&query);
            }
            if !self.is_current(query.request_id) {
                query.cancellation.cancel();
                return Err(SearchError::StaleRequest);
            }
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            let remaining = deadline.saturating_duration_since(now);
            let wait = remaining.min(Duration::from_millis(10));
            match receiver.recv_timeout(wait) {
                Ok((descriptor, Ok(results))) => {
                    pending.remove(&descriptor.id);
                    for candidate in results {
                        if query
                            .capabilities
                            .allows(candidate.visibility_capability.as_ref())
                        {
                            candidates.push((descriptor.clone(), candidate));
                        }
                    }
                }
                Ok((descriptor, Err(error))) => {
                    pending.remove(&descriptor.id);
                    if error != ProviderError::Cancelled {
                        issues.push(ProviderIssue {
                            provider_id: descriptor.id,
                            kind: ProviderIssueKind::Failed(error),
                        });
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        if !pending.is_empty() {
            for (descriptor, token) in pending.values() {
                issues.push(ProviderIssue {
                    provider_id: descriptor.id.clone(),
                    kind: ProviderIssueKind::TimedOut,
                });
                token.cancel();
            }
            // Only timed-out workers are cancelled. Other results already
            // collected remain valid for this request.
        }
        if query.cancellation.is_cancelled() {
            return self.cancel_or_stale(&query);
        }
        if !self.is_current(query.request_id) {
            query.cancellation.cancel();
            return Err(SearchError::StaleRequest);
        }

        let mut results = rank_candidates(candidates, &query, self.max_results);
        issues.sort_by(|left, right| {
            left.provider_id
                .cmp(&right.provider_id)
                .then_with(|| issue_order(&left.kind).cmp(&issue_order(&right.kind)))
        });
        results.truncate(self.max_results);
        Ok(SearchResponse {
            request_id: query.request_id,
            results,
            provider_issues: issues,
        })
    }

    fn begin_request(&self, query: &SearchQuery) -> Result<(), SearchError> {
        let mut active = self.active.lock().map_err(|_| SearchError::Cancelled)?;
        if let Some((latest_id, token)) = &active.latest {
            if query.request_id <= *latest_id {
                return Err(SearchError::StaleRequest);
            }
            token.cancel();
        }
        active.latest = Some((query.request_id, query.cancellation.clone()));
        let requested_next = query.request_id.0.saturating_add(1);
        self.next_request_id
            .fetch_max(requested_next, Ordering::Relaxed);
        Ok(())
    }

    fn is_current(&self, request_id: SearchRequestId) -> bool {
        self.active.lock().is_ok_and(|active| {
            active
                .latest
                .as_ref()
                .is_some_and(|(latest, _)| *latest == request_id)
        })
    }

    fn cancel_or_stale(&self, query: &SearchQuery) -> Result<SearchResponse, SearchError> {
        if self.is_current(query.request_id) {
            Err(SearchError::Cancelled)
        } else {
            Err(SearchError::StaleRequest)
        }
    }
}

#[derive(Clone, Debug)]
struct RankedResult {
    result: SearchResult,
    identity_key: SearchIdentityKey,
    normalized_title: String,
}

fn rank_candidates(
    candidates: Vec<(ProviderDescriptor, SearchCandidate)>,
    query: &SearchQuery,
    max_results: usize,
) -> Vec<SearchResult> {
    let mut ranked = Vec::with_capacity(candidates.len());
    for (provider, candidate) in candidates {
        let Some((base_score, reason)) = relevance(&candidate.text, &query.normalized_query) else {
            continue;
        };
        let workspace_boost = if query
            .context
            .current_workspace
            .is_some_and(|current| candidate.workspace_ids.contains(&current))
        {
            150
        } else {
            0
        };
        let recency_boost = candidate
            .modified_at_unix_seconds
            .filter(|modified| *modified <= query.context.now_unix_seconds)
            .map(|modified| {
                let age_days = (query.context.now_unix_seconds - modified) / 86_400;
                (100 / (age_days + 1).min(100)) as i32
            })
            .unwrap_or(0);
        let provider_boost = i32::from(provider.priority.clamp(-50, 50));
        let score = base_score + workspace_boost + recency_boost + provider_boost;
        let action_availability = candidate.action.as_ref().map(|action| {
            if !query
                .capabilities
                .allows(action.required_capability.as_ref())
            {
                ActionAvailability::PermissionRequired {
                    capability: action
                        .required_capability
                        .clone()
                        .expect("missing capability is checked above"),
                }
            } else {
                action.availability.clone()
            }
        });
        let identity_key = candidate.identity.stable_key();
        ranked.push(RankedResult {
            result: SearchResult {
                result_id: candidate.identity.stable_id(),
                provider_id: provider.id,
                identity: candidate.identity,
                category: candidate.category,
                title: candidate.text.title.clone(),
                subtitle: candidate.text.subtitle.clone(),
                score,
                match_reason: reason,
                action: candidate.action.map(|action| action.action),
                action_availability,
                preview: candidate.preview,
                is_fixture: candidate.is_fixture,
            },
            identity_key,
            normalized_title: normalize_search_text(&candidate.text.title),
        });
    }
    ranked.sort_by(|left, right| {
        right
            .result
            .score
            .cmp(&left.result.score)
            .then_with(|| left.normalized_title.cmp(&right.normalized_title))
            .then_with(|| left.identity_key.cmp(&right.identity_key))
            .then_with(|| left.result.provider_id.cmp(&right.result.provider_id))
    });

    // Cross-provider duplicates share the stable object/action identity. Keep
    // the best ranked visible entry, with deterministic provider tie-breaking.
    let mut unique = BTreeMap::new();
    for ranked in ranked {
        unique.entry(ranked.identity_key).or_insert(ranked.result);
    }
    let mut results: Vec<_> = unique.into_values().collect();
    results.sort_by(|left, right| {
        right
            .score
            .cmp(&left.score)
            .then_with(|| {
                normalize_search_text(&left.title).cmp(&normalize_search_text(&right.title))
            })
            .then_with(|| left.identity.stable_key().cmp(&right.identity.stable_key()))
            .then_with(|| left.provider_id.cmp(&right.provider_id))
    });
    results.truncate(max_results);
    results
}

fn relevance(text: &SearchText, query: &str) -> Option<(i32, MatchReason)> {
    let title = normalize_search_text(&text.title);
    if title == query {
        return Some((10_000, MatchReason::ExactTitle));
    }
    if title.starts_with(query) {
        return Some((8_000, MatchReason::TitlePrefix));
    }
    if token_prefix_match(&title, query) {
        return Some((6_500, MatchReason::TitleTokenPrefix));
    }
    if title.contains(query) {
        return Some((6_000, MatchReason::TitleSubstring));
    }
    if let Some(subtitle) = &text.subtitle {
        let subtitle = normalize_search_text(subtitle);
        if subtitle == query || subtitle.starts_with(query) || subtitle.contains(query) {
            return Some((4_600, MatchReason::SubtitleMatch));
        }
    }
    if text
        .tags
        .iter()
        .map(|tag| normalize_search_text(tag))
        .any(|tag| tag == query || tag.starts_with(query) || tag.contains(query))
    {
        return Some((4_100, MatchReason::MetadataMatch));
    }
    if text
        .content
        .as_ref()
        .is_some_and(|content| normalize_search_text(content).contains(query))
    {
        return Some((3_000, MatchReason::ContentMatch));
    }
    let query_tokens = unicode_tokens(query);
    if query_tokens.len() > 1 {
        let searchable = [
            Some(title),
            text.subtitle.as_deref().map(normalize_search_text),
            text.content.as_deref().map(normalize_search_text),
            Some(text.tags.join(" ").to_lowercase()),
        ]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
        if query_tokens.iter().all(|token| {
            searchable.iter().any(|field| {
                unicode_tokens(field)
                    .iter()
                    .any(|field_token| field_token.contains(token))
                    || field.contains(token)
            })
        }) {
            return Some((2_400, MatchReason::MetadataMatch));
        }
    }
    None
}

fn token_prefix_match(title: &str, query: &str) -> bool {
    let query_tokens = unicode_tokens(query);
    if query_tokens.is_empty() {
        return false;
    }
    let title_tokens = unicode_tokens(title);
    query_tokens.iter().all(|query_token| {
        title_tokens
            .iter()
            .any(|title_token| title_token.starts_with(query_token))
    })
}

fn unicode_tokens(value: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    for character in value.chars() {
        if character.is_alphanumeric() {
            current.push(character);
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// Case-folds text and maps full-width ASCII into its narrow equivalent while
/// retaining Japanese scripts and all other Unicode content. No ASCII-only
/// tokenization or destructive transliteration is used.
pub fn normalize_search_text(value: &str) -> String {
    let mut normalized = String::with_capacity(value.len());
    let mut prior_space = true;
    for character in value.chars() {
        let character = if character == '\u{3000}' {
            ' '
        } else if ('\u{ff01}'..='\u{ff5e}').contains(&character) {
            char::from_u32(character as u32 - 0xfee0).unwrap_or(character)
        } else {
            character
        };
        if character.is_whitespace() {
            if !prior_space {
                normalized.push(' ');
            }
            prior_space = true;
        } else {
            for folded in character.to_lowercase() {
                normalized.push(folded);
            }
            prior_space = false;
        }
    }
    if normalized.ends_with(' ') {
        normalized.pop();
    }
    normalized
}

fn stable_hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x1000_0000_01b3)
    })
}

fn issue_order(issue: &ProviderIssueKind) -> u8 {
    match issue {
        ProviderIssueKind::Failed(_) => 0,
        ProviderIssueKind::TimedOut => 1,
        ProviderIssueKind::StartFailed => 2,
        ProviderIssueKind::PermissionDenied => 3,
        ProviderIssueKind::DuplicateProvider => 4,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use nagi_model::{AppId, ObjectId, WorkspaceId};

    use crate::actions::{ActionAvailability, CapabilityContext, CapabilityId, TypedAction};
    use crate::fixtures::{demo_capabilities, demo_search_providers};
    use crate::localization::Locale;

    use super::{
        normalize_search_text, MatchReason, ProviderDescriptor, ProviderError, ProviderId,
        ProviderIssueKind, SearchAction, SearchCandidate, SearchCategory, SearchContext,
        SearchCoordinator, SearchIdentity, SearchProvider, SearchQuery, SearchRequestId,
        SearchText,
    };

    struct TestProvider {
        descriptor: ProviderDescriptor,
        candidates: Vec<SearchCandidate>,
        failure: Option<ProviderError>,
        delay: Duration,
        calls: AtomicUsize,
    }

    impl TestProvider {
        fn new(id: &str, priority: i16, candidates: Vec<SearchCandidate>) -> Self {
            Self {
                descriptor: ProviderDescriptor {
                    id: ProviderId::new(id).unwrap(),
                    priority,
                    required_capability: None,
                },
                candidates,
                failure: None,
                delay: Duration::ZERO,
                calls: AtomicUsize::new(0),
            }
        }

        fn with_required_capability(mut self, value: &str) -> Self {
            self.descriptor.required_capability = Some(CapabilityId::new(value).unwrap());
            self
        }
    }

    impl SearchProvider for TestProvider {
        fn descriptor(&self) -> ProviderDescriptor {
            self.descriptor.clone()
        }

        fn search(&self, query: &SearchQuery) -> Result<Vec<SearchCandidate>, ProviderError> {
            self.calls.fetch_add(1, Ordering::Relaxed);
            if let Some(failure) = self.failure.clone() {
                return Err(failure);
            }
            let until = std::time::Instant::now() + self.delay;
            while std::time::Instant::now() < until {
                if query.cancellation.is_cancelled() {
                    return Err(ProviderError::Cancelled);
                }
                thread::sleep(Duration::from_millis(1));
            }
            if query.cancellation.is_cancelled() {
                return Err(ProviderError::Cancelled);
            }
            Ok(self.candidates.clone())
        }
    }

    fn candidate(
        identity: SearchIdentity,
        title: &str,
        category: SearchCategory,
    ) -> SearchCandidate {
        SearchCandidate {
            identity,
            category,
            text: SearchText {
                title: title.to_owned(),
                subtitle: None,
                content: None,
                tags: Vec::new(),
            },
            modified_at_unix_seconds: None,
            workspace_ids: Vec::new(),
            visibility_capability: None,
            action: None,
            preview: None,
            is_fixture: false,
        }
    }

    fn query(text: &str, id: u64) -> SearchQuery {
        SearchQuery::new(
            text,
            SearchRequestId(id),
            SearchContext {
                current_workspace: None,
                current_app: None,
                now_unix_seconds: now(),
            },
            CapabilityContext::default(),
            Locale::EnUs,
        )
    }

    fn now() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
    }

    fn coordinator(timeout_ms: u64) -> SearchCoordinator {
        SearchCoordinator::new(Duration::from_millis(timeout_ms), 100_000)
    }

    #[test]
    fn empty_query_does_not_call_any_provider() {
        let provider = Arc::new(TestProvider::new(
            "files",
            0,
            vec![candidate(
                SearchIdentity::Object(ObjectId(1)),
                "file",
                SearchCategory::Files,
            )],
        ));
        let providers: Vec<Arc<dyn SearchProvider>> = vec![provider.clone()];
        let response = coordinator(100)
            .search(query("  \u{3000} ", 1), &providers)
            .unwrap();
        assert!(response.results.is_empty());
        assert_eq!(provider.calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn zero_results_is_a_successful_empty_response() {
        let provider = Arc::new(TestProvider::new(
            "files",
            0,
            vec![candidate(
                SearchIdentity::Object(ObjectId(1)),
                "Unrelated document",
                SearchCategory::Files,
            )],
        ));
        let providers: Vec<Arc<dyn SearchProvider>> = vec![provider];
        let response = coordinator(100)
            .search(query("missing title", 1), &providers)
            .unwrap();
        assert!(response.results.is_empty());
        assert!(response.provider_issues.is_empty());
    }

    #[test]
    fn exact_prefix_and_substring_ranking_is_deterministic() {
        let provider = Arc::new(TestProvider::new(
            "files",
            0,
            vec![
                candidate(
                    SearchIdentity::Object(ObjectId(3)),
                    "My Search Draft",
                    SearchCategory::Files,
                ),
                candidate(
                    SearchIdentity::Object(ObjectId(2)),
                    "Search Notes",
                    SearchCategory::Files,
                ),
                candidate(
                    SearchIdentity::Object(ObjectId(1)),
                    "search",
                    SearchCategory::Files,
                ),
            ],
        ));
        let providers: Vec<Arc<dyn SearchProvider>> = vec![provider];
        let response = coordinator(100)
            .search(query("search", 1), &providers)
            .unwrap();
        assert_eq!(
            response.results[0].identity,
            SearchIdentity::Object(ObjectId(1))
        );
        assert_eq!(response.results[0].match_reason, MatchReason::ExactTitle);
        assert!(response.results[1].score > response.results[2].score);
    }

    #[test]
    fn unicode_width_case_and_japanese_text_are_preserved_for_search() {
        assert_eq!(normalize_search_text("  ＨＯＭＥ　検索 "), "home 検索");
        let provider = Arc::new(TestProvider::new(
            "notes",
            0,
            vec![
                candidate(
                    SearchIdentity::Object(ObjectId(1)),
                    "Home 検索設計メモ",
                    SearchCategory::Notes,
                ),
                candidate(
                    SearchIdentity::Object(ObjectId(2)),
                    "ホーム画面",
                    SearchCategory::Notes,
                ),
            ],
        ));
        let providers: Vec<Arc<dyn SearchProvider>> = vec![provider];
        let response = coordinator(100)
            .search(query("検索", 1), &providers)
            .unwrap();
        assert_eq!(response.results.len(), 1);
        assert!(response.results[0].title.contains("検索"));
    }

    #[test]
    fn mixed_japanese_english_query_matches_without_ascii_tokenizer_loss() {
        let provider = Arc::new(TestProvider::new(
            "notes",
            0,
            vec![candidate(
                SearchIdentity::Object(ObjectId(10)),
                "Home 検索設計メモ",
                SearchCategory::Notes,
            )],
        ));
        let providers: Vec<Arc<dyn SearchProvider>> = vec![provider];
        let response = coordinator(100)
            .search(query("HOME 検索", 1), &providers)
            .unwrap();
        assert_eq!(response.results.len(), 1);
    }

    #[test]
    fn provider_failure_is_reported_without_failing_other_results() {
        let good = Arc::new(TestProvider::new(
            "apps",
            1,
            vec![candidate(
                SearchIdentity::App(AppId::from_identifier(b"com.nagi.search")),
                "Search",
                SearchCategory::Apps,
            )],
        ));
        let mut broken = TestProvider::new("notes", 0, Vec::new());
        broken.failure = Some(ProviderError::Unavailable);
        let broken = Arc::new(broken);
        let providers: Vec<Arc<dyn SearchProvider>> = vec![good, broken];
        let response = coordinator(100)
            .search(query("search", 1), &providers)
            .unwrap();
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.provider_issues.len(), 1);
        assert_eq!(
            response.provider_issues[0].kind,
            ProviderIssueKind::Failed(ProviderError::Unavailable)
        );
    }

    #[test]
    fn a_slow_provider_times_out_without_discarding_a_fast_provider() {
        let fast = Arc::new(TestProvider::new(
            "apps",
            1,
            vec![candidate(
                SearchIdentity::App(AppId::from_identifier(b"com.nagi.search")),
                "Search",
                SearchCategory::Apps,
            )],
        ));
        let mut slow = TestProvider::new("activity", 0, Vec::new());
        slow.delay = Duration::from_millis(100);
        let slow = Arc::new(slow);
        let providers: Vec<Arc<dyn SearchProvider>> = vec![fast, slow];
        let response = coordinator(20)
            .search(query("search", 1), &providers)
            .unwrap();
        assert_eq!(response.results.len(), 1);
        assert!(response
            .provider_issues
            .iter()
            .any(|issue| issue.kind == ProviderIssueKind::TimedOut));
    }

    #[test]
    fn starting_a_new_request_cancels_and_suppresses_the_old_result() {
        let (started_sender, started_receiver) = mpsc::channel();
        struct SlowOnceProvider {
            started: Mutex<Option<mpsc::Sender<()>>>,
        }
        impl SearchProvider for SlowOnceProvider {
            fn descriptor(&self) -> ProviderDescriptor {
                ProviderDescriptor {
                    id: ProviderId::new("notes").unwrap(),
                    priority: 0,
                    required_capability: None,
                }
            }
            fn search(&self, query: &SearchQuery) -> Result<Vec<SearchCandidate>, ProviderError> {
                if query.request_id == SearchRequestId(1) {
                    if let Some(sender) = self.started.lock().unwrap().take() {
                        let _ = sender.send(());
                    }
                    while !query.cancellation.is_cancelled() {
                        thread::sleep(Duration::from_millis(1));
                    }
                    return Err(ProviderError::Cancelled);
                }
                Ok(vec![candidate(
                    SearchIdentity::Object(ObjectId(2)),
                    "Search latest",
                    SearchCategory::Notes,
                )])
            }
        }
        let provider: Arc<dyn SearchProvider> = Arc::new(SlowOnceProvider {
            started: Mutex::new(Some(started_sender)),
        });
        let providers = vec![provider];
        let service = Arc::new(coordinator(500));
        let old_service = Arc::clone(&service);
        let old_providers = providers.clone();
        let old = thread::spawn(move || old_service.search(query("search", 1), &old_providers));
        started_receiver
            .recv_timeout(Duration::from_secs(1))
            .unwrap();
        let current = service.search(query("search", 2), &providers).unwrap();
        assert_eq!(current.results.len(), 1);
        assert_eq!(old.join().unwrap(), Err(super::SearchError::StaleRequest));
    }

    #[test]
    fn explicit_request_cancellation_stops_provider_work() {
        let (started_sender, started_receiver) = mpsc::channel();
        struct CancellableProvider {
            started: mpsc::Sender<()>,
        }
        impl SearchProvider for CancellableProvider {
            fn descriptor(&self) -> ProviderDescriptor {
                ProviderDescriptor {
                    id: ProviderId::new("notes").unwrap(),
                    priority: 0,
                    required_capability: None,
                }
            }

            fn search(&self, query: &SearchQuery) -> Result<Vec<SearchCandidate>, ProviderError> {
                let _ = self.started.send(());
                while !query.cancellation.is_cancelled() {
                    thread::sleep(Duration::from_millis(1));
                }
                Err(ProviderError::Cancelled)
            }
        }

        let service = Arc::new(coordinator(500));
        let provider: Arc<dyn SearchProvider> = Arc::new(CancellableProvider {
            started: started_sender,
        });
        let providers = vec![provider];
        let active_service = Arc::clone(&service);
        let active_providers = providers.clone();
        let active =
            thread::spawn(move || active_service.search(query("cancel me", 17), &active_providers));
        started_receiver
            .recv_timeout(Duration::from_secs(1))
            .unwrap();

        assert!(service.cancel(SearchRequestId(17)));
        assert_eq!(active.join().unwrap(), Err(super::SearchError::Cancelled));
    }

    #[test]
    fn source_and_result_capabilities_filter_before_exposing_text() {
        let secret = CapabilityId::new("notes.private.read").unwrap();
        let provider = Arc::new(
            TestProvider::new(
                "notes",
                0,
                vec![SearchCandidate {
                    visibility_capability: Some(secret),
                    ..candidate(
                        SearchIdentity::Object(ObjectId(9)),
                        "Confidential Project Phoenix",
                        SearchCategory::Notes,
                    )
                }],
            )
            .with_required_capability("search.notes"),
        );
        let providers: Vec<Arc<dyn SearchProvider>> = vec![provider.clone()];
        let denied = coordinator(100)
            .search(query("Phoenix", 1), &providers)
            .unwrap();
        assert!(denied.results.is_empty());
        assert_eq!(provider.calls.load(Ordering::Relaxed), 0);

        let visible = SearchQuery::new(
            "Phoenix",
            SearchRequestId(2),
            SearchContext::default(),
            CapabilityContext::from_grants([
                CapabilityId::new("search.notes").unwrap(),
                CapabilityId::new("notes.private.read").unwrap(),
            ]),
            Locale::EnUs,
        );
        assert_eq!(
            coordinator(100)
                .search(visible, &providers)
                .unwrap()
                .results
                .len(),
            1
        );
    }

    #[test]
    fn denied_open_permission_keeps_visible_metadata_but_disables_action() {
        let provider = Arc::new(TestProvider::new(
            "files",
            0,
            vec![SearchCandidate {
                visibility_capability: Some(CapabilityId::new("files.metadata.read").unwrap()),
                action: Some(SearchAction {
                    action: TypedAction::OpenObject {
                        object_id: ObjectId(7),
                        app_id: AppId::from_identifier(b"com.nagi.files"),
                    },
                    required_capability: Some(CapabilityId::new("files.open").unwrap()),
                    availability: ActionAvailability::Ready,
                }),
                ..candidate(
                    SearchIdentity::Object(ObjectId(7)),
                    "Project Brief",
                    SearchCategory::Files,
                )
            }],
        ));
        let providers: Vec<Arc<dyn SearchProvider>> = vec![provider];
        let request = SearchQuery::new(
            "Project Brief",
            SearchRequestId(1),
            SearchContext::default(),
            CapabilityContext::from_grants([CapabilityId::new("files.metadata.read").unwrap()]),
            Locale::EnUs,
        );
        let response = coordinator(100).search(request, &providers).unwrap();
        assert_eq!(response.results.len(), 1);
        assert_eq!(
            response.results[0].action_availability,
            Some(ActionAvailability::PermissionRequired {
                capability: CapabilityId::new("files.open").unwrap()
            })
        );
    }

    #[test]
    fn duplicate_identity_from_providers_returns_the_best_ranked_result_once() {
        let identity = SearchIdentity::Object(ObjectId(7));
        let weak = Arc::new(TestProvider::new(
            "activity",
            0,
            vec![candidate(
                identity.clone(),
                "A Search record",
                SearchCategory::Activity,
            )],
        ));
        let strong = Arc::new(TestProvider::new(
            "notes",
            0,
            vec![candidate(identity, "Search", SearchCategory::Notes)],
        ));
        let providers: Vec<Arc<dyn SearchProvider>> = vec![weak, strong];
        let response = coordinator(100)
            .search(query("search", 1), &providers)
            .unwrap();
        assert_eq!(response.results.len(), 1);
        assert_eq!(response.results[0].category, SearchCategory::Notes);
    }

    #[test]
    fn ranking_ties_have_repeatable_order() {
        let a = Arc::new(TestProvider::new(
            "files",
            0,
            vec![
                candidate(
                    SearchIdentity::Object(ObjectId(2)),
                    "Alpha Search",
                    SearchCategory::Files,
                ),
                candidate(
                    SearchIdentity::Object(ObjectId(1)),
                    "Alpha Search",
                    SearchCategory::Files,
                ),
            ],
        ));
        let providers: Vec<Arc<dyn SearchProvider>> = vec![a];
        let service = coordinator(100);
        let first = service
            .search(query("Alpha Search", 1), &providers)
            .unwrap();
        let second = service
            .search(query("Alpha Search", 2), &providers)
            .unwrap();
        let first_ids: Vec<_> = first.results.iter().map(|item| item.result_id).collect();
        let second_ids: Vec<_> = second.results.iter().map(|item| item.result_id).collect();
        assert_eq!(first_ids, second_ids);
    }

    #[test]
    fn result_limit_handles_a_large_synthetic_collection() {
        let candidates = (0..25_000)
            .map(|index| {
                candidate(
                    SearchIdentity::Object(ObjectId(index)),
                    &format!("Search document {index:05}"),
                    SearchCategory::Files,
                )
            })
            .collect();
        let provider = Arc::new(TestProvider::new("files", 0, candidates));
        let providers: Vec<Arc<dyn SearchProvider>> = vec![provider];
        let service = SearchCoordinator::new(Duration::from_secs(2), 75);
        let response = service.search(query("Search", 1), &providers).unwrap();
        assert_eq!(response.results.len(), 75);
    }

    #[test]
    fn query_provider_selection_is_enforced() {
        let providers = demo_search_providers();
        let request = query("Home", 1).with_provider_selection(super::ProviderSelection::only([
            ProviderId::new("apps").unwrap(),
        ]));
        let response = coordinator(100).search(request, &providers).unwrap();
        assert!(response
            .results
            .iter()
            .all(|result| result.provider_id.as_str() == "apps"));
    }

    #[test]
    fn current_workspace_and_recency_are_only_bounded_ranking_signals() {
        let mut related = candidate(
            SearchIdentity::Object(ObjectId(1)),
            "Nagi architecture",
            SearchCategory::Notes,
        );
        related.workspace_ids.push(WorkspaceId(42));
        related.modified_at_unix_seconds = Some(1_799_999_999);
        let other = candidate(
            SearchIdentity::Object(ObjectId(2)),
            "Nagi architecture",
            SearchCategory::Notes,
        );
        let provider = Arc::new(TestProvider::new("notes", 0, vec![other, related]));
        let providers: Vec<Arc<dyn SearchProvider>> = vec![provider];
        let request = SearchQuery::new(
            "Nagi architecture",
            SearchRequestId(1),
            SearchContext {
                current_workspace: Some(WorkspaceId(42)),
                current_app: None,
                now_unix_seconds: 1_800_000_000,
            },
            demo_capabilities(),
            Locale::EnUs,
        );
        let response = coordinator(100).search(request, &providers).unwrap();
        assert_eq!(
            response.results[0].identity,
            SearchIdentity::Object(ObjectId(1))
        );
    }
}
