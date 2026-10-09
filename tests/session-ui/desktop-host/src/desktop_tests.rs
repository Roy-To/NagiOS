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
