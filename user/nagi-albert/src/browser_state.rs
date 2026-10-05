//! Deterministic browser orchestration independent of Servo and remote transport.

use crate::address_bar::{normalize_address, AddressBar, AddressError};
use crate::bookmarks::{BookmarkError, BookmarkId, BookmarkStore};
use crate::history::{
    bounded_text, HistoryEntry, HistoryEntryId, MAX_HISTORY_ENTRIES, MAX_HISTORY_TITLE_BYTES,
    MAX_TAB_HISTORY_ENTRIES,
};
use crate::navigation::{NavigationId, NavigationReason, NavigationRequest};
use crate::tabs::{PendingKind, PendingNavigation, Tab, TabId, MAX_TABS};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BrowserStateError {
    NoActiveTab,
    TabNotFound,
    TabLimitReached,
    Address(AddressError),
    Bookmark(BookmarkError),
    HistoryUnavailable,
    IdsExhausted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NavigationEventResult {
    Applied,
    IgnoredStale,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StopRequest {
    pub tab_id: TabId,
    pub navigation_id: NavigationId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BrowserState {
    pub(crate) tabs: Vec<Tab>,
    pub(crate) active_tab_id: Option<TabId>,
    pub(crate) next_tab_id: u64,
    pub(crate) next_navigation_id: u64,
    pub(crate) next_history_id: u64,
    pub(crate) history_entries: Vec<HistoryEntry>,
    pub(crate) bookmarks: BookmarkStore,
    address_bar: AddressBar,
}

impl BrowserState {
    pub fn new() -> Self {
        let tab = Tab::new(TabId(1));
        let mut address_bar = AddressBar::new();
        address_bar
            .set_text("about:blank")
            .expect("static address fits");
        Self {
            tabs: vec![tab],
            active_tab_id: Some(TabId(1)),
            next_tab_id: 2,
            next_navigation_id: 1,
            next_history_id: 1,
            history_entries: Vec::new(),
            bookmarks: BookmarkStore::new(),
            address_bar,
        }
    }

    pub fn tabs(&self) -> &[Tab] {
        &self.tabs
    }

    pub fn active_tab_id(&self) -> Option<TabId> {
        self.active_tab_id
    }

    pub fn active_tab(&self) -> Option<&Tab> {
        self.active_tab_id
            .and_then(|id| self.tabs.iter().find(|tab| tab.id == id))
    }

    pub fn tab(&self, id: TabId) -> Option<&Tab> {
        self.tabs.iter().find(|tab| tab.id == id)
    }

    pub fn address_bar(&self) -> &AddressBar {
        &self.address_bar
    }

    pub fn address_bar_mut(&mut self) -> &mut AddressBar {
        &mut self.address_bar
    }

    pub fn history(&self) -> &[HistoryEntry] {
        &self.history_entries
    }

    pub fn bookmarks(&self) -> &BookmarkStore {
        &self.bookmarks
    }

    pub fn new_tab(&mut self) -> Result<TabId, BrowserStateError> {
        if self.tabs.len() >= MAX_TABS {
            return Err(BrowserStateError::TabLimitReached);
        }
        let id = TabId(self.next_tab_id);
        self.next_tab_id = self
            .next_tab_id
            .checked_add(1)
            .ok_or(BrowserStateError::IdsExhausted)?;
        self.tabs.push(Tab::new(id));
        self.active_tab_id = Some(id);
        self.address_bar
            .set_text("about:blank")
            .expect("static address fits");
        Ok(id)
    }

    /// Close a tab; closing the last tab opens a fresh blank tab in its place.
    pub fn close_tab(&mut self, id: TabId) -> Result<TabId, BrowserStateError> {
        let index = self
            .tabs
            .iter()
            .position(|tab| tab.id == id)
            .ok_or(BrowserStateError::TabNotFound)?;
        self.tabs.remove(index);
        if self.tabs.is_empty() {
            return self.new_tab();
        }
        if self.active_tab_id == Some(id) {
            let next_index = index.min(self.tabs.len() - 1);
            self.active_tab_id = Some(self.tabs[next_index].id);
            self.refresh_address_bar();
        }
        Ok(self.active_tab_id.unwrap_or(self.tabs[0].id))
    }

    pub fn select_tab(&mut self, id: TabId) -> Result<(), BrowserStateError> {
        if !self.tabs.iter().any(|tab| tab.id == id) {
            return Err(BrowserStateError::TabNotFound);
        }
        self.active_tab_id = Some(id);
        self.refresh_address_bar();
        Ok(())
    }

    pub fn can_go_back(&self, tab_id: TabId) -> bool {
        self.tab(tab_id).is_some_and(Tab::can_go_back)
    }

    pub fn can_go_forward(&self, tab_id: TabId) -> bool {
        self.tab(tab_id).is_some_and(Tab::can_go_forward)
    }

    pub fn submit_address_bar(&mut self) -> Result<NavigationRequest, BrowserStateError> {
        let url = self
            .address_bar
            .submit()
            .map_err(BrowserStateError::Address)?;
        let tab_id = self.active_tab_id.ok_or(BrowserStateError::NoActiveTab)?;
        let request = self.begin_navigation(
            tab_id,
            url,
            NavigationReason::AddressBar,
            PendingKind::NewEntry,
        )?;
        // A submitted address hands keyboard focus to the page, as in other
        // browsers; a rejected address keeps the bar focused for correction.
        self.address_bar.blur();
        Ok(request)
    }

    pub fn navigate(
        &mut self,
        tab_id: TabId,
        input: &str,
    ) -> Result<NavigationRequest, BrowserStateError> {
        let url = normalize_address(input).map_err(BrowserStateError::Address)?;
        self.begin_navigation(
            tab_id,
            url,
            NavigationReason::AddressBar,
            PendingKind::NewEntry,
        )
    }

    pub fn go_back(&mut self, tab_id: TabId) -> Result<NavigationRequest, BrowserStateError> {
        let tab = self.tab(tab_id).ok_or(BrowserStateError::TabNotFound)?;
        let cursor = tab
            .history_cursor
            .filter(|cursor| *cursor > 0)
            .ok_or(BrowserStateError::HistoryUnavailable)?;
        let target = cursor - 1;
        let entry_id = tab.history_ids[target];
        let url = self
            .history_entries
            .iter()
            .find(|entry| entry.id == entry_id)
            .map(|entry| entry.url.clone())
            .ok_or(BrowserStateError::HistoryUnavailable)?;
        self.begin_navigation(
            tab_id,
            url,
            NavigationReason::Back,
            PendingKind::Traverse(target),
        )
    }

    pub fn go_forward(&mut self, tab_id: TabId) -> Result<NavigationRequest, BrowserStateError> {
        let tab = self.tab(tab_id).ok_or(BrowserStateError::TabNotFound)?;
        let cursor = tab
            .history_cursor
            .filter(|cursor| cursor + 1 < tab.history_ids.len())
            .ok_or(BrowserStateError::HistoryUnavailable)?;
        let target = cursor + 1;
        let entry_id = tab.history_ids[target];
        let url = self
            .history_entries
            .iter()
            .find(|entry| entry.id == entry_id)
            .map(|entry| entry.url.clone())
            .ok_or(BrowserStateError::HistoryUnavailable)?;
        self.begin_navigation(
            tab_id,
            url,
            NavigationReason::Forward,
            PendingKind::Traverse(target),
        )
    }

    pub fn reload(&mut self, tab_id: TabId) -> Result<NavigationRequest, BrowserStateError> {
        let tab = self.tab(tab_id).ok_or(BrowserStateError::TabNotFound)?;
        let url = tab
            .navigation
            .display_url()
            .ok_or(BrowserStateError::HistoryUnavailable)?
            .to_owned();
        self.begin_navigation(tab_id, url, NavigationReason::Reload, PendingKind::Reload)
    }

    pub fn stop(&mut self, tab_id: TabId) -> Result<Option<StopRequest>, BrowserStateError> {
        let tab = self
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .ok_or(BrowserStateError::TabNotFound)?;
        let Some(pending) = tab.pending.take() else {
            return Ok(None);
        };
        let request = StopRequest {
            tab_id,
            navigation_id: pending.request.id,
        };
        tab.navigation.stop();
        if self.active_tab_id == Some(tab_id) {
            self.refresh_address_bar();
        }
        Ok(Some(request))
    }

    pub fn redirect(
        &mut self,
        tab_id: TabId,
        navigation_id: NavigationId,
        url: &str,
    ) -> Result<NavigationEventResult, BrowserStateError> {
        let normalized = normalize_address(url).map_err(BrowserStateError::Address)?;
        let tab = self
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .ok_or(BrowserStateError::TabNotFound)?;
        let Some(pending) = tab.pending.as_mut() else {
            return Ok(NavigationEventResult::IgnoredStale);
        };
        if pending.request.id != navigation_id {
            return Ok(NavigationEventResult::IgnoredStale);
        }
        pending.request.url = normalized.clone();
        tab.navigation.redirect(normalized);
        if self.active_tab_id == Some(tab_id) {
            self.refresh_address_bar();
        }
        Ok(NavigationEventResult::Applied)
    }

    pub fn complete_navigation(
        &mut self,
        tab_id: TabId,
        navigation_id: NavigationId,
        title: &str,
        visited_at: u64,
    ) -> Result<NavigationEventResult, BrowserStateError> {
        let tab = self
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .ok_or(BrowserStateError::TabNotFound)?;
        let Some(pending) = tab.pending.take() else {
            return Ok(NavigationEventResult::IgnoredStale);
        };
        if pending.request.id != navigation_id {
            tab.pending = Some(pending);
            return Ok(NavigationEventResult::IgnoredStale);
        }
        let url = tab
            .navigation
            .complete(bounded_text(title, MAX_HISTORY_TITLE_BYTES));
        match pending.kind {
            PendingKind::NewEntry => {
                if let Some(cursor) = tab.history_cursor {
                    tab.history_ids.truncate(cursor + 1);
                } else {
                    tab.history_ids.clear();
                }
                let id = HistoryEntryId(self.next_history_id);
                self.next_history_id = self
                    .next_history_id
                    .checked_add(1)
                    .ok_or(BrowserStateError::IdsExhausted)?;
                self.history_entries.push(HistoryEntry {
                    id,
                    url,
                    title: bounded_text(title, MAX_HISTORY_TITLE_BYTES),
                    visited_at,
                });
                tab.history_ids.push(id);
                if tab.history_ids.len() > MAX_TAB_HISTORY_ENTRIES {
                    tab.history_ids.remove(0);
                }
                tab.history_cursor = Some(tab.history_ids.len() - 1);
            }
            PendingKind::Traverse(target) => {
                if target < tab.history_ids.len() {
                    tab.history_cursor = Some(target);
                    if let Some(entry) = self
                        .history_entries
                        .iter_mut()
                        .find(|entry| entry.id == tab.history_ids[target])
                    {
                        entry.title = bounded_text(title, MAX_HISTORY_TITLE_BYTES);
                    }
                } else {
                    tab.history_cursor = None;
                }
            }
            PendingKind::Reload => {
                if let Some(id) = tab
                    .history_cursor
                    .and_then(|cursor| tab.history_ids.get(cursor).copied())
                {
                    if let Some(entry) =
                        self.history_entries.iter_mut().find(|entry| entry.id == id)
                    {
                        entry.title = bounded_text(title, MAX_HISTORY_TITLE_BYTES);
                    }
                }
            }
        }
        tab.pending = None;
        while self.history_entries.len() > MAX_HISTORY_ENTRIES {
            let removed = self.history_entries.remove(0).id;
            self.remove_history_reference(removed);
        }
        if self.active_tab_id == Some(tab_id) {
            self.refresh_address_bar();
        }
        Ok(NavigationEventResult::Applied)
    }

    pub fn fail_navigation(
        &mut self,
        tab_id: TabId,
        navigation_id: NavigationId,
        message: &str,
    ) -> Result<NavigationEventResult, BrowserStateError> {
        let tab = self
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .ok_or(BrowserStateError::TabNotFound)?;
        let Some(pending) = tab.pending.as_ref() else {
            return Ok(NavigationEventResult::IgnoredStale);
        };
        if pending.request.id != navigation_id {
            return Ok(NavigationEventResult::IgnoredStale);
        }
        tab.pending = None;
        tab.navigation
            .fail(bounded_text(message, MAX_HISTORY_TITLE_BYTES));
        if self.active_tab_id == Some(tab_id) {
            self.refresh_address_bar();
        }
        Ok(NavigationEventResult::Applied)
    }

    pub fn add_bookmark(
        &mut self,
        tab_id: TabId,
        now: u64,
    ) -> Result<BookmarkId, BrowserStateError> {
        let (url, title) = {
            let tab = self.tab(tab_id).ok_or(BrowserStateError::TabNotFound)?;
            let url = tab
                .navigation
                .current_url()
                .ok_or(BrowserStateError::HistoryUnavailable)?
                .to_owned();
            (url, tab.navigation.title().to_owned())
        };
        self.bookmarks
            .add_or_update(&url, &title, now)
            .map_err(BrowserStateError::Bookmark)
    }

    pub fn open_bookmark(
        &mut self,
        tab_id: TabId,
        bookmark_id: BookmarkId,
    ) -> Result<NavigationRequest, BrowserStateError> {
        let url = self
            .bookmarks
            .open_url(bookmark_id)
            .map_err(BrowserStateError::Bookmark)?
            .to_owned();
        self.begin_navigation(
            tab_id,
            url,
            NavigationReason::Bookmark,
            PendingKind::NewEntry,
        )
    }

    pub(crate) fn set_restored_parts(
        tabs: Vec<Tab>,
        active_tab_id: TabId,
        history_entries: Vec<HistoryEntry>,
        bookmarks: BookmarkStore,
    ) -> Result<Self, BrowserStateError> {
        if tabs.is_empty() || tabs.len() > MAX_TABS || history_entries.len() > MAX_HISTORY_ENTRIES {
            return Err(BrowserStateError::TabLimitReached);
        }
        if !tabs.iter().any(|tab| tab.id == active_tab_id) {
            return Err(BrowserStateError::TabNotFound);
        }
        let mut state = Self {
            next_tab_id: tabs
                .iter()
                .map(|tab| tab.id.0.saturating_add(1))
                .max()
                .unwrap_or(1),
            next_navigation_id: 1,
            next_history_id: history_entries
                .iter()
                .map(|entry| entry.id.0.saturating_add(1))
                .max()
                .unwrap_or(1),
            tabs,
            active_tab_id: Some(active_tab_id),
            history_entries,
            bookmarks,
            address_bar: AddressBar::new(),
        };
        state.validate_history_references()?;
        state.refresh_address_bar();
        Ok(state)
    }

    pub(crate) fn restored_navigation_requests(&mut self) -> Vec<NavigationRequest> {
        let mut requests = Vec::new();
        for tab in &mut self.tabs {
            let Some(url) = tab
                .navigation
                .current_url()
                .filter(|url| *url != "about:blank")
                .map(str::to_owned)
            else {
                continue;
            };
            let id = NavigationId(self.next_navigation_id);
            let Some(next) = self.next_navigation_id.checked_add(1) else {
                break;
            };
            self.next_navigation_id = next;
            tab.navigation.begin(url.clone());
            let request = NavigationRequest {
                id,
                tab_id: tab.id,
                url,
                reason: NavigationReason::Restore,
            };
            tab.pending = Some(PendingNavigation {
                request: request.clone(),
                kind: PendingKind::Reload,
            });
            requests.push(request);
        }
        requests
    }

    fn begin_navigation(
        &mut self,
        tab_id: TabId,
        url: String,
        reason: NavigationReason,
        kind: PendingKind,
    ) -> Result<NavigationRequest, BrowserStateError> {
        let id = NavigationId(self.next_navigation_id);
        self.next_navigation_id = self
            .next_navigation_id
            .checked_add(1)
            .ok_or(BrowserStateError::IdsExhausted)?;
        let request = NavigationRequest {
            id,
            tab_id,
            url: url.clone(),
            reason,
        };
        let tab = self
            .tabs
            .iter_mut()
            .find(|tab| tab.id == tab_id)
            .ok_or(BrowserStateError::TabNotFound)?;
        tab.navigation.begin(url);
        tab.pending = Some(PendingNavigation {
            request: request.clone(),
            kind,
        });
        if self.active_tab_id == Some(tab_id) {
            self.address_bar
                .set_text(request.url.clone())
                .expect("normalized URL fits the address entry");
        }
        Ok(request)
    }

    fn refresh_address_bar(&mut self) {
        if let Some(tab) = self.active_tab() {
            let value = tab.navigation.display_url().unwrap_or_default();
            let _ = self.address_bar.set_text(value.to_owned());
        } else {
            let _ = self.address_bar.set_text("");
        }
    }

    fn remove_history_reference(&mut self, id: HistoryEntryId) {
        for tab in &mut self.tabs {
            if let Some(index) = tab.history_ids.iter().position(|entry_id| *entry_id == id) {
                tab.history_ids.remove(index);
                tab.history_cursor = match tab.history_cursor {
                    Some(cursor) if index < cursor => Some(cursor - 1),
                    Some(cursor) if index == cursor && !tab.history_ids.is_empty() => {
                        Some(cursor.min(tab.history_ids.len() - 1))
                    }
                    Some(_) => None,
                    None => None,
                };
            }
        }
    }

    fn validate_history_references(&mut self) -> Result<(), BrowserStateError> {
        let available = self
            .history_entries
            .iter()
            .map(|entry| entry.id)
            .collect::<std::collections::HashSet<_>>();
        for tab in &mut self.tabs {
            if tab.history_ids.iter().any(|id| !available.contains(id)) {
                return Err(BrowserStateError::HistoryUnavailable);
            }
            if tab
                .history_cursor
                .is_some_and(|cursor| cursor >= tab.history_ids.len())
            {
                return Err(BrowserStateError::HistoryUnavailable);
            }
        }
        Ok(())
    }
}

impl Default for BrowserState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::navigation::NavigationPhase;

    fn complete(state: &mut BrowserState, request: NavigationRequest, title: &str, now: u64) {
        assert_eq!(
            state
                .complete_navigation(request.tab_id, request.id, title, now)
                .unwrap(),
            NavigationEventResult::Applied
        );
    }

    #[test]
    fn tabs_are_created_selected_closed_and_kept_isolated() {
        let mut browser = BrowserState::new();
        let first = browser.active_tab_id().unwrap();
        let request = browser.navigate(first, "one.example").unwrap();
        complete(&mut browser, request, "One", 1);
        let second = browser.new_tab().unwrap();
        assert_eq!(browser.active_tab_id(), Some(second));
        assert_eq!(
            browser.tab(second).unwrap().navigation().current_url(),
            Some("about:blank")
        );
        assert!(!browser.tab(second).unwrap().can_go_back());
        browser.select_tab(first).unwrap();
        assert_eq!(browser.address_bar().text(), "https://one.example/");
        let active = browser.close_tab(first).unwrap();
        assert_eq!(active, second);
        assert_eq!(browser.tabs().len(), 1);
    }

    #[test]
    fn navigation_redirect_back_forward_reload_and_failure_are_typed() {
        let mut browser = BrowserState::new();
        let tab = browser.active_tab_id().unwrap();
        let first = browser.navigate(tab, "one.example").unwrap();
        assert!(browser.tab(tab).unwrap().is_loading());
        browser
            .redirect(tab, first.id, "https://www.one.example/")
            .unwrap();
        complete(&mut browser, first, "One", 10);
        let second = browser.navigate(tab, "two.example").unwrap();
        complete(&mut browser, second, "Two", 20);
        assert!(browser.can_go_back(tab));
        let back = browser.go_back(tab).unwrap();
        assert_eq!(back.reason, NavigationReason::Back);
        complete(&mut browser, back, "One", 30);
        assert!(browser.can_go_forward(tab));
        let forward = browser.go_forward(tab).unwrap();
        complete(&mut browser, forward, "Two", 40);
        let reload = browser.reload(tab).unwrap();
        assert_eq!(reload.reason, NavigationReason::Reload);
        complete(&mut browser, reload, "Two refreshed", 50);
        let failed = browser.navigate(tab, "broken.example").unwrap();
        browser.fail_navigation(tab, failed.id, "offline").unwrap();
        assert!(matches!(
            browser.tab(tab).unwrap().navigation().phase(),
            NavigationPhase::Failed { .. }
        ));
        assert_eq!(browser.history().len(), 2);
    }

    #[test]
    fn reloading_a_failed_navigation_retries_the_displayed_request_url() {
        let mut browser = BrowserState::new();
        let tab = browser.active_tab_id().unwrap();
        let working = browser.navigate(tab, "working.example").unwrap();
        complete(&mut browser, working, "Working", 1);

        let failed = browser.navigate(tab, "broken.example").unwrap();
        browser.fail_navigation(tab, failed.id, "offline").unwrap();
        assert_eq!(
            browser.address_bar().committed_text(),
            "https://broken.example/"
        );

        let retry = browser.reload(tab).unwrap();
        assert_eq!(retry.url, "https://broken.example/");
        assert_eq!(retry.reason, NavigationReason::Reload);
    }

    #[test]
    fn completions_from_old_requests_are_ignored() {
        let mut browser = BrowserState::new();
        let tab = browser.active_tab_id().unwrap();
        let stale = browser.navigate(tab, "old.example").unwrap();
        let active = browser.navigate(tab, "new.example").unwrap();
        assert_eq!(
            browser
                .complete_navigation(tab, stale.id, "Old", 1)
                .unwrap(),
            NavigationEventResult::IgnoredStale
        );
        complete(&mut browser, active, "New", 2);
        assert_eq!(browser.history().len(), 1);
        assert_eq!(browser.history()[0].url, "https://new.example/");
    }

    #[test]
    fn address_bar_submission_moves_focus_to_the_page() {
        let mut browser = BrowserState::new();
        browser.address_bar_mut().focus();
        browser.address_bar_mut().set_text("javascript:x").unwrap();
        assert!(browser.submit_address_bar().is_err());
        assert!(browser.address_bar().is_focused());
        browser.address_bar_mut().set_text("example.net").unwrap();
        browser.submit_address_bar().unwrap();
        assert!(!browser.address_bar().is_focused());
    }

    #[test]
    fn address_bar_submission_returns_a_navigation_request() {
        let mut browser = BrowserState::new();
        browser
            .address_bar_mut()
            .set_text("example.net/path")
            .unwrap();
        let request = browser.submit_address_bar().unwrap();
        assert_eq!(request.url, "https://example.net/path");
        assert_eq!(request.reason, NavigationReason::AddressBar);
        assert_eq!(
            browser
                .tab(request.tab_id)
                .unwrap()
                .navigation()
                .requested_url(),
            Some(request.url.as_str())
        );
    }

    #[test]
    fn closing_the_last_tab_opens_a_blank_tab() {
        let mut browser = BrowserState::new();
        let old = browser.active_tab_id().unwrap();
        let new = browser.close_tab(old).unwrap();
        assert_ne!(new, old);
        assert_eq!(browser.tabs().len(), 1);
        assert_eq!(
            browser.tab(new).unwrap().navigation().current_url(),
            Some("about:blank")
        );
    }

    #[test]
    fn bookmark_opening_hands_back_a_typed_navigation_request() {
        let mut browser = BrowserState::new();
        let tab = browser.active_tab_id().unwrap();
        let request = browser.navigate(tab, "bookmark.example").unwrap();
        complete(&mut browser, request, "Bookmark", 1);
        let bookmark = browser.add_bookmark(tab, 2).unwrap();
        let open = browser.open_bookmark(tab, bookmark).unwrap();
        assert_eq!(open.reason, NavigationReason::Bookmark);
        assert_eq!(open.url, "https://bookmark.example/");
    }
}
