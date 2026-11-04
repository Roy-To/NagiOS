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
