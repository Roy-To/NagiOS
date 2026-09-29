//! Safe browser session/history/bookmark persistence with normal restore checks.

use std::collections::HashSet;

use crate::address_bar::normalize_address;
use crate::bookmarks::{Bookmark, BookmarkId, BookmarkStore, MAX_BOOKMARKS};
use crate::browser_state::BrowserState;
use crate::clipboard::BrowserClipboard;
use crate::downloads::DownloadsState;
use crate::history::{HistoryEntry, HistoryEntryId, MAX_HISTORY_ENTRIES, MAX_HISTORY_TITLE_BYTES};
use crate::navigation::NavigationRequest;
use crate::permissions::PermissionBrokerState;
use crate::persistence::{
    decode_record, encode_record, BrowserStorage, CodecError, Decoder, Encoder, RecordKind,
    StorageError, StorageRecord, StorageWrite, MAX_RECORD_BYTES,
};
use crate::tabs::{Tab, TabId, MAX_TABS};
use crate::uploads::UploadsState;

const MAX_SESSION_HISTORY_REFERENCES: usize = 200;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RestoreWarning {
    MissingSession,
    CorruptSession(CodecError),
    MissingHistory,
    CorruptHistory(CodecError),
    MissingBookmarks,
    CorruptBookmarks(CodecError),
    HistoryReferencesCleared,
    InvalidSessionState,
    StorageUnavailable(StorageError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RestoredBrowser {
    pub browser: BrowserState,
    pub navigation_requests: Vec<NavigationRequest>,
    pub permissions: PermissionBrokerState,
    pub clipboard: BrowserClipboard,
    pub downloads: DownloadsState,
    pub uploads: UploadsState,
    pub warnings: Vec<RestoreWarning>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PersistedTab {
    id: TabId,
    current_url: Option<String>,
    title: String,
    history_ids: Vec<HistoryEntryId>,
    history_cursor: Option<usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PersistedSession {
    active_tab_id: TabId,
    tabs: Vec<PersistedTab>,
}

pub fn save(browser: &BrowserState, storage: &mut impl BrowserStorage) -> Result<(), StorageError> {
    let session = encode_session(browser).map_err(|_| StorageError::Capacity)?;
    let history = encode_history(browser.history()).map_err(|_| StorageError::Capacity)?;
    let bookmarks = encode_bookmarks(browser.bookmarks()).map_err(|_| StorageError::Capacity)?;
    storage.write_batch(&[
        StorageWrite {
            record: StorageRecord::Session,
            bytes: session,
        },
        StorageWrite {
            record: StorageRecord::History,
            bytes: history,
        },
        StorageWrite {
            record: StorageRecord::Bookmarks,
            bytes: bookmarks,
        },
    ])
}

pub fn restore(storage: &mut impl BrowserStorage) -> RestoredBrowser {
    let session = storage.read_record(StorageRecord::Session);
    let history = storage.read_record(StorageRecord::History);
    let bookmarks = storage.read_record(StorageRecord::Bookmarks);
    match (session, history, bookmarks) {
        (Ok(session), Ok(history), Ok(bookmarks)) => {
            restore_bytes(session.as_deref(), history.as_deref(), bookmarks.as_deref())
        }
        (session, history, bookmarks) => {
            let error = session
                .err()
                .or_else(|| history.err())
                .or_else(|| bookmarks.err())
                .unwrap_or(StorageError::Unavailable);
            let mut restored = fresh();
            restored
                .warnings
                .push(RestoreWarning::StorageUnavailable(error));
            restored
        }
    }
}

pub fn restore_bytes(
    session: Option<&[u8]>,
    history: Option<&[u8]>,
    bookmarks: Option<&[u8]>,
) -> RestoredBrowser {
    let mut warnings = Vec::new();
    let Some(session) = session else {
        warnings.push(RestoreWarning::MissingSession);
        let mut restored = fresh();
        restored.warnings = warnings;
        return restored;
    };
    let mut persisted = match decode_session(session) {
        Ok(session) => session,
        Err(error) => {
            warnings.push(RestoreWarning::CorruptSession(error));
            let mut restored = fresh();
            restored.warnings = warnings;
            return restored;
        }
    };
    let history_entries = match history {
        Some(bytes) => match decode_history(bytes) {
            Ok(entries) => entries,
            Err(error) => {
                warnings.push(RestoreWarning::CorruptHistory(error));
                Vec::new()
            }
        },
        None => {
            warnings.push(RestoreWarning::MissingHistory);
            Vec::new()
        }
    };
    let bookmark_store = match bookmarks {
        Some(bytes) => match decode_bookmarks(bytes) {
            Ok(entries) => BookmarkStore::from_entries(entries).unwrap_or_default(),
            Err(error) => {
                warnings.push(RestoreWarning::CorruptBookmarks(error));
                BookmarkStore::new()
            }
        },
        None => {
            warnings.push(RestoreWarning::MissingBookmarks);
            BookmarkStore::new()
        }
    };
    let available: HashSet<_> = history_entries.iter().map(|entry| entry.id).collect();
    let mut references_cleared = false;
    for tab in &mut persisted.tabs {
        let old_len = tab.history_ids.len();
        let old_cursor = tab.history_cursor;
        let mut kept = Vec::with_capacity(old_len);
        let mut new_cursor = None;
        for (index, id) in tab.history_ids.iter().enumerate() {
            if available.contains(id) {
                kept.push(*id);
                if old_cursor.is_some_and(|cursor| index <= cursor) {
                    new_cursor = Some(kept.len() - 1);
                }
            } else {
                references_cleared = true;
            }
        }
        tab.history_ids = kept;
        tab.history_cursor = new_cursor;
    }
    if references_cleared {
        warnings.push(RestoreWarning::HistoryReferencesCleared);
    }
    let tabs = persisted
        .tabs
        .into_iter()
        .map(|tab| {
            Tab::restored(
                tab.id,
                tab.current_url,
                tab.title,
                tab.history_ids,
                tab.history_cursor,
            )
        })
        .collect();
    match BrowserState::set_restored_parts(
        tabs,
        persisted.active_tab_id,
        history_entries,
        bookmark_store,
    ) {
        Ok(mut browser) => {
            let navigation_requests = browser.restored_navigation_requests();
            RestoredBrowser {
                browser,
                navigation_requests,
                permissions: PermissionBrokerState::new(),
                clipboard: BrowserClipboard::new(),
                downloads: DownloadsState::new(),
                uploads: UploadsState::new(),
                warnings,
            }
        }
        Err(_) => {
            warnings.push(RestoreWarning::InvalidSessionState);
            let mut restored = fresh();
            restored.warnings = warnings;
            restored
        }
    }
}

pub fn encode_session(browser: &BrowserState) -> Result<Vec<u8>, CodecError> {
    let Some(active_tab_id) = browser.active_tab_id else {
        return Err(CodecError::InvalidValue);
    };
    if browser.tabs.is_empty() || browser.tabs.len() > MAX_TABS {
        return Err(CodecError::InvalidValue);
    }
    let mut payload = Encoder::new();
    payload.u64(active_tab_id.0);
    payload.u16(browser.tabs.len() as u16);
    for tab in &browser.tabs {
        payload.u64(tab.id.0);
        payload.optional_string(tab.navigation.current_url())?;
        payload.string(tab.navigation.title())?;
        if tab.history_ids.len() > MAX_SESSION_HISTORY_REFERENCES {
            return Err(CodecError::TooLarge);
        }
        payload.u16(tab.history_ids.len() as u16);
        for id in &tab.history_ids {
            payload.u64(id.0);
        }
        match tab.history_cursor {
            Some(cursor) if cursor <= u16::MAX as usize => payload.u16(cursor as u16),
            None => payload.u16(u16::MAX),
            Some(_) => return Err(CodecError::TooLarge),
        }
    }
    encode_record(RecordKind::Session, payload.finish())
}

pub fn encode_history(entries: &[HistoryEntry]) -> Result<Vec<u8>, CodecError> {
    if entries.len() > MAX_HISTORY_ENTRIES {
        return Err(CodecError::TooLarge);
    }
    let mut payload = Encoder::new();
    payload.u16(entries.len() as u16);
    for entry in entries {
        payload.u64(entry.id.0);
        payload.string(&entry.url)?;
        payload.string(&entry.title)?;
        payload.u64(entry.visited_at);
    }
    encode_record(RecordKind::History, payload.finish())
}

pub fn encode_bookmarks(bookmarks: &BookmarkStore) -> Result<Vec<u8>, CodecError> {
    if bookmarks.entries().len() > MAX_BOOKMARKS {
        return Err(CodecError::TooLarge);
    }
    let mut payload = Encoder::new();
    payload.u16(bookmarks.entries().len() as u16);
    for bookmark in bookmarks.entries() {
        payload.u64(bookmark.id.0);
        payload.string(&bookmark.url)?;
        payload.string(&bookmark.title)?;
        payload.u64(bookmark.created_at);
        payload.u64(bookmark.updated_at);
    }
    encode_record(RecordKind::Bookmarks, payload.finish())
}

fn decode_session(bytes: &[u8]) -> Result<PersistedSession, CodecError> {
    let payload = decode_record(bytes, RecordKind::Session)?;
    if bytes.len() > MAX_RECORD_BYTES {
        return Err(CodecError::TooLarge);
    }
    let mut decoder = Decoder::new(payload);
    let active_tab_id = TabId(decoder.u64()?);
    let count = usize::from(decoder.u16()?);
    if count == 0 || count > MAX_TABS {
        return Err(CodecError::InvalidValue);
    }
    let mut tabs = Vec::with_capacity(count);
    let mut ids = HashSet::new();
    for _ in 0..count {
        let id = TabId(decoder.u64()?);
        if id.0 == 0 || !ids.insert(id) {
            return Err(CodecError::InvalidValue);
        }
        let current_url = validate_optional_url(decoder.optional_string()?)?;
        let title = bounded_title(decoder.string()?);
        let history_count = usize::from(decoder.u16()?);
        if history_count > MAX_SESSION_HISTORY_REFERENCES {
            return Err(CodecError::TooLarge);
        }
        let mut history_ids = Vec::with_capacity(history_count);
        for _ in 0..history_count {
            let entry_id = HistoryEntryId(decoder.u64()?);
            if entry_id.0 == 0 {
                return Err(CodecError::InvalidValue);
            }
            history_ids.push(entry_id);
        }
        let raw_cursor = decoder.u16()?;
        let history_cursor = if raw_cursor == u16::MAX {
            None
        } else {
            let cursor = usize::from(raw_cursor);
            if cursor >= history_ids.len() {
                return Err(CodecError::InvalidValue);
            }
            Some(cursor)
        };
        tabs.push(PersistedTab {
            id,
            current_url,
            title,
            history_ids,
            history_cursor,
        });
    }
    decoder.finish()?;
    if !ids.contains(&active_tab_id) {
        return Err(CodecError::InvalidValue);
    }
    Ok(PersistedSession {
        active_tab_id,
        tabs,
    })
}

fn decode_history(bytes: &[u8]) -> Result<Vec<HistoryEntry>, CodecError> {
    let payload = decode_record(bytes, RecordKind::History)?;
    let mut decoder = Decoder::new(payload);
    let count = usize::from(decoder.u16()?);
    if count > MAX_HISTORY_ENTRIES {
        return Err(CodecError::TooLarge);
    }
    let mut entries = Vec::with_capacity(count);
    let mut ids = HashSet::new();
    for _ in 0..count {
        let id = HistoryEntryId(decoder.u64()?);
        if id.0 == 0 || !ids.insert(id) {
            return Err(CodecError::InvalidValue);
        }
        entries.push(HistoryEntry {
            id,
            url: validate_url(decoder.string()?)?,
            title: bounded_title(decoder.string()?),
            visited_at: decoder.u64()?,
        });
    }
    decoder.finish()?;
    Ok(entries)
}

fn decode_bookmarks(bytes: &[u8]) -> Result<Vec<Bookmark>, CodecError> {
    let payload = decode_record(bytes, RecordKind::Bookmarks)?;
    let mut decoder = Decoder::new(payload);
    let count = usize::from(decoder.u16()?);
    if count > MAX_BOOKMARKS {
        return Err(CodecError::TooLarge);
    }
    let mut entries = Vec::with_capacity(count);
    let mut ids = HashSet::new();
    let mut urls = HashSet::new();
    for _ in 0..count {
        let id = BookmarkId(decoder.u64()?);
        if id.0 == 0 || !ids.insert(id) {
            return Err(CodecError::InvalidValue);
        }
        let url = validate_url(decoder.string()?)?;
        if !urls.insert(url.clone()) {
            return Err(CodecError::InvalidValue);
        }
        let title = bounded_title(decoder.string()?);
        let created_at = decoder.u64()?;
        let updated_at = decoder.u64()?;
        if updated_at < created_at {
            return Err(CodecError::InvalidValue);
        }
        entries.push(Bookmark {
            id,
            url,
            title,
            created_at,
            updated_at,
        });
    }
    decoder.finish()?;
    Ok(entries)
}

fn validate_optional_url(url: Option<String>) -> Result<Option<String>, CodecError> {
    url.map(validate_url).transpose()
}

fn validate_url(url: String) -> Result<String, CodecError> {
    let normalized = normalize_address(&url).map_err(|_| CodecError::InvalidValue)?;
    if normalized != url {
        return Err(CodecError::InvalidValue);
    }
    Ok(url)
}

fn bounded_title(title: String) -> String {
    crate::history::bounded_text(&title, MAX_HISTORY_TITLE_BYTES)
}

fn fresh() -> RestoredBrowser {
    RestoredBrowser {
        browser: BrowserState::new(),
        navigation_requests: Vec::new(),
        permissions: PermissionBrokerState::new(),
        clipboard: BrowserClipboard::new(),
        downloads: DownloadsState::new(),
        uploads: UploadsState::new(),
        warnings: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::browser_state::NavigationEventResult;
    use crate::navigation::NavigationReason;
    use crate::persistence::{StorageRecord, StorageWrite};
    use crate::storage_bundle::{self, MAX_STORAGE_BUNDLE_BYTES};
    use std::collections::HashMap;

    #[derive(Default)]
    struct MemoryStorage {
        records: HashMap<StorageRecord, Vec<u8>>,
    }

    impl BrowserStorage for MemoryStorage {
        fn read_record(&mut self, record: StorageRecord) -> Result<Option<Vec<u8>>, StorageError> {
            Ok(self.records.get(&record).cloned())
        }

        fn write_batch(&mut self, writes: &[StorageWrite]) -> Result<(), StorageError> {
            let staged: Vec<_> = writes
                .iter()
                .map(|write| (write.record, write.bytes.clone()))
                .collect();
            for (record, bytes) in staged {
                self.records.insert(record, bytes);
            }
            Ok(())
        }
    }

    fn populated_browser() -> BrowserState {
        let mut browser = BrowserState::new();
        let tab = browser.active_tab_id().unwrap();
        let first = browser.navigate(tab, "first.example").unwrap();
        assert_eq!(
            browser
                .complete_navigation(tab, first.id, "First", 10)
                .unwrap(),
            NavigationEventResult::Applied
        );
        let bookmark = browser.add_bookmark(tab, 11).unwrap();
        let second = browser.navigate(tab, "second.example").unwrap();
        browser
            .complete_navigation(tab, second.id, "Second", 20)
            .unwrap();
        assert_eq!(bookmark.0, 1);
        browser
    }

    #[test]
    fn history_bookmarks_and_tabs_round_trip_without_restoring_transient_authority() {
        let browser = populated_browser();
        let mut storage = MemoryStorage::default();
        save(&browser, &mut storage).unwrap();
        let restored = restore(&mut storage);
        assert!(restored.warnings.is_empty());
        assert_eq!(restored.browser.tabs().len(), 1);
        assert_eq!(restored.browser.history().len(), 2);
        assert_eq!(restored.browser.bookmarks().entries().len(), 1);
        assert_eq!(restored.navigation_requests.len(), 1);
        assert_eq!(
            restored.navigation_requests[0].reason,
            NavigationReason::Restore
        );
        assert_eq!(
            restored.browser.active_tab().unwrap().navigation().phase(),
            &crate::navigation::NavigationPhase::Loading
        );
        assert!(restored.permissions.requests().is_empty());
        assert_eq!(
            restored.clipboard.status(),
            crate::clipboard::ClipboardStatus::Unavailable
        );
        assert!(restored.downloads.items().is_empty());
        assert!(restored.uploads.sessions().is_empty());
    }

    #[test]
    fn three_site_acceptance_state_fits_the_guest_snapshot_capacity() {
        let mut browser = populated_browser();
        let tab = browser.active_tab_id().unwrap();
        let third = browser.navigate(tab, "example.net").unwrap();
        browser
            .complete_navigation(tab, third.id, "Example Network", 30)
            .unwrap();

        let mut storage = MemoryStorage::default();
        save(&browser, &mut storage).unwrap();
        let writes = [
            StorageRecord::Session,
            StorageRecord::History,
            StorageRecord::Bookmarks,
        ]
        .into_iter()
        .map(|record| StorageWrite {
            record,
            bytes: storage.records[&record].clone(),
        })
        .collect::<Vec<_>>();

        let bundle = storage_bundle::encode(&writes).unwrap();
        assert!(bundle.len() <= MAX_STORAGE_BUNDLE_BYTES);
    }

    #[test]
    fn corrupt_or_partial_records_reset_safely_without_authority() {
        let browser = populated_browser();
        let session = encode_session(&browser).unwrap();
        let history = encode_history(browser.history()).unwrap();
        let bookmarks = encode_bookmarks(browser.bookmarks()).unwrap();
        let restored = restore_bytes(
            Some(&session[..session.len() - 1]),
            Some(&history),
            Some(&bookmarks),
        );
        assert!(matches!(
            restored.warnings.first(),
            Some(RestoreWarning::CorruptSession(_))
        ));
        assert_eq!(restored.browser.tabs().len(), 1);
        assert!(restored.navigation_requests.is_empty());
        let partial = restore_bytes(Some(&session), None, None);
        assert_eq!(partial.browser.history().len(), 0);
        assert!(partial.warnings.contains(&RestoreWarning::MissingHistory));
        assert!(partial.warnings.contains(&RestoreWarning::MissingBookmarks));
        assert_eq!(partial.browser.tabs().len(), 1);
    }

    #[test]
    fn unsafe_url_and_dangling_history_are_removed_on_recovery() {
        let mut payload = Encoder::new();
        payload.u64(1);
        payload.u16(1);
        payload.u64(1);
        payload
            .optional_string(Some("javascript:alert(1)"))
            .unwrap();
        payload.string("Untrusted").unwrap();
        payload.u16(0);
        payload.u16(u16::MAX);
        let unsafe_session = encode_record(RecordKind::Session, payload.finish()).unwrap();
        let empty_history = encode_history(&[]).unwrap();
        let empty_bookmarks = encode_bookmarks(&BookmarkStore::new()).unwrap();
        let restored = restore_bytes(
            Some(&unsafe_session),
            Some(&empty_history),
            Some(&empty_bookmarks),
        );
        assert!(matches!(
            restored.warnings.first(),
            Some(RestoreWarning::CorruptSession(CodecError::InvalidValue))
        ));

        let mut payload = Encoder::new();
        payload.u64(1);
        payload.u16(1);
        payload.u64(1);
        payload.optional_string(Some("about:blank")).unwrap();
        payload.string("New Tab").unwrap();
        payload.u16(1);
        payload.u64(99);
        payload.u16(0);
        let dangling_session = encode_record(RecordKind::Session, payload.finish()).unwrap();
        let restored = restore_bytes(
            Some(&dangling_session),
            Some(&empty_history),
            Some(&empty_bookmarks),
        );
        assert!(restored
            .warnings
            .contains(&RestoreWarning::HistoryReferencesCleared));
        assert!(!restored.browser.active_tab().unwrap().can_go_back());
    }

    #[test]
    fn history_and_bookmark_records_reject_wrong_kind_or_truncated_bytes() {
        let browser = populated_browser();
        let history = encode_history(browser.history()).unwrap();
        assert_eq!(decode_bookmarks(&history), Err(CodecError::WrongRecordKind));
        assert!(matches!(
            decode_history(&history[..history.len() - 1]),
            Err(CodecError::Truncated)
        ));
    }
}
