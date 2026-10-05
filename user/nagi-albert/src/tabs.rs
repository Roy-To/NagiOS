//! Per-tab browser state and history cursors.

use crate::history::HistoryEntryId;
use crate::navigation::{NavigationRequest, NavigationState};

pub const MAX_TABS: usize = 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct TabId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PendingKind {
    NewEntry,
    Traverse(usize),
    Reload,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PendingNavigation {
    pub request: NavigationRequest,
    pub kind: PendingKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Tab {
    pub(crate) id: TabId,
    pub(crate) navigation: NavigationState,
    pub(crate) history_ids: Vec<HistoryEntryId>,
    pub(crate) history_cursor: Option<usize>,
    pub(crate) pending: Option<PendingNavigation>,
}

impl Tab {
    pub(crate) fn new(id: TabId) -> Self {
        Self {
            id,
            navigation: NavigationState::new(),
            history_ids: Vec::new(),
            history_cursor: None,
            pending: None,
        }
    }

    pub fn id(&self) -> TabId {
        self.id
    }

    pub fn navigation(&self) -> &NavigationState {
        &self.navigation
    }

    pub fn can_go_back(&self) -> bool {
        self.history_cursor.is_some_and(|cursor| cursor > 0)
    }

    pub fn can_go_forward(&self) -> bool {
        self.history_cursor
            .is_some_and(|cursor| cursor + 1 < self.history_ids.len())
    }

    pub fn is_loading(&self) -> bool {
        self.navigation.is_loading()
    }

    pub fn pending_request(&self) -> Option<&NavigationRequest> {
        self.pending.as_ref().map(|pending| &pending.request)
    }

    pub(crate) fn restored(
        id: TabId,
        current_url: Option<String>,
        title: String,
        history_ids: Vec<HistoryEntryId>,
        history_cursor: Option<usize>,
    ) -> Self {
        let history_cursor = history_cursor.filter(|cursor| *cursor < history_ids.len());
        let current_url = current_url.filter(|url| !url.is_empty());
        Self {
            id,
            navigation: NavigationState::restore(current_url, title),
            history_ids,
            history_cursor,
            pending: None,
        }
    }
}
