//! Exact UI + Shared queue with scripted completion, not Browser/guest inference.
use crate::session_services::{self, ReplyProgress, SessionServices, SubmitError};
use crate::session_ui::{Controller, Services, Status};
use libnagi::security::{AccountStore, Role, Session};
use nagi_ai::{CallerIdentity, ContextAuthority, ContextRequest, PublicBrowserContextApi};
use nagi_model::{AppSessionId, NodeId, ObjectId};
use nagi_model_manager::ResourceBudget;
use std::{borrow::ToOwned, rc::Rc};

struct Owner {
    // Deliberately !Send. Browser owner state must never enter a worker job.
    _owner_thread: Rc<()>,
    denied: bool,
    body: &'static str,
}
impl ContextAuthority for Owner {
    fn can_read_object(&self, _: CallerIdentity, _: ObjectId) -> bool {
        false
    }
}
impl PublicBrowserContextApi for Owner {
    fn current_page_context(
        &mut self,
        request: nagi_ai::BrowserContextRequest,
    ) -> Result<Option<nagi_ai::BrowserPageSnapshot>, nagi_ai::BrowserContextApiError> {
        assert_eq!(request.caller(), actor());
        if self.denied {
            return Err(nagi_ai::BrowserContextApiError::Denied);
        }
        Ok(Some(nagi_ai::BrowserPageSnapshot::new(
            1,
            None,
            None,
            None,
            Some(self.body.into()),
        )))
    }
}
struct Authority;
impl ContextAuthority for Authority {
    fn can_read_object(&self, _: CallerIdentity, _: ObjectId) -> bool {
        false
    }
}
fn actor() -> CallerIdentity {
    CallerIdentity {
        app_id: nagi_ai::NAGI_BAR_APP_ID,
        app_session_id: AppSessionId(8),
        node_id: NodeId(9),
        workspace_id: None,
    }
}
fn request() -> ContextRequest {
    ContextRequest {
        caller: actor(),
        selected_object: None,
        candidate_objects: std::vec::Vec::new(),
    }
}
struct Transport(SessionServices);
impl Services for Transport {
    fn on_signed_in(&mut self, user: &Session) -> Result<(), SubmitError> {
        session_services::host_test_bind_for_test(&self.0, user)
    }
    fn submit_text(&mut self, user: &Session, id: u64, text: &str) -> Result<(), SubmitError> {
        self.0.submit_text(user, id, text)
    }
    fn poll_reply(&mut self, user: &Session, id: u64) -> Result<ReplyProgress, SubmitError> {
        self.0.poll_reply(user, id)
    }
    fn cancel_request(&mut self, user: &Session, id: u64) -> Result<(), SubmitError> {
        self.0.cancel_request(user, id)
    }
    fn on_lock_or_signout(&mut self, user: &Session) {
        self.0.on_lock_or_signout(user);
    }
    fn acknowledge_model_terms(
        &mut self,
        user: &Session,
        reference: &str,
    ) -> Result<(), SubmitError> {
        self.0.acknowledge_model_terms(user, reference)
    }
    fn submit_page_summary(
        &mut self,
        user: &Session,
        caller: CallerIdentity,
        id: u64,
        request: ContextRequest,
        authority: &dyn ContextAuthority,
        browser: &mut dyn PublicBrowserContextApi,
    ) -> Result<(), SubmitError> {
        self.0
            .submit_page_summary(user, caller, id, request, authority, browser)
    }
    fn poll_page_summary(
        &mut self,
        user: &Session,
        id: u64,
        authority: &dyn ContextAuthority,
        browser: &mut dyn PublicBrowserContextApi,
    ) -> Result<ReplyProgress, SubmitError> {
        self.0.poll_page_summary(user, id, authority, browser)
    }
}
fn setup() -> (Controller, Transport, Session, Session, Owner) {
    let mut accounts = AccountStore::new();
    accounts
        .add_account(b"owner", Role::Owner, b"public fixture password")
        .unwrap();
    let user = accounts
        .authenticate(b"owner", b"public fixture password")
        .unwrap();
    let other = accounts
        .authenticate(b"owner", b"public fixture password")
        .unwrap();
    let mut transport = Transport(
        SessionServices::new(
            42,
            ResourceBudget {
                available_ram_bytes: 3072 * 1024 * 1024,
                available_storage_bytes: 2560 * 1024 * 1024,
                cpu_cores: 1,
                gpu_available: false,
            },
        )
        .unwrap(),
    );
    let mut ui = Controller::new();
    ui.sign_in(&user, &mut transport);
    let reference = transport.0.model_terms_reference().to_owned();
    // Explicit synthetic consent in this orchestration test, never operator consent.
    assert!(ui.offer_terms("fixture", &reference));
    assert!(ui.accept_request());
    ui.tick(Some(&user), &mut transport);
    (
        ui,
        transport,
        user,
        other,
        Owner {
            _owner_thread: Rc::new(()),
            denied: false,
            body: "untrusted fixture",
        },
    )
}
#[test]
fn summary_ui_needs_live_context_before_displaying_scripted_worker_reply() {
    let (mut ui, mut service, user, _, mut browser) = setup();
    assert!(ui.request_page_summary(
        &user,
        actor(),
        request(),
        &Authority,
        &mut browser,
        &mut service
    ));
    assert!(session_services::host_test_complete_queued_request(
        &service.0
    ));
    ui.tick(Some(&user), &mut service);
    assert_eq!(ui.status(), Status::Waiting);
    assert!(ui.output().is_empty());
    ui.tick_page_summary(Some(&user), &mut service, &Authority, &mut browser);
    assert_eq!(ui.status(), Status::Complete);
    assert_eq!(ui.output(), "scripted close/reopen reply");
}
#[test]
fn sharing_revocation_and_navigation_prevent_summary_display() {
    for denied in [false, true] {
        let (mut ui, mut service, user, _, mut browser) = setup();
        assert!(ui.request_page_summary(
            &user,
            actor(),
            request(),
            &Authority,
            &mut browser,
            &mut service
        ));
        assert!(session_services::host_test_complete_queued_request(
            &service.0
        ));
        browser.denied = denied;
        browser.body = "different page";
        ui.tick_page_summary(Some(&user), &mut service, &Authority, &mut browser);
        assert_eq!(ui.status(), Status::Denied);
        assert!(ui.output().is_empty());
    }
}
#[test]
fn lock_and_reauthentication_clear_summary_and_require_new_consent() {
    let (mut ui, mut service, user, other, mut browser) = setup();
    assert!(ui.request_page_summary(
        &user,
        actor(),
        request(),
        &Authority,
        &mut browser,
        &mut service
    ));
    assert!(session_services::host_test_complete_queued_request(
        &service.0
    ));
    let mut locked = user;
    locked.lock();
    ui.tick(Some(&locked), &mut service);
    assert_eq!(ui.status(), Status::SignedOut);
    assert!(ui.output().is_empty());
    ui.sign_in(&other, &mut service);
    assert!(!ui.request_page_summary(
        &other,
        actor(),
        request(),
        &Authority,
        &mut browser,
        &mut service
    ));
    assert_eq!(ui.status(), Status::TermsRequired);
    assert!(ui.output().is_empty());
    assert!(ui.request_id().is_none());
}
#[test]
fn cancelled_completed_summary_is_drained_without_display() {
    let (mut ui, mut service, user, _, mut browser) = setup();
    assert!(ui.request_page_summary(
        &user,
        actor(),
        request(),
        &Authority,
        &mut browser,
        &mut service
    ));
    assert!(session_services::host_test_complete_queued_request(
        &service.0
    ));
    ui.cancel(&mut service);
    ui.tick_page_summary(Some(&user), &mut service, &Authority, &mut browser);
    assert_eq!(ui.status(), Status::Cancelled);
    assert!(ui.output().is_empty());
    assert!(!ui.is_busy());
}
