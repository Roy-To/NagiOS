//! Albert's side of the public Browser Context API (M23).
//!
//! Only the OS assistant (Nagi Bar) may ask for the current page, and only
//! after the user enabled page sharing for it. Page data leaves Albert as an
//! untrusted snapshot; `nagi-ai` wraps it as untrusted context.

use nagi_ai::{BrowserContextApiError, BrowserPageSnapshot, CallerIdentity};
use nagi_model::AppId;

/// Logical identity of the Nagi Bar assistant.
pub const NAGI_BAR_APP_ID: AppId = AppId::from_identifier(b"org.nagi.bar");

/// User-controlled permission for the assistant to read the current page.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ContextSharingPolicy {
    enabled: bool,
}

impl ContextSharingPolicy {
    /// Sharing starts disabled.
    pub const fn disabled() -> Self {
        Self { enabled: false }
    }

    /// Called only from Albert's trusted settings or chrome UI.
    pub fn enable_by_user(&mut self) {
        self.enabled = true;
    }

    pub fn disable_by_user(&mut self) {
        self.enabled = false;
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Whether `caller` may receive the current page.
    pub fn check(&self, caller: CallerIdentity) -> Result<(), BrowserContextApiError> {
        if self.enabled && caller.app_id == NAGI_BAR_APP_ID {
            Ok(())
        } else {
            Err(BrowserContextApiError::Denied)
        }
    }
}

/// Assemble the snapshot handed to the public API, omitting fields the
/// request did not ask for and blank text.
pub fn page_snapshot(
    tab_id: u64,
    url: Option<String>,
    title: Option<String>,
    selected_text: Option<String>,
    visible_text: Option<String>,
    include_selected: bool,
    include_visible: bool,
) -> BrowserPageSnapshot {
    let keep = |text: Option<String>, include: bool| {
        text.filter(|value| include && !value.trim().is_empty())
    };
    BrowserPageSnapshot::new(
        tab_id,
        url,
        title,
        keep(selected_text, include_selected),
        keep(visible_text, include_visible),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use nagi_ai::{BrowserContextApiError, ContextAuthority, ContextRequest, ContextResolver};
    use nagi_ai::{BrowserContextError, PublicBrowserContextApi};
    use nagi_model::{AppSessionId, NodeId, ObjectId};

    fn caller(app_id: AppId) -> CallerIdentity {
        CallerIdentity {
            app_id,
            app_session_id: AppSessionId(1),
            node_id: NodeId(1),
            workspace_id: None,
        }
    }

    struct NoObjects;

    impl ContextAuthority for NoObjects {
        fn can_read_object(&self, _caller: CallerIdentity, _object_id: ObjectId) -> bool {
            false
        }
    }

    struct FakeAlbert {
        policy: ContextSharingPolicy,
    }

    impl PublicBrowserContextApi for FakeAlbert {
        fn current_page_context(
            &mut self,
            request: nagi_ai::BrowserContextRequest,
        ) -> Result<Option<BrowserPageSnapshot>, BrowserContextApiError> {
            self.policy.check(request.caller())?;
            Ok(Some(page_snapshot(
                7,
                Some("https://example.net/".to_owned()),
                Some("Example Domain".to_owned()),
                Some("   ".to_owned()),
                Some("Example Domain body".to_owned()),
                request.includes_selected_text(),
                request.includes_visible_text(),
            )))
        }
    }

    fn resolve(
        app_id: AppId,
        policy: ContextSharingPolicy,
    ) -> Result<nagi_ai::ResolvedContext, BrowserContextError> {
        ContextResolver.resolve_with_browser_api(
            ContextRequest {
                caller: caller(app_id),
                selected_object: None,
                candidate_objects: Vec::new(),
            },
            &NoObjects,
            &mut FakeAlbert { policy },
        )
    }

    #[test]
    fn sharing_is_denied_until_the_user_enables_it() {
        assert_eq!(
            resolve(NAGI_BAR_APP_ID, ContextSharingPolicy::disabled()).err(),
            Some(BrowserContextError::ApiDenied)
        );
    }

    #[test]
    fn only_the_assistant_receives_the_page_as_untrusted_context() {
        let mut policy = ContextSharingPolicy::disabled();
        policy.enable_by_user();
        let resolved = resolve(NAGI_BAR_APP_ID, policy).expect("assistant context");
        let page = resolved.browser_page().expect("page");
        assert!(page.is_untrusted());
        assert_eq!(page.title(), Some("Example Domain"));
        assert_eq!(page.visible_text(), Some("Example Domain body"));
        assert_eq!(page.selected_text(), None, "blank selection is omitted");

        let other_app = AppId::from_identifier(b"org.example.notes");
        assert_eq!(
            resolve(other_app, policy).err(),
            Some(BrowserContextError::ApiDenied)
        );
    }

    #[test]
    fn disabling_sharing_revokes_access() {
        let mut policy = ContextSharingPolicy::disabled();
        policy.enable_by_user();
        policy.disable_by_user();
        assert!(!policy.is_enabled());
        assert_eq!(
            policy.check(caller(NAGI_BAR_APP_ID)),
            Err(BrowserContextApiError::Denied)
        );
    }
}
