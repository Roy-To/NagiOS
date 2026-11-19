// Standalone host regression for the exact recovery modules used by main.rs.
// The app binary is Linux-targeted and links libnagi's x86-64 syscall ABI, so
// this test imports only the ownership/proof/state-machine modules.
#[path = "../src/input_failure_guard.rs"]
mod input_failure_guard;
#[path = "../src/input_reconnect.rs"]
mod input_reconnect;
#[path = "../src/input_recovery.rs"]
mod input_recovery;
#[path = "../src/linux_input_reopen.rs"]
mod linux_input_reopen;
#[path = "../src/recovery_main_dispatch.rs"]
mod recovery_main_dispatch;
#[path = "../src/recovery_ui_commands.rs"]
mod recovery_ui_commands;

#[cfg(test)]
mod repeated_reconnect_tests {
    use super::input_failure_guard::InputFailureAction;
    use super::input_reconnect::DeviceIdentity;
    use super::input_recovery::{
        RecoveryHost, RecoveryNotice, RecoveryOwner, RecoveryPhase, RecoveryStart, RecoveryStep,
    };
    use super::linux_input_reopen::{LinuxGrantedReopen, LinuxInputGrant, ReopenAttemptError};
    use super::recovery_main_dispatch::{
        dispatch_initial_grant, dispatch_reconnected_input_loss, next_recovery_wait,
        InitialGrantDispatch, ReconnectedInputLoss, ReconnectedInputLossDispatch,
    };
    use super::recovery_ui_commands::{RecoveryUiCommand, RecoveryUiCommandAdapter};
    use std::fs::{self, File, OpenOptions};
    use std::io;
    use std::os::unix::fs::MetadataExt;
    use std::panic::{catch_unwind, AssertUnwindSafe};
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    #[derive(Default)]
    struct TestHost {
        fail_next_notice: bool,
        notices: Vec<RecoveryNotice>,
    }

    impl RecoveryHost<String> for TestHost {
        type SaveError = &'static str;
        type NotificationError = &'static str;

        fn retry_save(&mut self, _draft: &mut String) -> Result<(), Self::SaveError> {
            Ok(())
        }

        fn notify(&mut self, notice: RecoveryNotice) -> Result<(), Self::NotificationError> {
            self.notices.push(notice);
            if std::mem::take(&mut self.fail_next_notice) {
                Err("notification transport unavailable")
            } else {
                Ok(())
            }
        }
    }

    fn held_owner() -> RecoveryOwner<String> {
        let RecoveryStart::Held(owner) =
            RecoveryOwner::from_input_failure(InputFailureAction::HoldUnsavedDraft {
                owner: "dirty draft retained across input loss".to_owned(),
                input_error: "initial evdev source closed",
                save_error: "initial flush failed".to_owned(),
                notification_error: None,
            })
        else {
            panic!("fixture must keep its unsaved owner");
        };
        owner
    }

    static NEXT_FILE: AtomicU64 = AtomicU64::new(1);

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    struct FileIdentity {
        device: u64,
        inode: u64,
    }

    impl FileIdentity {
        fn of(metadata: &fs::Metadata) -> Self {
            Self {
                device: metadata.dev(),
                inode: metadata.ino(),
            }
        }
    }

    struct PrivateTestFile {
        path: PathBuf,
        anchor: Option<File>,
        original_identity: FileIdentity,
        backup_path: Option<PathBuf>,
        replacement_anchor: Option<File>,
        replacement_identity: Option<FileIdentity>,
    }

    impl PrivateTestFile {
        fn create() -> Self {
            let path = std::env::temp_dir().join(format!(
                "nagi-repeated-reconnect-{}-{}",
                std::process::id(),
                NEXT_FILE.fetch_add(1, Ordering::Relaxed)
            ));
            let anchor = OpenOptions::new()
                .create_new(true)
                .read(true)
                .write(true)
                .open(&path)
                .expect("create a private fixture file exclusively");
            let original_identity = FileIdentity::of(
                &anchor
                    .metadata()
                    .expect("read metadata for the private fixture file"),
            );
            Self {
                path,
                anchor: Some(anchor),
                original_identity,
                backup_path: None,
                replacement_anchor: None,
                replacement_identity: None,
            }
        }

        fn path(&self) -> &Path {
            &self.path
        }

        fn open_original(&self, path: &Path) -> io::Result<File> {
            let opened = File::open(path)?;
            let opened_identity = FileIdentity::of(&opened.metadata()?);
            let path_metadata = fs::symlink_metadata(path)?;
            if path_metadata.file_type().is_symlink()
                || opened_identity != self.original_identity
                || FileIdentity::of(&path_metadata) != self.original_identity
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "fixture grant path no longer names the originally created file",
                ));
            }
            Ok(opened)
        }

        fn replace_path_for_test(&mut self) -> io::Result<()> {
            if self.backup_path.is_some() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "fixture path already has a replacement",
                ));
            }
            let backup = self.path.with_extension("original-fixture");
            fs::hard_link(&self.path, &backup)?;
            self.backup_path = Some(backup);
            fs::remove_file(&self.path)?;
            let replacement = OpenOptions::new()
                .create_new(true)
                .read(true)
                .write(true)
                .open(&self.path)?;
            self.replacement_identity = Some(FileIdentity::of(&replacement.metadata()?));
            self.replacement_anchor = Some(replacement);
            Ok(())
        }

        fn restore_original(&mut self) -> io::Result<()> {
            let Some(backup) = self.backup_path.as_ref() else {
                return Ok(());
            };
            drop(self.replacement_anchor.take());
            match fs::symlink_metadata(&self.path) {
                Ok(metadata)
                    if !metadata.file_type().is_symlink()
                        && FileIdentity::of(&metadata) == self.original_identity =>
                {
                    fs::remove_file(backup)?;
                    self.backup_path = None;
                    self.replacement_identity = None;
                    return Ok(());
                }
                Ok(metadata)
                    if !metadata.file_type().is_symlink()
                        && self.replacement_identity == Some(FileIdentity::of(&metadata)) =>
                {
                    fs::remove_file(&self.path)?;
                }
                Ok(_) => {
                    return Err(io::Error::new(
                        io::ErrorKind::PermissionDenied,
                        "refusing to remove a path that is not this fixture's replacement",
                    ));
                }
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            fs::hard_link(backup, &self.path)?;
            fs::remove_file(backup)?;
            self.backup_path = None;
            self.replacement_identity = None;
            Ok(())
        }

        fn cleanup(&mut self) -> io::Result<()> {
            self.restore_original()?;
            drop(self.anchor.take());
            if self.path.as_os_str().is_empty() {
                return Ok(());
            }
            let metadata = fs::symlink_metadata(&self.path)?;
            if metadata.file_type().is_symlink()
                || FileIdentity::of(&metadata) != self.original_identity
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "refusing to remove a path that is not this fixture's original file",
                ));
            }
            fs::remove_file(&self.path)?;
            self.path.clear();
            Ok(())
        }
    }

    impl Drop for PrivateTestFile {
        fn drop(&mut self) {
            let _ = self.cleanup();
        }
    }

    struct RepeatedReconnectFixture {
        recovery: Option<LinuxGrantedReopen<String>>,
        host: TestHost,
        file: PrivateTestFile,
        identity: DeviceIdentity,
    }

    impl RepeatedReconnectFixture {
        fn recovery(&self) -> &LinuxGrantedReopen<String> {
            self.recovery.as_ref().expect("fixture owns recovery")
        }

        fn reopen(
            &mut self,
            identity: DeviceIdentity,
        ) -> Result<super::linux_input_reopen::ReopenedGrantedInput, ReopenAttemptError> {
            let recovery = self.recovery.as_mut().expect("fixture owns recovery");
            reopen(recovery, identity, &mut self.host, &self.file)
        }

        fn retry_notice(&mut self) {
            retry_notice_while_waiting(
                self.recovery.as_mut().expect("fixture owns recovery"),
                &mut self.host,
            );
        }

        fn relose(&mut self, partial_record_bytes: usize, reason: &str) {
            let recovery = self.recovery.as_mut().expect("fixture owns recovery");
            let loss = ReconnectedInputLoss::from_read_error(reason, partial_record_bytes);
            match dispatch_reconnected_input_loss(recovery, &loss, &mut self.host) {
                ReconnectedInputLossDispatch::RetryOriginalGrant { notification_error } => {
                    assert_eq!(
                        notification_error.as_deref(),
                        Some("notification transport unavailable")
                    );
                }
                ReconnectedInputLossDispatch::ReArmFailed { error } => {
                    panic!("connected stream loss could not re-arm: {error}");
                }
            }
            assert_eq!(recovery.owner().phase(), RecoveryPhase::WaitingForInput);
            assert_eq!(
                recovery.owner().draft().map(String::as_str),
                Some("dirty draft retained across input loss")
            );
        }

        fn resolve_for_cleanup(&mut self) {
            let Some(recovery) = self.recovery.as_mut() else {
                return;
            };
            if matches!(
                recovery.owner().phase(),
                RecoveryPhase::WaitingForInput | RecoveryPhase::NotificationUnavailable
            ) {
                let identity = self.identity.clone();
                if let Ok(reopened) = reopen(recovery, identity, &mut self.host, &self.file) {
                    drop(reopened);
                }
            }

            for _ in 0..3 {
                let phase = recovery.owner().phase();
                if phase != RecoveryPhase::NotificationUnavailable {
                    break;
                }
                let mut ui = RecoveryUiCommandAdapter::borrow(recovery.owner_mut());
                let _ = ui.apply(RecoveryUiCommand::RetryNotification, &mut self.host);
            }

            match recovery.owner().phase() {
                RecoveryPhase::AwaitingUserChoice => {
                    let mut ui = RecoveryUiCommandAdapter::borrow(recovery.owner_mut());
                    let _ = ui.apply(RecoveryUiCommand::BeginDiscard, &mut self.host);
                    let _ = ui.apply(RecoveryUiCommand::ConfirmDiscard, &mut self.host);
                }
                RecoveryPhase::ConfirmingDiscard => {
                    let mut ui = RecoveryUiCommandAdapter::borrow(recovery.owner_mut());
                    let _ = ui.apply(RecoveryUiCommand::ConfirmDiscard, &mut self.host);
                }
                _ => {}
            }
        }

        fn finish(&mut self) {
            let ticket = self.recovery().owner().ticket();
            self.resolve_for_cleanup();
            let recovery = self.recovery.take().expect("fixture owns recovery");
            assert!(recovery.owner().is_resolved());
            drop(recovery);
            assert!(!RecoveryOwner::<String>::pending_tickets().contains(&ticket));
            self.file
                .cleanup()
                .expect("remove only this test's original fixture file");
        }
    }

    impl Drop for RepeatedReconnectFixture {
        fn drop(&mut self) {
            let _ = self.file.restore_original();
            self.resolve_for_cleanup();
            self.recovery.take();
            let _ = self.file.cleanup();
        }
    }

    fn private_file() -> PrivateTestFile {
        PrivateTestFile::create()
    }

    fn reopen(
        recovery: &mut LinuxGrantedReopen<String>,
        identity: DeviceIdentity,
        host: &mut TestHost,
        file: &PrivateTestFile,
    ) -> Result<super::linux_input_reopen::ReopenedGrantedInput, ReopenAttemptError> {
        recovery.try_reopen_once_with_identity(
            |path: &Path| file.open_original(path),
            |_device| Ok::<_, &'static str>(identity),
            host,
        )
    }

    fn reconnect_identity(value: &str) -> DeviceIdentity {
        DeviceIdentity::from_test_identity(value).expect("fixture identity is nonempty")
    }

    fn retry_notice_while_waiting(recovery: &mut LinuxGrantedReopen<String>, host: &mut TestHost) {
        assert!(recovery.owner().notification_retry_available());
        let due = recovery
            .owner()
            .next_periodic_notification_retry_at()
            .expect("failed reconnect notice schedules a retry");
        let result = recovery
            .owner_mut()
            .recovery()
            .retry_notification_if_due(due, host)
            .expect("the scheduled notice retry is due");
        assert_eq!(
            result,
            RecoveryStep::NotificationRetried {
                notification_error: None
            }
        );
        assert_eq!(recovery.owner().phase(), RecoveryPhase::WaitingForInput);
        assert!(!recovery.owner().notification_retry_available());
    }

    #[test]
    fn repeated_disconnects_reopen_same_identity_with_fresh_epochs_and_notice_retries() {
        let file = private_file();
        let identity = reconnect_identity("fixture:keyboard-42");
        let other_identity = reconnect_identity("fixture:keyboard-99");
        let grant =
            LinuxInputGrant::from_test_identity(file.path().to_path_buf(), identity.clone());
        let recovery = grant.into_recovery(5, held_owner());
        let mut fixture = RepeatedReconnectFixture {
            recovery: Some(recovery),
            host: TestHost::default(),
            file,
            identity: identity.clone(),
        };

        let first = fixture
            .reopen(identity.clone())
            .expect("first identity-verified reopen");
        assert_eq!(first.parser_epoch, 1);
        assert_eq!(first.discarded_partial_record_bytes, 5);
        assert_eq!(
            fixture.recovery().owner().phase(),
            RecoveryPhase::AwaitingUserChoice
        );
        drop(first);

        fixture.host.fail_next_notice = true;
        fixture.relose(7, "second evdev stream closed");
        assert_eq!(
            fixture.recovery().owner().draft().map(String::as_str),
            Some("dirty draft retained across input loss")
        );
        fixture.retry_notice();

        let mismatch = fixture.reopen(other_identity);
        assert!(matches!(
            mismatch,
            Err(ReopenAttemptError::Unavailable { ref reason, .. })
                if reason.contains("does not match the original evdev unique ID")
        ));
        assert_eq!(
            fixture.recovery().owner().phase(),
            RecoveryPhase::WaitingForInput
        );
        assert_eq!(
            fixture.recovery().owner().draft().map(String::as_str),
            Some("dirty draft retained across input loss")
        );

        let second = fixture
            .reopen(identity.clone())
            .expect("same identity reopens after a rejected candidate");
        assert_eq!(second.parser_epoch, 2);
        assert_eq!(second.discarded_partial_record_bytes, 7);
        assert_eq!(
            fixture.recovery().owner().phase(),
            RecoveryPhase::AwaitingUserChoice
        );
        let mut ui = RecoveryUiCommandAdapter::borrow(
            fixture
                .recovery
                .as_mut()
                .expect("fixture owns recovery")
                .owner_mut(),
        );
        ui.apply(RecoveryUiCommand::BeginDiscard, &mut fixture.host)
            .expect("the test enters, but does not confirm, discard");
        assert_eq!(
            fixture.recovery().owner().phase(),
            RecoveryPhase::ConfirmingDiscard
        );
        drop(second);

        fixture.host.fail_next_notice = true;
        fixture.relose(9, "third evdev stream closed");
        assert_eq!(
            fixture.recovery().owner().draft().map(String::as_str),
            Some("dirty draft retained across input loss")
        );
        fixture.retry_notice();

        let third = fixture
            .reopen(identity)
            .expect("same identity recovers after a second disconnect");
        assert_eq!(third.parser_epoch, 3);
        assert_eq!(third.discarded_partial_record_bytes, 9);
        assert_eq!(
            fixture.recovery().owner().phase(),
            RecoveryPhase::AwaitingUserChoice
        );
        assert_eq!(
            fixture.recovery().owner().draft().map(String::as_str),
            Some("dirty draft retained across input loss")
        );
        drop(third);
        fixture.finish();
    }

    #[test]
    fn shared_main_dispatch_keeps_owner_waiting_after_path_replacement_and_rearm_failure() {
        let file = private_file();
        let identity = reconnect_identity("fixture:main-dispatch-keyboard");
        let grant =
            LinuxInputGrant::from_test_identity(file.path().to_path_buf(), identity.clone());
        let recovery = grant.into_recovery(5, held_owner());
        let mut fixture = RepeatedReconnectFixture {
            recovery: Some(recovery),
            host: TestHost::default(),
            file,
            identity: identity.clone(),
        };

        let first = fixture
            .reopen(identity.clone())
            .expect("the original identity opens before the scripted read error");
        assert_eq!(first.parser_epoch, 1);
        drop(first);
        let ticket = fixture.recovery().owner().ticket();
        fixture.host.fail_next_notice = true;
        let loss = ReconnectedInputLoss::from_read_error("scripted evdev read error", 11);
        assert!(matches!(
            dispatch_reconnected_input_loss(
                fixture.recovery.as_mut().expect("fixture owns recovery"),
                &loss,
                &mut fixture.host,
            ),
            ReconnectedInputLossDispatch::RetryOriginalGrant {
                notification_error: Some(ref error)
            } if error == "notification transport unavailable"
        ));
        assert_eq!(
            fixture.recovery().owner().phase(),
            RecoveryPhase::WaitingForInput,
            "delivery failure stays separate from the input-wait phase"
        );
        assert_eq!(fixture.recovery().owner().ticket(), ticket);
        assert_eq!(
            fixture.recovery().owner().draft().map(String::as_str),
            Some("dirty draft retained across input loss")
        );
        assert!(fixture.recovery().owner().notification_retry_available());
        let retry_at = fixture
            .recovery()
            .owner()
            .next_periodic_notification_retry_at()
            .expect("failed notification has a retry deadline");
        assert_eq!(
            next_recovery_wait(
                fixture.recovery().owner(),
                Duration::from_secs(30),
                retry_at - Duration::from_secs(1),
            ),
            Duration::from_secs(1),
            "main waits only until the scripted notification deadline"
        );
        fixture.retry_notice();
        assert_eq!(
            fixture.recovery().owner().phase(),
            RecoveryPhase::WaitingForInput
        );

        fixture
            .file
            .replace_path_for_test()
            .expect("replace only the fixture's private grant path");
        assert!(matches!(
            fixture.reopen(identity.clone()),
            Err(ReopenAttemptError::Unavailable { ref reason, .. })
                if reason.contains("originally created file")
        ));
        assert_eq!(
            fixture.recovery().owner().phase(),
            RecoveryPhase::WaitingForInput
        );
        assert_eq!(fixture.recovery().owner().ticket(), ticket);
        assert_eq!(
            fixture.recovery().owner().draft().map(String::as_str),
            Some("dirty draft retained across input loss")
        );
        assert!(
            !RecoveryOwner::<String>::pending_tickets().contains(&ticket),
            "a rejected path replacement leaves the live owner out of the keeper"
        );

        let failed_rearm_loss =
            ReconnectedInputLoss::from_read_error("scripted loss while waiting", 0);
        assert!(matches!(
            dispatch_reconnected_input_loss(
                fixture.recovery.as_mut().expect("fixture owns recovery"),
                &failed_rearm_loss,
                &mut fixture.host,
            ),
            ReconnectedInputLossDispatch::ReArmFailed { .. }
        ));
        assert_eq!(
            fixture.recovery().owner().phase(),
            RecoveryPhase::WaitingForInput
        );
        assert_eq!(fixture.recovery().owner().ticket(), ticket);
        assert_eq!(
            fixture.recovery().owner().draft().map(String::as_str),
            Some("dirty draft retained across input loss")
        );

        fixture
            .file
            .restore_original()
            .expect("restore only the original fixture identity");
        let second = fixture
            .reopen(identity)
            .expect("the same identity is still accepted after failure");
        assert_eq!(second.parser_epoch, 2);
        assert_eq!(
            fixture.recovery().owner().draft().map(String::as_str),
            Some("dirty draft retained across input loss")
        );
        drop(second);
        fixture.finish();
    }

    #[test]
    fn shared_main_dispatch_with_no_original_grant_holds_owner_without_reopen() {
        let file = private_file();
        let identity = reconnect_identity("fixture:no-original-grant-keyboard");
        let ticket;
        let owner = held_owner();
        ticket = owner.ticket();
        let dispatch = dispatch_initial_grant(None, 7, owner);
        let owner = match dispatch {
            InitialGrantDispatch::MissingOriginalGrant(owner) => owner,
            InitialGrantDispatch::Granted(_) => {
                panic!("an absent original grant cannot create a reopen adapter")
            }
        };

        assert_eq!(owner.phase(), RecoveryPhase::WaitingForInput);
        assert_eq!(owner.ticket(), ticket);
        assert_eq!(
            owner.draft().map(String::as_str),
            Some("dirty draft retained across input loss")
        );
        assert!(
            !RecoveryOwner::<String>::pending_tickets().contains(&ticket),
            "main keeps the owner live while it waits instead of dropping it"
        );

        // Test teardown resumes only this fixture's original identity so it
        // can exercise the explicit discard cleanup path; the `None` outcome
        // above itself contains no path or reopen-capable adapter.
        let grant =
            LinuxInputGrant::from_test_identity(file.path().to_path_buf(), identity.clone());
        let recovery = match dispatch_initial_grant(Some(grant), 7, owner) {
            InitialGrantDispatch::Granted(recovery) => recovery,
            InitialGrantDispatch::MissingOriginalGrant(_) => {
                panic!("the private cleanup fixture supplies its original grant")
            }
        };
        let mut fixture = RepeatedReconnectFixture {
            recovery: Some(recovery),
            host: TestHost::default(),
            file,
            identity: identity.clone(),
        };
        let reopened = fixture
            .reopen(identity)
            .expect("cleanup uses only the original fixture identity");
        drop(reopened);
        let mut ui = RecoveryUiCommandAdapter::borrow(
            fixture
                .recovery
                .as_mut()
                .expect("fixture owns recovery")
                .owner_mut(),
        );
        ui.apply(RecoveryUiCommand::BeginDiscard, &mut fixture.host)
            .expect("cleanup begins an explicit discard");
        ui.apply(RecoveryUiCommand::ConfirmDiscard, &mut fixture.host)
            .expect("cleanup confirms an explicit discard");
        fixture.finish();
    }

    #[test]
    fn fixture_drop_resolves_owner_and_removes_private_file() {
        let file = private_file();
        let path = file.path().to_path_buf();
        let identity = reconnect_identity("fixture:cleanup-keyboard");
        let grant = LinuxInputGrant::from_test_identity(path.clone(), identity.clone());
        let mut fixture = RepeatedReconnectFixture {
            recovery: Some(grant.into_recovery(0, held_owner())),
            host: TestHost::default(),
            file,
            identity: identity.clone(),
        };
        let reopened = fixture
            .reopen(identity)
            .expect("cleanup fixture reaches user choice");
        drop(reopened);
        let ticket = fixture.recovery().owner().ticket();

        drop(fixture);

        assert!(!path.exists(), "fixture Drop removes its private file");
        assert!(
            !RecoveryOwner::<String>::pending_tickets().contains(&ticket),
            "fixture Drop explicitly resolves its recovery owner"
        );
    }

    #[test]
    fn setup_unwind_removes_only_the_exclusive_fixture_file() {
        let mut created_path = None;
        let result = catch_unwind(AssertUnwindSafe(|| {
            let file = private_file();
            created_path = Some(file.path().to_path_buf());
            panic!("injected failure after fixture setup");
        }));

        assert!(result.is_err(), "the injected setup failure must unwind");
        let path = created_path.expect("the fixture file was created before unwind");
        assert!(!path.exists(), "RAII removes its own file after unwind");
    }

    #[test]
    fn failed_reopen_after_same_uid_path_replacement_does_not_park_owner() {
        let file = private_file();
        let path = file.path().to_path_buf();
        let identity = reconnect_identity("fixture:path-identity-keyboard");
        let grant = LinuxInputGrant::from_test_identity(path.clone(), identity.clone());
        let mut fixture = RepeatedReconnectFixture {
            recovery: Some(grant.into_recovery(0, held_owner())),
            host: TestHost::default(),
            file,
            identity: identity.clone(),
        };
        let ticket = fixture.recovery().owner().ticket();
        fixture
            .file
            .replace_path_for_test()
            .expect("replace only the private fixture path");

        let result = fixture.reopen(identity);
        assert!(matches!(
            result,
            Err(ReopenAttemptError::Unavailable { ref reason, .. })
                if reason.contains("originally created file")
        ));
        assert!(
            !RecoveryOwner::<String>::pending_tickets().contains(&ticket),
            "failed reopen keeps the active owner out of the process keeper"
        );

        drop(fixture);

        assert!(
            !path.exists(),
            "cleanup removes the restored original fixture"
        );
        assert!(
            !RecoveryOwner::<String>::pending_tickets().contains(&ticket),
            "cleanup restores the identity and explicitly resolves the owner"
        );
    }
}
