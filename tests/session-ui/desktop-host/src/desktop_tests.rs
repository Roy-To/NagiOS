//! Exercises the exact production Desktop, LoginScreen, credentials, VFS and
//! font renderer. Only syscall transport/readiness/time/console are host seams.
use super::*;
use libnagi::credential::{AccountRecord, Credential, MIN_ITERATIONS, SALT_BYTES};
use nagi_localization::Locale;

fn key(code: u16) -> InputEvent {
    InputEvent {
        event_type: libnagi::INPUT_EVENT_KEY,
        code,
        value: 1,
    }
}
fn existing_account(locale: Locale) -> (Desktop, UserDataVolume, Vec<u32>, u64) {
    let credential = Credential::derive(b"nagi1", [7; SALT_BYTES], MIN_ITERATIONS).unwrap();
    let record = AccountRecord::new(b"owner", credential).unwrap();
    let (mut volume, _) = Vfs::mount_or_format(SyscallBlockDevice::new(42)).unwrap();
    let account = volume.create_path(b"owner-account").unwrap();
    volume.write(account, &record.encode()).unwrap();
    volume.flush().unwrap();
    let mut desktop = Desktop::new(locale);
    desktop.login = Some(crate::login_screen::LoginScreen::new(
        crate::login_screen::load(&mut volume),
    ));
    let mut surface = vec![0; libnagi::SURFACE_BYTES / core::mem::size_of::<u32>()];
    desktop.render(&mut surface);
    let initial = frame_hash(&surface);
    (desktop, volume, surface, initial)
}
fn unlock(desktop: &mut Desktop, volume: &mut UserDataVolume) {
    for code in [49, 30, 34, 23, 2, libnagi::INPUT_KEY_ENTER] {
        assert!(desktop.handle_event(key(code), volume));
    }
    assert!(desktop.login.is_none());
    assert!(desktop.session.is_some_and(|session| !session.is_locked()));
}

#[test]
#[cfg(feature = "m20-model-service")]
fn existing_account_lock_relogin_reuses_initial_frame_without_production_failure() {
    for locale in [Locale::EnUs, Locale::JaJp] {
        libnagi::set_readiness(true);
        let (mut desktop, mut volume, mut surface, initial) = existing_account(locale);
        unlock(&mut desktop, &mut volume);
        assert!(desktop.model_services_ready);
        assert_eq!(libnagi::readiness_calls(), 1);
        desktop.render(&mut surface);
        assert_ne!(frame_hash(&surface), initial);
        // A moved pointer must not conceal the locked-form collision: the
        // actual login renderer returns before drawing the Desktop pointer.
        desktop.pointer_x = 218;
        desktop.pointer_y = 175;
        assert!(desktop.handle_event(key(62), &mut volume)); // Actual F4 event.
        assert!(desktop.session.is_none());
        assert!(!desktop.model_services_ready);
        assert!(desktop.login.is_some());
        assert_eq!(desktop.bar.controller.output(), "");
        desktop.render(&mut surface);
        assert_eq!(
            frame_hash(&surface),
            initial,
            "reproduces the pre-fix collision"
        );
        assert!(
            !unchanged_frame_fails_acceptance(frame_hash(&surface), initial),
            "production must allow its initial lock screen"
        );
        assert!(!desktop.acceptance_ready());
        unlock(&mut desktop, &mut volume);
        assert!(desktop.model_services_ready);
        assert_eq!(libnagi::readiness_calls(), 2);
        desktop.render(&mut surface);
        assert_ne!(frame_hash(&surface), initial);
    }
}

#[test]
#[cfg(feature = "m20-model-service")]
fn relogin_failed_readiness_does_not_enable_services() {
    libnagi::set_readiness(true);
    let (mut desktop, mut volume, mut surface, initial) = existing_account(Locale::EnUs);
    desktop.model_services = Some(bar_adapter::Adapter::new(42));
    unlock(&mut desktop, &mut volume);
    assert!(desktop.handle_event(key(62), &mut volume));
    desktop.render(&mut surface);
    assert_eq!(frame_hash(&surface), initial);
    libnagi::set_readiness(false);
    unlock(&mut desktop, &mut volume);
    assert!(!desktop.model_services_ready);
    assert_eq!(libnagi::readiness_calls(), 1);
    desktop.bar.toggle();
    desktop.render(&mut surface);
    let before_readiness_step = frame_hash(&surface);
    assert!(desktop.model_service_step(|| {}));
    desktop.render(&mut surface);
    assert_ne!(frame_hash(&surface), before_readiness_step);
    assert_eq!(
        desktop.bar.controller.status(),
        session_ui::Status::Unavailable
    );
    assert!(!unchanged_frame_fails_acceptance(initial, initial));
}

#[test]
fn fixture_assertion_remains_active_for_acceptance_configuration() {
    let (_, _, surface, initial) = existing_account(Locale::EnUs);
    assert_eq!(frame_hash(&surface), initial);
    assert_eq!(
        unchanged_frame_fails_acceptance(initial, initial),
        !cfg!(feature = "production-session")
    );
    assert!(!unchanged_frame_fails_acceptance(
        initial.wrapping_add(1),
        initial
    ));
}

#[test]
#[cfg(all(
    feature = "desktop-login-acceptance",
    not(feature = "production-session")
))]
fn login_acceptance_still_completes_after_successful_signin() {
    libnagi::set_readiness(true);
    let (mut desktop, mut volume, mut surface, initial) = existing_account(Locale::EnUs);
    assert!(!desktop.acceptance_ready());
    unlock(&mut desktop, &mut volume);
    assert_eq!(libnagi::readiness_calls(), 1);
    assert!(desktop.acceptance_ready());
    desktop.render(&mut surface);
    assert!(!unchanged_frame_fails_acceptance(
        frame_hash(&surface),
        initial
    ));
}

// HOST presentation regression only: seed controller state with a scripted
// transport; the real Shared/service lock path is exercised separately below.
// This transport supplies no inference and never replaces production services.
#[cfg(feature = "m20-model-service")]
struct PresentationTransport {
    reply: Option<nagi_model_manager::ModelResponse>,
}
#[cfg(feature = "m20-model-service")]
impl session_ui::Services for PresentationTransport {
    fn on_signed_in(
        &mut self,
        _: &libnagi::security::Session,
    ) -> Result<(), session_ui::SubmitError> {
        Ok(())
    }
    fn submit_text(
        &mut self,
        _: &libnagi::security::Session,
        _: u64,
        _: &str,
    ) -> Result<(), session_ui::SubmitError> {
        Ok(())
    }
    fn poll_reply(
        &mut self,
        _: &libnagi::security::Session,
        _: u64,
    ) -> Result<session_ui::Reply, session_ui::SubmitError> {
        Ok(self
            .reply
            .take()
            .map_or(session_ui::Reply::Pending, session_ui::Reply::Ready))
    }
    fn cancel_request(
        &mut self,
        _: &libnagi::security::Session,
        _: u64,
    ) -> Result<(), session_ui::SubmitError> {
        Ok(())
    }
    fn on_lock_or_signout(&mut self, _: &libnagi::security::Session) {}
    fn acknowledge_model_terms(
        &mut self,
        _: &libnagi::security::Session,
        _: &str,
    ) -> Result<(), session_ui::SubmitError> {
        Ok(())
    }
}

#[test]
#[cfg(feature = "m20-model-service")]
fn scripted_presentation_state_is_cleared_by_desktop_lock_and_relogin_render() {
    use nagi_model_manager::{BackendId, ModelId, ModelResponse, ProviderId, TokenUsage};
    for locale in [Locale::EnUs, Locale::JaJp] {
        for stage in 0..3 {
            // queued submit, in-flight submit, displayed completion
            libnagi::set_readiness(true);
            let (mut desktop, mut volume, mut surface, initial) = existing_account(locale);
            unlock(&mut desktop, &mut volume);
            let session = desktop.session.unwrap();
            desktop.model_services = Some(bar_adapter::Adapter::new(42));
            let mut transport = PresentationTransport {
                reply: Some(ModelResponse {
                    request_id: 1,
                    model_id: ModelId::new("host-ui-model").unwrap(),
                    provider_id: ProviderId::new("host-ui-provider").unwrap(),
                    backend_id: BackendId::new("host-ui-backend").unwrap(),
                    text: "HOST ONLY private presentation".into(),
                    usage: TokenUsage {
                        input_tokens: 1,
                        output_tokens: 1,
                    },
                }),
            };
            let ui = &mut desktop.bar.controller;
            ui.sign_in(&session, &mut transport);
            ui.offer_terms("Host UI", "host-ui-test-terms");
            assert!(ui.accept_request());
            ui.tick(Some(&session), &mut transport);
            assert!(ui.terms().unwrap().acknowledged());
            assert!(ui.set_input("HOST ONLY private input"));
            assert!(ui.request_submit());
            if stage >= 1 {
                ui.tick(Some(&session), &mut transport);
            }
            if stage == 2 {
                ui.tick(Some(&session), &mut transport);
                assert_eq!(ui.output(), "HOST ONLY private presentation");
            } else {
                assert!(ui.is_busy());
            }
            assert!(desktop.handle_event(key(61), &mut volume)); // Actual F3.
            desktop.bar.cancel_requested = true;
            desktop.render(&mut surface);
            assert_ne!(frame_hash(&surface), initial);
            if stage == 2 {
                desktop.pointer_x = bar_panel::LOCK_BUTTON.x + 2;
                desktop.pointer_y = bar_panel::LOCK_BUTTON.y + 2;
                assert!(desktop.handle_event(key(libnagi::INPUT_KEY_LEFT), &mut volume));
            } else {
                assert!(desktop.handle_event(key(62), &mut volume));
            }
            assert!(desktop.session.is_none());
            assert!(!desktop.model_services_ready);
            assert!(!desktop.bar.open);
            assert!(!desktop.bar.cancel_requested);
            let ui = &desktop.bar.controller;
            assert_eq!(ui.session(), None);
            assert_eq!(ui.status(), session_ui::Status::SignedOut);
            assert_eq!(ui.input(), "");
            assert_eq!(ui.output(), "");
            assert!(ui.terms().is_none());
            assert!(!ui.is_busy());
            desktop.render(&mut surface);
            assert_eq!(
                frame_hash(&surface),
                initial,
                "full unlock frame contains no stale pixels"
            );
            unlock(&mut desktop, &mut volume);
            assert!(desktop.handle_event(key(61), &mut volume));
            assert_eq!(desktop.bar.controller.input(), "");
            assert_eq!(desktop.bar.controller.output(), "");
            assert!(desktop.bar.controller.terms().is_none());
            desktop.render(&mut surface);
            assert_ne!(frame_hash(&surface), initial);
            // Even the bootstrap login's reused token cannot reuse request IDs.
            let new_session = desktop.session.unwrap();
            desktop.bar.controller.sign_in(&new_session, &mut transport);
            assert!(desktop.bar.controller.set_input("fresh input"));
            assert!(desktop.bar.controller.request_submit());
            assert_eq!(desktop.bar.controller.request_id(), Some(2));
        }
    }
}

#[test]
#[cfg(all(feature = "m20-model-service", feature = "consent-dialog"))]
fn real_consent_modal_is_discarded_by_the_desktop_lock_handler() {
    use libnagi::launch::{AppManifest, LaunchPlacement, LaunchRegistry};
    use nagi_model::{AppSessionId, NodeId};

    libnagi::set_readiness(true);
    let (mut desktop, mut volume, mut surface, initial) = existing_account(Locale::EnUs);
    unlock(&mut desktop, &mut volume);

    let manifest =
        AppManifest::from_declaration(b"org.nagi.host-test", [&b"test.display"[..]]).unwrap();
    let app = manifest.app_id();
    let mut registry = LaunchRegistry::new();
    registry.register_manifest(manifest).unwrap();
    let placement = LaunchPlacement {
        app_session_id: AppSessionId(0x51),
        node_id: NodeId(0x52),
        workspace_id: None,
    };
    registry.record_launch(7, app, placement).unwrap();
    let request = registry
        .request_consent(app, placement.app_session_id, b"test.display")
        .unwrap();
    desktop.open_consent(crate::consent_dialog::ConsentDialog::new(request));
    desktop.arm_consent();
    assert!(desktop.consent.as_ref().unwrap().is_armed());
    desktop.render(&mut surface);
    assert_ne!(frame_hash(&surface), initial);

    // F4 must take precedence over the actual modal's event capture.
    assert!(desktop.handle_event(key(62), &mut volume));
    assert!(desktop.session.is_none());
    assert!(!desktop.model_services_ready);
    assert!(desktop.login.is_some());
    assert!(desktop.consent.is_none());
    assert!(desktop.take_consent_decision().is_none());
    desktop.render(&mut surface);
    assert_eq!(frame_hash(&surface), initial);
}

#[test]
#[cfg(feature = "m20-model-service")]
fn desktop_service_step_binds_acknowledges_submits_cancels_and_unbinds_real_shared() {
    use session_ui::{Services, Status, SubmitError};

    libnagi::set_readiness(true);
    let (mut desktop, mut volume, mut surface, initial) = existing_account(Locale::EnUs);
    desktop.model_services = Some(bar_adapter::Adapter::new_host_shared_session(42));
    unlock(&mut desktop, &mut volume);
    desktop.bar.toggle();
    let session = desktop.session.expect("signed-in session");
    let mut yields = 0;

    assert!(desktop.model_service_step(|| yields += 1));
    assert_eq!(yields, 1);
    assert_eq!(desktop.bar.controller.status(), Status::TermsOffered);
    assert!(!desktop.bar.controller.terms().unwrap().acknowledged());
    let bound = desktop
        .model_services
        .as_ref()
        .unwrap()
        .host_shared_snapshot()
        .unwrap();
    assert_eq!(bound.user_token, Some(session.token()));
    assert!(!bound.terms_confirmed);
    assert!(bound.active_request_id.is_none());
    assert!(!bound.queued);

    assert!(desktop.bar.controller.accept_request());
    assert!(desktop.model_service_step(|| yields += 1));
    assert_eq!(yields, 2);
    assert_eq!(desktop.bar.controller.status(), Status::TermsAccepted);
    assert!(desktop.bar.controller.terms().unwrap().acknowledged());
    let accepted = desktop
        .model_services
        .as_ref()
        .unwrap()
        .host_shared_snapshot()
        .unwrap();
    assert!(accepted.terms_confirmed);

    assert!(desktop.bar.controller.set_input("shared request"));
    assert!(desktop.bar.controller.request_submit());
    assert!(desktop.model_service_step(|| yields += 1));
    assert_eq!(yields, 3);
    assert_eq!(desktop.bar.controller.status(), Status::Waiting);
    assert_eq!(desktop.bar.controller.request_id(), Some(1));
    let submitted = desktop
        .model_services
        .as_ref()
        .unwrap()
        .host_shared_snapshot()
        .unwrap();
    assert_eq!(submitted.active_request_id, Some(1));
    assert!(submitted.queued);
    assert_eq!(
        Services::submit_text(
            desktop.model_services.as_mut().unwrap(),
            &session,
            2,
            "must stay single-slot",
        ),
        Err(SubmitError::Busy)
    );

    desktop.render(&mut surface);
    let waiting_frame = frame_hash(&surface);
    desktop.bar.cancel_requested = true;
    assert!(desktop.model_service_step(|| yields += 1));
    assert_eq!(yields, 4);
    assert!(!desktop.bar.cancel_requested);
    assert_eq!(desktop.bar.controller.status(), Status::Cancelling);
    let cancelling = desktop
        .model_services
        .as_ref()
        .unwrap()
        .host_shared_snapshot()
        .unwrap();
    assert_eq!(cancelling.active_request_id, Some(1));
    assert!(cancelling.cancellation_requested);
    desktop.render(&mut surface);
    assert_ne!(frame_hash(&surface), waiting_frame);

    let generation_before_lock = cancelling.generation;
    desktop.pointer_x = bar_panel::LOCK_BUTTON.x + 2;
    desktop.pointer_y = bar_panel::LOCK_BUTTON.y + 2;
    assert!(desktop.handle_event(key(libnagi::INPUT_KEY_LEFT), &mut volume));
    assert!(desktop.session.is_none());
    assert!(!desktop.bar.open);
    assert_eq!(desktop.bar.controller.status(), Status::SignedOut);
    let unbound = desktop
        .model_services
        .as_ref()
        .unwrap()
        .host_shared_snapshot()
        .unwrap();
    assert_eq!(unbound.user_token, None);
    assert!(!unbound.terms_confirmed);
    assert!(unbound.active_request_id.is_none());
    assert!(!unbound.queued);
    assert!(unbound.generation > generation_before_lock);
    assert_eq!(
        Services::submit_text(
            desktop.model_services.as_mut().unwrap(),
            &session,
            2,
            "after lock",
        ),
        Err(SubmitError::Denied)
    );
    assert_eq!(
        Services::acknowledge_model_terms(
            desktop.model_services.as_mut().unwrap(),
            &session,
            "stale terms",
        ),
        Err(SubmitError::Denied)
    );
    desktop.render(&mut surface);
    assert_eq!(frame_hash(&surface), initial);
}

#[test]
#[cfg(feature = "m20-model-service")]
fn closed_bar_keeps_draining_shared_request_and_reopens_with_reply() {
    use session_ui::Status;

    libnagi::set_readiness(true);
    let (mut desktop, mut volume, mut surface, _) = existing_account(Locale::EnUs);
    desktop.model_services = Some(bar_adapter::Adapter::new_host_shared_session(42));
    unlock(&mut desktop, &mut volume);

    assert!(desktop.handle_event(key(61), &mut volume));
    assert!(desktop.bar.open);
    assert!(desktop.model_service_step(|| {}));
    assert_eq!(desktop.bar.controller.status(), Status::TermsOffered);
    assert!(desktop.bar.controller.accept_request());
    assert!(desktop.model_service_step(|| {}));
    assert_eq!(desktop.bar.controller.status(), Status::TermsAccepted);

    assert!(desktop.bar.controller.set_input("hidden panel request"));
    assert!(desktop.bar.controller.request_submit());
    assert!(desktop.model_service_step(|| {}));
    assert_eq!(desktop.bar.controller.status(), Status::Waiting);
    let request_id = desktop.bar.controller.request_id().unwrap();
    let submitted = desktop
        .model_services
        .as_ref()
        .unwrap()
        .host_shared_snapshot()
        .unwrap();
    assert_eq!(submitted.active_request_id, Some(request_id));
    assert!(submitted.queued);

    assert!(desktop.handle_event(key(libnagi::INPUT_KEY_ESCAPE), &mut volume));
    assert!(!desktop.bar.open);
    assert_eq!(desktop.bar.controller.request_id(), Some(request_id));

    let adapter = desktop.model_services.as_ref().unwrap();
    assert!(adapter.host_test_complete_request());
    let completed_in_shared = adapter.host_shared_snapshot().unwrap();
    assert!(completed_in_shared.has_reply);
    assert_eq!(completed_in_shared.active_request_id, Some(request_id));

    // Desktop continues its production service step while the panel is hidden.
    assert!(!desktop.model_service_step(|| {}));
    assert_eq!(desktop.bar.controller.status(), Status::Complete);
    assert_eq!(
        desktop.bar.controller.output(),
        "scripted close/reopen reply"
    );
    let drained = desktop
        .model_services
        .as_ref()
        .unwrap()
        .host_shared_snapshot()
        .unwrap();
    assert_eq!(drained.last_request_id, request_id);
    assert_eq!(drained.active_request_id, None);
    assert!(!drained.queued);
    assert!(!drained.has_reply);

    desktop.render(&mut surface);
    let closed_frame = frame_hash(&surface);
    assert!(desktop.handle_event(key(61), &mut volume));
    assert!(desktop.bar.open);
    assert_eq!(desktop.bar.controller.status(), Status::Complete);
    assert_eq!(
        desktop.bar.controller.output(),
        "scripted close/reopen reply"
    );
    desktop.render(&mut surface);
    assert_ne!(frame_hash(&surface), closed_frame);
}

#[test]
#[cfg(feature = "m20-model-service")]
fn generated_control_characters_render_as_safe_glyphs() {
    let mut line = [0; bar_panel::WRAPPED_LINE_CAPACITY];
    assert_eq!(
        bar_panel::wrapped_line("head\n\u{001b}[2J\u{007f}日本語", 0, &mut line),
        "head??[2J?日本語"
    );
}

// Exact Files wait loop + actual Desktop/consent UI and Shared queue. The reply
// is deliberately scripted orchestration data; this does not run inference.
#[cfg(all(feature = "m20-model-service", feature = "consent-dialog"))]
fn pending_model_with_consent() -> (Desktop, UserDataVolume, Vec<u32>) {
    use libnagi::launch::{AppManifest, LaunchPlacement, LaunchRegistry};
    use nagi_model::{AppSessionId, NodeId};
    libnagi::set_readiness(true);
    let (mut desktop, mut volume, surface, _) = existing_account(Locale::EnUs);
    desktop.model_services = Some(bar_adapter::Adapter::new_host_shared_session(42));
    unlock(&mut desktop, &mut volume);
    desktop.bar.toggle();
    desktop.model_service_step(|| {});
    assert!(desktop.bar.controller.accept_request());
    desktop.model_service_step(|| {});
    assert!(desktop
        .bar
        .controller
        .set_input("pending during Files consent"));
    assert!(desktop.bar.controller.request_submit());
    desktop.model_service_step(|| {});
    assert_eq!(desktop.bar.controller.status(), session_ui::Status::Waiting);
    let manifest =
        AppManifest::from_declaration(b"org.nagi.host-modal", [b"test.display".as_slice()])
            .unwrap();
    let app = manifest.app_id();
    let mut registry = LaunchRegistry::new();
    registry.register_manifest(manifest).unwrap();
    let placement = LaunchPlacement {
        app_session_id: AppSessionId(0x601),
        node_id: NodeId(0x602),
        workspace_id: None,
    };
    registry.record_launch(7, app, placement).unwrap();
    let request = registry
        .request_consent(app, placement.app_session_id, b"test.display")
        .unwrap();
    // This registry is deliberately separate from Supervisor: answering it
    // fails closed with NotLive. No grant or signed launch is fabricated.
    desktop.open_consent(crate::consent_dialog::ConsentDialog::new(request));
    desktop.arm_consent();
    (desktop, volume, surface)
}

#[test]
#[cfg(all(feature = "m20-model-service", feature = "consent-dialog"))]
fn files_modal_drains_reply_and_yields_with_idle_or_continuous_input() {
    for idle in [true, false] {
        let (mut desktop, mut volume, mut surface) = pending_model_with_consent();
        assert!(desktop
            .model_services
            .as_ref()
            .unwrap()
            .host_test_complete_request());
        let user = desktop.signed_in_session().unwrap();
        let ignored = key(61); // F3 stays captured by the modal, never toggles Bar.
        let mut input = vec![if idle { None } else { Some(ignored) }; 3];
        input.push(Some(key(libnagi::INPUT_KEY_ESCAPE)));
        libnagi::script_modal(input, false);
        let answer = crate::files_consent_wait::run(&mut desktop, &mut surface, &mut volume, &user);
        assert_eq!(answer.unwrap().1, libnagi::launch::GrantCheck::NotLive);
        assert_eq!(libnagi::modal_counts().0, 4);
        assert_eq!(libnagi::modal_counts().1, 4);
        assert_eq!(
            desktop.bar.controller.status(),
            session_ui::Status::Complete
        );
        assert_eq!(
            desktop.bar.controller.output(),
            "scripted close/reopen reply"
        );
        assert!(desktop.bar.open);
        assert_eq!(desktop.signed_in_session(), Some(user));
        assert!(
            !desktop
                .model_services
                .as_ref()
                .unwrap()
                .host_shared_snapshot()
                .unwrap()
                .has_reply
        );
    }
}

#[test]
#[cfg(all(feature = "m20-model-service", feature = "consent-dialog"))]
fn files_modal_lock_cancels_pending_work_and_aborts_without_answer() {
    let (mut desktop, mut volume, mut surface) = pending_model_with_consent();
    let user = desktop.signed_in_session().unwrap();
    let before = desktop
        .model_services
        .as_ref()
        .unwrap()
        .host_shared_snapshot()
        .unwrap();
    libnagi::script_modal(vec![None, None, Some(key(62))], false);
    assert!(
        crate::files_consent_wait::run(&mut desktop, &mut surface, &mut volume, &user).is_none()
    );
    assert_eq!(libnagi::modal_counts().0, 3);
    let after = desktop
        .model_services
        .as_ref()
        .unwrap()
        .host_shared_snapshot()
        .unwrap();
    assert!(after.generation > before.generation);
    assert_eq!(after.user_token, None);
    assert!(!after.queued && !after.has_reply);
    assert!(desktop.session.is_none());
    assert!(desktop.consent.is_none());
    assert!(desktop.take_consent_decision().is_none());
    assert_eq!(desktop.bar.controller.output(), "");
}

#[test]
#[cfg(all(feature = "m20-model-service", feature = "consent-dialog"))]
fn files_modal_redraw_failure_aborts_before_reading_or_granting() {
    let (mut desktop, mut volume, mut surface) = pending_model_with_consent();
    assert!(desktop
        .model_services
        .as_ref()
        .unwrap()
        .host_test_complete_request());
    let user = desktop.signed_in_session().unwrap();
    libnagi::script_modal(vec![Some(key(62))], true);
    assert!(
        crate::files_consent_wait::run(&mut desktop, &mut surface, &mut volume, &user).is_none()
    );
    assert_eq!(libnagi::modal_counts(), (0, 0, 1));
    assert!(desktop.consent.is_some());
    assert!(desktop.take_consent_decision().is_none());
}

#[test]
#[cfg(all(feature = "m20-model-service", feature = "consent-dialog"))]
fn files_modal_without_adapter_still_yields_and_handles_lock() {
    let (mut desktop, mut volume, mut surface) = pending_model_with_consent();
    desktop.model_services = None;
    let user = desktop.signed_in_session().unwrap();
    libnagi::script_modal(vec![None, Some(key(62))], false);
    assert!(
        crate::files_consent_wait::run(&mut desktop, &mut surface, &mut volume, &user).is_none()
    );
    assert_eq!(libnagi::modal_counts().0, 2);
}

#[test]
#[cfg(all(feature = "m20-model-service", feature = "consent-dialog"))]
fn files_modal_processes_cancel_and_discards_completed_reply() {
    let (mut desktop, mut volume, mut surface) = pending_model_with_consent();
    desktop.bar.cancel_requested = true;
    assert!(desktop
        .model_services
        .as_ref()
        .unwrap()
        .host_test_complete_request());
    let user = desktop.signed_in_session().unwrap();
    libnagi::script_modal(vec![None, Some(key(libnagi::INPUT_KEY_ESCAPE))], false);
    let answer = crate::files_consent_wait::run(&mut desktop, &mut surface, &mut volume, &user);
    assert_eq!(answer.unwrap().1, libnagi::launch::GrantCheck::NotLive);
    assert_eq!(
        desktop.bar.controller.status(),
        session_ui::Status::Cancelled
    );
    assert_eq!(desktop.bar.controller.output(), "");
    assert!(!desktop.bar.cancel_requested);
    assert_eq!(libnagi::modal_counts().0, 2);
}

#[test]
#[cfg(feature = "consent-dialog")]
fn files_modal_rejects_lost_session_before_service_or_input() {
    libnagi::set_readiness(true);
    let (mut desktop, mut volume, mut surface, _) = existing_account(Locale::EnUs);
    unlock(&mut desktop, &mut volume);
    let user = desktop.signed_in_session().unwrap();
    desktop.session = None;
    libnagi::script_modal(vec![], false);
    assert!(
        crate::files_consent_wait::run(&mut desktop, &mut surface, &mut volume, &user).is_none()
    );
    assert_eq!(libnagi::modal_counts(), (0, 0, 0));
}

// Exact Desktop input/session/renderer + Shared queue. Only public Browser data
// and worker completion are synthetic; these cases do not prove guest inference
// or manufacture production caller provenance.
#[cfg(feature = "m20-model-service")]
struct SummaryBrowserFixture {
    epoch: nagi_ai::BrowserContextEpoch,
    _owner_thread: std::rc::Rc<()>,
    calls: usize,
    denied: bool,
}
#[cfg(feature = "m20-model-service")]
impl nagi_ai::PublicBrowserContextApi for SummaryBrowserFixture {
    fn current_page_context(
        &mut self,
        request: nagi_ai::BrowserContextRequest,
    ) -> Result<Option<nagi_ai::BrowserPageSnapshot>, nagi_ai::BrowserContextApiError> {
        self.calls += 1;
        assert_eq!(request.caller(), summary_actor_fixture());
        if self.denied {
            return Err(nagi_ai::BrowserContextApiError::Denied);
        }
        Ok(Some(
            nagi_ai::BrowserPageSnapshot::new(
                7,
                None,
                None,
                None,
                Some("untrusted desktop fixture page".into()),
            )
            .with_revision(self.epoch.revision()),
        ))
    }
}
#[cfg(feature = "m20-model-service")]
struct SummaryAuthorityFixture;
#[cfg(feature = "m20-model-service")]
impl nagi_ai::ContextAuthority for SummaryAuthorityFixture {
    fn can_read_object(&self, _: nagi_ai::CallerIdentity, _: nagi_model::ObjectId) -> bool {
        false
    }
}
#[cfg(feature = "m20-model-service")]
fn summary_actor_fixture() -> nagi_ai::CallerIdentity {
    nagi_ai::CallerIdentity {
        app_id: nagi_ai::NAGI_BAR_APP_ID,
        app_session_id: nagi_model::AppSessionId(8),
        node_id: nagi_model::NodeId(9),
        workspace_id: None,
    }
}
#[cfg(feature = "m20-model-service")]
fn summary_request_fixture() -> nagi_ai::ContextRequest {
    nagi_ai::ContextRequest {
        caller: summary_actor_fixture(),
        selected_object: None,
        candidate_objects: Vec::new(),
    }
}
#[cfg(feature = "m20-model-service")]
fn summary_browser_fixture() -> SummaryBrowserFixture {
    SummaryBrowserFixture {
        epoch: nagi_ai::BrowserContextEpoch::new(),
        _owner_thread: std::rc::Rc::new(()),
        calls: 0,
        denied: false,
    }
}
#[cfg(feature = "m20-model-service")]
fn summary_owner_step(
    desktop: &mut Desktop,
    user: &libnagi::security::Session,
    request: nagi_ai::ContextRequest,
    browser: &mut dyn nagi_ai::PublicBrowserContextApi,
) -> bool {
    let mut yields = 0;
    let binding = desktop
        .page_summary_binding()
        .expect("bound fixture Desktop");
    assert_eq!(binding.session, *user);
    let changed = desktop.model_service_step_with_page_context(
        Some(DesktopPageContext {
            binding,
            caller: request.caller,
            request,
            authority: &SummaryAuthorityFixture,
            browser,
        }),
        || yields += 1,
    );
    assert_eq!(yields, 1);
    changed
}
#[cfg(feature = "m20-model-service")]
fn summary_desktop_fixture(locale: Locale, consent: bool) -> (Desktop, UserDataVolume, Vec<u32>) {
    libnagi::set_readiness(true);
    let (mut desktop, mut volume, surface, _) = existing_account(locale);
    desktop.model_services = Some(bar_adapter::Adapter::new_host_shared_session(42));
    unlock(&mut desktop, &mut volume);
    assert!(desktop.handle_event(key(61), &mut volume)); // Ordinary F3 Bar input.
    desktop.model_service_step(|| {});
    assert_eq!(
        desktop.bar.controller.status(),
        session_ui::Status::TermsOffered
    );
    if consent {
        // Explicit synthetic user click on the existing terms control; never
        // an acknowledgement for an operator or real model artifact.
        desktop.pointer_x = 29;
        desktop.pointer_y = 155;
        assert!(desktop.handle_event(key(libnagi::INPUT_KEY_LEFT), &mut volume));
        desktop.model_service_step(|| {});
        assert_eq!(
            desktop.bar.controller.status(),
            session_ui::Status::TermsAccepted
        );
    }
    (desktop, volume, surface)
}

#[test]
#[cfg(feature = "m20-model-service")]
fn summary_input_without_live_owner_is_localized_unavailable_and_never_retries() {
    for locale in [Locale::EnUs, Locale::JaJp] {
        for mouse in [false, true] {
            let (mut desktop, mut volume, mut surface) = summary_desktop_fixture(locale, false);
            assert!(desktop.bar.controller.set_input("ordinary text stays text"));
            desktop.render(&mut surface);
            let before = frame_hash(&surface);
            if mouse {
                desktop.pointer_x = bar_panel::SUMMARY.x + 2;
                desktop.pointer_y = bar_panel::SUMMARY.y + 2;
                assert!(desktop.handle_event(key(libnagi::INPUT_KEY_LEFT), &mut volume));
            } else {
                assert!(desktop.handle_event(key(bar_panel::SUMMARY_KEY), &mut volume));
            }
            assert!(desktop.bar.summary_requested);
            assert!(desktop.model_service_step(|| {}));
            assert!(!desktop.bar.summary_requested);
            assert_eq!(
                desktop.bar.controller.status(),
                session_ui::Status::PageSummaryUnavailable
            );
            assert_eq!(desktop.bar.controller.input(), "ordinary text stays text");
            assert!(desktop.bar.controller.output().is_empty());
            assert!(!desktop.bar.controller.terms().unwrap().acknowledged());
            let state = desktop
                .model_services
                .as_ref()
                .unwrap()
                .host_shared_snapshot()
                .unwrap();
            assert!(!state.terms_confirmed);
            assert!(!state.queued);
            assert_eq!(state.last_request_id, 0);
            desktop.render(&mut surface);
            assert_ne!(frame_hash(&surface), before);
            let user = desktop.session.unwrap();
            let mut browser = summary_browser_fixture();
            summary_owner_step(&mut desktop, &user, summary_request_fixture(), &mut browser);
            assert_eq!(browser.calls, 0); // Owner arrival does not replay intent.
            assert!(
                !desktop
                    .model_services
                    .as_ref()
                    .unwrap()
                    .host_shared_snapshot()
                    .unwrap()
                    .queued
            );
        }
    }
}

#[test]
#[cfg(feature = "m20-model-service")]
fn summary_desktop_input_requires_explicit_terms_before_any_context_read() {
    let (mut desktop, mut volume, _) = summary_desktop_fixture(Locale::EnUs, false);
    let user = desktop.session.unwrap();
    let mut browser = summary_browser_fixture();
    assert!(desktop.handle_event(key(bar_panel::SUMMARY_KEY), &mut volume));
    summary_owner_step(&mut desktop, &user, summary_request_fixture(), &mut browser);
    assert_eq!(
        desktop.bar.controller.status(),
        session_ui::Status::TermsRequired
    );
    assert_eq!(browser.calls, 0);
    assert!(
        !desktop
            .model_services
            .as_ref()
            .unwrap()
            .host_shared_snapshot()
            .unwrap()
            .terms_confirmed
    );
}

#[test]
#[cfg(feature = "m20-model-service")]
fn summary_desktop_owner_step_queues_polls_and_renders_only_current_scripted_reply() {
    let (mut desktop, mut volume, mut surface) = summary_desktop_fixture(Locale::EnUs, true);
    let user = desktop.session.unwrap();
    let mut browser = summary_browser_fixture();
    assert!(desktop.handle_event(key(bar_panel::SUMMARY_KEY), &mut volume));
    summary_owner_step(&mut desktop, &user, summary_request_fixture(), &mut browser);
    assert_eq!(desktop.bar.controller.status(), session_ui::Status::Waiting);
    assert_eq!(browser.calls, 1);
    let state = desktop
        .model_services
        .as_ref()
        .unwrap()
        .host_shared_snapshot()
        .unwrap();
    assert!(state.queued);
    assert_eq!(state.active_request_id, Some(1));
    assert!(desktop.handle_event(key(bar_panel::SUMMARY_KEY), &mut volume));
    assert!(!desktop.bar.summary_requested); // Busy input cannot queue a retry.
    desktop.render(&mut surface);
    let waiting = frame_hash(&surface);
    assert!(desktop
        .model_services
        .as_ref()
        .unwrap()
        .host_test_complete_request());
    summary_owner_step(&mut desktop, &user, summary_request_fixture(), &mut browser);
    assert_eq!(
        desktop.bar.controller.status(),
        session_ui::Status::Complete
    );
    assert_eq!(
        desktop.bar.controller.output(),
        "scripted close/reopen reply"
    );
    assert_eq!(browser.calls, 2);
    desktop.render(&mut surface);
    assert_ne!(frame_hash(&surface), waiting);
}

#[test]
#[cfg(feature = "m20-model-service")]
fn summary_desktop_discards_completed_reply_after_page_aba_with_identical_text() {
    let (mut desktop, mut volume, mut surface) = summary_desktop_fixture(Locale::EnUs, true);
    let user = desktop.session.unwrap();
    let mut browser = summary_browser_fixture();
    assert!(desktop.handle_event(key(bar_panel::SUMMARY_KEY), &mut volume));
    summary_owner_step(&mut desktop, &user, summary_request_fixture(), &mut browser);
    assert_eq!(desktop.bar.controller.status(), session_ui::Status::Waiting);
    assert!(desktop
        .model_services
        .as_ref()
        .unwrap()
        .host_test_complete_request());
    browser.epoch.invalidate(); // Navigation A -> B on the owner event thread.
    browser.epoch.invalidate(); // B -> A; same URL/tab/title/body at next read.
    summary_owner_step(&mut desktop, &user, summary_request_fixture(), &mut browser);
    assert_eq!(desktop.bar.controller.status(), session_ui::Status::Denied);
    assert_eq!(desktop.bar.controller.output(), "");
    desktop.render(&mut surface);
    assert!(desktop
        .model_services
        .as_ref()
        .unwrap()
        .host_shared_snapshot()
        .unwrap()
        .active_request_id
        .is_none());
}

#[test]
#[cfg(feature = "m20-model-service")]
fn summary_owner_loss_drains_only_cancelled_failure_without_browser() {
    for complete in [false, true] {
        let (mut desktop, mut volume, _) = summary_desktop_fixture(Locale::EnUs, true);
        let user = desktop.session.unwrap();
        let mut browser = summary_browser_fixture();
        desktop.handle_event(key(bar_panel::SUMMARY_KEY), &mut volume);
        summary_owner_step(&mut desktop, &user, summary_request_fixture(), &mut browser);
        if complete {
            assert!(desktop
                .model_services
                .as_ref()
                .unwrap()
                .host_test_complete_request());
        }
        desktop.model_service_step(|| {}); // Real ordinary runtime has no owner.
        assert!(desktop.bar.controller.output().is_empty());
        if !complete {
            assert_eq!(
                desktop.bar.controller.status(),
                session_ui::Status::Cancelling
            );
            assert!(desktop
                .model_services
                .as_ref()
                .unwrap()
                .host_test_complete_request());
            desktop.model_service_step(|| {});
        }
        assert_eq!(
            desktop.bar.controller.status(),
            session_ui::Status::Cancelled
        );
        assert!(!desktop.bar.controller.is_busy());
        assert_eq!(browser.calls, 1);
        assert!(desktop
            .model_services
            .as_ref()
            .unwrap()
            .host_shared_snapshot()
            .unwrap()
            .active_request_id
            .is_none());
    }
}

#[test]
#[cfg(feature = "m20-model-service")]
fn changed_summary_actor_or_selection_discards_completed_reply_before_context_read() {
    for actor_changed in [false, true] {
        let (mut desktop, mut volume, _) = summary_desktop_fixture(Locale::EnUs, true);
        let user = desktop.session.unwrap();
        let mut browser = summary_browser_fixture();
        desktop.handle_event(key(bar_panel::SUMMARY_KEY), &mut volume);
        summary_owner_step(&mut desktop, &user, summary_request_fixture(), &mut browser);
        assert!(desktop
            .model_services
            .as_ref()
            .unwrap()
            .host_test_complete_request());
        let mut changed = summary_request_fixture();
        if actor_changed {
            changed.caller.node_id = nagi_model::NodeId(88);
        } else {
            changed.candidate_objects.push(nagi_model::ObjectId(99));
        }
        summary_owner_step(&mut desktop, &user, changed, &mut browser);
        assert!(desktop.bar.controller.output().is_empty());
        assert_eq!(browser.calls, 1);
        assert_eq!(
            desktop.bar.controller.status(),
            if actor_changed {
                session_ui::Status::Cancelled
            } else {
                session_ui::Status::Denied
            }
        );
    }
}

#[test]
#[cfg(feature = "m20-model-service")]
fn locked_summary_intent_is_erased_before_reauthentication() {
    let (mut desktop, mut volume, _) = summary_desktop_fixture(Locale::EnUs, true);
    desktop.handle_event(key(bar_panel::SUMMARY_KEY), &mut volume);
    assert!(desktop.bar.summary_requested);
    desktop.handle_event(key(62), &mut volume); // Ordinary F4 lock input.
    assert!(!desktop.bar.summary_requested);
    assert!(desktop.session.is_none());
    unlock(&mut desktop, &mut volume);
    desktop.handle_event(key(61), &mut volume);
    desktop.model_service_step(|| {});
    assert_eq!(
        desktop.bar.controller.status(),
        session_ui::Status::TermsOffered
    );
    let state = desktop
        .model_services
        .as_ref()
        .unwrap()
        .host_shared_snapshot()
        .unwrap();
    assert!(!state.queued);
    assert!(!state.terms_confirmed);
    assert!(state.active_request_id.is_none());
}

#[test]
#[cfg(feature = "m20-model-service")]
fn summary_cancel_input_wins_over_same_step_start_and_never_reads_context() {
    let (mut desktop, mut volume, _) = summary_desktop_fixture(Locale::EnUs, true);
    let user = desktop.session.unwrap();
    let mut browser = summary_browser_fixture();
    desktop.handle_event(key(bar_panel::SUMMARY_KEY), &mut volume);
    desktop.pointer_x = 110;
    desktop.pointer_y = 70; // Ordinary Cancel button.
    desktop.handle_event(key(libnagi::INPUT_KEY_LEFT), &mut volume);
    summary_owner_step(&mut desktop, &user, summary_request_fixture(), &mut browser);
    assert_eq!(browser.calls, 0);
    assert!(!desktop.bar.summary_requested);
    assert!(
        !desktop
            .model_services
            .as_ref()
            .unwrap()
            .host_shared_snapshot()
            .unwrap()
            .queued
    );
}

#[test]
#[cfg(feature = "m20-model-service")]
fn desktop_step_cooperates_even_without_a_model_adapter() {
    let mut desktop = Desktop::new(Locale::EnUs);
    let mut yields = 0;
    assert!(desktop.model_services.is_none());
    desktop.model_service_step(|| yields += 1);
    assert_eq!(yields, 1);
}

#[test]
#[cfg(feature = "m20-model-service")]
fn summary_binding_from_before_lock_cannot_start_after_reauthentication() {
    let (mut desktop, mut volume, _) = summary_desktop_fixture(Locale::EnUs, true);
    let old_binding = desktop.page_summary_binding().unwrap();
    desktop.handle_event(key(62), &mut volume);
    assert!(desktop.page_summary_binding().is_none());
    unlock(&mut desktop, &mut volume);
    desktop.handle_event(key(61), &mut volume);
    desktop.model_service_step(|| {});
    let current = desktop.page_summary_binding().unwrap();
    assert_ne!(old_binding, current); // independent of possible Session-value reuse
    desktop.handle_event(key(bar_panel::SUMMARY_KEY), &mut volume);
    let mut browser = summary_browser_fixture();
    desktop.model_service_step_with_page_context(
        Some(DesktopPageContext {
            binding: old_binding,
            caller: summary_actor_fixture(),
            request: summary_request_fixture(),
            authority: &SummaryAuthorityFixture,
            browser: &mut browser,
        }),
        || {},
    );
    assert_eq!(desktop.bar.controller.status(), session_ui::Status::Denied);
    assert_eq!(browser.calls, 0);
    assert!(
        !desktop
            .model_services
            .as_ref()
            .unwrap()
            .host_shared_snapshot()
            .unwrap()
            .queued
    );
}

#[test]
#[cfg(feature = "m20-model-service")]
fn failed_readiness_consumes_summary_intent_and_revokes_owner_binding() {
    let (mut desktop, mut volume, _) = summary_desktop_fixture(Locale::EnUs, true);
    let old_binding = desktop.page_summary_binding().unwrap();
    desktop.handle_event(key(bar_panel::SUMMARY_KEY), &mut volume);
    desktop.model_services_ready = false; // existing host readiness seam, no guest claim
    let mut browser = summary_browser_fixture();
    desktop.model_service_step_with_page_context(
        Some(DesktopPageContext {
            binding: old_binding.clone(),
            caller: summary_actor_fixture(),
            request: summary_request_fixture(),
            authority: &SummaryAuthorityFixture,
            browser: &mut browser,
        }),
        || {},
    );
    assert_eq!(browser.calls, 0);
    assert!(!desktop.bar.summary_requested);
    assert_eq!(
        desktop.bar.controller.status(),
        session_ui::Status::Unavailable
    );
    assert!(desktop.page_summary_binding().is_none());
    desktop.model_services_ready = true;
    desktop.model_service_step(|| {});
    assert_ne!(desktop.page_summary_binding().unwrap(), old_binding);
    let state = desktop
        .model_services
        .as_ref()
        .unwrap()
        .host_shared_snapshot()
        .unwrap();
    assert!(!state.terms_confirmed);
    assert!(!state.queued);
}

#[test]
#[cfg(feature = "m20-model-service")]
fn owner_binding_counter_exhaustion_fails_closed_across_lock_and_rebind() {
    let (mut desktop, mut volume, _) = summary_desktop_fixture(Locale::EnUs, true);
    desktop.summary_binding_generation = u64::MAX;
    assert!(desktop.page_summary_binding().is_some());
    desktop.handle_event(key(62), &mut volume);
    assert_eq!(desktop.summary_binding_generation, 0);
    unlock(&mut desktop, &mut volume);
    desktop.model_service_step(|| {});
    assert_eq!(desktop.summary_binding_generation, 0);
    assert!(desktop.page_summary_binding().is_none());
}

#[test]
#[cfg(feature = "m20-model-service")]
fn binding_from_another_desktop_instance_cannot_authorize_context_even_if_values_repeat() {
    let (first, _, _) = summary_desktop_fixture(Locale::EnUs, true);
    let first_binding = first.page_summary_binding().unwrap();
    let (mut second, mut volume, _) = summary_desktop_fixture(Locale::EnUs, true);
    let second_binding = second.page_summary_binding().unwrap();
    assert_eq!(first_binding.session, second_binding.session); // deterministic host login fixture
    assert_eq!(first_binding.generation, second_binding.generation);
    assert_ne!(first_binding, second_binding); // private allocation identity still differs
    second.handle_event(key(bar_panel::SUMMARY_KEY), &mut volume);
    let mut browser = summary_browser_fixture();
    second.model_service_step_with_page_context(
        Some(DesktopPageContext {
            binding: first_binding,
            caller: summary_actor_fixture(),
            request: summary_request_fixture(),
            authority: &SummaryAuthorityFixture,
            browser: &mut browser,
        }),
        || {},
    );
    assert_eq!(second.bar.controller.status(), session_ui::Status::Denied);
    assert_eq!(browser.calls, 0);
}

// Exact Albert transaction owner + exact Desktop/queue. The engine output and
// requesting actor are synthetic, never kernel-sender/Servo/guest evidence.
#[cfg(feature = "m20-model-service")]
struct AlbertTransactionFixture {
    owner: crate::albert_assistant_context::BrowserContextOwner,
    mutate_during_read: bool,
    calls: usize,
}
#[cfg(feature = "m20-model-service")]
impl nagi_ai::PublicBrowserContextApi for AlbertTransactionFixture {
    fn current_page_context(
        &mut self,
        request: nagi_ai::BrowserContextRequest,
    ) -> Result<Option<nagi_ai::BrowserPageSnapshot>, nagi_ai::BrowserContextApiError> {
        assert_eq!(request.caller(), summary_actor_fixture());
        let mutate = self.mutate_during_read;
        let calls = &mut self.calls;
        self.owner.extract(request, |owner, request| {
            *calls += 1;
            if mutate {
                owner.invalidate_page_context();
                owner.invalidate_page_context();
            }
            Ok(Some(crate::albert_assistant_context::page_snapshot(
                7,
                Some("https://example.net/".into()),
                Some("page".into()),
                None,
                Some("same page fixture".into()),
                request.includes_selected_text(),
                request.includes_visible_text(),
            )))
        })
    }
}
#[test]
#[cfg(feature = "m20-model-service")]
fn albert_transaction_owner_discards_desktop_reply_on_policy_or_extraction_aba() {
    for during_read in [false, true] {
        let (mut desktop, mut volume, mut surface) = summary_desktop_fixture(Locale::EnUs, true);
        let user = desktop.session.unwrap();
        let mut owner = crate::albert_assistant_context::BrowserContextOwner::new();
        owner.set_sharing_by_user(true); // Explicit synthetic sharing choice.
        let mut browser = AlbertTransactionFixture {
            owner,
            mutate_during_read: false,
            calls: 0,
        };
        assert!(desktop.handle_event(key(bar_panel::SUMMARY_KEY), &mut volume));
        summary_owner_step(&mut desktop, &user, summary_request_fixture(), &mut browser);
        assert_eq!(desktop.bar.controller.status(), session_ui::Status::Waiting);
        assert!(desktop
            .model_services
            .as_ref()
            .unwrap()
            .host_test_complete_request());
        if during_read {
            browser.mutate_during_read = true;
        } else {
            browser.owner.set_sharing_by_user(false);
            browser.owner.set_sharing_by_user(true);
        }
        summary_owner_step(&mut desktop, &user, summary_request_fixture(), &mut browser);
        assert_eq!(
            desktop.bar.controller.status(),
            if during_read {
                session_ui::Status::PageSummaryUnavailable
            } else {
                session_ui::Status::Denied
            }
        );
        assert_eq!(desktop.bar.controller.output(), "");
        assert_eq!(
            browser.calls, 2,
            "one bounded extraction per UI step, no retry"
        );
        assert!(desktop
            .model_services
            .as_ref()
            .unwrap()
            .host_shared_snapshot()
            .unwrap()
            .active_request_id
            .is_none());
        desktop.render(&mut surface);
    }
}
