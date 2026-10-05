//! Typed navigation requests and per-tab page-load status.

use crate::tabs::TabId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NavigationId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NavigationReason {
    AddressBar,
    Back,
    Forward,
    Reload,
    Bookmark,
    Restore,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NavigationRequest {
    pub id: NavigationId,
    pub tab_id: TabId,
    pub url: String,
    pub reason: NavigationReason,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NavigationPhase {
    Idle,
    Loading,
    Completed,
    Failed { message: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NavigationState {
    current_url: Option<String>,
    requested_url: Option<String>,
    title: String,
    phase: NavigationPhase,
}

impl NavigationState {
    pub(crate) fn new() -> Self {
        Self {
            current_url: Some("about:blank".to_owned()),
            requested_url: None,
            title: "New Tab".to_owned(),
            phase: NavigationPhase::Idle,
        }
    }

    pub fn current_url(&self) -> Option<&str> {
        self.current_url.as_deref()
    }

    pub fn requested_url(&self) -> Option<&str> {
        self.requested_url.as_deref()
    }

    pub fn display_url(&self) -> Option<&str> {
        self.requested_url
            .as_deref()
            .or(self.current_url.as_deref())
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn phase(&self) -> &NavigationPhase {
        &self.phase
    }

    pub fn is_loading(&self) -> bool {
        matches!(self.phase, NavigationPhase::Loading)
    }

    pub(crate) fn begin(&mut self, url: String) {
        self.requested_url = Some(url);
        self.phase = NavigationPhase::Loading;
    }

    pub(crate) fn redirect(&mut self, url: String) {
        self.requested_url = Some(url);
    }

    pub(crate) fn complete(&mut self, title: String) -> String {
        let url = self
            .requested_url
            .take()
            .unwrap_or_else(|| "about:blank".to_owned());
        self.current_url = Some(url.clone());
        self.title = title;
        self.phase = NavigationPhase::Completed;
        url
    }

    pub(crate) fn fail(&mut self, message: String) {
        self.phase = NavigationPhase::Failed { message };
    }

    pub(crate) fn stop(&mut self) {
        self.requested_url = None;
        self.phase = if self.current_url.is_some() {
            NavigationPhase::Completed
        } else {
            NavigationPhase::Idle
        };
    }

    pub(crate) fn restore(current_url: Option<String>, title: String) -> Self {
        Self {
            current_url,
            requested_url: None,
            title,
            phase: NavigationPhase::Idle,
        }
    }
}

impl Default for NavigationState {
    fn default() -> Self {
        Self::new()
    }
}
