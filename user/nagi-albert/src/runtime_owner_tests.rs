//! Synthetic host orchestration only. No authenticated IPC, Servo or OS adapter.
use super::*;
use crate::assistant_context::{page_snapshot, NAGI_BAR_APP_ID};
use nagi_ai::{
    BrowserContextError, CallerIdentity, ContextAuthority, ContextRequest, ContextResolver,
    ResolvedContext,
};
use nagi_model::{AppId, ObjectId};

fn binding() -> SurfaceBinding {
    SurfaceBinding {
        app_session_id: AppSessionId(4),
        node_id: NodeId(3),
        workspace_id: Some(WorkspaceId(2)),
        session_epoch: 1,
        surface_id: 5,
        width: 320,
        height: 200,
    }
}

fn attached() -> (AlbertRuntimeOwner, Attachment) {
    let mut owner = AlbertRuntimeOwner::new();
    let handle = owner.attach(binding()).unwrap();
    owner.set_focus(&handle, true).unwrap();
    owner.set_sharing_by_user(&handle, true).unwrap();
    (owner, handle)
}

fn navigate(
    owner: &mut AlbertRuntimeOwner,
    handle: &Attachment,
    sequence: u64,
) -> NavigationRequest {
    owner
        .dispatch_chrome(
            handle,
            sequence,
            BrowserChromeAction::SetAddressText("https://example.com/".into()),
            0,
        )
        .unwrap();
    match owner
        .dispatch_chrome(handle, sequence + 1, BrowserChromeAction::SubmitAddress, 0)
        .unwrap()
    {
        BrowserChromeOutcome::Navigate(request) => request,
        _ => panic!("expected navigation"),
    }
}

struct NoObjects;
impl ContextAuthority for NoObjects {
    fn can_read_object(&self, _: CallerIdentity, _: ObjectId) -> bool {
        false
    }
    fn can_read_workspace(&self, _: CallerIdentity, _: WorkspaceId) -> bool {
        true
    }
}

fn caller() -> CallerIdentity {
    CallerIdentity {
        app_id: NAGI_BAR_APP_ID,
        app_session_id: AppSessionId(99), // synthetic Bar, NOT Albert's session
        node_id: binding().node_id,
        workspace_id: binding().workspace_id,
    }
}

fn request(caller: CallerIdentity) -> ContextRequest {
    ContextRequest {
        caller,
        selected_object: None,
        candidate_objects: vec![],
    }
}

/// Test-only source bridge. These identity comparisons test local freshness,
/// NOT authentication. Production has no bridge or enable-source switch.
struct SyntheticSource<'a> {
    owner: &'a mut AlbertRuntimeOwner,
    handle: &'a Attachment,
    binding: SurfaceBinding,
    ticket: PageTicket,
    invalidate_during_read: bool,
}

impl PublicBrowserContextApi for SyntheticSource<'_> {
    fn current_page_context(
        &mut self,
        request: BrowserContextRequest,
    ) -> Result<Option<BrowserPageSnapshot>, BrowserContextApiError> {
        self.owner
            .validate_binding(self.handle, self.binding)
            .map_err(|_| BrowserContextApiError::Unavailable)?;
        self.owner
            .validate_page(&self.ticket)
            .map_err(|_| BrowserContextApiError::Unavailable)?;
        self.owner
            .require_focus(self.handle)
            .map_err(|_| BrowserContextApiError::Unavailable)?;
        if request.caller().node_id != self.binding.node_id
            || request.caller().workspace_id != self.binding.workspace_id
        {
            return Err(BrowserContextApiError::Denied);
        }
        let tab = self
            .owner
            .browser
            .active_tab()
            .ok_or(BrowserContextApiError::Unavailable)?;
        if tab.is_loading() {
            return Err(BrowserContextApiError::Unavailable);
        }
        let tab_id = tab.id().0;
        let url = tab.navigation().current_url().map(str::to_owned);
        let title = Some(tab.navigation().title().to_owned());
        let changed = self.invalidate_during_read;
        let events = self.owner.context.events();
        self.owner.context.extract(request, |_, request| {
            if changed {
                assert!(events.invalidate());
            }
            Ok(Some(page_snapshot(
                tab_id,
                url,
                title,
                None,
                Some("synthetic source".into()),
                request.includes_selected_text(),
                request.includes_visible_text(),
            )))
        })
    }
}

fn capture(owner: &mut AlbertRuntimeOwner, handle: &Attachment) -> ResolvedContext {
    let ticket = owner.page_ticket(handle).unwrap();
    let mut source = SyntheticSource {
        owner,
        handle,
        binding: binding(),
        ticket,
        invalidate_during_read: false,
    };
    ContextResolver
        .resolve_with_browser_api(request(caller()), &NoObjects, &mut source)
        .unwrap()
}

fn current(context: &ResolvedContext) -> bool {
    context.browser_page().unwrap().has_current_revision()
}

#[test]
fn invalid_missing_and_foreign_attachments_fail_closed() {
    let mut owner = AlbertRuntimeOwner::new();
    for bad in [
        SurfaceBinding {
            surface_id: 0,
            ..binding()
        },
        SurfaceBinding {
            session_epoch: 0,
            ..binding()
        },
        SurfaceBinding {
            height: PAGE_TOP,
            ..binding()
        },
        SurfaceBinding {
            app_session_id: AppSessionId(0),
            ..binding()
        },
    ] {
        assert_eq!(owner.attach(bad).err(), Some(OwnerError::InvalidBinding));
    }
    let (_, foreign) = attached();
    assert_eq!(
        owner.page_ticket(&foreign).err(),
        Some(OwnerError::StaleAttachment)
    );
    let handle = owner.attach(binding()).unwrap();
    assert_eq!(
        owner.attach(binding()).err(),
        Some(OwnerError::AlreadyAttached)
    );
    assert_eq!(
        owner
            .dispatch_chrome(&handle, 1, BrowserChromeAction::NewTab, 0)
            .err(),
        Some(OwnerError::NotFocused)
    );
    assert_eq!(
        owner.set_focus(&foreign, true),
        Err(OwnerError::StaleAttachment)
    );
    assert_eq!(owner.view().tabs.len(), 1);
}

#[test]
fn binding_comparison_rejects_cross_session_workspace_surface_node_and_epoch() {
    let (owner, handle) = attached();
    owner.validate_binding(&handle, binding()).unwrap();
    for other in [
        SurfaceBinding {
            app_session_id: AppSessionId(99),
            ..binding()
        },
        SurfaceBinding {
            workspace_id: None,
            ..binding()
        },
        SurfaceBinding {
            workspace_id: Some(WorkspaceId(9)),
            ..binding()
        },
        SurfaceBinding {
            session_epoch: 2,
            ..binding()
        },
        SurfaceBinding {
            surface_id: 9,
            ..binding()
        },
        SurfaceBinding {
            node_id: NodeId(9),
            ..binding()
        },
    ] {
        assert_eq!(
            owner.validate_binding(&handle, other),
            Err(OwnerError::InvalidBinding)
        );
    }
}

#[test]
fn repeated_input_does_not_repeat_navigation_or_tab_creation() {
    let (mut owner, handle) = attached();
    owner
        .dispatch_chrome(&handle, 1, BrowserChromeAction::NewTab, 0)
        .unwrap();
    for sequence in [0, 1] {
        assert_eq!(
            owner
                .dispatch_chrome(&handle, sequence, BrowserChromeAction::NewTab, 0)
                .err(),
            Some(OwnerError::RepeatedInput)
        );
    }
    assert_eq!(owner.view().tabs.len(), 2);
    assert!(owner
        .dispatch_chrome(&handle, 2, BrowserChromeAction::CloseTab(TabId(999)), 0)
        .is_err());
    assert_eq!(
        owner
            .dispatch_chrome(&handle, 2, BrowserChromeAction::NewTab, 0)
            .err(),
        Some(OwnerError::RepeatedInput)
    );
    let request = navigate(&mut owner, &handle, 3);
    assert_eq!(
        owner
            .dispatch_chrome(&handle, 4, BrowserChromeAction::SubmitAddress, 0)
            .err(),
        Some(OwnerError::RepeatedInput)
    );
    assert_eq!(
        owner.browser.active_tab().unwrap().pending_request(),
        Some(&request)
    );
}

#[test]
fn pointer_hit_testing_and_page_keyboard_preserve_focus() {
    let (mut owner, handle) = attached();
    owner.primary_press(&handle, 1, 100, 30, 0).unwrap();
    assert!(owner.view().address_focused);
    assert_eq!(
        owner.page_key(&handle, 2, 30, true).err(),
        Some(OwnerError::ChromeFocused)
    );
    let InputOutcome::Page { ticket, input } = owner.primary_press(&handle, 3, 100, 70, 0).unwrap()
    else {
        panic!("page")
    };
    assert_eq!(input, PageInput::PrimaryPress { x: 100, y: 8 });
    assert!(!owner.view().address_focused);
    owner.validate_page_input(&ticket).unwrap();
    let InputOutcome::Page {
        ticket: key_ticket,
        input,
    } = owner.page_key(&handle, 4, 30, true).unwrap()
    else {
        panic!("key")
    };
    assert_eq!(
        input,
        PageInput::Key {
            code: 30,
            pressed: true
        }
    );
    assert_eq!(
        owner.validate_page_input(&ticket),
        Err(OwnerError::StalePage)
    );
    owner.validate_page_input(&key_ticket).unwrap();
    owner.set_focus(&handle, false).unwrap();
    assert!(owner.validate_page_input(&key_ticket).is_err());
    assert_eq!(
        owner.page_key(&handle, 5, 30, false).err(),
        Some(OwnerError::NotFocused)
    );
    owner.set_focus(&handle, true).unwrap();
    assert_eq!(
        owner.primary_press(&handle, 5, 320, 70, 0).err(),
        Some(OwnerError::OutsideSurface)
    );
    assert_eq!(
        owner.primary_press(&handle, 5, 100, 70, 0).err(),
        Some(OwnerError::RepeatedInput)
    );
}

#[test]
fn focus_loss_cancels_preedit_without_committing_and_revokes_context() {
    let (mut owner, handle) = attached();
    owner
        .dispatch_chrome(&handle, 1, BrowserChromeAction::FocusAddressBar, 0)
        .unwrap();
    owner
        .dispatch_chrome(
            &handle,
            2,
            BrowserChromeAction::UpdateAddressComposition {
                text: "仮".into(),
                selection: 0..3,
            },
            0,
        )
        .unwrap();
    assert!(owner.view().address_composing);
    let old = capture(&mut owner, &handle);
    owner.set_focus(&handle, false).unwrap();
    assert!(!current(&old));
    assert!(!owner.view().address_composing);
    assert!(!owner.view().address_text.contains('仮'));
    owner.set_focus(&handle, true).unwrap();
    assert!(!current(&old));
}

#[test]
fn navigation_reload_and_stale_completion_are_epoch_bound() {
    let (mut owner, handle) = attached();
    let before = capture(&mut owner, &handle);
    let old_ticket = owner.page_ticket(&handle).unwrap();
    let first = navigate(&mut owner, &handle, 1);
    assert!(!current(&before));
    assert_eq!(owner.validate_page(&old_ticket), Err(OwnerError::StalePage));
    assert_eq!(
        owner
            .complete_navigation(&handle, &first, "first", 1)
            .unwrap(),
        NavigationEventResult::Applied
    );
    let page = capture(&mut owner, &handle);
    assert_eq!(page.browser_page().unwrap().title(), Some("first"));
    let BrowserChromeOutcome::Navigate(reload) = owner
        .dispatch_chrome(&handle, 3, BrowserChromeAction::Reload, 0)
        .unwrap()
    else {
        panic!("reload")
    };
    assert_eq!(first.url, reload.url);
    assert_ne!(first.id, reload.id);
    assert!(!current(&page));
    assert_eq!(
        owner
            .complete_navigation(&handle, &first, "stale", 2)
            .unwrap(),
        NavigationEventResult::IgnoredStale
    );
    assert_eq!(
        owner
            .complete_navigation(&handle, &reload, "new", 3)
            .unwrap(),
        NavigationEventResult::Applied
    );
    let new = capture(&mut owner, &handle);
    assert_eq!(
        owner
            .complete_navigation(&handle, &reload, "duplicate", 4)
            .unwrap(),
        NavigationEventResult::IgnoredStale
    );
    assert!(
        current(&new),
        "stale callback must not invalidate newer state"
    );
    assert_eq!(owner.view().page_title, "new");
}

#[test]
fn new_tab_selection_close_and_content_aba_revoke_old_pages() {
    let (mut owner, handle) = attached();
    let old = capture(&mut owner, &handle);
    let old_ticket = owner.page_ticket(&handle).unwrap();
    owner
        .dispatch_chrome(&handle, 1, BrowserChromeAction::NewTab, 0)
        .unwrap();
    assert!(!current(&old));
    assert_eq!(
        owner.page_will_change(&old_ticket).err(),
        Some(OwnerError::StalePage)
    );
    let new = capture(&mut owner, &handle);
    owner
        .dispatch_chrome(&handle, 2, BrowserChromeAction::SelectTab(TabId(1)), 0)
        .unwrap();
    assert!(!current(&new));
    assert!(!current(&old));
    let restored = capture(&mut owner, &handle);
    let ticket = owner.page_ticket(&handle).unwrap();
    let fresh = owner.page_will_change(&ticket).unwrap();
    assert!(!current(&restored));
    assert_eq!(
        owner.page_will_change(&ticket).err(),
        Some(OwnerError::StalePage)
    );
    owner.validate_page(&fresh).unwrap();
    let after_change = capture(&mut owner, &handle);
    owner
        .dispatch_chrome(&handle, 3, BrowserChromeAction::CloseTab(TabId(1)), 0)
        .unwrap();
    assert!(!current(&after_change));
    assert_ne!(owner.view().active_tab, Some(TabId(1)));
}

#[test]
fn detach_lock_signout_close_and_drop_revoke_retained_context() {
    for event in 0..5 {
        let (mut owner, handle) = attached();
        let old = capture(&mut owner, &handle);
        let ticket = owner.page_ticket(&handle).unwrap();
        let events = owner.context.events();
        match event {
            0 => owner.detach(&handle).unwrap(),
            1 => owner.lock(),
            2 => owner.sign_out(),
            3 => owner.close(),
            _ => {
                drop(owner);
                assert!(!current(&old));
                assert!(!events.invalidate());
                continue;
            }
        }
        assert!(!current(&old));
        assert!(
            !events.invalidate(),
            "old callback handles cannot survive reset"
        );
        assert!(owner.validate_page(&ticket).is_err());
        if event == 3 {
            assert_eq!(owner.attach(binding()).err(), Some(OwnerError::Closed));
        } else {
            let new_handle = owner.attach(binding()).unwrap();
            assert!(owner.validate_binding(&handle, binding()).is_err());
            assert!(owner.validate_page(&ticket).is_err());
            assert_eq!(owner.view().tabs.len(), 1);
            owner.set_focus(&new_handle, true).unwrap();
            let ticket = owner.page_ticket(&new_handle).unwrap();
            let mut source = SyntheticSource {
                owner: &mut owner,
                handle: &new_handle,
                binding: binding(),
                ticket,
                invalidate_during_read: false,
            };
            assert_eq!(
                ContextResolver
                    .resolve_with_browser_api(request(caller()), &NoObjects, &mut source)
                    .err(),
                Some(BrowserContextError::ApiDenied)
            );
        }
    }
}

#[test]
fn callback_invalidation_during_extraction_rejects_the_capture_without_retry() {
    let (mut owner, handle) = attached();
    let ticket = owner.page_ticket(&handle).unwrap();
    let mut source = SyntheticSource {
        owner: &mut owner,
        handle: &handle,
        binding: binding(),
        ticket,
        invalidate_during_read: true,
    };
    assert_eq!(
        ContextResolver
            .resolve_with_browser_api(request(caller()), &NoObjects, &mut source)
            .err(),
        Some(BrowserContextError::ApiUnavailable)
    );
}

#[test]
fn synthetic_bridge_rejects_missing_policy_invalid_owner_stale_page_and_cross_workspace() {
    let (mut owner, handle) = attached();
    let ticket = owner.page_ticket(&handle).unwrap();
    let mut source = SyntheticSource {
        owner: &mut owner,
        handle: &handle,
        binding: binding(),
        ticket,
        invalidate_during_read: false,
    };
    for other in [
        CallerIdentity {
            workspace_id: None,
            ..caller()
        },
        CallerIdentity {
            node_id: NodeId(8),
            ..caller()
        },
        CallerIdentity {
            app_id: AppId::from_identifier(b"other"),
            ..caller()
        },
    ] {
        assert_eq!(
            ContextResolver
                .resolve_with_browser_api(request(other), &NoObjects, &mut source)
                .err(),
            Some(BrowserContextError::ApiDenied)
        );
    }
    source.binding.session_epoch += 1;
    assert_eq!(
        ContextResolver
            .resolve_with_browser_api(request(caller()), &NoObjects, &mut source)
            .err(),
        Some(BrowserContextError::ApiUnavailable)
    );
    source.binding = binding();
    source.owner.set_sharing_by_user(&handle, false).unwrap();
    source.ticket = source.owner.page_ticket(&handle).unwrap();
    assert_eq!(
        ContextResolver
            .resolve_with_browser_api(request(caller()), &NoObjects, &mut source)
            .err(),
        Some(BrowserContextError::ApiDenied)
    );
    source.owner.set_sharing_by_user(&handle, true).unwrap();
    assert_eq!(
        ContextResolver
            .resolve_with_browser_api(request(caller()), &NoObjects, &mut source)
            .err(),
        Some(BrowserContextError::ApiUnavailable)
    );
}

#[test]
fn public_production_api_is_unavailable_even_with_sharing_and_plausible_raw_identity() {
    let (mut owner, handle) = attached();
    assert!(
        current(&capture(&mut owner, &handle)),
        "synthetic seam can exercise epoch only"
    );
    assert_eq!(
        ContextResolver
            .resolve_with_browser_api(request(caller()), &NoObjects, &mut owner)
            .err(),
        Some(BrowserContextError::ApiUnavailable)
    );
    owner.detach(&handle).unwrap();
    assert_eq!(
        ContextResolver
            .resolve_with_browser_api(request(caller()), &NoObjects, &mut owner)
            .err(),
        Some(BrowserContextError::ApiUnavailable)
    );
}

#[test]
fn sharing_toggle_and_same_value_revoke_instead_of_restoring_old_context() {
    let (mut owner, handle) = attached();
    let old = capture(&mut owner, &handle);
    owner.set_sharing_by_user(&handle, true).unwrap();
    assert!(!current(&old));
    let enabled = capture(&mut owner, &handle);
    owner.set_sharing_by_user(&handle, false).unwrap();
    owner.set_sharing_by_user(&handle, true).unwrap();
    assert!(!current(&enabled));
    assert!(current(&capture(&mut owner, &handle)));
}

#[test]
fn redirect_failure_and_source_navigation_use_the_existing_state_paths() {
    let (mut owner, handle) = attached();
    let first = navigate(&mut owner, &handle, 1);
    let pending_ticket = owner.page_ticket(&handle).unwrap();
    assert_eq!(
        owner
            .redirect_navigation(&handle, &first, "https://example.org/")
            .unwrap(),
        NavigationEventResult::Applied
    );
    assert!(owner.validate_page(&pending_ticket).is_err());
    assert_eq!(
        owner
            .complete_navigation(&handle, &first, "redirected", 1)
            .unwrap(),
        NavigationEventResult::Applied
    );
    assert_eq!(owner.view().address_text, "https://example.org/");
    let old = capture(&mut owner, &handle);
    let BrowserChromeOutcome::Navigate(reload) = owner
        .dispatch_chrome(&handle, 3, BrowserChromeAction::Reload, 0)
        .unwrap()
    else {
        panic!("reload")
    };
    assert!(!current(&old));
    assert_eq!(
        owner.fail_navigation(&handle, &first, "stale").unwrap(),
        NavigationEventResult::IgnoredStale
    );
    assert_eq!(
        owner.fail_navigation(&handle, &reload, "failed").unwrap(),
        NavigationEventResult::Applied
    );
    assert_eq!(owner.view().page_status, "browser.status.failed");
    let ticket = owner.page_ticket(&handle).unwrap();
    let before_mutation = capture(&mut owner, &handle);
    let changed = owner.page_will_change(&ticket).unwrap();
    assert!(!current(&before_mutation));
    assert!(owner
        .content_navigated(&ticket, "https://example.net/", "stale", 2)
        .is_err());
    assert_eq!(
        owner
            .content_navigated(&changed, "https://example.net/", "content", 2)
            .unwrap(),
        NavigationEventResult::Applied
    );
    assert_eq!(owner.view().address_text, "https://example.net/");
    assert_eq!(owner.view().page_title, "content");
    let current = owner.page_ticket(&handle).unwrap();
    assert_eq!(
        owner
            .redirect_navigation(&handle, &reload, "https://example.com/")
            .unwrap(),
        NavigationEventResult::IgnoredStale
    );
    owner.validate_page(&current).unwrap();
}

#[test]
fn rendering_and_pointer_routing_share_the_actual_page_rectangle() {
    let (mut owner, handle) = attached();
    let ticket = owner.page_ticket(&handle).unwrap();
    let stride = binding().width as usize * 4;
    let mut frame = vec![0u8; stride * binding().height as usize];
    let page = vec![73u8; stride * (binding().height - PAGE_TOP) as usize];
    owner
        .compose_page(&ticket, &mut frame, stride, &page)
        .unwrap();
    assert_eq!(&frame[PAGE_TOP as usize * stride..], page.as_slice());
    assert_ne!(&frame[..stride], &page[..stride]);
    assert_eq!(
        owner.compose_page(&ticket, &mut frame, stride, &page[..10]),
        Err(OwnerError::Render(ChromeRenderError::TruncatedFrame))
    );
    owner.primary_press(&handle, 1, 100, 30, 0).unwrap();
    for (sequence, y) in [(2, 48), (3, PAGE_TOP - 1)] {
        assert!(matches!(
            owner.primary_press(&handle, sequence, 100, y, 0).unwrap(),
            InputOutcome::Chrome(_)
        ));
        assert!(
            owner.view().address_focused,
            "status strip cannot route into page"
        );
    }
    let InputOutcome::Page {
        ticket: page_ticket,
        input,
    } = owner.primary_press(&handle, 4, 100, PAGE_TOP, 0).unwrap()
    else {
        panic!("page")
    };
    assert_eq!(input, PageInput::PrimaryPress { x: 100, y: 0 });
    owner.validate_page_input(&page_ticket).unwrap();
    assert_eq!(page_ticket.tab_id(), owner.view().active_tab.unwrap());
    let untouched = frame.clone();
    assert!(owner
        .compose_page(&ticket, &mut frame, stride, &page)
        .is_err());
    assert_eq!(
        frame, untouched,
        "stale callbacks cannot touch the destination"
    );
}

#[test]
fn later_attachment_cannot_use_old_session_events_or_input_even_if_ids_repeat() {
    let (mut owner, old) = attached();
    let request = navigate(&mut owner, &old, 1);
    owner.detach(&old).unwrap();
    assert_eq!(owner.detach(&old), Err(OwnerError::StaleAttachment));
    let later = SurfaceBinding {
        session_epoch: 2,
        workspace_id: Some(WorkspaceId(8)),
        app_session_id: AppSessionId(9),
        ..binding()
    };
    let new = owner.attach(later).unwrap();
    owner.set_focus(&new, true).unwrap();
    let fresh_request = navigate(&mut owner, &new, 1);
    assert_eq!(
        request.id, fresh_request.id,
        "reset restarts local navigation IDs"
    );
    assert_eq!(
        owner.complete_navigation(&old, &request, "old session", 2),
        Err(OwnerError::StaleAttachment)
    );
    assert_eq!(
        owner.page_key(&old, 3, 30, true).err(),
        Some(OwnerError::StaleAttachment)
    );
    assert_eq!(
        owner.validate_binding(&new, binding()),
        Err(OwnerError::InvalidBinding)
    );
    owner.validate_binding(&new, later).unwrap();
    assert_eq!(
        owner
            .complete_navigation(&new, &fresh_request, "new session", 2)
            .unwrap(),
        NavigationEventResult::Applied
    );
    assert_eq!(owner.view().page_title, "new session");
}
