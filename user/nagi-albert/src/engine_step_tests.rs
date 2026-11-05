//! Synthetic backend tests of actual orchestration; no Servo/Surface claim.
use super::*;
use nagi_model::{AppSessionId, NodeId, WorkspaceId};
use std::cell::RefCell;

#[derive(Default)]
struct Trace {
    events: Vec<&'static str>,
    views: Vec<ViewToken>,
    navigations: Vec<NavigationToken>,
    inputs: Vec<(TabId, PageInput)>,
    fail: Option<&'static str>,
}
struct SyntheticEngine(Rc<RefCell<Trace>>);
impl SyntheticEngine {
    fn event(&self, name: &'static str) -> Result<(), &'static str> {
        let mut trace = self.0.borrow_mut();
        trace.events.push(name);
        if trace.fail == Some(name) {
            Err(name)
        } else {
            Ok(())
        }
    }
}
impl Drop for SyntheticEngine {
    fn drop(&mut self) {
        self.0.borrow_mut().events.push("drop");
    }
}
impl EngineBackend for SyntheticEngine {
    type Error = &'static str;
    fn create_view(&mut self, view: &ViewToken) -> Result<(), Self::Error> {
        self.0.borrow_mut().views.push(view.clone());
        self.event("create")
    }
    fn remove_view(&mut self, _: &ViewToken) -> Result<(), Self::Error> {
        self.event("remove")
    }
    fn activate(&mut self, _: &ViewToken) -> Result<(), Self::Error> {
        self.event("activate")
    }
    fn navigate(&mut self, token: &NavigationToken) -> Result<(), Self::Error> {
        self.0.borrow_mut().navigations.push(token.clone());
        self.event("navigate")
    }
    fn stop(&mut self, _: &ViewToken) -> Result<(), Self::Error> {
        self.event("stop")
    }
    fn clear_input(&mut self) -> Result<(), Self::Error> {
        self.event("clear")
    }
    fn input(&mut self, view: &ViewToken, input: PageInput) -> Result<(), Self::Error> {
        self.0.borrow_mut().inputs.push((view.tab_id(), input));
        self.event("input")
    }
    fn spin_once(&mut self) -> Result<bool, Self::Error> {
        self.event("spin").map(|_| false)
    }
}
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
fn setup() -> (AlbertEngineStep<SyntheticEngine>, Rc<RefCell<Trace>>) {
    let trace = Rc::new(RefCell::new(Trace::default()));
    (
        AlbertEngineStep::new(binding(), SyntheticEngine(trace.clone())).unwrap(),
        trace,
    )
}
fn nav(step: &mut AlbertEngineStep<SyntheticEngine>, sequence: u64) {
    step.chrome(
        sequence,
        BrowserChromeAction::SetAddressText("https://example.com/".into()),
        0,
    )
    .unwrap();
    step.chrome(sequence + 1, BrowserChromeAction::SubmitAddress, 0)
        .unwrap();
}
#[test]
fn starts_unfocused_and_steps_exactly_once_per_call() {
    let (mut step, trace) = setup();
    assert_eq!(
        step.page_key(1, 30, true),
        Err(StepError::Owner(OwnerError::NotFocused))
    );
    assert!(!step.step().unwrap());
    assert!(!step.step().unwrap());
    assert_eq!(
        trace
            .borrow()
            .events
            .iter()
            .filter(|event| **event == "spin")
            .count(),
        2
    );
    assert!(trace.borrow().inputs.is_empty());
}
#[test]
fn delivers_once_and_consumes_error_sequences() {
    let (mut step, trace) = setup();
    step.set_focus(true).unwrap();
    step.primary_press(1, 10, 80, 0).unwrap();
    assert_eq!(
        trace.borrow().inputs[0].1,
        PageInput::PrimaryPress { x: 10, y: 18 }
    );
    step.page_key(2, 30, true).unwrap();
    assert_eq!(
        step.page_key(2, 30, true),
        Err(StepError::Owner(OwnerError::RepeatedInput))
    );
    assert_eq!(
        step.primary_press(3, 320, 80, 0),
        Err(StepError::Owner(OwnerError::OutsideSurface))
    );
    assert_eq!(
        step.page_key(3, 30, false),
        Err(StepError::Owner(OwnerError::RepeatedInput))
    );
    assert_eq!(trace.borrow().inputs.len(), 2);
}
#[test]
fn focus_loss_clears_before_another_step_and_refocus_does_not_replay() {
    let (mut step, trace) = setup();
    step.set_focus(true).unwrap();
    step.primary_press(1, 10, 80, 0).unwrap();
    step.page_key(2, 42, true).unwrap();
    let ticket = step.page_ticket(&trace.borrow().views[0]).unwrap();
    trace.borrow_mut().events.clear();
    step.set_focus(false).unwrap();
    assert!(step.validate_page(&ticket).is_err());
    assert_eq!(
        step.page_key(3, 42, false),
        Err(StepError::Owner(OwnerError::NotFocused))
    );
    step.step().unwrap();
    assert_eq!(trace.borrow().events, vec!["clear", "spin"]);
    step.set_focus(true).unwrap();
    step.step().unwrap();
    assert_eq!(trace.borrow().inputs.len(), 2);
}
#[test]
fn chrome_and_tab_changes_clear_engine_input_and_revoke_page_tickets() {
    let (mut step, trace) = setup();
    step.set_focus(true).unwrap();
    let ticket = step.page_ticket(&trace.borrow().views[0]).unwrap();
    trace.borrow_mut().events.clear();
    step.chrome(1, BrowserChromeAction::NewTab, 0).unwrap();
    assert_eq!(trace.borrow().events, vec!["clear", "create", "activate"]);
    assert!(step.validate_page(&ticket).is_err());
    step.chrome(2, BrowserChromeAction::FocusAddressBar, 0)
        .unwrap();
    assert_eq!(
        step.page_key(3, 30, true),
        Err(StepError::Owner(OwnerError::ChromeFocused))
    );
    assert!(trace.borrow().inputs.is_empty());
}
#[test]
fn reload_same_url_rejects_old_completion_and_duplicate() {
    let (mut step, trace) = setup();
    step.set_focus(true).unwrap();
    nav(&mut step, 1);
    let first = trace.borrow().navigations[0].clone();
    step.chrome(3, BrowserChromeAction::Reload, 0).unwrap();
    let second = trace.borrow().navigations[1].clone();
    assert_eq!(first.request().url, second.request().url);
    assert_ne!(first.request().id, second.request().id);
    assert_eq!(
        step.complete_navigation(&first, "old", 0),
        Err(StepError::StaleCallback)
    );
    assert_eq!(
        step.complete_navigation(&second, "new", 0),
        Ok(NavigationEventResult::Applied)
    );
    assert_eq!(
        step.complete_navigation(&second, "duplicate", 0),
        Err(StepError::StaleCallback)
    );
    assert_eq!(step.view().page_title, "new");
}
#[test]
fn closed_view_callbacks_and_foreign_owner_tokens_fail() {
    let (mut step, trace) = setup();
    step.set_focus(true).unwrap();
    nav(&mut step, 1);
    let view = trace.borrow().views[0].clone();
    let navigation = trace.borrow().navigations[0].clone();
    step.chrome(3, BrowserChromeAction::NewTab, 0).unwrap();
    step.chrome(4, BrowserChromeAction::CloseTab(view.tab_id()), 0)
        .unwrap();
    assert_eq!(step.validate_view(&view), Err(StepError::StaleCallback));
    assert_eq!(
        step.complete_navigation(&navigation, "old", 0),
        Err(StepError::StaleCallback)
    );
    let (other, _) = setup();
    assert_eq!(other.validate_view(&view), Err(StepError::StaleCallback));
    assert!(trace.borrow().events.contains(&"remove"));
}
#[test]
fn stop_consumes_navigation_mapping() {
    let (mut step, trace) = setup();
    step.set_focus(true).unwrap();
    nav(&mut step, 1);
    let navigation = trace.borrow().navigations[0].clone();
    step.chrome(3, BrowserChromeAction::Stop, 0).unwrap();
    assert_eq!(
        step.complete_navigation(&navigation, "late", 0),
        Err(StepError::StaleCallback)
    );
    assert_eq!(trace.borrow().events.last(), Some(&"stop"));
}
#[test]
fn captured_frame_is_never_relabeled_after_mutation() {
    let (mut step, trace) = setup();
    let ticket = step.page_ticket(&trace.borrow().views[0]).unwrap();
    let next = step.page_will_change(&ticket).unwrap();
    let mut frame = vec![77; 320 * 200 * 4];
    let page = vec![18; 320 * 138 * 4];
    assert!(step.compose_page(&ticket, &mut frame, 1280, &page).is_err());
    assert!(frame.iter().all(|value| *value == 77));
    step.compose_page(&next, &mut frame, 1280, &page).unwrap();
    assert_eq!(frame[62 * 1280], 18);
    step.validate_page(&next).unwrap();
}
#[test]
fn uncertain_input_failure_retires_without_retry_or_further_dispatch() {
    let (mut step, trace) = setup();
    step.set_focus(true).unwrap();
    trace.borrow_mut().fail = Some("input");
    assert_eq!(
        step.primary_press(1, 10, 80, 0),
        Err(StepError::Backend("input"))
    );
    assert_eq!(step.primary_press(1, 10, 80, 0), Err(StepError::Retired));
    assert_eq!(step.step(), Err(StepError::Retired));
    assert_eq!(trace.borrow().inputs.len(), 1);
    assert_eq!(trace.borrow().events.last(), Some(&"drop"));
}
#[test]
fn initialization_and_step_failures_drop_backend_once() {
    let trace = Rc::new(RefCell::new(Trace {
        fail: Some("create"),
        ..Trace::default()
    }));
    assert!(matches!(
        AlbertEngineStep::new(binding(), SyntheticEngine(trace.clone())),
        Err(StepError::Backend("create"))
    ));
    assert_eq!(trace.borrow().events, vec!["create", "drop"]);
    let (mut step, trace) = setup();
    trace.borrow_mut().fail = Some("spin");
    assert_eq!(step.step(), Err(StepError::Backend("spin")));
    drop(step);
    assert_eq!(
        trace
            .borrow()
            .events
            .iter()
            .filter(|event| **event == "drop")
            .count(),
        1
    );
}
#[test]
fn retire_invalidates_tokens_even_with_identical_replacement_binding() {
    let (mut step, trace) = setup();
    let view = trace.borrow().views[0].clone();
    let page = step.page_ticket(&view).unwrap();
    step.retire().unwrap();
    step.retire().unwrap();
    assert_eq!(step.validate_view(&view), Err(StepError::Retired));
    assert_eq!(step.validate_page(&page), Err(StepError::Retired));
    let (other, _) = setup();
    assert_eq!(other.validate_view(&view), Err(StepError::StaleCallback));
    assert_eq!(
        trace
            .borrow()
            .events
            .iter()
            .filter(|event| **event == "drop")
            .count(),
        1
    );
}
#[test]
fn production_context_api_remains_unavailable_with_live_backend() {
    // Existing owner tests cover raw actor requests; this checks the generic
    // production wrapper also has no route to a source adapter.
    fn assert_api<T: PublicBrowserContextApi>() {}
    assert_api::<AlbertEngineStep<SyntheticEngine>>();
    let (mut step, _) = setup();
    step.set_focus(true).unwrap();
    use nagi_ai::{
        BrowserContextError, CallerIdentity, ContextAuthority, ContextRequest, ContextResolver,
    };
    use nagi_model::{AppId, ObjectId};
    struct NoObjects;
    impl ContextAuthority for NoObjects {
        fn can_read_object(&self, _: CallerIdentity, _: ObjectId) -> bool {
            false
        }
        fn can_read_workspace(&self, _: CallerIdentity, _: WorkspaceId) -> bool {
            true
        }
    }
    let request = ContextRequest {
        caller: CallerIdentity {
            app_id: AppId(1),
            app_session_id: AppSessionId(9),
            node_id: NodeId(3),
            workspace_id: Some(WorkspaceId(2)),
        },
        selected_object: None,
        candidate_objects: vec![],
    };
    assert_eq!(
        ContextResolver
            .resolve_with_browser_api(request.clone(), &NoObjects, &mut step)
            .err(),
        Some(BrowserContextError::ApiUnavailable)
    );
    step.retire().unwrap();
    assert_eq!(
        ContextResolver
            .resolve_with_browser_api(request, &NoObjects, &mut step)
            .err(),
        Some(BrowserContextError::ApiUnavailable)
    );
}

#[test]
fn redirect_and_failure_use_original_navigation_mapping() {
    let (mut step, trace) = setup();
    step.set_focus(true).unwrap();
    nav(&mut step, 1);
    let navigation = trace.borrow().navigations[0].clone();
    let page = step.page_ticket(&navigation.view).unwrap();
    assert_eq!(
        step.redirect_navigation(&navigation, "https://example.org/"),
        Ok(NavigationEventResult::Applied)
    );
    assert!(step.validate_page(&page).is_err());
    assert_eq!(
        step.fail_navigation(&navigation, "network error"),
        Ok(NavigationEventResult::Applied)
    );
    assert_eq!(
        step.complete_navigation(&navigation, "late", 0),
        Err(StepError::StaleCallback)
    );
    assert_eq!(
        step.fail_navigation(&navigation, "duplicate"),
        Err(StepError::StaleCallback)
    );
    assert_eq!(step.view().page_status, "browser.status.failed");
}

#[test]
fn content_navigation_preserves_pending_load_and_invalidates_old_frames() {
    let (mut step, trace) = setup();
    step.set_focus(true).unwrap();
    nav(&mut step, 1);
    let navigation = trace.borrow().navigations[0].clone();
    let page = step.page_ticket(navigation.view()).unwrap();
    let changed = step.page_will_change(&page).unwrap();
    assert!(step
        .content_navigated(&page, "https://example.org/", "stale", 0)
        .is_err());
    // The existing core ignores source navigation while a normal load is
    // pending; do not lose that load's retained callback mapping.
    assert_eq!(
        step.content_navigated(&changed, "https://example.org/", "content", 0),
        Ok(NavigationEventResult::IgnoredStale)
    );
    assert_eq!(
        step.complete_navigation(&navigation, "loaded", 0),
        Ok(NavigationEventResult::Applied)
    );
    let page = step.page_ticket(navigation.view()).unwrap();
    let changed = step.page_will_change(&page).unwrap();
    assert_eq!(
        step.content_navigated(&changed, "https://example.org/", "content", 0),
        Ok(NavigationEventResult::Applied)
    );
    assert_eq!(
        step.complete_navigation(&navigation, "old request", 0),
        Err(StepError::StaleCallback)
    );
    assert!(step.validate_page(&changed).is_err());
    assert_eq!(step.view().address_text, "https://example.org/");
}

#[test]
fn clear_failure_prevents_step_and_always_releases_backend() {
    let (mut step, trace) = setup();
    step.set_focus(true).unwrap();
    trace.borrow_mut().fail = Some("clear");
    assert_eq!(step.set_focus(false), Err(StepError::Backend("clear")));
    assert_eq!(step.step(), Err(StepError::Retired));
    assert_eq!(trace.borrow().events.last(), Some(&"drop"));
    assert!(!trace.borrow().events.contains(&"spin"));
    assert_eq!(
        trace
            .borrow()
            .events
            .iter()
            .filter(|event| **event == "clear")
            .count(),
        2
    ); // initial focus, then failed focus loss; no retry
}

#[test]
fn ambiguous_navigation_failure_is_never_submitted_again() {
    let (mut step, trace) = setup();
    step.set_focus(true).unwrap();
    step.chrome(
        1,
        BrowserChromeAction::SetAddressText("https://example.com/".into()),
        0,
    )
    .unwrap();
    trace.borrow_mut().fail = Some("navigate");
    assert_eq!(
        step.chrome(2, BrowserChromeAction::SubmitAddress, 0),
        Err(StepError::Backend("navigate"))
    );
    assert_eq!(
        step.chrome(2, BrowserChromeAction::SubmitAddress, 0),
        Err(StepError::Retired)
    );
    assert_eq!(trace.borrow().navigations.len(), 1);
}

#[test]
fn invalid_binding_drops_provided_backend_without_engine_setup() {
    let trace = Rc::new(RefCell::new(Trace::default()));
    let mut invalid = binding();
    invalid.surface_id = 0;
    assert!(matches!(
        AlbertEngineStep::new(invalid, SyntheticEngine(trace.clone())),
        Err(StepError::Owner(OwnerError::InvalidBinding))
    ));
    assert_eq!(trace.borrow().events, vec!["drop"]);
}

#[test]
fn explicit_retire_reports_cleanup_failure_but_never_leaves_live_resources() {
    let (mut step, trace) = setup();
    trace.borrow_mut().fail = Some("clear");
    assert_eq!(step.retire(), Err("clear"));
    assert_eq!(step.retire(), Ok(()));
    assert_eq!(step.step(), Err(StepError::Retired));
    assert_eq!(
        trace
            .borrow()
            .events
            .iter()
            .filter(|event| **event == "clear")
            .count(),
        1
    );
    assert_eq!(trace.borrow().events.last(), Some(&"drop"));
}
