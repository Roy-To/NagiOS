#![cfg(target_os = "linux")]

#[path = "pid1_live_peer_supplier.rs"]
mod supplier;

use nagi_sdk::app_contract::{
    CorrelationId, IpcEnvelope, MessageKind, OrderingMetadata, RequestId, IPC_PROTOCOL_VERSION,
};
use nagi_sdk::AppId;
use nagi_service_contract::{
    CapabilityId, ContractVersion, OperationId, ServiceDescriptor, ServiceId, ServiceOperation,
};
use nagi_task37_receipt_supplier::service_effect::ServiceEffectOwnerId;
use std::io::{self, Read, Write};
use std::os::fd::FromRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::PathBuf;
use std::time::Duration;
use supplier::pidfd_process_instance::{
    Clone3PidfdOutcome, LinuxPidfdKernel, PidfdKernel, PidfdProcessInstanceAuthority,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("PIDFD_RUNTIME_FIXTURE=FAIL error={error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn std::error::Error>> {
    check_clone3_errno_negative()?;
    check_clone3_unavailable_negative()?;
    check_peer_pidfd_errno_negative()?;
    let path = PathBuf::from(format!("/tmp/task41-pidfd-{}.sock", unsafe {
        libc::getpid()
    }));
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path)?;
    listener.set_nonblocking(true)?;
    println!("UNIX_LISTENER=PASS path=/tmp/task41-pidfd-*.sock");
    let runtime = supplier::linux_runtime_fixture_support::RuntimeAuthorityFixture::new(listener)
        .map_err(peer_denial_to_io)?;

    let authority = PidfdProcessInstanceAuthority::new(LinuxPidfdKernel);
    let pending =
        match unsafe { authority.clone3_with_pidfd(libc::CLONE_NEWPID as libc::c_ulonglong) }? {
            Clone3PidfdOutcome::Parent(child) => authority
                .capture_clone3_child(child)
                .map_err(peer_denial_to_io)?,
            Clone3PidfdOutcome::Child => child_connect_and_wait(&path),
        };
    let process = pending.process();
    println!("PID1_SPAWN_CAPTURE=PASS child_pid={}", process.pid());
    if !authority
        .confirms_owned_process_live(process)
        .map_err(peer_denial_to_io)?
    {
        return Err("spawn pidfd did not report its child live".into());
    }
    println!("CLONE3_PIDFD_SPAWN=PASS child_pid={}", process.pid());

    // The test-only authorities below register the PID 1-owned clone3 spawn
    // proof and the exact endpoint. Kernel peer credentials and the process
    // instance still come from the real accepted stream and pidfds.
    let (record, registration) = runtime
        .register_spawn(&authority, process)
        .map_err(peer_denial_to_io)?;
    if pending.commit_after_supervisor_registration() != process {
        return Err("pending-spawn guard changed its process key".into());
    }
    println!(
        "PID1_LIVE_REGISTRATION=PASS child_pid={} registration={registration:?} source=fixture-supervisor-inventory-session-consent-route",
        process.pid()
    );
    if !runtime.route_mismatch_is_denied_for_spawn(&authority, registration) {
        return Err("public adapter did not reject route mismatch for a live registration".into());
    }
    println!("ROUTE_MISMATCH_NEGATIVE=PASS live-registration fixture-authorities");
    println!(
        "CHILD_LIVE_AFTER_ROUTE_NEGATIVE={}",
        authority
            .confirms_owned_process_live(process)
            .map_err(peer_denial_to_io)?
    );

    let mut peer = runtime
        .authorize_child(&authority, registration)
        .map_err(peer_denial_to_io)?;
    println!(
        "PUBLIC_ADAPTER_KERNEL_AUTH=PASS source=SO_PEERCRED+SO_COOKIE+SO_PEERPIDFD+spawn-pidfd fixture=other-authorities"
    );
    let operation_mapper = fixture_operation_mapper(peer.route())?;
    println!("NIPC_OPERATION_MAPPER=PASS fixture-route");

    println!("FIRST_REQUEST_READ_START");
    let (first_request, _) = read_nipc_request(&mut peer)?;
    println!("FIRST_REQUEST_READ=PASS");
    let decoded_wire_id = first_request.wire_request_id().as_bytes();
    let decoded_source = first_request.source_app_id();
    let decoded_payload_type = first_request.payload_type().to_owned();
    let decoded_body_len = first_request.payload().len();
    let first_binding = operation_mapper
        .bind_request(&first_request)
        .map_err(peer_denial_to_io)?;
    let first_admission = peer
        .admit_nipc_request(first_request, first_binding)
        .map_err(peer_denial_to_io)?;
    println!(
        "NIPC_REQUEST_DECODE=PASS request_id={decoded_wire_id:?} source_app_id={decoded_source} payload_type={decoded_payload_type} body_bytes={decoded_body_len} fingerprint={:?}",
        first_admission.fingerprint().as_bytes()
    );
    let effect_owner = runtime.new_effect_owner(first_admission.clone());
    if !runtime.one_shot_concurrent_commit(&peer, &first_admission, &effect_owner) {
        return Err(
            "same-request concurrent commit was not exactly one success plus one denial".into(),
        );
    }
    if !runtime.replay_is_denied(&peer, &first_admission.clone(), &effect_owner) {
        return Err("cloned/replayed request admission committed twice".into());
    }
    println!(
        "SERVICE_REQUEST_ONE_SHOT=PASS request_id={} concurrent=one-success-one-denial replay=denied fixture=kernel-peer+test-owner",
        first_admission.request_id()
    );

    // Complete the second child's request frame on its own accepted stream.
    // The partial-I/O request below uses a third stream and a different stable
    // identity; it never reuses the first connection's request slot.
    println!("FIRST_REQUEST_ACK_START");
    peer.with_live_io(|io| io.write_all(b"R"))
        .map_err(peer_denial_to_io)??;
    println!("FIRST_REQUEST_ACK=PASS");

    let mut partial_peer = runtime
        .authorize_child(&authority, registration)
        .map_err(peer_denial_to_io)?;
    println!("PARTIAL_PEER_AUTH=PASS");
    println!("PARTIAL_REQUEST_READ_START");
    let (partial_request, saw_would_block) = read_nipc_request(&mut partial_peer)?;
    println!("PARTIAL_REQUEST_READ=PASS");
    if !saw_would_block {
        return Err("partial NIPC frame did not exercise WouldBlock".into());
    }
    let partial_binding = operation_mapper
        .bind_request(&partial_request)
        .map_err(peer_denial_to_io)?;
    let partial_admission = partial_peer
        .admit_nipc_request(partial_request, partial_binding)
        .map_err(peer_denial_to_io)?;
    println!("NIPC_PARTIAL_FRAME=PASS same_decoder=retained fixed_deadline=true");
    if partial_admission.request_id() == first_admission.request_id() {
        return Err("separate request admissions reused a request ID".into());
    }
    let partial_owner = runtime.new_effect_owner(partial_admission.clone());
    let sent = runtime
        .send_partial_request(&mut partial_peer, &partial_admission, &partial_owner)
        .map_err(peer_denial_to_io)?;
    if sent == 0 {
        return Err("partial-write fixture sent no bytes".into());
    }
    if !runtime.partial_write_revoke_denies_commit(
        &partial_peer,
        &partial_admission,
        &partial_owner,
    ) {
        return Err(
            "owner revoke after partial write did not deny the effect commit as not-invoked".into(),
        );
    }
    println!(
        "PARTIAL_WRITE_REVOKE=PASS task39_deadline_hook=active kernel_accepted_bytes={sent} post_revoke_io=denied post_revoke_commit=not-invoked rollback=not-claimed sent_byte_cancel=not-claimed fixture=kernel-peer+test-owner"
    );

    let mut stale_peer = runtime
        .authorize_child(&authority, registration)
        .map_err(peer_denial_to_io)?;
    println!("STALE_PEER_AUTH=PASS");
    println!("STALE_REQUEST_READ_START");
    let (stale_request, _) = read_nipc_request(&mut stale_peer)?;
    println!("STALE_REQUEST_READ=PASS");
    let stale_binding = operation_mapper
        .bind_request(&stale_request)
        .map_err(peer_denial_to_io)?;
    let stale_admission = stale_peer
        .admit_nipc_request(stale_request, stale_binding)
        .map_err(peer_denial_to_io)?;
    let stale_owner = runtime.new_effect_owner(stale_admission.clone());
    if !runtime.stale_during_request_io_denies_success(
        &mut stale_peer,
        &stale_admission,
        &stale_owner,
    ) {
        return Err("request I/O returned success after the connection stale latch was set".into());
    }
    println!("REQUEST_IO_STALE_DURING_CALLBACK=PASS fixture=kernel-peer+test-owner");

    runtime
        .wait_for_exit(&authority, process, Duration::from_secs(10))
        .map_err(peer_denial_to_io)?;
    if !matches!(
        peer.with_live_io(|_| ()),
        Err(supplier::PeerDenial::StalePeer)
    ) {
        return Err("exited child retained an authorized live I/O path".into());
    }
    println!("EXITED_CHILD_DENIED=PASS source=spawn-pidfd-liveness");
    drop(partial_peer);
    drop(stale_peer);
    drop(peer);
    runtime
        .retire_after_exit(&authority, record, process, registration)
        .map_err(peer_denial_to_io)?;
    reap_direct_child(process.pid())?;
    println!(
        "REGISTERED_CHILD_EXIT_REVOKE_REAP=PASS child_pid={} supervisor=revoked directory=retired pidfd=retired",
        process.pid()
    );

    // Exercise the failure path while a second task-owned child is still live:
    // dropping an unregistered spawn must signal via pidfd and waitid(P_PIDFD).
    let pending_abort =
        match unsafe { authority.clone3_with_pidfd(libc::CLONE_NEWPID as libc::c_ulonglong) }? {
            Clone3PidfdOutcome::Parent(child) => authority
                .capture_clone3_child(child)
                .map_err(peer_denial_to_io)?,
            Clone3PidfdOutcome::Child => child_connect_and_wait(&path),
        };
    let abort_process = pending_abort.process();
    if !authority
        .confirms_owned_process_live(abort_process)
        .map_err(peer_denial_to_io)?
    {
        return Err("cleanup child exited before the live-child abort".into());
    }
    pending_abort.abort().map_err(peer_denial_to_io)?;
    if authority.confirms_owned_process_live(abort_process).is_ok() {
        return Err("live-child abort retained a PID 1 pidfd entry".into());
    }
    println!(
        "UNREGISTERED_LIVE_CHILD_ABORT=PASS child_pid={} signal=pidfd wait=waitid(P_PIDFD)",
        abort_process.pid()
    );

    drop(runtime);
    std::fs::remove_file(&path)?;
    println!(
        "PIDFD_RUNTIME_FIXTURE=PASS kernel=Linux clone3=real pidfd=real adapter=fixture-owner"
    );
    guest_poweroff()?;
    Ok(())
}

fn guest_poweroff() -> Result<(), io::Error> {
    // The fixture is guest PID 1. Shut down QEMU through the Linux reboot ABI
    // so a successful run does not turn into an expected PID-1-exited panic.
    let result = unsafe {
        libc::syscall(
            libc::SYS_reboot,
            libc::LINUX_REBOOT_MAGIC1,
            libc::LINUX_REBOOT_MAGIC2,
            libc::RB_POWER_OFF,
            0,
        )
    };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    Err(io::Error::other(
        "Linux poweroff syscall unexpectedly returned",
    ))
}

fn peer_denial_to_io(error: supplier::PeerDenial) -> io::Error {
    io::Error::other(format!("pidfd adapter denied: {error:?}"))
}

fn check_clone3_errno_negative() -> Result<(), Box<dyn std::error::Error>> {
    const INVALID_FLAG: libc::c_ulonglong = 1u64 << 63;
    let authority = PidfdProcessInstanceAuthority::new(LinuxPidfdKernel);
    let outcome = unsafe { authority.clone3_with_pidfd(INVALID_FLAG) };
    let result = match outcome {
        Err(error) if error.raw_os_error() == Some(libc::EINVAL) => {
            println!("CLONE3_ERRNO_NEGATIVE=PASS errno=EINVAL");
            Ok(())
        }
        Err(error) if error.raw_os_error() == Some(libc::ENOSYS) => {
            println!("CLONE3_UNAVAILABLE=errno=ENOSYS");
            Err("clone3 syscall unavailable in guest kernel".into())
        }
        Err(error) => Err(format!("unexpected clone3 invalid-flag errno: {error}").into()),
        Ok(Clone3PidfdOutcome::Parent(child)) => {
            drop(child);
            Err("invalid clone3 flag unexpectedly created a child".into())
        }
        Ok(Clone3PidfdOutcome::Child) => child_exit(120),
    };
    result
}

fn check_clone3_unavailable_negative() -> Result<(), Box<dyn std::error::Error>> {
    // A child-local seccomp filter makes clone3 report ENOSYS without changing
    // the parent or guest policy. This checks the unavailable-syscall error
    // path on this kernel, where clone3 itself is available.
    let probe = unsafe { libc::fork() };
    if probe < 0 {
        return Err(io::Error::last_os_error().into());
    }
    if probe == 0 {
        seccomp_clone3_enosys_child();
    }
    let mut status = 0;
    loop {
        let result = unsafe { libc::waitpid(probe, &mut status, 0) };
        if result == probe {
            break;
        }
        let source = io::Error::last_os_error();
        if source.raw_os_error() != Some(libc::EINTR) {
            return Err(source.into());
        }
    }
    if status != 0 {
        return Err(format!("seccomp clone3 probe exited with wait status {status:#x}").into());
    }
    println!("CLONE3_UNAVAILABLE_NEGATIVE=PASS errno=ENOSYS source=child-seccomp-filter");
    Ok(())
}

fn seccomp_clone3_enosys_child() -> ! {
    let no_new_privs = unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
    if no_new_privs < 0 {
        unsafe { libc::_exit(122) }
    }

    let mut filter = [
        libc::sock_filter {
            code: (libc::BPF_LD | libc::BPF_W | libc::BPF_ABS) as u16,
            jt: 0,
            jf: 0,
            k: 0,
        },
        libc::sock_filter {
            code: (libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K) as u16,
            jt: 0,
            jf: 1,
            k: libc::SYS_clone3 as u32,
        },
        libc::sock_filter {
            code: (libc::BPF_RET | libc::BPF_K) as u16,
            jt: 0,
            jf: 0,
            k: libc::SECCOMP_RET_ERRNO | libc::ENOSYS as u32,
        },
        libc::sock_filter {
            code: (libc::BPF_RET | libc::BPF_K) as u16,
            jt: 0,
            jf: 0,
            k: libc::SECCOMP_RET_ALLOW,
        },
    ];
    let mut program = libc::sock_fprog {
        len: filter.len() as libc::c_ushort,
        filter: filter.as_mut_ptr(),
    };
    let installed = unsafe {
        libc::prctl(
            libc::PR_SET_SECCOMP,
            libc::SECCOMP_MODE_FILTER,
            &mut program as *mut libc::sock_fprog,
            0,
            0,
        )
    };
    if installed < 0 {
        unsafe { libc::_exit(123) }
    }

    let authority = PidfdProcessInstanceAuthority::new(LinuxPidfdKernel);
    let code = match unsafe { authority.clone3_with_pidfd(0) } {
        Err(error) if error.raw_os_error() == Some(libc::ENOSYS) => 0,
        Err(_) => 124,
        Ok(Clone3PidfdOutcome::Parent(child)) => {
            drop(child);
            125
        }
        Ok(Clone3PidfdOutcome::Child) => 126,
    };
    unsafe { libc::_exit(code) }
}

fn check_peer_pidfd_errno_negative() -> Result<(), Box<dyn std::error::Error>> {
    let raw = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    if raw < 0 {
        return Err(io::Error::last_os_error().into());
    }
    // SAFETY: `raw` is a newly created, unconnected AF_UNIX stream socket.
    let unconnected = unsafe { UnixStream::from_raw_fd(raw) };
    match LinuxPidfdKernel.peer_pidfd(&unconnected) {
        Err(supplier::ProcessInstanceError::KernelRead { source, .. }) => {
            let errno = source.raw_os_error().unwrap_or_default();
            println!("SO_PEERPIDFD_ERRNO_NEGATIVE=PASS errno={errno}");
            Ok(())
        }
        Err(supplier::ProcessInstanceError::Unavailable) => {
            println!("SO_PEERPIDFD_UNAVAILABLE=typed-unavailable");
            Ok(())
        }
        Err(error) => Err(format!("unexpected SO_PEERPIDFD error: {error}").into()),
        Ok(pidfd) => {
            drop(pidfd);
            Err("unconnected socket unexpectedly returned a peer pidfd".into())
        }
    }
}

fn child_connect_and_wait(path: &std::path::Path) -> ! {
    let result = (|| -> io::Result<()> {
        // Route mismatch is rejected before an accepted-stream checkout, so
        // do not queue a placeholder connection that could be mistaken for
        // the next valid endpoint. These three streams are respectively the
        // decoded request, partial-write request, and stale-latch request.
        let mut stream = UnixStream::connect(path)?;
        let mut partial_stream = UnixStream::connect(path)?;
        let mut stale_stream = UnixStream::connect(path)?;
        stream.write_all(&encoded_request([1; 16])?)?;
        let partial_request = encoded_request([2; 16])?;
        partial_stream.write_all(&partial_request[..50])?;
        std::thread::sleep(Duration::from_millis(40));
        partial_stream.write_all(&partial_request[50..])?;
        stale_stream.write_all(&encoded_request([3; 16])?)?;
        child_serial_marker(b"CHILD_EXACT_ROUTE_QUEUED=PASS\n");

        let mut release = [0u8; 1];
        stream.read_exact(&mut release)?;
        stream.write_all(b"CONSUMED")?;
        let mut partial_release = [0u8; 1];
        partial_stream.read_exact(&mut partial_release)?;
        partial_stream.write_all(b"CONSUMED")?;
        let mut stale_release = [0u8; 1];
        stale_stream.read_exact(&mut stale_release)?;
        if stale_release != [b'S'] {
            return Err(io::Error::other(
                "stale-connection probe byte did not match",
            ));
        }
        child_serial_marker(b"CHILD_STALE_IO_PROBE=PASS\n");
        Ok(())
    })();
    unsafe { libc::_exit(if result.is_ok() { 0 } else { 121 }) }
}

fn read_nipc_request(
    peer: &mut supplier::AuthorizedPeer<'_, impl supplier::LaunchRecordView>,
) -> Result<(supplier::DecodedNipcRequest, bool), Box<dyn std::error::Error>> {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut decoder = peer
        .new_nipc_request_decoder(deadline)
        .map_err(peer_denial_to_io)?;
    let mut saw_would_block = false;
    loop {
        let result = peer.read_nipc_request(&mut decoder);
        if let Err(error) = &result {
            eprintln!("NIPC_FRAME_READ_DENIED={error:?}");
        }
        match result.map_err(peer_denial_to_io)? {
            supplier::NipcRequestRead::Complete(request) => return Ok((request, saw_would_block)),
            supplier::NipcRequestRead::Pending { .. } => {
                saw_would_block = true;
                std::thread::sleep(Duration::from_millis(2));
            }
        }
    }
}

fn encoded_request(request_id: [u8; 16]) -> io::Result<Vec<u8>> {
    let envelope = IpcEnvelope {
        protocol_version: IPC_PROTOCOL_VERSION,
        source: AppId(7),
        destination: Some(AppId(1)),
        kind: MessageKind::Request,
        correlation_id: CorrelationId([4; 16]),
        request_id: Some(RequestId(request_id)),
        ordering: OrderingMetadata {
            timestamp_micros: None,
            sequence: None,
        },
        payload_type: "svc.fixture@1",
        payload: b"test request body",
    };
    let mut bytes = vec![0; 78 + envelope.payload_type.len() + envelope.payload.len()];
    let length = envelope
        .encode(&mut bytes)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid fixture envelope"))?;
    bytes.truncate(length);
    Ok(bytes)
}

fn fixture_operation_mapper(
    route: supplier::TrustedRouteKey,
) -> Result<supplier::CanonicalNipcOperationMapper, Box<dyn std::error::Error>> {
    let service_id = ServiceId::new("fixture.files")?;
    let contract_version = ContractVersion::new(1, 0)?;
    let operation_id = OperationId::new("read")?;
    let capability = CapabilityId::new("files.read")?;
    let descriptor = ServiceDescriptor::new(
        service_id,
        [contract_version],
        [ServiceOperation::new(
            operation_id.clone(),
            Some(capability),
        )],
    )?;
    let route = supplier::TrustedNipcOperationRoute::from_service_contract(
        route,
        1,
        "svc.fixture@1",
        &descriptor,
        contract_version,
        &operation_id,
        // Test-only fixture identity. Production route construction remains
        // unavailable until the canonical owner supplies this value.
        ServiceEffectOwnerId::new([0x41; 16]),
    )
    .map_err(peer_denial_to_io)?;
    Ok(
        supplier::CanonicalNipcOperationMapper::from_trusted_routes([route])
            .map_err(peer_denial_to_io)?,
    )
}

fn child_serial_marker(marker: &[u8]) {
    // SAFETY: the inherited serial stdout descriptor is open in the child and
    // the slice remains valid for this synchronous write.
    unsafe {
        let _ = libc::write(libc::STDOUT_FILENO, marker.as_ptr().cast(), marker.len());
    }
}

fn reap_direct_child(pid: u32) -> io::Result<()> {
    let mut status = 0;
    loop {
        let result = unsafe { libc::waitpid(pid as libc::pid_t, &mut status, 0) };
        if result == pid as libc::pid_t {
            break;
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EINTR) {
            return Err(error);
        }
    }
    if !libc::WIFEXITED(status) || libc::WEXITSTATUS(status) != 0 {
        return Err(io::Error::other(format!(
            "owned fixture child exited with wait status {status:#x}"
        )));
    }
    Ok(())
}

fn child_exit(code: i32) -> ! {
    unsafe { libc::_exit(code) }
}
