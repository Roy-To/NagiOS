//! Browser chrome view model and action routing; no network/runtime internals.

use crate::bookmarks::BookmarkId;
use crate::browser_state::{BrowserState, BrowserStateError, StopRequest};
use crate::navigation::{NavigationPhase, NavigationRequest};
use crate::tabs::TabId;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TabView {
    pub id: TabId,
    pub title: String,
    pub url: String,
    pub active: bool,
    pub loading: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BrowserChromeView {
    pub tabs: Vec<TabView>,
    pub active_tab: Option<TabId>,
    pub address_text: String,
    pub address_invalid: bool,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub loading: bool,
    pub page_title: String,
    pub page_status: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BrowserChromeAction {
    NewTab,
    CloseTab(TabId),
    SelectTab(TabId),
    SubmitAddress,
    Back,
    Forward,
    Reload,
    Stop,
    AddBookmark,
    OpenBookmark(BookmarkId),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BrowserChromeOutcome {
    Changed,
    Navigate(NavigationRequest),
    Stop(StopRequest),
}

pub fn view(state: &BrowserState) -> BrowserChromeView {
    let tabs = state
        .tabs()
        .iter()
        .map(|tab| TabView {
            id: tab.id(),
            title: tab.navigation().title().to_owned(),
            url: tab
                .navigation()
                .display_url()
                .unwrap_or_default()
                .to_owned(),
            active: state.active_tab_id() == Some(tab.id()),
            loading: tab.is_loading(),
        })
        .collect();
    let active = state.active_tab();
    let page_status = match active.map(|tab| tab.navigation().phase()) {
        Some(NavigationPhase::Idle) => "browser.status.idle",
        Some(NavigationPhase::Loading) => "browser.status.loading",
        Some(NavigationPhase::Completed) => "browser.status.complete",
        Some(NavigationPhase::Failed { .. }) => "browser.status.failed",
        None => "browser.status.no-tab",
    };
    BrowserChromeView {
        tabs,
        active_tab: state.active_tab_id(),
        address_text: state.address_bar().text(),
        address_invalid: state.address_bar().is_invalid(),
        can_go_back: active.is_some_and(|tab| tab.can_go_back()),
        can_go_forward: active.is_some_and(|tab| tab.can_go_forward()),
        loading: active.is_some_and(|tab| tab.is_loading()),
        page_title: active
            .map(|tab| tab.navigation().title().to_owned())
            .unwrap_or_default(),
        page_status: page_status.to_owned(),
    }
}

pub fn dispatch(
    state: &mut BrowserState,
    action: BrowserChromeAction,
    now: u64,
) -> Result<BrowserChromeOutcome, BrowserStateError> {
    match action {
        BrowserChromeAction::NewTab => {
            state.new_tab()?;
            Ok(BrowserChromeOutcome::Changed)
        }
        BrowserChromeAction::CloseTab(tab_id) => {
            state.close_tab(tab_id)?;
            Ok(BrowserChromeOutcome::Changed)
        }
        BrowserChromeAction::SelectTab(tab_id) => {
            state.select_tab(tab_id)?;
            Ok(BrowserChromeOutcome::Changed)
        }
        BrowserChromeAction::SubmitAddress => state
            .submit_address_bar()
            .map(BrowserChromeOutcome::Navigate),
        BrowserChromeAction::Back => {
            let tab = state
                .active_tab_id()
                .ok_or(BrowserStateError::NoActiveTab)?;
            state.go_back(tab).map(BrowserChromeOutcome::Navigate)
        }
        BrowserChromeAction::Forward => {
            let tab = state
                .active_tab_id()
                .ok_or(BrowserStateError::NoActiveTab)?;
            state.go_forward(tab).map(BrowserChromeOutcome::Navigate)
        }
        BrowserChromeAction::Reload => {
            let tab = state
                .active_tab_id()
                .ok_or(BrowserStateError::NoActiveTab)?;
            state.reload(tab).map(BrowserChromeOutcome::Navigate)
        }
        BrowserChromeAction::Stop => {
            let tab = state
                .active_tab_id()
                .ok_or(BrowserStateError::NoActiveTab)?;
            state
                .stop(tab)?
                .map(BrowserChromeOutcome::Stop)
                .ok_or(BrowserStateError::HistoryUnavailable)
        }
        BrowserChromeAction::AddBookmark => {
            let tab = state
                .active_tab_id()
                .ok_or(BrowserStateError::NoActiveTab)?;
            state.add_bookmark(tab, now)?;
            Ok(BrowserChromeOutcome::Changed)
        }
        BrowserChromeAction::OpenBookmark(id) => {
            let tab = state
                .active_tab_id()
                .ok_or(BrowserStateError::NoActiveTab)?;
            state
                .open_bookmark(tab, id)
                .map(BrowserChromeOutcome::Navigate)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chrome_projects_per_tab_controls_and_accessible_message_keys() {
        let mut state = BrowserState::new();
        let first = state.active_tab_id().unwrap();
        let request = state.navigate(first, "one.example").unwrap();
        state
            .complete_navigation(first, request.id, "One", 1)
            .unwrap();
        dispatch(&mut state, BrowserChromeAction::NewTab, 2).unwrap();
        let chrome = view(&state);
        assert_eq!(chrome.tabs.len(), 2);
        assert_eq!(chrome.address_text, "about:blank");
        assert!(!chrome.can_go_back);
        assert_eq!(chrome.page_status, "browser.status.idle");
    }

    #[test]
    fn ui_action_returns_navigation_request_for_runtime_adapter() {
        let mut state = BrowserState::new();
        state.address_bar_mut().set_text("web.example").unwrap();
        assert!(matches!(
            dispatch(&mut state, BrowserChromeAction::SubmitAddress, 1),
            Ok(BrowserChromeOutcome::Navigate(NavigationRequest { .. }))
        ));
    }
}
