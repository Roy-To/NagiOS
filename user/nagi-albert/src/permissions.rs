//! Browser permission prompts. Requests begin pending and are never auto-granted.

use url::Url;

use crate::tabs::TabId;

pub const MAX_PERMISSION_REQUESTS: usize = 256;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct PermissionRequestId(pub u64);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionKind {
    ClipboardRead,
    ClipboardWrite,
    FileUpload,
    Download,
    Microphone,
    Camera,
    Location,
    Notifications,
    Push,
    Midi,
    Speaker,
    DeviceInfo,
    BackgroundSync,
    Bluetooth,
    PersistentStorage,
    ScreenWakeLock,
    Gamepad,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PermissionStatus {
    Requested,
    Allowed,
    Denied,
    Dismissed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UserDecision {
    Allow,
    Deny,
    Dismiss,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PermissionRequest {
    pub id: PermissionRequestId,
    pub tab_id: TabId,
    pub origin: String,
    pub kind: PermissionKind,
    pub status: PermissionStatus,
    pub requested_at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PermissionError {
    InvalidOrigin,
    CapacityReached,
    IdsExhausted,
    RequestNotFound,
    AlreadyResolved,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PermissionBrokerState {
    requests: Vec<PermissionRequest>,
    next_id: u64,
}

impl PermissionBrokerState {
    pub fn new() -> Self {
        Self {
            requests: Vec::new(),
            next_id: 1,
        }
    }

    pub fn requests(&self) -> &[PermissionRequest] {
        &self.requests
    }

    pub fn request(
        &mut self,
        tab_id: TabId,
        page_url: &str,
        kind: PermissionKind,
        requested_at: u64,
    ) -> Result<PermissionRequestId, PermissionError> {
        let origin = parse_origin(page_url)?;
        if self.requests.len() >= MAX_PERMISSION_REQUESTS {
            return Err(PermissionError::CapacityReached);
        }
        let id = PermissionRequestId(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or(PermissionError::IdsExhausted)?;
        self.requests.push(PermissionRequest {
            id,
            tab_id,
            origin,
            kind,
            status: PermissionStatus::Requested,
            requested_at,
        });
        Ok(id)
    }

    pub fn respond(
        &mut self,
        id: PermissionRequestId,
        decision: UserDecision,
    ) -> Result<&PermissionRequest, PermissionError> {
        let request = self
            .requests
            .iter_mut()
            .find(|request| request.id == id)
            .ok_or(PermissionError::RequestNotFound)?;
        if request.status != PermissionStatus::Requested {
            return Err(PermissionError::AlreadyResolved);
        }
        request.status = match decision {
            UserDecision::Allow => PermissionStatus::Allowed,
            UserDecision::Deny => PermissionStatus::Denied,
            UserDecision::Dismiss => PermissionStatus::Dismissed,
        };
        Ok(request)
    }

    /// Record and deny a site request when no trusted prompt service is
    /// connected. This fallback never grants page authority.
    pub fn deny_without_prompt(
        &mut self,
        tab_id: TabId,
        page_url: &str,
        kind: PermissionKind,
        requested_at: u64,
    ) -> Result<PermissionRequestId, PermissionError> {
        let id = self.request(tab_id, page_url, kind, requested_at)?;
        self.respond(id, UserDecision::Deny)?;
        Ok(id)
    }

    pub fn pending(&self) -> impl Iterator<Item = &PermissionRequest> {
        self.requests
            .iter()
            .filter(|request| request.status == PermissionStatus::Requested)
    }
}

impl Default for PermissionBrokerState {
    fn default() -> Self {
        Self::new()
    }
}

pub fn parse_origin(page_url: &str) -> Result<String, PermissionError> {
    let url = Url::parse(page_url).map_err(|_| PermissionError::InvalidOrigin)?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(PermissionError::InvalidOrigin);
    }
    Ok(url.origin().ascii_serialization())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_requests_start_pending_and_only_explicit_decision_resolves_them() {
        let mut broker = PermissionBrokerState::new();
        let request = broker
            .request(
                TabId(4),
                "https://news.example/article",
                PermissionKind::Microphone,
                12,
            )
            .unwrap();
        assert_eq!(broker.pending().count(), 1);
        assert_eq!(broker.requests()[0].origin, "https://news.example");
        assert_eq!(broker.requests()[0].status, PermissionStatus::Requested);
        assert_eq!(
            broker
                .respond(request, UserDecision::Dismiss)
                .unwrap()
                .status,
            PermissionStatus::Dismissed
        );
        assert_eq!(broker.pending().count(), 0);
    }

    #[test]
    fn non_web_and_opaque_origins_cannot_request_site_permissions() {
        let mut broker = PermissionBrokerState::new();
        assert_eq!(
            broker.request(TabId(1), "about:blank", PermissionKind::ClipboardRead, 0),
            Err(PermissionError::InvalidOrigin)
        );
        assert_eq!(
            broker.request(TabId(1), "javascript:alert(1)", PermissionKind::Location, 0),
            Err(PermissionError::InvalidOrigin)
        );
    }

    #[test]
    fn missing_prompt_service_records_a_denial_without_granting_access() {
        let mut broker = PermissionBrokerState::new();
        let id = broker
            .deny_without_prompt(
                TabId(7),
                "https://example.test/camera",
                PermissionKind::Camera,
                42,
            )
            .unwrap();

        assert_eq!(broker.requests().len(), 1);
        assert_eq!(broker.requests()[0].id, id);
        assert_eq!(broker.requests()[0].tab_id, TabId(7));
        assert_eq!(broker.requests()[0].origin, "https://example.test");
        assert_eq!(broker.requests()[0].kind, PermissionKind::Camera);
        assert_eq!(broker.requests()[0].status, PermissionStatus::Denied);
        assert_eq!(broker.pending().count(), 0);
    }
}
