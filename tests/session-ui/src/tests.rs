use super::session_ui::{
    Controller, Reply, Services, Status, SubmitError, INPUT_CAPACITY, OUTPUT_CAPACITY,
    TERMS_REFERENCE_CAPACITY,
};
use libnagi::security::{AccountStore, Role, Session};
use nagi_model_manager::{BackendId, ModelId, ModelResponse, ProviderId, TokenUsage};
use std::{collections::VecDeque, format, string::String, vec, vec::Vec};

#[derive(Debug, Eq, PartialEq)]
enum Event {
    SignedIn(Session),
    Submit(Session, u64, String),
    Poll(Session, u64),
    Cancel(Session, u64),
    Invalidated(Session),
    Acknowledge(Session, String),
}

#[derive(Default)]
struct FakeServices {
    events: Vec<Event>,
    submits: VecDeque<Result<(), SubmitError>>,
    acknowledgements: VecDeque<Result<(), SubmitError>>,
    replies: VecDeque<Result<Reply, SubmitError>>,
    bindings: VecDeque<Result<(), SubmitError>>,
    cancellations: VecDeque<Result<(), SubmitError>>,
}

impl Services for FakeServices {
    fn on_signed_in(&mut self, session: &Session) -> Result<(), SubmitError> {
        self.events.push(Event::SignedIn(*session));
        self.bindings.pop_front().unwrap_or(Ok(()))
    }

    fn submit_text(&mut self, session: &Session, id: u64, text: &str) -> Result<(), SubmitError> {
        self.events.push(Event::Submit(*session, id, text.into()));
        self.submits.pop_front().unwrap_or(Ok(()))
    }

    fn poll_reply(&mut self, session: &Session, id: u64) -> Result<Reply, SubmitError> {
        self.events.push(Event::Poll(*session, id));
        self.replies.pop_front().unwrap_or(Ok(Reply::Pending))
    }

    fn cancel_request(&mut self, session: &Session, id: u64) -> Result<(), SubmitError> {
        self.events.push(Event::Cancel(*session, id));
        self.cancellations.pop_front().unwrap_or(Ok(()))
    }

    fn on_lock_or_signout(&mut self, session: &Session) {
        self.events.push(Event::Invalidated(*session));
    }

    fn acknowledge_model_terms(
        &mut self,
        session: &Session,
        reference: &str,
    ) -> Result<(), SubmitError> {
        self.events
            .push(Event::Acknowledge(*session, reference.into()));
        self.acknowledgements.pop_front().unwrap_or(Ok(()))
    }
}

fn sessions() -> (Session, Session) {
    let mut accounts = AccountStore::new();
    accounts
        .add_account(b"owner", Role::Owner, b"password")
        .unwrap();
    accounts
        .add_account(b"guest", Role::Guest, b"password")
        .unwrap();
    (
        accounts.authenticate(b"owner", b"password").unwrap(),
        accounts.authenticate(b"guest", b"password").unwrap(),
    )
}

// UI transport values only: no model artifact, provider backend or inference
// fixture is constructed. Model/provider/backend IDs remain opaque to the UI.
fn response(id: u64, text: &str) -> Result<Reply, SubmitError> {
    Ok(Reply::Ready(ModelResponse {
        request_id: id,
        model_id: ModelId::new("ui-test-model").unwrap(),
        provider_id: ProviderId::new("ui-test-provider").unwrap(),
        backend_id: BackendId::new("ui-test-backend").unwrap(),
        text: text.into(),
        usage: TokenUsage {
            input_tokens: 1,
            output_tokens: 1,
        },
    }))
}

fn started(session: &Session, fake: &mut FakeServices) -> Controller {
    let mut ui = Controller::new();
    ui.sign_in(session, fake);
    assert!(ui.set_input("hello"));
    assert!(ui.request_submit());
    ui.tick(Some(session), fake);
    ui
}

#[test]
fn unsigned_and_locked_sessions_cannot_queue_any_service_work() {
    let (mut session, _) = sessions();
    let mut fake = FakeServices::default();
    let mut ui = Controller::new();
    assert!(!ui.set_input("private"));
    assert!(!ui.push_text("x"));
    assert!(!ui.request_submit());
    assert!(!ui.offer_terms("Model", "pinned-reference"));
    assert!(!ui.accept_request());
    ui.tick(None, &mut fake);
    session.lock();
    ui.sign_in(&session, &mut fake);
    ui.tick(Some(&session), &mut fake);
    assert!(fake.events.is_empty());
    assert_eq!(ui.status(), Status::SignedOut);
    assert_eq!(ui.input(), "");
    assert_eq!(ui.output(), "");
}

#[test]
fn submit_is_deferred_and_polling_is_one_step_per_idle_tick() {
    let (session, _) = sessions();
    let mut fake = FakeServices::default();
    let mut ui = Controller::new();
    ui.sign_in(&session, &mut fake);
    ui.sign_in(&session, &mut fake);
    assert!(ui.set_input("  名前について  "));
    assert!(ui.request_submit());
    assert_eq!(ui.status(), Status::Queued);
    assert_eq!(fake.events, vec![Event::SignedIn(session)]);
    ui.tick(Some(&session), &mut fake);
    assert_eq!(
        fake.events.last(),
        Some(&Event::Submit(session, 1, "  名前について  ".into()))
    );
    assert_eq!(ui.status(), Status::Waiting);
    assert!(!ui.set_input("overwrite in-flight input"));
    assert!(!ui.request_submit());
    ui.tick(Some(&session), &mut fake);
    assert_eq!(fake.events.last(), Some(&Event::Poll(session, 1)));
    assert_eq!(ui.output(), "");
    fake.replies.push_back(response(1, "回答"));
    ui.tick(Some(&session), &mut fake);
    assert_eq!(ui.output(), "回答");
    assert_eq!(ui.status(), Status::Complete);
    assert!(!ui.is_busy());
    let events = fake.events.len();
    ui.tick(Some(&session), &mut fake);
    assert_eq!(fake.events.len(), events);
}

#[test]
fn queued_cancel_never_submits_and_retains_editable_input() {
    let (session, _) = sessions();
    let mut fake = FakeServices::default();
    let mut ui = Controller::new();
    ui.sign_in(&session, &mut fake);
    ui.set_input("first");
    ui.request_submit();
    ui.cancel(&mut fake);
    ui.tick(Some(&session), &mut fake);
    assert_eq!(fake.events, vec![Event::SignedIn(session)]);
    assert_eq!(ui.input(), "first");
    assert!(ui.set_input("retry"));
    assert!(ui.request_submit());
    assert_eq!(ui.request_id(), Some(2));
    ui.tick(Some(&session), &mut fake);
    assert_eq!(
        fake.events.last(),
        Some(&Event::Submit(session, 2, "retry".into()))
    );
}

#[test]
fn in_flight_cancel_rejects_late_reply_and_retry_uses_a_new_id() {
    let (session, _) = sessions();
    let mut fake = FakeServices::default();
    let mut ui = started(&session, &mut fake);
    ui.cancel(&mut fake);
    assert_eq!(fake.events.last(), Some(&Event::Cancel(session, 1)));
    assert_eq!(ui.input(), "hello");
    assert!(!ui.push_text(" again"));
    assert!(!ui.request_submit());
    fake.replies
        .push_back(response(1, "discard cancelled result"));
    ui.tick(Some(&session), &mut fake);
    assert_eq!(ui.output(), "");
    assert!(!ui.is_busy());
    assert!(ui.push_text(" again"));
    ui.request_submit();
    ui.tick(Some(&session), &mut fake);
    fake.replies.push_back(response(1, "stale private reply"));
    ui.tick(Some(&session), &mut fake);
    assert_eq!(ui.status(), Status::InvalidReply);
    assert_eq!(ui.output(), "");
    assert_eq!(fake.events.last(), Some(&Event::Cancel(session, 2)));
    assert!(!ui.is_busy());
    assert!(ui.request_submit());
    ui.tick(Some(&session), &mut fake);
    fake.replies.push_back(response(3, "current reply"));
    ui.tick(Some(&session), &mut fake);
    assert_eq!(ui.output(), "current reply");
}

#[test]
fn live_session_switch_cancels_old_request_before_invalidation_and_never_polls_it() {
    let (first, second) = sessions();
    let mut fake = FakeServices::default();
    let mut ui = started(&first, &mut fake);
    fake.replies
        .push_back(response(1, "old account private reply"));
    let before = fake.events.len();
    ui.tick(Some(&second), &mut fake);
    assert_eq!(
        &fake.events[before..],
        &[
            Event::Cancel(first, 1),
            Event::Invalidated(first),
            Event::SignedIn(second)
        ]
    );
    assert_eq!(ui.session(), Some(second));
    assert_eq!(ui.input(), "");
    assert_eq!(ui.output(), "");
    assert!(ui.terms().is_none());
    ui.set_input("new account");
    ui.request_submit();
    assert_eq!(ui.request_id(), Some(2));
    ui.tick(Some(&second), &mut fake);
    ui.tick(Some(&second), &mut fake);
    assert_eq!(ui.output(), "");
    assert_eq!(ui.status(), Status::InvalidReply);
}

#[test]
fn lock_and_signout_erase_completed_output_and_terms() {
    let (session, _) = sessions();
    let mut fake = FakeServices::default();
    let mut ui = started(&session, &mut fake);
    fake.replies
        .push_back(response(1, "private completed reply"));
    ui.tick(Some(&session), &mut fake);
    ui.offer_terms("Model", "exact-pinned-reference");
    ui.accept_request();
    let mut locked = session;
    locked.lock();
    let before = fake.events.len();
    ui.tick(Some(&locked), &mut fake);
    assert_eq!(&fake.events[before..], &[Event::Invalidated(session)]);
    assert_eq!(ui.session(), None);
    assert_eq!(ui.input(), "");
    assert_eq!(ui.output(), "");
    assert!(ui.terms().is_none());
    assert!(!ui.is_busy());
    ui.tick(None, &mut fake);
    assert_eq!(fake.events.len(), before + 1);
    ui.sign_in(&session, &mut fake);
    ui.set_input("relogin");
    ui.request_submit();
    assert_eq!(ui.request_id(), Some(2));
}

#[test]
fn losing_live_session_clears_queued_submit_without_executing_it() {
    let (session, _) = sessions();
    let mut fake = FakeServices::default();
    let mut ui = Controller::new();
    ui.sign_in(&session, &mut fake);
    ui.set_input("do not submit after logout");
    ui.request_submit();
    ui.tick(None, &mut fake);
    assert_eq!(
        fake.events,
        vec![Event::SignedIn(session), Event::Invalidated(session)]
    );
    assert!(!ui.is_busy());
    assert_eq!(ui.input(), "");
}

#[test]
fn utf8_input_is_bounded_without_partial_appends_and_backspace_removes_one_character() {
    let (session, _) = sessions();
    let mut fake = FakeServices::default();
    let mut ui = Controller::new();
    ui.sign_in(&session, &mut fake);
    let count = INPUT_CAPACITY / 3;
    let original = "語".repeat(count);
    assert!(ui.set_input(&original));
    assert_eq!(ui.input().len(), INPUT_CAPACITY);
    assert!(!ui.push_text("語"));
    assert_eq!(ui.input(), original);
    assert!(!ui.set_input(&"a".repeat(INPUT_CAPACITY + 1)));
    assert_eq!(ui.input(), original);
    assert!(ui.backspace());
    assert_eq!(ui.input(), "語".repeat(count - 1));
    assert!(!ui.push_text("🙂"));
    assert!(ui.backspace());
    assert!(ui.push_text("🙂"));
    assert!(ui.backspace());
    assert_eq!(ui.input(), "語".repeat(count - 2));
    assert!(ui.set_input(&"x".repeat(INPUT_CAPACITY)));
    assert!(!ui.push_text("x"));
    assert_eq!(ui.status(), Status::InputFull);
}

#[test]
fn oversized_response_is_rejected_and_exact_bound_is_retained() {
    let (session, _) = sessions();
    let mut fake = FakeServices::default();
    let mut ui = started(&session, &mut fake);
    fake.replies
        .push_back(response(1, &"語".repeat(OUTPUT_CAPACITY / 3 + 1)));
    ui.tick(Some(&session), &mut fake);
    assert_eq!(ui.output(), "");
    assert_eq!(ui.status(), Status::InvalidReply);
    ui.request_submit();
    ui.tick(Some(&session), &mut fake);
    fake.replies
        .push_back(response(2, &"x".repeat(OUTPUT_CAPACITY)));
    ui.tick(Some(&session), &mut fake);
    assert_eq!(ui.output().len(), OUTPUT_CAPACITY);
    assert_eq!(ui.status(), Status::Complete);
}

#[test]
fn submit_errors_have_localization_keys_and_allow_explicit_edit_retry() {
    for (error, expected) in [
        (SubmitError::Unavailable, Status::Unavailable),
        (SubmitError::Busy, Status::Busy),
        (SubmitError::TermsRequired, Status::TermsRequired),
        (SubmitError::WorkerUnavailable, Status::WorkerUnavailable),
        (SubmitError::Denied, Status::Denied),
        (SubmitError::InvalidInput, Status::InvalidInput),
        (SubmitError::InputTooLarge, Status::InputFull),
        (SubmitError::InvalidTerms, Status::TermsFailed),
        (SubmitError::InvalidRequestId, Status::InvalidReply),
    ] {
        let (session, _) = sessions();
        let mut fake = FakeServices::default();
        fake.submits.push_back(Err(error));
        let mut ui = started(&session, &mut fake);
        assert_eq!(ui.status(), expected);
        assert!(ui.status_key().starts_with("desktop.ai.status."));
        assert_eq!(ui.input(), "hello");
        assert_eq!(ui.output(), "");
        assert!(!ui.is_busy());
        assert!(ui.set_input("retry after error"));
        assert!(ui.request_submit());
        ui.tick(Some(&session), &mut fake);
        assert_eq!(
            fake.events.last(),
            Some(&Event::Submit(session, 2, "retry after error".into()))
        );
    }
}

#[test]
fn failed_poll_has_no_displayed_response_and_releases_request_for_retry() {
    let (session, _) = sessions();
    let mut fake = FakeServices::default();
    let mut ui = started(&session, &mut fake);
    fake.replies.push_back(Ok(Reply::Failed(
        crate::session_services::ReplyError::Model(
            nagi_model_manager::RuntimeError::Cancelled.into(),
        ),
    )));
    ui.tick(Some(&session), &mut fake);
    assert_eq!(ui.status(), Status::Failed);
    assert_eq!(ui.output(), "");
    assert!(!ui.is_busy());
    assert!(ui.request_submit());
    assert_eq!(ui.request_id(), Some(2));
}

#[test]
fn terms_offer_requires_explicit_session_acknowledgement_then_explicit_submit() {
    let (session, _) = sessions();
    let mut fake = FakeServices::default();
    let mut ui = Controller::new();
    ui.sign_in(&session, &mut fake);
    ui.set_input("retained prompt");
    assert!(ui.offer_terms("名前", "model-terms:pinned-v1"));
    ui.tick(Some(&session), &mut fake);
    assert_eq!(fake.events, vec![Event::SignedIn(session)]);
    assert!(!ui.terms().unwrap().acknowledged());
    assert!(!ui.request_submit());
    fake.acknowledgements
        .push_back(Err(SubmitError::InvalidTerms));
    assert!(ui.accept_request());
    assert!(!ui.accept_request());
    assert_eq!(fake.events.len(), 1);
    ui.tick(Some(&session), &mut fake);
    assert_eq!(
        fake.events.last(),
        Some(&Event::Acknowledge(session, "model-terms:pinned-v1".into()))
    );
    assert!(!ui.terms().unwrap().acknowledged());
    assert_eq!(ui.status(), Status::TermsFailed);
    assert!(!ui.request_submit());
    assert!(ui.accept_request());
    ui.tick(Some(&session), &mut fake);
    assert!(ui.terms().unwrap().acknowledged());
    assert_eq!(ui.status(), Status::TermsAccepted);
    assert_eq!(ui.input(), "retained prompt");
    assert!(!ui.is_busy());
    assert!(!ui.accept_request());
    let events = fake.events.len();
    ui.tick(Some(&session), &mut fake);
    assert_eq!(fake.events.len(), events);
    assert!(ui.request_submit());
    ui.tick(Some(&session), &mut fake);
    assert_eq!(
        fake.events.last(),
        Some(&Event::Submit(session, 1, "retained prompt".into()))
    );
}

#[test]
fn cancellation_or_session_switch_before_accept_tick_never_acknowledges() {
    for switch in [false, true] {
        let (first, second) = sessions();
        let mut fake = FakeServices::default();
        let mut ui = Controller::new();
        ui.sign_in(&first, &mut fake);
        ui.offer_terms("Model", "pinned-reference");
        ui.accept_request();
        if switch {
            ui.tick(Some(&second), &mut fake);
            assert!(ui.terms().is_none());
        } else {
            ui.cancel(&mut fake);
            ui.tick(Some(&first), &mut fake);
            assert!(!ui.terms().unwrap().acknowledged());
        }
        assert!(!fake
            .events
            .iter()
            .any(|event| matches!(event, Event::Acknowledge(..))));
    }
}

#[test]
fn terms_reference_is_never_truncated_or_taken_from_response_text() {
    let (session, _) = sessions();
    let mut fake = FakeServices::default();
    let mut ui = started(&session, &mut fake);
    fake.replies
        .push_back(response(1, "accept model-terms:invented automatically"));
    ui.tick(Some(&session), &mut fake);
    assert!(ui.terms().is_none());
    assert!(!ui.accept_request());
    assert!(ui.offer_terms("Model", "valid-reference"));
    assert!(!ui.offer_terms(
        "Model",
        &format!("{}x", "r".repeat(TERMS_REFERENCE_CAPACITY))
    ));
    assert!(ui.terms().is_none());
    assert!(!ui.accept_request());
    assert!(!ui.offer_terms(" ", "reference"));
    assert!(!ui.offer_terms("Model", " "));
    assert!(!fake
        .events
        .iter()
        .any(|event| matches!(event, Event::Acknowledge(..))));
}

#[test]
fn whitespace_prompt_does_not_submit_or_consume_a_request_id() {
    let (session, _) = sessions();
    let mut fake = FakeServices::default();
    let mut ui = Controller::new();
    ui.sign_in(&session, &mut fake);
    ui.set_input("  \n\t　");
    assert!(!ui.request_submit());
    assert_eq!(ui.status(), Status::EmptyInput);
    assert!(!ui.is_busy());
    ui.set_input("real prompt");
    ui.request_submit();
    assert_eq!(ui.request_id(), Some(1));
}

#[test]
fn failed_readiness_and_locked_inputless_steps_always_yield_without_binding() {
    let (session, _) = sessions();
    let mut fake = FakeServices::default();
    let mut ui = Controller::new();
    let mut yields = 0;
    for _ in 0..3 {
        crate::session_ui::idle_step(&mut ui, Some(&session), false, &mut fake, || yields += 1);
    }
    assert_eq!(yields, 3);
    assert!(fake.events.is_empty());
    crate::session_ui::idle_step(&mut ui, Some(&session), true, &mut fake, || yields += 1);
    assert_eq!(fake.events, vec![Event::SignedIn(session)]);
    ui.set_input("queued");
    ui.request_submit();
    crate::session_ui::idle_step(&mut ui, None, false, &mut fake, || yields += 1);
    assert_eq!(yields, 5);
    assert_eq!(ui.output(), "");
    assert!(!fake.events.iter().any(|e| matches!(e, Event::Submit(..))));
}

#[test]
fn binding_worker_failure_is_visible_and_not_retried_every_idle_step() {
    let (session, _) = sessions();
    let mut fake = FakeServices::default();
    fake.bindings.push_back(Err(SubmitError::WorkerUnavailable));
    let mut ui = Controller::new();
    for _ in 0..3 {
        ui.tick(Some(&session), &mut fake);
    }
    assert_eq!(ui.status(), Status::WorkerUnavailable);
    assert_eq!(fake.events, vec![Event::SignedIn(session)]);
    assert!(!ui.set_input("not bound"));
    assert!(!ui.request_submit());
}

#[test]
fn poll_errors_and_failed_worker_results_never_publish_text() {
    let (session, _) = sessions();
    for (reply, status) in [
        (Err(SubmitError::Denied), Status::Denied),
        (Err(SubmitError::InvalidRequestId), Status::InvalidReply),
        (
            Ok(Reply::Failed(
                crate::session_services::ReplyError::WorkerUnavailable,
            )),
            Status::WorkerUnavailable,
        ),
    ] {
        let mut fake = FakeServices::default();
        let mut ui = started(&session, &mut fake);
        fake.replies.push_back(reply);
        ui.tick(Some(&session), &mut fake);
        assert_eq!(ui.status(), status);
        assert_eq!(ui.output(), "");
        assert!(!ui.is_busy());
    }
}

#[test]
fn invalid_input_and_excess_tokens_are_rejected() {
    let (session, _) = sessions();
    let mut fake = FakeServices::default();
    let mut ui = Controller::new();
    ui.sign_in(&session, &mut fake);
    assert!(!ui.set_input("contains\0nul"));
    assert!(!ui.push_text("\0"));
    assert_eq!(ui.status(), Status::InvalidInput);
    ui.set_input("hello");
    ui.request_submit();
    ui.tick(Some(&session), &mut fake);
    let Ok(Reply::Ready(mut value)) = response(1, "oversized token count") else {
        unreachable!()
    };
    value.usage.output_tokens = 257;
    fake.replies.push_back(Ok(Reply::Ready(value)));
    ui.tick(Some(&session), &mut fake);
    assert_eq!(ui.output(), "");
    assert_eq!(ui.status(), Status::InvalidReply);
}

#[test]
fn real_adapter_configuration_has_no_host_worker_or_implicit_consent() {
    use crate::bar_adapter::{admission_budget, Adapter};
    assert_eq!(admission_budget().available_ram_bytes, 3072 * 1024 * 1024);
    let (session, _) = sessions();
    let mut missing = Adapter::new(0);
    assert_eq!(
        missing.on_signed_in(&session),
        Err(SubmitError::Unavailable)
    );
    let mut real = Adapter::new(42);
    let reference = String::from(real.terms().unwrap().1);
    assert_eq!(
        real.on_signed_in(&session),
        Err(SubmitError::WorkerUnavailable)
    );
    assert_eq!(
        real.acknowledge_model_terms(&session, &reference),
        Err(SubmitError::Denied)
    );
}

#[test]
fn panel_open_and_typing_do_not_accept_terms_or_submit_implicitly() {
    let (session, _) = sessions();
    let mut fake = FakeServices::default();
    let mut panel = crate::bar_panel::Panel::new();
    panel.controller.sign_in(&session, &mut fake);
    panel.controller.offer_terms("Model", "pinned");
    panel.toggle();
    let key = |code| libnagi::InputEvent {
        event_type: libnagi::INPUT_EVENT_KEY,
        code,
        value: 1,
    };
    panel.event(key(35), 0, 0); // h
    panel.event(key(libnagi::INPUT_KEY_SPACE), 0, 0);
    panel.event(key(libnagi::INPUT_KEY_ENTER), 0, 0);
    assert_eq!(panel.controller.input(), "h ");
    assert_eq!(panel.controller.status(), Status::TermsRequired);
    panel.controller.tick(Some(&session), &mut fake);
    assert_eq!(fake.events, vec![Event::SignedIn(session)]);
    for _ in 0..3 {
        panel.event(key(libnagi::INPUT_KEY_TAB), 0, 0);
    }
    panel.event(key(libnagi::INPUT_KEY_ENTER), 0, 0);
    panel.controller.tick(Some(&session), &mut fake);
    assert_eq!(
        fake.events.last(),
        Some(&Event::Acknowledge(session, "pinned".into()))
    );
    assert!(panel.controller.terms().unwrap().acknowledged());
    assert!(!fake.events.iter().any(|e| matches!(e, Event::Submit(..))));
}

#[test]
fn every_bar_status_and_control_has_both_localizations() {
    use nagi_localization::{text, Locale};
    for status in [
        Status::SignedOut,
        Status::Ready,
        Status::Queued,
        Status::Waiting,
        Status::Complete,
        Status::Cancelled,
        Status::Unavailable,
        Status::Busy,
        Status::TermsRequired,
        Status::Failed,
        Status::InvalidReply,
        Status::InputFull,
        Status::EmptyInput,
        Status::InvalidInput,
        Status::TermsOffered,
        Status::TermsPending,
        Status::TermsAccepted,
        Status::TermsFailed,
        Status::Denied,
        Status::WorkerUnavailable,
    ] {
        for locale in [Locale::EnUs, Locale::JaJp] {
            assert_ne!(text(locale, status.key()), "Text unavailable.");
        }
    }
    for key in [
        "desktop.ai.title",
        "desktop.lock",
        "desktop.ai.send",
        "desktop.ai.cancel",
        "desktop.ai.accept_session",
    ] {
        for locale in [Locale::EnUs, Locale::JaJp] {
            assert_ne!(text(locale, key), "Text unavailable.");
        }
    }
}

#[test]
fn cancellation_drains_pending_slot_before_retry_and_keeps_late_text_hidden() {
    let (session, _) = sessions();
    let mut fake = FakeServices::default();
    let mut ui = started(&session, &mut fake);
    ui.cancel(&mut fake);
    fake.replies.push_back(Ok(Reply::Pending));
    ui.tick(Some(&session), &mut fake);
    assert!(ui.is_busy());
    assert!(!ui.request_submit());
    assert_eq!(ui.output(), "");
    fake.replies
        .push_back(response(1, "late after cancellation"));
    ui.tick(Some(&session), &mut fake);
    assert!(!ui.is_busy());
    assert_eq!(ui.output(), "");
    assert_eq!(ui.status(), Status::Cancelled);
    assert!(ui.request_submit());
    assert_eq!(ui.request_id(), Some(2));
}

#[test]
fn relock_and_relogin_with_same_token_cannot_display_an_old_request() {
    let (session, _) = sessions();
    let mut fake = FakeServices::default();
    let mut ui = started(&session, &mut fake);
    ui.lock_or_signout(&mut fake);
    ui.sign_in(&session, &mut fake);
    ui.set_input("new generation");
    ui.request_submit();
    ui.tick(Some(&session), &mut fake);
    assert_eq!(ui.request_id(), Some(2));
    fake.replies.push_back(response(1, "previous login text"));
    ui.tick(Some(&session), &mut fake);
    assert_eq!(ui.status(), Status::InvalidReply);
    assert_eq!(ui.output(), "");
}

#[test]
fn worker_license_gate_requires_explicit_reacceptance_without_automatic_retry() {
    let (session, _) = sessions();
    let mut fake = FakeServices::default();
    let mut ui = Controller::new();
    ui.sign_in(&session, &mut fake);
    ui.offer_terms("Model", "pinned");
    ui.accept_request();
    ui.tick(Some(&session), &mut fake);
    ui.set_input("hello");
    ui.request_submit();
    ui.tick(Some(&session), &mut fake);
    fake.replies.push_back(Ok(Reply::Failed(
        crate::session_services::ReplyError::Model(nagi_model_manager::ModelServiceError::Store(
            nagi_model_manager::StoreError::LicenseAcknowledgementRequired,
        )),
    )));
    ui.tick(Some(&session), &mut fake);
    assert_eq!(ui.status(), Status::TermsRequired);
    assert!(!ui.terms().unwrap().acknowledged());
    assert!(!ui.request_submit());
    assert_eq!(ui.output(), "");
    let count = fake.events.len();
    ui.tick(Some(&session), &mut fake);
    assert_eq!(fake.events.len(), count);
}

#[test]
fn failed_cancel_still_hides_late_text_and_preserves_the_occupied_slot() {
    for error in [SubmitError::Denied, SubmitError::WorkerUnavailable] {
        let (session, _) = sessions();
        let mut fake = FakeServices::default();
        fake.cancellations.push_back(Err(error));
        let mut ui = started(&session, &mut fake);
        ui.cancel(&mut fake);
        fake.replies.push_back(Ok(Reply::Pending));
        ui.tick(Some(&session), &mut fake);
        assert!(ui.is_busy());
        assert!(!ui.request_submit());
        assert_eq!(ui.output(), "");
        fake.replies
            .push_back(response(1, "late despite failed cancellation"));
        ui.tick(Some(&session), &mut fake);
        assert_eq!(ui.status(), Status::Cancelled);
        assert_eq!(ui.output(), "");
        assert!(!ui.is_busy());
        assert!(ui.request_submit());
        assert_eq!(ui.request_id(), Some(2));
    }
}

#[test]
fn new_login_token_for_same_account_clears_completed_text_and_accepted_terms() {
    let mut accounts = AccountStore::new();
    accounts
        .add_account(b"owner", Role::Owner, b"password")
        .unwrap();
    let first = accounts.authenticate(b"owner", b"password").unwrap();
    let second = accounts.authenticate(b"owner", b"password").unwrap();
    assert_ne!(first.token(), second.token());
    assert_eq!(first.role(), second.role());
    let mut fake = FakeServices::default();
    let mut ui = Controller::new();
    ui.sign_in(&first, &mut fake);
    ui.offer_terms("Model", "pinned-first-session");
    ui.accept_request();
    ui.tick(Some(&first), &mut fake);
    ui.set_input("private first login input");
    ui.request_submit();
    ui.tick(Some(&first), &mut fake);
    fake.replies
        .push_back(response(1, "private first login reply"));
    ui.tick(Some(&first), &mut fake);
    assert_eq!(ui.output(), "private first login reply");
    assert!(ui.terms().unwrap().acknowledged());
    let before = fake.events.len();
    ui.tick(Some(&second), &mut fake);
    assert_eq!(
        &fake.events[before..],
        &[Event::Invalidated(first), Event::SignedIn(second)]
    );
    assert_eq!(ui.input(), "");
    assert_eq!(ui.output(), "");
    assert!(ui.terms().is_none());
    assert!(!ui.is_busy());
    ui.offer_terms("Model", "pinned-second-session");
    ui.set_input("new login input");
    assert!(!ui.request_submit());
    assert_eq!(ui.status(), Status::TermsRequired);
    ui.accept_request();
    ui.tick(Some(&second), &mut fake);
    assert!(ui.request_submit());
    assert_eq!(ui.request_id(), Some(2));
}

#[test]
fn bar_catalogs_remain_key_value_rows_after_windows_checkout() {
    for source in [
        include_str!("../../../user/nagi-localization/locales/en-US.lang"),
        include_str!("../../../user/nagi-localization/locales/ja-JP.lang"),
    ] {
        let windows = source.replace("\r\n", "\n").replace('\n', "\r\n");
        for (row, line) in windows.split('\n').enumerate() {
            if !line.is_empty() {
                assert!(
                    line.contains('='),
                    "Windows catalog row {} is not key/value",
                    row + 1
                );
            }
        }
    }
}
