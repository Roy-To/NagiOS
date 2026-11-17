use linux_peer_credentials_supplier::{
    read_kernel_peer_credentials, KernelPeerCredentials, OwnedPeerConnection, PeerConnectionError,
    PeerCredentialError,
};
use std::fs;
use std::io::{Read, Write};
use std::mem::{size_of, MaybeUninit};
use std::net::Shutdown;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::thread;
use std::time::{Duration, Instant};

const CHILD_STOP_BYTE: u8 = 0x5a;

struct OwnedChild {
    process: Child,
    reaped: bool,
}

impl OwnedChild {
    fn spawn(socket_path: &Path) -> Result<Self, String> {
        let process = Command::new("/init")
            .arg("--peer-child")
            .arg(socket_path)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("spawn-owned-child: {error}"))?;
        Ok(Self {
            process,
            reaped: false,
        })
    }

    fn id(&self) -> u32 {
        self.process.id()
    }

    fn try_wait(&mut self) -> Result<Option<ExitStatus>, String> {
        self.process
            .try_wait()
            .map_err(|error| format!("poll-owned-child: {error}"))
    }

    fn wait_for_exit(&mut self, timeout: Duration) -> Result<ExitStatus, String> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Some(status) = self.try_wait()? {
                self.reaped = true;
                return Ok(status);
            }
            thread::sleep(Duration::from_millis(10));
        }
        Err("owned child did not exit after the stop byte".to_owned())
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        if self.reaped {
            return;
        }
        if self.process.try_wait().ok().flatten().is_none() {
            let _ = self.process.kill();
        }
        let _ = self.process.wait();
        self.reaped = true;
    }
}

fn child_mode(socket_path: &Path) -> Result<(), String> {
    let _first = UnixStream::connect(socket_path)
        .map_err(|error| format!("child-first-connect: {error}"))?;
    let mut second = UnixStream::connect(socket_path)
        .map_err(|error| format!("child-second-connect: {error}"))?;
    let mut stop = [0u8; 1];
    second
        .read_exact(&mut stop)
        .map_err(|error| format!("child-control-read: {error}"))?;
    if stop[0] != CHILD_STOP_BYTE {
        return Err("child-control-byte-mismatch".to_owned());
    }
    Ok(())
}

fn accept_owned_child_connection(
    listener: &UnixListener,
    child: &mut OwnedChild,
    label: &str,
) -> Result<UnixStream, String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match listener.accept() {
            Ok((stream, _address)) => return Ok(stream),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
            Err(error) => return Err(format!("accept-{label}: {error}")),
        }
        if let Some(status) = child.try_wait()? {
            return Err(format!(
                "owned child exited before {label} accept: {status}"
            ));
        }
        if Instant::now() >= deadline {
            return Err(format!("timeout accepting {label} owned-child connection"));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn local_socket_cookie(stream: &UnixStream) -> Result<u64, String> {
    let mut cookie = 0u64;
    let mut length = size_of::<u64>() as libc::socklen_t;
    raw_getsockopt(
        stream.as_raw_fd(),
        libc::SO_COOKIE,
        (&mut cookie as *mut u64).cast(),
        &mut length,
    )
    .map_err(|error| format!("SO_COOKIE read failed: {error}"))?;
    if length as usize != size_of::<u64>() {
        return Err(format!("SO_COOKIE returned malformed length {length}"));
    }
    Ok(cookie)
}

fn raw_getsockopt(
    fd: libc::c_int,
    option: libc::c_int,
    value: *mut libc::c_void,
    length: &mut libc::socklen_t,
) -> Result<(), std::io::Error> {
    loop {
        // SAFETY: the caller provides a writable value buffer and length for
        // the selected socket option; EINTR retries reuse initialized storage.
        let result = unsafe { libc::getsockopt(fd, libc::SOL_SOCKET, option, value, length) };
        if result == 0 {
            return Ok(());
        }
        let source = std::io::Error::last_os_error();
        if source.raw_os_error() == Some(libc::EINTR) {
            continue;
        }
        return Err(source);
    }
}

fn raw_peer_credentials(
    fd: libc::c_int,
    requested_length: libc::socklen_t,
) -> Result<libc::socklen_t, std::io::Error> {
    let mut credentials = MaybeUninit::<libc::ucred>::zeroed();
    let mut length = requested_length;
    // SAFETY: zeroed ucred storage is large enough for SO_PEERCRED; the caller
    // chooses an optlen no greater than that storage for the short-length probe.
    raw_getsockopt(
        fd,
        libc::SO_PEERCRED,
        credentials.as_mut_ptr().cast(),
        &mut length,
    )?;
    Ok(length)
}

fn probe_bad_descriptors(connected: &UnixStream) -> Result<(), String> {
    let invalid_error =
        raw_peer_credentials(-1, size_of::<libc::ucred>() as libc::socklen_t).unwrap_err();
    if invalid_error.raw_os_error() != Some(libc::EBADF) {
        return Err(format!(
            "invalid fd returned unexpected errno: {invalid_error}"
        ));
    }
    println!("NEGATIVE invalid_fd=PASS errno=EBADF");

    let mut pipe_fds = [-1; 2];
    // SAFETY: pipe_fds is a writable two-fd output array.
    if unsafe { libc::pipe(pipe_fds.as_mut_ptr()) } != 0 {
        return Err(format!(
            "pipe setup failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    let non_socket_error =
        raw_peer_credentials(pipe_fds[0], size_of::<libc::ucred>() as libc::socklen_t).unwrap_err();
    // SAFETY: both descriptors are the exact descriptors returned by pipe().
    unsafe {
        libc::close(pipe_fds[0]);
        libc::close(pipe_fds[1]);
    }
    if non_socket_error.raw_os_error() != Some(libc::ENOTSOCK) {
        return Err(format!(
            "pipe fd SO_PEERCRED outcome was errno={non_socket_error}"
        ));
    }
    println!("NEGATIVE non_socket=PASS errno=ENOTSOCK");

    // SAFETY: socket() returns a new AF_UNIX stream fd owned below by
    // UnixStream::from_raw_fd. It is intentionally left unconnected.
    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC, 0) };
    if fd < 0 {
        return Err(format!(
            "unconnected socket setup failed: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: fd is a valid, newly-created AF_UNIX SOCK_STREAM descriptor and
    // ownership transfers to the UnixStream exactly once.
    let unconnected = unsafe { UnixStream::from_raw_fd(fd) };
    match read_kernel_peer_credentials(&unconnected) {
        Err(PeerCredentialError::NotConnected { .. }) => {
            println!("NEGATIVE unconnected_unix_stream=PASS outcome=NotConnected");
        }
        other => {
            return Err(format!("unconnected stream outcome was {other:?}"));
        }
    }

    let mut short_buffer = [0xa5u8; size_of::<libc::ucred>()];
    let before = short_buffer;
    let mut short_length = 1 as libc::socklen_t;
    let short_result = raw_getsockopt(
        connected.as_raw_fd(),
        libc::SO_PEERCRED,
        short_buffer.as_mut_ptr().cast(),
        &mut short_length,
    );
    if short_buffer[1..] != before[1..] {
        return Err("short SO_PEERCRED optlen changed bytes beyond its buffer".to_owned());
    }
    if short_result.is_ok() && short_length as usize > size_of::<libc::ucred>() {
        return Err(format!(
            "short SO_PEERCRED probe returned invalid length {short_length}"
        ));
    }
    println!(
        "NEGATIVE short_optlen=PASS syscall_result={:?} returned_len={short_length} canary=unchanged",
        short_result
            .as_ref()
            .map(|_| 0)
            .map_err(|error| error.raw_os_error())
    );
    Ok(())
}

fn verify_live_peer(
    first: &UnixStream,
    second: &UnixStream,
    credentials: &KernelPeerCredentials,
    second_credentials: &KernelPeerCredentials,
    child_pid: u32,
) -> Result<(), String> {
    let expected_pid = libc::pid_t::try_from(child_pid)
        .map_err(|_| format!("child PID is outside pid_t range: {child_pid}"))?;
    let expected_uid = unsafe { libc::getuid() };
    let expected_gid = unsafe { libc::getgid() };
    if credentials.pid() != expected_pid {
        return Err(format!(
            "SO_PEERCRED PID mismatch: got {} expected {expected_pid}",
            credentials.pid()
        ));
    }
    if credentials.uid() != expected_uid || credentials.gid() != expected_gid {
        return Err(format!(
            "SO_PEERCRED UID/GID mismatch: got {}/{} expected {expected_uid}/{expected_gid}",
            credentials.uid(),
            credentials.gid()
        ));
    }
    if second_credentials.pid() != credentials.pid()
        || second_credentials.uid() != credentials.uid()
        || second_credentials.gid() != credentials.gid()
    {
        return Err("same-child credential values differed across connections".to_owned());
    }

    credentials
        .require_same_connection(first)
        .map_err(|error| format!("same-connection check failed: {error:?}"))?;
    let first_cookie = local_socket_cookie(first)?;
    let first_cookie_repeat = local_socket_cookie(first)?;
    let second_cookie = local_socket_cookie(second)?;
    if first_cookie != first_cookie_repeat {
        return Err("SO_COOKIE changed on the same local socket".to_owned());
    }
    if first_cookie == second_cookie {
        return Err("distinct accepted sockets returned the same SO_COOKIE".to_owned());
    }
    match credentials.require_same_connection(second) {
        Err(PeerCredentialError::ConnectionMismatch) => {
            println!("NEGATIVE connection_mismatch=PASS");
        }
        other => return Err(format!("wrong-connection outcome was {other:?}")),
    }
    println!(
        "POSITIVE live_child_peer=PASS pid={} uid={} gid={} cookie_first={first_cookie:016x} cookie_second={second_cookie:016x} cookie_repeat=stable",
        credentials.pid(),
        credentials.uid(),
        credentials.gid()
    );
    Ok(())
}

fn verify_owned_socket_lifetime() -> Result<(), String> {
    let (stream, peer) = UnixStream::pair().map_err(|error| format!("socketpair-old: {error}"))?;
    let mut escaped_alias = stream
        .try_clone()
        .map_err(|error| format!("preexisting-fd-alias: {error}"))?;
    let old_cookie = local_socket_cookie(&stream)?;
    let mut owned = OwnedPeerConnection::from_connected(stream)
        .map_err(|error| format!("owned-old-connection: {error:?}"))?;
    let current_pid = unsafe { libc::getpid() };
    owned
        .with_connection_io_checked(
            || Ok(()),
            |_io, credentials| {
                if credentials.pid() != current_pid {
                    return Err(format!(
                        "socketpair SO_PEERCRED PID {} expected owner PID {current_pid}",
                        credentials.pid()
                    ));
                }
                Ok(())
            },
        )
        .map_err(|error| format!("owned live check: {error:?}"))??;

    let (wrong_stream, wrong_peer) =
        UnixStream::pair().map_err(|error| format!("socketpair-wrong-fd: {error}"))?;
    match owned.require_same_socket(&wrong_stream) {
        Err(PeerConnectionError::CredentialRead(PeerCredentialError::ConnectionMismatch)) => {
            println!("NEGATIVE socketpair_wrong_fd=PASS");
        }
        other => return Err(format!("wrong FD outcome was {other:?}")),
    }
    drop(wrong_stream);
    drop(wrong_peer);

    drop(peer);
    let mut disconnected_callback_ran = false;
    match owned.with_connection_io_checked(|| Ok(()), |_, _| disconnected_callback_ran = true) {
        Err(PeerConnectionError::PeerDisconnected) => {
            println!("NEGATIVE socketpair_disconnect=PASS callback_skipped=true");
        }
        other => return Err(format!("disconnect outcome was {other:?}")),
    }
    if disconnected_callback_ran {
        return Err("disconnected snapshot reached its callback".to_owned());
    }
    if !matches!(
        owned.with_connection_io_checked(|| Ok(()), |_, _| ()),
        Err(PeerConnectionError::Stale)
    ) {
        return Err("observed closed connection was not latched stale".to_owned());
    }
    let alias_cookie = local_socket_cookie(&escaped_alias)?;
    let mut alias_byte = [0u8; 1];
    let alias_read = escaped_alias
        .read(&mut alias_byte)
        .map_err(|error| format!("preexisting alias peek: {error}"))?;
    if alias_cookie != old_cookie || alias_read != 0 {
        return Err("preexisting FD alias did not remain independently observable".to_owned());
    }
    println!(
        "BOUNDARY preexisting_fd_alias_after_invalidation=PASS cookie_still_readable=true peer_eof=true revocation_claim=none"
    );

    let (reconnected_stream, reconnected_peer) =
        UnixStream::pair().map_err(|error| format!("socketpair-reconnect: {error}"))?;
    let new_cookie = local_socket_cookie(&reconnected_stream)?;
    if old_cookie == new_cookie {
        return Err("reconnected socket reused the old SO_COOKIE".to_owned());
    }
    if !matches!(
        owned.require_same_socket(&reconnected_stream),
        Err(PeerConnectionError::Stale)
    ) {
        return Err("stale adapter accepted a reconnected stream".to_owned());
    }
    let mut reconnected = OwnedPeerConnection::from_connected(reconnected_stream)
        .map_err(|error| format!("owned-reconnect: {error:?}"))?;
    reconnected
        .with_connection_io_checked(|| Ok(()), |_, credentials| credentials.pid() == current_pid)
        .map_err(|error| format!("reconnected live check: {error:?}"))?;
    println!(
        "POSITIVE socketpair_reconnect=PASS old_cookie={old_cookie:016x} new_cookie={new_cookie:016x} snapshot=new-adapter"
    );
    drop(reconnected);
    drop(reconnected_peer);
    drop(owned);
    Ok(())
}

fn verify_buffered_eof_delay() -> Result<(), String> {
    let (stream, mut peer) =
        UnixStream::pair().map_err(|error| format!("socketpair-buffered-eof: {error}"))?;
    let mut owned = OwnedPeerConnection::from_connected(stream)
        .map_err(|error| format!("owned-buffered-connection: {error:?}"))?;
    peer.write_all(&[0xa5])
        .map_err(|error| format!("write-buffered-peer-byte: {error}"))?;
    drop(peer);

    match owned.with_connection_io_checked(|| Ok(()), |_, _| ()) {
        Ok(()) => println!(
            "BOUNDARY buffered_data_masks_eof=PASS peer_closed_before_check=true data_still_queued=true no_auth_inference=true"
        ),
        Err(error) => {
            return Err(format!("pending-data probe did not defer EOF: {error:?}"));
        }
    }

    let mut consumed = 0u8;
    match owned.with_connection_io_checked(
        || Ok(()),
        |io, _| io.read_exact(std::slice::from_mut(&mut consumed)),
    ) {
        Err(PeerConnectionError::PeerDisconnected) if consumed == 0xa5 => {
            println!(
                "NEGATIVE buffered_data_drain=PASS consumed=1 postcheck=PeerDisconnected callback_result_discarded=true"
            );
        }
        other => {
            return Err(format!(
                "buffered-data EOF outcome was {other:?}, byte={consumed}"
            ))
        }
    }
    if !matches!(
        owned.with_connection_io_checked(|| Ok(()), |_, _| ()),
        Err(PeerConnectionError::Stale)
    ) {
        return Err("buffered-data disconnect was not latched stale".to_owned());
    }
    drop(owned);
    Ok(())
}

fn verify_partial_stream_io_and_would_block() -> Result<(), String> {
    let (stream, mut peer) =
        UnixStream::pair().map_err(|error| format!("socketpair-partial-read: {error}"))?;
    peer.write_all(b"ab")
        .map_err(|error| format!("write-partial-read-input: {error}"))?;
    let mut owned = OwnedPeerConnection::from_connected(stream)
        .map_err(|error| format!("owned-partial-read-connection: {error:?}"))?;
    let mut operation_hook_checks = 0usize;
    let operation = owned
        .with_nonblocking_io_until(
            Instant::now() + Duration::from_secs(2),
            || {
                operation_hook_checks += 1;
                Ok(())
            },
            |io, _| -> std::io::Result<(usize, [u8; 8], usize)> {
                let mut input = [0u8; 8];
                let read = io.read(&mut input)?;
                let written = io.write(b"response")?;
                io.flush()?;
                Ok((read, input, written))
            },
        )
        .map_err(|error| format!("partial stream I/O guard: {error:?}"))?
        .map_err(|error| format!("partial stream I/O: {error}"))?;
    if operation.0 == 0 || operation.0 >= operation.1.len() || operation.1[..operation.0] != *b"ab"
    {
        return Err(format!(
            "read did not return a partial prefix: {operation:?}"
        ));
    }
    if operation.2 != b"response".len() {
        return Err(format!(
            "small response write was unexpectedly partial: {operation:?}"
        ));
    }
    if operation_hook_checks != 8 {
        return Err(format!(
            "read/write/flush hook count was {operation_hook_checks}, expected 8"
        ));
    }
    let mut response = [0u8; 8];
    peer.read_exact(&mut response)
        .map_err(|error| format!("read-partial-test-response: {error}"))?;
    if &response != b"response" {
        return Err("partial stream I/O response mismatch".to_owned());
    }
    println!(
        "NEGATIVE partial_read_write_flush=PASS read_count={} requested={} authority_hook_checks={operation_hook_checks}",
        operation.0,
        operation.1.len()
    );

    let (stream, peer) =
        UnixStream::pair().map_err(|error| format!("socketpair-partial-write: {error}"))?;
    let mut owned = OwnedPeerConnection::from_connected(stream)
        .map_err(|error| format!("owned-partial-write-connection: {error:?}"))?;
    let buffer = vec![0x5a; 8 * 1024 * 1024];
    let mut write_hook_checks = 0usize;
    let (written, saw_partial, saw_would_block, attempts) = owned
        .with_nonblocking_io_until(
            Instant::now() + Duration::from_secs(2),
            || {
                write_hook_checks += 1;
                Ok(())
            },
            |io, _| -> std::io::Result<_> {
                let mut total = 0usize;
                let mut partial = false;
                let mut would_block = false;
                let mut attempts = 0usize;
                for _ in 0..64 {
                    attempts += 1;
                    match io.write(&buffer) {
                        Ok(0) => {
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::WriteZero,
                                "zero write",
                            ))
                        }
                        Ok(count) => {
                            total += count;
                            partial |= count < buffer.len();
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            would_block = true;
                            break;
                        }
                        Err(error) => return Err(error),
                    }
                }
                Ok((total, partial, would_block, attempts))
            },
        )
        .map_err(|error| format!("partial write guard: {error:?}"))?
        .map_err(|error| format!("partial write: {error}"))?;
    if written == 0 || !saw_partial || !saw_would_block {
        return Err(format!(
            "nonblocking write did not exercise partial/WouldBlock: bytes={written} partial={saw_partial} would_block={saw_would_block}"
        ));
    }
    if write_hook_checks != 2 + attempts * 2 {
        return Err(format!(
            "write hook count={write_hook_checks} did not cover {attempts} operations"
        ));
    }
    owned
        .with_connection_io_checked(|| Ok(()), |_, _| ())
        .map_err(|error| format!("WouldBlock invalidated adapter: {error:?}"))?;
    println!(
        "NEGATIVE partial_write_would_block=PASS bytes={written} attempts={attempts} partial_write=true would_block=true authority_hook_checks={write_hook_checks} adapter_still_open=true"
    );
    drop(owned);
    drop(peer);
    Ok(())
}

fn verify_nonblocking_deadline_expiry() -> Result<(), String> {
    let (stream, mut peer) =
        UnixStream::pair().map_err(|error| format!("socketpair-deadline-expiry: {error}"))?;
    let mut owned = OwnedPeerConnection::from_connected(stream)
        .map_err(|error| format!("owned-deadline-expiry: {error:?}"))?;
    let mut authority_checks = 0usize;
    let late_write_denied = std::cell::Cell::new(false);
    let result = owned.with_nonblocking_io_until(
        Instant::now() + Duration::from_millis(250),
        || {
            authority_checks += 1;
            Ok(())
        },
        |io, _| {
            let _ = io.write(b"sent");
            thread::sleep(Duration::from_millis(300));
            late_write_denied.set(io.write(b"late").is_err());
        },
    );
    if !matches!(result, Err(PeerConnectionError::DeadlineExceeded))
        || !late_write_denied.get()
        || authority_checks != 3
    {
        return Err(format!(
            "deadline expiry result={result:?} late_write_denied={} authority_checks={authority_checks}",
            late_write_denied.get()
        ));
    }
    let mut sent = [0u8; 4];
    peer.read_exact(&mut sent)
        .map_err(|error| format!("read-completed-predeadline-write: {error}"))?;
    if &sent != b"sent" {
        return Err(format!("predeadline write changed: {sent:?}"));
    }
    peer.set_nonblocking(true)
        .map_err(|error| format!("set-peer-nonblocking-after-expiry: {error}"))?;
    let mut late = [0u8; 4];
    match peer.read(&mut late) {
        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
        other => return Err(format!("late bytes appeared after expiry: {other:?}")),
    }
    if !matches!(
        owned.with_connection_io_checked(|| Ok(()), |_, _| ()),
        Err(PeerConnectionError::Stale)
    ) {
        return Err("deadline-expired adapter allowed its next operation".to_owned());
    }
    println!(
        "NEGATIVE deadline_expiry=PASS deadline_crossed=true late_write_denied=true prior_write_preserved=true next_operation=Stale"
    );
    drop(owned);
    drop(peer);
    Ok(())
}

fn verify_half_close_and_independent_rejection() -> Result<(), String> {
    let (stream, mut peer) =
        UnixStream::pair().map_err(|error| format!("socketpair-half-close: {error}"))?;
    let mut owned = OwnedPeerConnection::from_connected(stream)
        .map_err(|error| format!("owned-half-close-connection: {error:?}"))?;
    peer.write_all(b"h")
        .map_err(|error| format!("write-half-close-data: {error}"))?;
    peer.shutdown(Shutdown::Write)
        .map_err(|error| format!("half-close-peer-write: {error}"))?;
    let mut consumed = 0u8;
    match owned.with_connection_io_checked(|| Ok(()), |io, _| {
        io.read_exact(std::slice::from_mut(&mut consumed))
    }) {
        Err(PeerConnectionError::PeerDisconnected) if consumed == b'h' => println!(
            "NEGATIVE peer_half_close=PASS queued_byte_consumed=true post_read_guard=PeerDisconnected"
        ),
        other => return Err(format!("half-close outcome={other:?} consumed={consumed}")),
    }
    drop(owned);
    drop(peer);

    let (stream, peer) =
        UnixStream::pair().map_err(|error| format!("socketpair-peer-read-half-close: {error}"))?;
    let mut owned = OwnedPeerConnection::from_connected(stream)
        .map_err(|error| format!("owned-peer-read-half-close-connection: {error:?}"))?;
    peer.shutdown(Shutdown::Read)
        .map_err(|error| format!("peer-read-half-close: {error}"))?;
    match owned.with_connection_io_checked(|| Ok(()), |io, _| io.write(b"after-read-close")) {
        Ok(Ok(count)) if count > 0 => println!(
            "BOUNDARY peer_read_half_close=PASS write_still_accepted={count} revoke_claim=none"
        ),
        Err(PeerConnectionError::IoFailure { kind, .. })
            if kind == std::io::ErrorKind::BrokenPipe
                || kind == std::io::ErrorKind::ConnectionReset =>
        {
            println!("NEGATIVE peer_read_half_close=PASS write_failed_closed=true kind={kind:?}");
        }
        other => return Err(format!("peer read half-close write outcome={other:?}")),
    }
    drop(owned);
    drop(peer);

    let (stream, mut peer) =
        UnixStream::pair().map_err(|error| format!("socketpair-authority-reject: {error}"))?;
    let mut owned = OwnedPeerConnection::from_connected(stream)
        .map_err(|error| format!("owned-authority-reject-connection: {error:?}"))?;
    let mut hook_calls = 0usize;
    let mut callback_saw_rejection = false;
    let result = owned.with_connection_io_checked(
        || {
            hook_calls += 1;
            if hook_calls == 3 {
                Err(PeerConnectionError::AuthorityRejected)
            } else {
                Ok(())
            }
        },
        |io, _| callback_saw_rejection = io.write_all(b"req").is_err(),
    );
    if !matches!(result, Err(PeerConnectionError::AuthorityRejected))
        || !callback_saw_rejection
        || hook_calls != 4
    {
        return Err(format!(
            "authority hook did not propagate between write and EOF: result={result:?} callback_saw_rejection={callback_saw_rejection} hook_calls={hook_calls}"
        ));
    }
    let mut request = [0u8; 3];
    peer.read_exact(&mut request)
        .map_err(|error| format!("read-already-written-request: {error}"))?;
    peer.write_all(b"peer-open")
        .map_err(|error| format!("verify-peer-still-open: {error}"))?;
    if &request != b"req" {
        return Err("authority rejection changed bytes already sent".to_owned());
    }
    println!(
        "NEGATIVE independent_authority_rejection=PASS eof_observed=false partial_side_effect_preserved=true callback_error_discarded=true"
    );
    drop(owned);
    drop(peer);

    let (stream, mut peer) = UnixStream::pair()
        .map_err(|error| format!("socketpair-explicit-authority-recheck: {error}"))?;
    let mut owned = OwnedPeerConnection::from_connected(stream)
        .map_err(|error| format!("owned-explicit-authority-recheck: {error:?}"))?;
    let mut hook_calls = 0usize;
    let mut callback_saw_rejection = false;
    let result = owned.with_connection_io_checked(
        || {
            hook_calls += 1;
            if hook_calls == 2 {
                Err(PeerConnectionError::AuthorityRejected)
            } else {
                Ok(())
            }
        },
        |io, _| callback_saw_rejection = io.recheck_before_side_effect().is_err(),
    );
    if !matches!(result, Err(PeerConnectionError::AuthorityRejected))
        || !callback_saw_rejection
        || hook_calls != 3
    {
        return Err(format!(
            "explicit authority recheck outcome={result:?}, callback_saw_rejection={callback_saw_rejection}, hook_calls={hook_calls}"
        ));
    }
    peer.write_all(b"still-open")
        .map_err(|error| format!("explicit-recheck-peer-still-open: {error}"))?;
    println!(
        "NEGATIVE explicit_authority_recheck=PASS eof_observed=false callback_rejection_propagated=true"
    );
    drop(owned);
    drop(peer);
    Ok(())
}

fn run_consumer() -> Result<(), String> {
    verify_owned_socket_lifetime()?;
    verify_buffered_eof_delay()?;
    verify_partial_stream_io_and_would_block()?;
    verify_nonblocking_deadline_expiry()?;
    verify_half_close_and_independent_rejection()?;

    let parent_pid = unsafe { libc::getpid() };
    let socket_path = format!("/tmp/nagi-peercred-{parent_pid}.sock");
    if Path::new(&socket_path).exists() {
        return Err(format!("refusing pre-existing socket path {socket_path}"));
    }
    let listener =
        UnixListener::bind(&socket_path).map_err(|error| format!("bind-listener: {error}"))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| format!("set-listener-nonblocking: {error}"))?;
    let mut child = OwnedChild::spawn(Path::new(&socket_path))?;
    let child_pid = child.id();
    println!("CHILD_STARTED owned_pid={child_pid}");

    let first = accept_owned_child_connection(&listener, &mut child, "first")?;
    let second = accept_owned_child_connection(&listener, &mut child, "second")?;
    let credentials = read_kernel_peer_credentials(&first)
        .map_err(|error| format!("candidate-reader: {error:?}"))?;
    let second_credentials = read_kernel_peer_credentials(&second)
        .map_err(|error| format!("candidate-reader-second-connection: {error:?}"))?;
    verify_live_peer(
        &first,
        &second,
        &credentials,
        &second_credentials,
        child_pid,
    )?;

    probe_bad_descriptors(&first)?;

    let mut child_control = &second;
    child_control
        .write_all(&[CHILD_STOP_BYTE])
        .map_err(|error| format!("send-owned-child-stop: {error}"))?;
    let child_status = child.wait_for_exit(Duration::from_secs(5))?;
    if !child_status.success() {
        return Err(format!("owned child exit was {child_status}"));
    }
    println!("CHILD_REAPED owned_pid={child_pid} exit=0");

    let mut probe = [0u8; 1];
    let eof = first
        .try_clone()
        .and_then(|mut stream| stream.read(&mut probe))
        .map_err(|error| format!("read-after-child-exit: {error}"))?
        == 0;
    if !eof {
        return Err("peer stream did not report EOF after owned child exit".to_owned());
    }
    match read_kernel_peer_credentials(&first) {
        Ok(stale_snapshot) if stale_snapshot.pid() == child_pid as libc::pid_t => println!(
            "NEGATIVE disconnected_peer=EOF credential_snapshot_retained=true pid={} authority=none",
            stale_snapshot.pid()
        ),
        Ok(other) => {
            return Err(format!(
                "post-disconnect credential PID changed unexpectedly: {}",
                other.pid()
            ));
        }
        Err(error) => println!(
            "NEGATIVE disconnected_peer=EOF reader_rejected={error:?} authority=none"
        ),
    }

    drop(first);
    drop(second);
    drop(listener);
    fs::remove_file(&socket_path).map_err(|error| format!("remove-owned-socket-path: {error}"))?;
    Ok(())
}

fn power_off_guest() -> ! {
    // SAFETY: this binary is installed as init (PID 1) in the disposable QEMU
    // guest. It requests guest shutdown only after its owned child was reaped.
    let result = unsafe { libc::reboot(libc::LINUX_REBOOT_CMD_POWER_OFF) };
    eprintln!(
        "GUEST_POWER_OFF_RETURNED result={result} error={}",
        std::io::Error::last_os_error()
    );
    loop {
        thread::sleep(Duration::from_secs(1));
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("--peer-child") {
        let Some(socket_path) = args.get(2) else {
            eprintln!("PEERCRED_CHILD_FAIL missing socket path");
            std::process::exit(2);
        };
        let result = child_mode(Path::new(socket_path));
        if let Err(error) = result {
            eprintln!("PEERCRED_CHILD_FAIL {error}");
            std::process::exit(2);
        }
        std::process::exit(0);
    }

    println!("PEERCRED_RUNTIME_BEGIN linux=true candidate=task39-owned-connection-adapter");
    match run_consumer() {
        Ok(()) => println!("PEERCRED_RUNTIME_RESULT=PASS"),
        Err(error) => println!("PEERCRED_RUNTIME_RESULT=FAIL reason={error}"),
    }
    power_off_guest();
}
