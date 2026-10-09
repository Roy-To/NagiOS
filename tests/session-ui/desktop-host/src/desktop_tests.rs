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
    unlock(&mut desktop, &mut volume);
    assert!(desktop.handle_event(key(62), &mut volume));
    desktop.render(&mut surface);
    assert_eq!(frame_hash(&surface), initial);
    libnagi::set_readiness(false);
    unlock(&mut desktop, &mut volume);
    assert!(!desktop.model_services_ready);
    assert_eq!(libnagi::readiness_calls(), 1);
    assert_eq!(
        desktop.bar.controller.status(),
        session_ui::Status::SignedOut
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

// HOST orchestration only: seed the actual controller with display/pending
// state. Desktop locking below uses its real Adapter, event handler and renderer.
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
fn actual_lock_handlers_erase_populated_bar_state_before_lock_and_relogin_render() {
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
            assert!(desktop.lock_requested);
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
