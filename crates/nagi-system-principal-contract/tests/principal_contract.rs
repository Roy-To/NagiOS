//! Contract tests for the shared C3 system-service principal types.
//!
//! These tests were previously compiled only inside nagi-init's test-only
//! `linux_init_process_owner_candidate` module (through an old copy of
//! `linux_x86_64_process.rs`). They exercise this crate's public API directly,
//! so they now live with the crate. Moved without changing assertions.

#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

mod owner_proof {
    use nagi_init_spawn_contract::test_pidfd_for_existing_process;
    use nagi_system_principal_contract::{
        LinuxPeerCredentials, SupervisorLaunchRole, SystemServiceEndpointOwnerProof,
        SystemServiceLaunchRecord,
    };
    use std::io;
    use std::num::NonZeroU64;
    use std::os::fd::OwnedFd;
    use std::os::unix::net::UnixStream;
    use std::sync::mpsc;
    use std::time::Duration;

    fn current_uid_gid() -> (u32, u32) {
        // The owner of /proc/self is this process's real UID/GID; this avoids
        // a raw-syscall helper from nagi-init in the shared contract crate.
        use std::os::unix::fs::MetadataExt;
        let metadata = std::fs::metadata("/proc/self").expect("stat /proc/self");
        (metadata.uid(), metadata.gid())
    }

    fn process_pidfd(process_id: u32) -> nagi_init_spawn_contract::SpawnedProcessPidfd {
        test_pidfd_for_existing_process(process_id)
            .unwrap_or_else(|error| panic!("test pidfd_open({process_id}) failed: {error}"))
    }

    fn launch_record(
        process: &nagi_init_spawn_contract::SpawnedProcessPidfd,
        generation: u64,
    ) -> SystemServiceLaunchRecord {
        // SAFETY: this fixture binds the selected canonical service slot
        // and catalog generation to the exact live process token. The
        // maintained production issuer is still a separate integration.
        unsafe {
            SystemServiceLaunchRecord::after_supervisor_system_service_slot_validation(
                process,
                "fixture.identity",
                NonZeroU64::new(9).unwrap(),
                NonZeroU64::new(generation).unwrap(),
            )
        }
    }

    fn launch_record_with_role(
        process: &nagi_init_spawn_contract::SpawnedProcessPidfd,
        generation: u64,
        role: SupervisorLaunchRole,
    ) -> SystemServiceLaunchRecord {
        // SAFETY: the test binds this generation to the exact live process
        // token it just opened; production uses the C3 Supervisor wrapper.
        unsafe {
            SystemServiceLaunchRecord::after_supervisor_record_validation(
                process,
                NonZeroU64::new(generation).unwrap(),
                role,
            )
        }
    }

    fn into_owned_fd(stream: UnixStream) -> OwnedFd {
        stream.into()
    }

    #[test]
    fn kernel_peer_credentials_bind_exact_process_and_launch() {
        let process_id = std::process::id();
        let process = process_pidfd(process_id);
        let record = launch_record(&process, 41);
        let (endpoint, peer) = UnixStream::pair().unwrap();
        let (uid, gid) = current_uid_gid();

        let proof = SystemServiceEndpointOwnerProof::authenticate(
            &process,
            &record,
            into_owned_fd(endpoint),
            uid,
            gid,
        )
        .unwrap();

        assert_eq!(proof.version(), 3);
        assert_eq!(proof.process_id(), process_id);
        assert_eq!(proof.launch_generation().get(), 41);
        assert_eq!(
            proof.service_slot_identity().service_id(),
            "fixture.identity"
        );
        assert_eq!(proof.service_slot_identity().catalog_generation().get(), 9);
        assert_ne!(proof.endpoint_generation().get(), 0);
        assert_eq!(
            proof.peer_credentials(),
            LinuxPeerCredentials {
                process_id,
                uid,
                gid,
            }
        );
        assert!(proof.endpoint().is_some());
        assert!(proof.is_live().unwrap());
        assert_eq!(proof.with_live_authority_effect(|| 7).unwrap(), 7);
        drop(peer);
        assert!(!proof.is_live().unwrap());
    }

    #[test]
    fn self_reported_credentials_and_a_different_process_endpoint_are_rejected() {
        let process_id = std::process::id();
        let process = process_pidfd(process_id);
        let record = launch_record(&process, 42);
        let (endpoint, _peer) = UnixStream::pair().unwrap();
        let (uid, gid) = current_uid_gid();
        let error = SystemServiceEndpointOwnerProof::authenticate(
            &process,
            &record,
            into_owned_fd(endpoint),
            uid.wrapping_add(1),
            gid,
        )
        .err()
        .expect("self-reported credentials were accepted");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);

        use std::io::BufRead;
        let module = module_path!().split_once("::").unwrap().1;
        let helper = format!("{module}::endpoint_mismatch_waiting_child");
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "--nocapture", &helper])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .expect("spawn a distinct process for the endpoint mismatch case");
        let mut output = io::BufReader::new(child.stdout.take().unwrap());
        let ready = (&mut output)
            .lines()
            .map_while(Result::ok)
            .any(|line| line.ends_with("NAGI_C3_ENDPOINT_MISMATCH_CHILD_READY"));
        if !ready {
            drop(child.stdin.take());
            let _ = child.wait();
            panic!("endpoint mismatch fixture child did not become ready");
        }
        let other_process = process_pidfd(child.id());
        let other_record = launch_record(&other_process, 43);
        let (endpoint, _peer) = UnixStream::pair().unwrap();
        let result = SystemServiceEndpointOwnerProof::authenticate(
            &other_process,
            &other_record,
            into_owned_fd(endpoint),
            uid,
            gid,
        );
        let live = child.try_wait().expect("check the endpoint mismatch child");
        drop(child.stdin.take());
        let status = child.wait().expect("reap the endpoint mismatch child");
        assert!(live.is_none(), "endpoint mismatch fixture child exited early");
        assert!(status.success(), "endpoint mismatch fixture child failed");
        let error = result.err().expect("endpoint from another process was accepted");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(
            error.to_string(),
            "kernel peer credentials do not match the exact C3 process launch"
        );
    }

    #[test]
    #[ignore = "subprocess fixture for the endpoint mismatch test"]
    fn endpoint_mismatch_waiting_child() {
        use std::io::Write;
        let mut stdout = io::stdout().lock();
        writeln!(stdout, "NAGI_C3_ENDPOINT_MISMATCH_CHILD_READY").unwrap();
        stdout.flush().unwrap();
        drop(stdout);
        io::copy(&mut io::stdin().lock(), &mut io::sink()).unwrap();
    }

    #[test]
    fn dropping_one_proof_does_not_revoke_its_launch_or_sibling_proof() {
        let process = process_pidfd(std::process::id());
        let record = launch_record(&process, 44);
        let (first_endpoint, _first_peer) = UnixStream::pair().unwrap();
        let (second_endpoint, _second_peer) = UnixStream::pair().unwrap();
        let (uid, gid) = current_uid_gid();
        let first = SystemServiceEndpointOwnerProof::authenticate(
            &process,
            &record,
            into_owned_fd(first_endpoint),
            uid,
            gid,
        )
        .unwrap();
        let proof = SystemServiceEndpointOwnerProof::authenticate(
            &process,
            &record,
            into_owned_fd(second_endpoint),
            uid,
            gid,
        )
        .unwrap();

        drop(first);
        assert!(record.is_live());
        assert!(proof.is_live().unwrap());
        assert_eq!(proof.with_live_authority_effect(|| 44).unwrap(), 44);

        // SAFETY: this fixture models exact removal from the Supervisor
        // record slot while the launch gate is held.
        assert!(unsafe { record.revoke_after_supervisor_record_validation(|| true) });
        assert!(!proof.is_live().unwrap());
        assert!(proof.with_live_authority_effect(|| ()).is_err());
    }

    #[test]
    fn proof_revoke_is_local_and_closes_only_its_own_endpoint() {
        let process = process_pidfd(std::process::id());
        let record = launch_record(&process, 45);
        let (endpoint, _peer) = UnixStream::pair().unwrap();
        let (sibling_endpoint, _sibling_peer) = UnixStream::pair().unwrap();
        let (uid, gid) = current_uid_gid();
        let mut proof = SystemServiceEndpointOwnerProof::authenticate(
            &process,
            &record,
            into_owned_fd(endpoint),
            uid,
            gid,
        )
        .unwrap();
        let sibling = SystemServiceEndpointOwnerProof::authenticate(
            &process,
            &record,
            into_owned_fd(sibling_endpoint),
            uid,
            gid,
        )
        .unwrap();

        assert!(proof.revoke());
        assert!(!proof.revoke());
        assert!(proof.endpoint().is_none());
        assert!(!proof.is_live().unwrap());
        assert!(record.is_live());
        assert!(sibling.is_live().unwrap());
    }

    #[test]
    fn exact_supervisor_replacement_rejects_stale_generation_and_stale_revoke() {
        let process = process_pidfd(std::process::id());
        let record = launch_record(&process, 46);
        let (endpoint, _peer) = UnixStream::pair().unwrap();
        let (uid, gid) = current_uid_gid();
        let proof = SystemServiceEndpointOwnerProof::authenticate(
            &process,
            &record,
            into_owned_fd(endpoint),
            uid,
            gid,
        )
        .unwrap();

        let mut registry_called = false;
        // SAFETY: this negative fixture cannot replace a different
        // canonical service slot and must fail before the registry call.
        let wrong_slot = unsafe {
            record.replace_after_supervisor_system_service_slot_validation(
                &process,
                "fixture.other",
                NonZeroU64::new(9).unwrap(),
                NonZeroU64::new(10).unwrap(),
                NonZeroU64::new(47).unwrap(),
                || {
                    registry_called = true;
                    true
                },
            )
        };
        assert!(wrong_slot.is_err());
        assert!(!registry_called);

        // SAFETY: this negative fixture supplies the wrong exact prior
        // catalog generation; the registry callback must not run.
        let wrong_previous_catalog = unsafe {
            record.replace_after_supervisor_system_service_slot_validation(
                &process,
                "fixture.identity",
                NonZeroU64::new(8).unwrap(),
                NonZeroU64::new(10).unwrap(),
                NonZeroU64::new(47).unwrap(),
                || {
                    registry_called = true;
                    true
                },
            )
        };
        assert!(wrong_previous_catalog.is_err());
        assert!(!registry_called);

        // SAFETY: catalog generations cannot be reused for a new launch
        // in the same exact slot.
        let reused_catalog_generation = unsafe {
            record.replace_after_supervisor_system_service_slot_validation(
                &process,
                "fixture.identity",
                NonZeroU64::new(9).unwrap(),
                NonZeroU64::new(9).unwrap(),
                NonZeroU64::new(47).unwrap(),
                || {
                    registry_called = true;
                    true
                },
            )
        };
        assert!(reused_catalog_generation.is_err());
        assert!(!registry_called);

        // SAFETY: a bound service slot cannot be replaced through the
        // generic unbound API.
        let generic_replacement = unsafe {
            record.replace_after_supervisor_record_validation(
                &process,
                NonZeroU64::new(47).unwrap(),
                SupervisorLaunchRole::SystemService,
                || {
                    registry_called = true;
                    true
                },
            )
        };
        assert!(generic_replacement.is_err());
        assert!(!registry_called);

        // SAFETY: the fixture acts as the serialized Supervisor slot and
        // installs a new catalog and C3 launch generation for the same
        // canonical service slot.
        let replacement = unsafe {
            record
                .replace_after_supervisor_system_service_slot_validation(
                    &process,
                    "fixture.identity",
                    NonZeroU64::new(9).unwrap(),
                    NonZeroU64::new(10).unwrap(),
                    NonZeroU64::new(47).unwrap(),
                    || true,
                )
                .unwrap()
        };
        assert_eq!(
            replacement
                .system_service_slot_identity()
                .unwrap()
                .catalog_generation()
                .get(),
            10
        );
        assert!(!proof.is_live().unwrap());
        assert_eq!(proof.service_slot_identity().catalog_generation().get(), 9);
        assert!(proof.with_live_authority_effect(|| ()).is_err());
        assert!(
            !unsafe {
                record.revoke_after_supervisor_record_validation(|| {
                    panic!("stale record reached the Supervisor registry callback")
                })
            },
            "stale record revoked its replacement"
        );
        assert!(replacement.is_live());
    }

    #[test]
    fn catalog_generation_rollback_cannot_reactivate_stale_proof() {
        let process = process_pidfd(std::process::id());
        let record = launch_record(&process, 46);
        let (endpoint, _peer) = UnixStream::pair().unwrap();
        let (uid, gid) = current_uid_gid();
        let proof = SystemServiceEndpointOwnerProof::authenticate(
            &process,
            &record,
            into_owned_fd(endpoint),
            uid,
            gid,
        )
        .unwrap();

        // SAFETY: this fixture models the exact serialized Supervisor
        // replacement from catalog generation 9 to generation 10.
        let generation_10 = unsafe {
            record
                .replace_after_supervisor_system_service_slot_validation(
                    &process,
                    "fixture.identity",
                    NonZeroU64::new(9).unwrap(),
                    NonZeroU64::new(10).unwrap(),
                    NonZeroU64::new(47).unwrap(),
                    || true,
                )
                .unwrap()
        };
        assert!(!proof.is_live().unwrap());

        let mut registry_called = false;
        // SAFETY: this deliberately supplies a stale catalog record after
        // the accepted 9 -> 10 transition; it must not restore old proof.
        let rollback = unsafe {
            generation_10.replace_after_supervisor_system_service_slot_validation(
                &process,
                "fixture.identity",
                NonZeroU64::new(10).unwrap(),
                NonZeroU64::new(9).unwrap(),
                NonZeroU64::new(48).unwrap(),
                || {
                    registry_called = true;
                    true
                },
            )
        };
        assert!(
            rollback.is_err(),
            "catalog rollback was accepted; the stale generation-9 proof became live: {}",
            proof.is_live().unwrap()
        );
        assert!(!registry_called);
        assert!(generation_10.is_live());
        assert!(!proof.is_live().unwrap());
    }

    #[test]
    fn reused_launch_generation_is_rejected_before_slot_replacement() {
        let process = process_pidfd(std::process::id());
        let record = launch_record(&process, 46);
        let (endpoint, _peer) = UnixStream::pair().unwrap();
        let (uid, gid) = current_uid_gid();
        let proof = SystemServiceEndpointOwnerProof::authenticate(
            &process,
            &record,
            into_owned_fd(endpoint),
            uid,
            gid,
        )
        .unwrap();

        let mut registry_called = false;
        // SAFETY: a new service-slot record cannot reuse the C3 launch
        // generation already bound to this exact prior record.
        let error = match unsafe {
            record.replace_after_supervisor_system_service_slot_validation(
                &process,
                "fixture.identity",
                NonZeroU64::new(9).unwrap(),
                NonZeroU64::new(10).unwrap(),
                NonZeroU64::new(46).unwrap(),
                || {
                    registry_called = true;
                    true
                },
            )
        } {
            Ok(_) => panic!("replacement reused the current C3 launch generation"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(!registry_called);
        assert!(record.is_live());
        assert!(proof.is_live().unwrap());
    }

    #[test]
    fn unbound_system_service_record_cannot_issue_endpoint_owner_proof() {
        let process = process_pidfd(std::process::id());
        let record = launch_record_with_role(&process, 47, SupervisorLaunchRole::SystemService);
        let (endpoint, _peer) = UnixStream::pair().unwrap();
        let (uid, gid) = current_uid_gid();
        let error = match SystemServiceEndpointOwnerProof::authenticate(
            &process,
            &record,
            into_owned_fd(endpoint),
            uid,
            gid,
        ) {
            Ok(_) => panic!("an unbound Supervisor record issued an endpoint proof"),
            Err(error) => error,
        };
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn interactive_application_record_cannot_issue_system_service_proof() {
        let process = process_pidfd(std::process::id());
        let record =
            launch_record_with_role(&process, 48, SupervisorLaunchRole::InteractiveApplication);
        let (endpoint, _peer) = UnixStream::pair().unwrap();
        let (uid, gid) = current_uid_gid();
        let error = SystemServiceEndpointOwnerProof::authenticate(
            &process,
            &record,
            into_owned_fd(endpoint),
            uid,
            gid,
        )
        .err()
        .expect("interactive application record was accepted as a service");
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }

    #[test]
    fn supervisor_revoke_waits_until_admitted_authority_effect_returns() {
        let process = process_pidfd(std::process::id());
        let record = launch_record(&process, 49);
        let (endpoint, _peer) = UnixStream::pair().unwrap();
        let (uid, gid) = current_uid_gid();
        let proof = SystemServiceEndpointOwnerProof::authenticate(
            &process,
            &record,
            into_owned_fd(endpoint),
            uid,
            gid,
        )
        .unwrap();
        let (effect_started_tx, effect_started_rx) = mpsc::channel();
        let (effect_release_tx, effect_release_rx) = mpsc::channel();
        let effect = std::thread::spawn(move || {
            proof.with_live_authority_effect(|| {
                effect_started_tx.send(()).unwrap();
                effect_release_rx.recv().unwrap();
                1
            })
        });
        effect_started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("authority callback did not start");

        let revoke_record = record.clone();
        let (revoke_started_tx, revoke_started_rx) = mpsc::channel();
        let (revoke_done_tx, revoke_done_rx) = mpsc::channel();
        let revoker = std::thread::spawn(move || {
            revoke_started_tx.send(()).unwrap();
            // SAFETY: the test models the exact Supervisor record removal
            // in the callback, serialized with the authority effect.
            revoke_done_tx
                .send(unsafe {
                    revoke_record.revoke_after_supervisor_record_validation(|| true)
                })
                .unwrap();
        });
        revoke_started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("revoke worker did not start");
        assert!(
            revoke_done_rx
                .recv_timeout(Duration::from_millis(40))
                .is_err(),
            "revoke completed while the authority callback held the record gate"
        );

        effect_release_tx.send(()).unwrap();
        assert_eq!(
            effect.join().expect("authority worker panicked").unwrap(),
            1
        );
        assert!(
            revoke_done_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("revoke remained blocked after authority callback returned")
        );
        revoker.join().expect("revoke worker panicked");
    }

    #[test]
    fn supervisor_replacement_waits_until_admitted_authority_effect_returns() {
        let process = process_pidfd(std::process::id());
        let replacement_process = process_pidfd(std::process::id());
        let record = launch_record(&process, 50);
        let (endpoint, _peer) = UnixStream::pair().unwrap();
        let (uid, gid) = current_uid_gid();
        let proof = SystemServiceEndpointOwnerProof::authenticate(
            &process,
            &record,
            into_owned_fd(endpoint),
            uid,
            gid,
        )
        .unwrap();
        let (effect_started_tx, effect_started_rx) = mpsc::channel();
        let (effect_release_tx, effect_release_rx) = mpsc::channel();
        let effect = std::thread::spawn(move || {
            proof.with_live_authority_effect(|| {
                effect_started_tx.send(()).unwrap();
                effect_release_rx.recv().unwrap();
                1
            })
        });
        effect_started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("authority callback did not start");

        let replace_record = record.clone();
        let (replace_started_tx, replace_started_rx) = mpsc::channel();
        let (replace_done_tx, replace_done_rx) = mpsc::channel();
        let replacer = std::thread::spawn(move || {
            replace_started_tx.send(()).unwrap();
            // SAFETY: this fixture models the canonical Supervisor lock
            // and exact slot replacement inside the launch gate callback.
            let replacement = unsafe {
                replace_record.replace_after_supervisor_system_service_slot_validation(
                    &replacement_process,
                    "fixture.identity",
                    NonZeroU64::new(9).unwrap(),
                    NonZeroU64::new(10).unwrap(),
                    NonZeroU64::new(51).unwrap(),
                    || true,
                )
            };
            let result =
                replacement.map(|record| (record.generation().get(), record.is_live()));
            replace_done_tx.send(result).unwrap();
        });
        replace_started_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("replacement worker did not start");
        assert!(
            replace_done_rx
                .recv_timeout(Duration::from_millis(40))
                .is_err(),
            "record replacement completed while the authority callback held the gate"
        );

        effect_release_tx.send(()).unwrap();
        assert_eq!(
            effect.join().expect("authority worker panicked").unwrap(),
            1
        );
        assert_eq!(
            replace_done_rx
                .recv_timeout(Duration::from_secs(2))
                .expect("replacement remained blocked after authority callback returned")
                .unwrap(),
            (51, true)
        );
        replacer.join().expect("replacement worker panicked");
        assert!(!record.is_live());
    }
}
