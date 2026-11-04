//! Albert's side of the public Browser Context API (M23).
//!
//! Only the OS assistant (Nagi Bar) may ask for the current page, and only
//! after the user enabled page sharing for it. Page data leaves Albert as an
//! untrusted snapshot; `nagi-ai` wraps it as untrusted context.

use nagi_ai::{BrowserContextApiError, BrowserPageSnapshot, CallerIdentity};
#[cfg(test)]
use nagi_model::AppId;

/// Logical identity of the Nagi Bar assistant.
pub use nagi_ai::NAGI_BAR_APP_ID;

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

/// Event-thread owner for coherent, versioned public page extraction.
///
/// This owns freshness and Albert's sharing decision, NOT OS caller provenance.
/// A production connector must authenticate the request using its kernel-stamped
/// sender and live Supervisor launch record, and bind the authenticated Session
/// before invoking it. A freely constructed CallerIdentity cannot satisfy that
/// missing connector. This helper itself launches no channel or Browser runtime.
#[derive(Debug, Default)]
pub struct BrowserContextOwner {
    policy: ContextSharingPolicy,
    epoch: nagi_ai::BrowserContextEpoch,
}

impl BrowserContextOwner {
    /// Sharing starts disabled; construction grants no page access.
    pub fn new() -> Self {
        Self::default()
    }

    /// Trusted chrome/settings only. Revoke retained snapshots before changing
    /// policy, even if the same value is restored before the next Desktop poll.
    pub fn set_sharing_by_user(&mut self, enabled: bool) {
        self.epoch.invalidate();
        if enabled {
            self.policy.enable_by_user();
        } else {
            self.policy.disable_by_user();
        }
    }

    /// The real event adapter must call this BEFORE navigation, active-tab,
    /// selection or relevant DOM/text changes. IDs/text returning to their old
    /// values must still revoke old snapshots. No content hash can replace it.
    pub fn invalidate_page_context(&mut self) {
        self.epoch.invalidate();
    }

    /// Run one extraction transaction on the owner thread. The extractor must
    /// obey the request flags, bound its work and propagate engine failures.
    /// If it pumps events, those events must update THIS owner before mutation.
    /// A changed epoch is rejected, never silently retried or stamped as fresh.
    /// No worker reference or model invocation occurs here.
    pub fn extract(
        &mut self,
        request: nagi_ai::BrowserContextRequest,
        extract: impl FnOnce(
            &mut Self,
            nagi_ai::BrowserContextRequest,
        ) -> Result<Option<BrowserPageSnapshot>, BrowserContextApiError>,
    ) -> Result<Option<BrowserPageSnapshot>, BrowserContextApiError> {
        self.policy.check(request.caller())?;
        let revision = self.epoch.revision();
        // Keep the Result until AFTER checking policy and the capture's epoch.
        // A callback that restores identical state cannot revive this capture.
        let result = extract(self, request);
        self.policy.check(request.caller())?;
        if !revision.is_current() {
            return Err(BrowserContextApiError::Unavailable);
        }
        result.map(|snapshot| snapshot.map(|page| page.with_revision(revision)))
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
    #[derive(Clone, Copy)]
    enum Change {
        None,
        NavigateAndRestore,
        Disable,
        DisableAndRestore,
        EngineError,
        NoPage,
        ReplaceStamp,
    }

    struct OwnerApi {
        owner: BrowserContextOwner,
        change: Change,
        calls: usize,
    }
    impl PublicBrowserContextApi for OwnerApi {
        fn current_page_context(
            &mut self,
            request: nagi_ai::BrowserContextRequest,
        ) -> Result<Option<BrowserPageSnapshot>, BrowserContextApiError> {
            let change = self.change;
            let calls = &mut self.calls;
            self.owner.extract(request, |owner, request| {
                *calls += 1;
                assert_eq!(request.caller(), caller(NAGI_BAR_APP_ID));
                assert!(request.includes_selected_text());
                assert!(request.includes_visible_text());
                match change {
                    Change::None => {}
                    Change::NavigateAndRestore => {
                        owner.invalidate_page_context();
                        owner.invalidate_page_context();
                    }
                    Change::Disable => owner.set_sharing_by_user(false),
                    Change::DisableAndRestore => {
                        owner.set_sharing_by_user(false);
                        owner.set_sharing_by_user(true);
                    }
                    Change::EngineError => return Err(BrowserContextApiError::Unavailable),
                    Change::NoPage => return Ok(None),
                    Change::ReplaceStamp => {
                        owner.invalidate_page_context();
                        return Ok(Some(
                            page_snapshot(7, None, None, None, Some("page".into()), true, true)
                                .with_revision(owner.epoch.revision()),
                        ));
                    }
                }
                Ok(Some(page_snapshot(
                    7,
                    Some("https://example.net/".into()),
                    Some("Page".into()),
                    Some(" ".into()),
                    Some("page".into()),
                    request.includes_selected_text(),
                    request.includes_visible_text(),
                )))
            })
        }
    }
    fn owner_api(enabled: bool, change: Change) -> OwnerApi {
        let mut owner = BrowserContextOwner::new();
        if enabled {
            owner.set_sharing_by_user(true);
        }
        OwnerApi {
            owner,
            change,
            calls: 0,
        }
    }
    fn resolve_owner(
        api: &mut OwnerApi,
        app_id: AppId,
    ) -> Result<nagi_ai::ResolvedContext, BrowserContextError> {
        // Synthetic actor only; this tests public API orchestration and makes
        // no assertion of kernel/OS sender provenance.
        ContextResolver.resolve_with_browser_api(
            ContextRequest {
                caller: caller(app_id),
                selected_object: None,
                candidate_objects: Vec::new(),
            },
            &NoObjects,
            api,
        )
    }

    #[test]
    fn coherent_owner_denies_before_engine_read_without_sharing_or_bar_app() {
        let mut api = owner_api(false, Change::None);
        assert_eq!(
            resolve_owner(&mut api, NAGI_BAR_APP_ID).err(),
            Some(BrowserContextError::ApiDenied)
        );
        assert_eq!(api.calls, 0);
        api.owner.set_sharing_by_user(true);
        assert_eq!(
            resolve_owner(&mut api, AppId::from_identifier(b"org.example.other")).err(),
            Some(BrowserContextError::ApiDenied)
        );
        assert_eq!(api.calls, 0);
    }

    #[test]
    fn coherent_owner_returns_live_untrusted_snapshot_and_preserves_blank_filter() {
        let mut api = owner_api(true, Change::None);
        let context = resolve_owner(&mut api, NAGI_BAR_APP_ID).unwrap();
        let page = context.browser_page().unwrap();
        assert!(page.has_current_revision());
        assert!(page.is_untrusted());
        assert_eq!(page.visible_text(), Some("page"));
        assert_eq!(page.selected_text(), None);
        let again = resolve_owner(&mut api, NAGI_BAR_APP_ID).unwrap();
        assert_eq!(context, again, "stable extraction uses the same epoch");
        assert_eq!(api.calls, 2);
    }

    #[test]
    fn navigation_and_policy_aba_during_extraction_discard_identical_final_page() {
        for change in [
            Change::NavigateAndRestore,
            Change::DisableAndRestore,
            Change::ReplaceStamp,
        ] {
            let mut api = owner_api(true, change);
            assert_eq!(
                resolve_owner(&mut api, NAGI_BAR_APP_ID).err(),
                Some(BrowserContextError::ApiUnavailable)
            );
            assert_eq!(api.calls, 1, "changed extraction must not retry");
        }
    }

    #[test]
    fn sharing_revocation_during_extraction_overrides_completed_engine_data() {
        let mut api = owner_api(true, Change::Disable);
        assert_eq!(
            resolve_owner(&mut api, NAGI_BAR_APP_ID).err(),
            Some(BrowserContextError::ApiDenied)
        );
        assert_eq!(api.calls, 1);
        api.change = Change::None;
        assert_eq!(
            resolve_owner(&mut api, NAGI_BAR_APP_ID).err(),
            Some(BrowserContextError::ApiDenied)
        );
        assert_eq!(api.calls, 1, "revoked policy prevents another engine read");
    }

    #[test]
    fn retained_snapshot_is_revoked_by_policy_restore_and_owner_destruction() {
        let mut api = owner_api(true, Change::None);
        let old = resolve_owner(&mut api, NAGI_BAR_APP_ID).unwrap();
        api.owner.set_sharing_by_user(false);
        api.owner.set_sharing_by_user(true);
        assert!(!old.browser_page().unwrap().has_current_revision());
        let restored = resolve_owner(&mut api, NAGI_BAR_APP_ID).unwrap();
        assert!(restored.browser_page().unwrap().has_current_revision());
        assert_ne!(old, restored);
        assert_eq!(
            old.browser_page().unwrap().visible_text(),
            restored.browser_page().unwrap().visible_text()
        );
        drop(api);
        assert!(!restored.browser_page().unwrap().has_current_revision());
    }

    #[test]
    fn coherent_owner_preserves_engine_failure_and_absent_page_without_retry() {
        for (change, expected) in [
            (Change::EngineError, BrowserContextError::ApiUnavailable),
            (Change::NoPage, BrowserContextError::NoActivePage),
        ] {
            let mut api = owner_api(true, change);
            assert_eq!(
                resolve_owner(&mut api, NAGI_BAR_APP_ID).err(),
                Some(expected)
            );
            assert_eq!(api.calls, 1);
        }
    }
}
