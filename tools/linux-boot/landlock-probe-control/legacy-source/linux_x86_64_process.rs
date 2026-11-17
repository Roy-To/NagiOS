pub(crate) mod linux_x86_64_process {
    use super::*;
    use super::pid1_child_reaper::{
        ChildReapState, Pid1ChildReaper, ReapKernel, ReapTicket, ReapTicketReservation,
    };
    use nagi_init_spawn_contract::{
        clone3_pid1_service_child, Clone3Pid1Child, SpawnedProcessIdentity,
        SpawnedProcessPidfd,
    };
    use core::arch::asm;
    use std::io;
    use std::ptr;

    const NR_READ: usize = 0;
    const NR_WRITE: usize = 1;
    const NR_CLOSE: usize = 3;
    const NR_DUP2: usize = 33;
    const NR_SOCKETPAIR: usize = 53;
    #[cfg(linux_pid1_safety_smoke)]
    const NR_CLONE: usize = 56;
    const NR_EXECVEAT: usize = 322;
    const NR_MEMFD_CREATE: usize = 319;
    const NR_EXIT_GROUP: usize = 231;
    const NR_WAIT4: usize = 61;
    const NR_WAITID: usize = 247;
    const NR_KILL: usize = 62;
    const NR_PIDFD_SEND_SIGNAL: usize = 424;
    const NR_SETGROUPS: usize = 116;
    const NR_SETRESUID: usize = 117;
    const NR_SETRESGID: usize = 119;
    const NR_PRCTL: usize = 157;
    const NR_OPENAT: usize = 257;
    const NR_PIPE2: usize = 293;
    const NR_FCNTL: usize = 72;
    const F_ADD_SEALS: usize = 1033;
    const F_GETFL: usize = 3;
    const F_SEAL_SEAL: usize = 0x0001;
    const F_SEAL_SHRINK: usize = 0x0002;
    const F_SEAL_GROW: usize = 0x0004;
    const F_SEAL_WRITE: usize = 0x0008;
    const MFD_CLOEXEC: usize = 0x0001;
    const MFD_ALLOW_SEALING: usize = 0x0002;
    const MFD_EXEC: usize = 0x0010;
    const AT_EMPTY_PATH: usize = 0x1000;
    const NR_SENDTO: usize = 44;
    const NR_CLOSE_RANGE: usize = 436;
    const NR_POLL: usize = 7;
    const NR_RT_SIGPROCMASK: usize = 14;
    const NR_SIGNALFD4: usize = 289;
    const NR_LANDLOCK_CREATE_RULESET: usize = 444;
    const NR_LANDLOCK_ADD_RULE: usize = 445;
    const NR_LANDLOCK_RESTRICT_SELF: usize = 446;
    #[cfg(linux_pid1_safety_smoke)]
    const NR_NANOSLEEP: usize = 35;

    const EINTR: isize = 4;
    const ESRCH: isize = 3;
    const ECHILD: isize = 10;
    const EAGAIN: isize = 11;
    #[cfg(linux_pid1_safety_smoke)]
    const EBUSY: i32 = 16;
    const SIGCHLD: usize = 17;
    const SIGKILL: usize = 9;
    const SIG_BLOCK: usize = 0;
    const SIG_SETMASK: usize = 2;
    #[cfg(linux_pid1_safety_smoke)]
    const CLONE_NEWPID: usize = 0x2000_0000;
    const WNOHANG: usize = 1;
    const WEXITED: usize = 4;
    const WNOWAIT: usize = 0x0100_0000;
    const P_PID: usize = 1;
    const P_ALL: usize = 0;
    const P_PIDFD: usize = 3;
    const SFD_CLOEXEC: usize = 0x80000;
    const LINUX_SIGNAL_INFO_BYTES: usize = 128;
    const AF_UNIX: usize = 1;
    const SOCK_STREAM: usize = 1;
    const SOCK_CLOEXEC: usize = 0x80000;
    const O_RDWR: usize = 2;
    const O_CLOEXEC: usize = 0x80000;
    const O_PATH: usize = 0x0020_0000;
    const AT_FDCWD: usize = (-100isize) as usize;
    const F_DUPFD_CLOEXEC: usize = 1030;
    const F_SETFL: usize = 4;
    const O_NONBLOCK: usize = 0x800;
    const MSG_NOSIGNAL: usize = 0x4000;
    const CLOSE_RANGE_CLOEXEC: usize = 1 << 2;
    const PR_SET_NO_NEW_PRIVS: usize = 38;
    const LANDLOCK_CREATE_RULESET_VERSION: usize = 1;
    const LANDLOCK_RULE_PATH_BENEATH: usize = 1;
    const LANDLOCK_ABI_NOTES_MIN: usize = 5;
    const LANDLOCK_ACCESS_FS_EXECUTE: u64 = 1 << 0;
    const LANDLOCK_ACCESS_FS_WRITE_FILE: u64 = 1 << 1;
    const LANDLOCK_ACCESS_FS_READ_FILE: u64 = 1 << 2;
    const LANDLOCK_ACCESS_FS_READ_DIR: u64 = 1 << 3;
    const LANDLOCK_ACCESS_FS_REMOVE_DIR: u64 = 1 << 4;
    const LANDLOCK_ACCESS_FS_REMOVE_FILE: u64 = 1 << 5;
    const LANDLOCK_ACCESS_FS_MAKE_CHAR: u64 = 1 << 6;
    const LANDLOCK_ACCESS_FS_MAKE_DIR: u64 = 1 << 7;
    const LANDLOCK_ACCESS_FS_MAKE_REG: u64 = 1 << 8;
    const LANDLOCK_ACCESS_FS_MAKE_SOCK: u64 = 1 << 9;
    const LANDLOCK_ACCESS_FS_MAKE_FIFO: u64 = 1 << 10;
    const LANDLOCK_ACCESS_FS_MAKE_BLOCK: u64 = 1 << 11;
    const LANDLOCK_ACCESS_FS_MAKE_SYM: u64 = 1 << 12;
    const LANDLOCK_ACCESS_FS_REFER: u64 = 1 << 13;
    const LANDLOCK_ACCESS_FS_TRUNCATE: u64 = 1 << 14;
    const LANDLOCK_ACCESS_FS_IOCTL_DEV: u64 = 1 << 15;
    const LANDLOCK_HANDLED_FS_NOTES: u64 = (1 << 16) - 1;
    const LANDLOCK_NOTES_DATA_ACCESS: u64 = LANDLOCK_ACCESS_FS_WRITE_FILE
        | LANDLOCK_ACCESS_FS_READ_FILE
        | LANDLOCK_ACCESS_FS_READ_DIR
        | LANDLOCK_ACCESS_FS_REMOVE_FILE
        | LANDLOCK_ACCESS_FS_MAKE_DIR
        | LANDLOCK_ACCESS_FS_MAKE_REG
        | LANDLOCK_ACCESS_FS_REFER
        | LANDLOCK_ACCESS_FS_TRUNCATE;
    // C3 may explicitly grant device ioctl to a specific resource, but it is
    // never implied by the ordinary Notes data rights above.
    const LANDLOCK_NOTES_ALLOWED_FS: u64 =
        LANDLOCK_NOTES_DATA_ACCESS | LANDLOCK_ACCESS_FS_IOCTL_DEV;
    const SERVICE_SANDBOX_ID: usize = 65_534;
    const RELEASE_BYTE: u8 = 1;

    // Cache only a successfully created descriptor. Transient setup failures
    // (including fd exhaustion) must remain retryable on the next timed wait.
    static PID1_SIGNAL_FD: std::sync::OnceLock<OwnedFd> = std::sync::OnceLock::new();
    static PID1_CHILD_REAPER: std::sync::OnceLock<std::sync::Mutex<Pid1ChildReaper>> =
        std::sync::OnceLock::new();
    static PID1_REAPER_WARNED: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);

    pub(super) fn wait_for_os_event() {
        // ProcessLifecycle polling and PID-1 idle waiting both use the same
        // coordinator. It routes tracked exits to their pidfd ticket and
        // consumes exact-PID orphan exits without a wildcard wait4(-1).
        if let Err(error) = reap_pending_child_exits() {
            warn_reaper_once("child exit scan failed", &error);
            std::thread::park_timeout(std::time::Duration::from_millis(250));
            return;
        }
        let result = pid1_sigchld_fd().and_then(|signal_fd| wait_for_sigchld(signal_fd));
        match result {
            Ok(()) => {
                if let Err(error) = reap_pending_child_exits() {
                    warn_reaper_once("child reap failed", &error);
                    std::thread::park_timeout(std::time::Duration::from_millis(250));
                }
            }
            Err(error) => {
                warn_reaper_once("SIGCHLD wait failed", &error);
                std::thread::park_timeout(std::time::Duration::from_millis(250));
            }
        }
    }

    fn warn_reaper_once(context: &str, error: &io::Error) {
        if !PID1_REAPER_WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            eprintln!("nagi-init: {context} ({error}); retrying with timed wait");
        }
    }

    fn child_reaper() -> &'static std::sync::Mutex<Pid1ChildReaper> {
        PID1_CHILD_REAPER.get_or_init(|| std::sync::Mutex::new(Pid1ChildReaper::default()))
    }

    fn reap_pending_child_exits() -> io::Result<usize> {
        let mut reaper = child_reaper()
            .lock()
            .map_err(|_| io::Error::other("PID-1 child reaper lock is poisoned"))?;
        reaper.drain(&mut LinuxReapKernel)
    }

    fn reap_tracked_child(ticket: &ReapTicket) -> io::Result<Option<ServiceExitStatus>> {
        reap_pending_child_exits()?;
        ticket.exit_status()
    }

    fn pid1_sigchld_fd() -> io::Result<&'static OwnedFd> {
        if let Some(fd) = PID1_SIGNAL_FD.get() {
            return Ok(fd);
        }

        let signal_mask = 1u64 << (SIGCHLD - 1);
        let blocked = unsafe {
            syscall4(
                NR_RT_SIGPROCMASK,
                SIG_BLOCK,
                &signal_mask as *const u64 as usize,
                0,
                core::mem::size_of::<u64>(),
            )
        };
        syscall_result(blocked)?;

        let fd = unsafe {
            syscall4(
                NR_SIGNALFD4,
                usize::MAX,
                &signal_mask as *const u64 as usize,
                core::mem::size_of::<u64>(),
                SFD_CLOEXEC,
            )
        };
        let fd = OwnedFd(syscall_result(fd)? as i32);
        if let Err(unused_fd) = PID1_SIGNAL_FD.set(fd) {
            drop(unused_fd);
        }
        PID1_SIGNAL_FD.get().ok_or_else(|| {
            io::Error::other("SIGCHLD descriptor initialization did not publish a descriptor")
        })
    }

    #[cfg(linux_pid1_safety_smoke)]
    fn reap_after_notification_failure(
        error: io::Error,
        mut reap: impl FnMut() -> io::Result<(usize, bool)>,
    ) -> io::Result<usize> {
        let (reaped, _has_live_children) = reap()?;
        if reaped != 0 {
            Ok(reaped)
        } else {
            Err(error)
        }
    }

    /// Safety-smoke-only wildcard reaper for fixture descendants. Production
    /// PID 1 never calls wait4(-1); service children are reaped by their
    /// ProcessLifecycle owner through the per-PID waitid(WNOWAIT)+wait4 path.
    #[cfg(linux_pid1_safety_smoke)]
    fn reap_available_children() -> io::Result<(usize, bool)> {
        let mut reaped = 0usize;
        loop {
            let mut status = 0i32;
            let result = unsafe {
                syscall4(
                    NR_WAIT4,
                    usize::MAX,
                    &mut status as *mut i32 as usize,
                    WNOHANG,
                    0,
                )
            };
            if result > 0 {
                let exit = decode_wait_status(status);
                eprintln!(
                    "nagi-init: PID 1 reaped child pid={} status={exit:?}",
                    result
                );
                reaped += 1;
                continue;
            }
            if result == 0 {
                return Ok((reaped, true));
            }
            if result == -ECHILD {
                return Ok((reaped, false));
            }
            if result == -EINTR {
                continue;
            }
            return Err(io::Error::from_raw_os_error((-result) as i32));
        }
    }

    fn wait_for_sigchld(signal_fd: &OwnedFd) -> io::Result<()> {
        let mut signal_info = [0u8; LINUX_SIGNAL_INFO_BYTES];
        loop {
            let result = unsafe {
                syscall3(
                    NR_READ,
                    signal_fd.raw() as usize,
                    signal_info.as_mut_ptr() as usize,
                    signal_info.len(),
                )
            };
            if result == LINUX_SIGNAL_INFO_BYTES as isize {
                return Ok(());
            }
            if result == -EINTR {
                continue;
            }
            if result < 0 {
                return Err(io::Error::from_raw_os_error((-result) as i32));
            }
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "signalfd returned a short SIGCHLD record",
            ));
        }
    }

    #[cfg(linux_pid1_safety_smoke)]
    fn wait_for_any_child_event() -> io::Result<usize> {
        // Block SIGCHLD before the first wait4 probe. Otherwise an exit between
        // wait4(WNOHANG) and sigprocmask could be discarded under the default
        // disposition and leave PID 1 asleep with an unreaped zombie.
        let signal_fd = match pid1_sigchld_fd() {
            Ok(fd) => fd,
            Err(error) => return reap_after_notification_failure(error, reap_available_children),
        };
        loop {
            let (reaped, _has_live_children) = reap_available_children()?;
            if reaped != 0 {
                return Ok(reaped);
            }
            if let Err(error) = wait_for_sigchld(signal_fd) {
                return reap_after_notification_failure(error, reap_available_children);
            }
        }
    }

    #[cfg(linux_pid1_safety_smoke)]
    fn wait_for_tracked_child(pid: u32) -> io::Result<ServiceExitStatus> {
        loop {
            if let Some(status) = wait_child_nonblocking(pid)? {
                return Ok(status);
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[cfg(linux_pid1_safety_smoke)]
    pub(super) fn run_pid1_safety_smoke() -> ! {
        eprintln!("nagi-init: PID 1 safety smoke started; production services are disabled");

        match mount_tmpfs_on_dev_for_safety_smoke() {
            Ok(()) => match super::mount_devtmpfs_for_pid1() {
                Ok(()) => {
                    use std::os::unix::fs::FileTypeExt;
                    let null_is_device = std::fs::metadata("/dev/null")
                        .map(|metadata| metadata.file_type().is_char_device())
                        .unwrap_or(false);
                    if null_is_device {
                        eprintln!("nagi-init: PASS foreign tmpfs at /dev was overmounted by devtmpfs; /dev/null is a character device");
                    } else {
                        eprintln!("nagi-init: FAIL devtmpfs mount succeeded but /dev/null is not a character device");
                    }
                }
                Err(error) if error.raw_os_error() == Some(EBUSY) => eprintln!(
                    "nagi-init: devtmpfs mount returned ambiguous EBUSY ({EBUSY}); no filesystem is inferred and startup fails closed"
                ),
                Err(error) => eprintln!("nagi-init: FAIL devtmpfs mount over tmpfs failed: {error}"),
            },
            Err(error) => eprintln!("nagi-init: FAIL could not mount test tmpfs at /dev: {error}"),
        }

        match super::mount_devtmpfs_at(b"/__nagi_missing_devtmpfs_target__\0") {
            Err(error) if error.raw_os_error() == Some(2) => eprintln!(
                "nagi-init: PASS missing devtmpfs target returned ENOENT; no service startup is attempted"
            ),
            Err(error) => eprintln!(
                "nagi-init: FAIL missing devtmpfs target returned unexpected error: {error}"
            ),
            Ok(()) => eprintln!(
                "nagi-init: FAIL missing devtmpfs target unexpectedly mounted"
            ),
        }

        match spawn_pid1_orphan_fixture(1) {
            Ok(parent_pid) => match wait_for_tracked_child(parent_pid) {
                Ok(status) => {
                    eprintln!("nagi-init: orphan fixture direct child exited ({status:?})");
                    match wait_for_any_child_event() {
                        Ok(reaped) if reaped > 0 => eprintln!(
                            "nagi-init: PASS PID 1 reaped orphan descendant after direct child exit"
                        ),
                        Ok(_) => eprintln!("nagi-init: FAIL orphan reaper woke without a reap"),
                        Err(error) => eprintln!("nagi-init: FAIL orphan reaper failed: {error}"),
                    }
                }
                Err(error) => eprintln!("nagi-init: FAIL direct child wait failed: {error}"),
            },
            Err(error) => eprintln!("nagi-init: FAIL could not create orphan fixture: {error}"),
        }

        match run_reaper_setup_failure_smoke() {
            Ok(()) => eprintln!(
                "nagi-init: PASS injected SIGCHLD setup failure used timed wait4 fallback and reaped an orphan"
            ),
            Err(error) => eprintln!(
                "nagi-init: FAIL injected SIGCHLD setup failure did not reap an orphan: {error}"
            ),
        }

        match run_pid_namespace_boundary_smoke() {
            Ok(()) => eprintln!(
                "nagi-init: PASS tracked service namespace was stopped and reaped while unrelated helper survived"
            ),
            Err(error) => eprintln!(
                "nagi-init: FAIL service namespace boundary did not contain shutdown: {error}"
            ),
        }

        super::wait_in_pid1(wait_for_os_event)
    }

    #[cfg(linux_pid1_safety_smoke)]
    fn mount_tmpfs_on_dev_for_safety_smoke() -> io::Result<()> {
        const MS_NOSUID: std::os::raw::c_ulong = 2;
        const MS_NOEXEC: std::os::raw::c_ulong = 8;
        const SOURCE: &[u8] = b"tmpfs\0";
        const TARGET: &[u8] = b"/dev\0";
        const FILESYSTEM: &[u8] = b"tmpfs\0";
        const OPTIONS: &[u8] = b"mode=0755\0";
        let result = unsafe {
            super::mount(
                SOURCE.as_ptr().cast(),
                TARGET.as_ptr().cast(),
                FILESYSTEM.as_ptr().cast(),
                MS_NOSUID | MS_NOEXEC,
                OPTIONS.as_ptr().cast(),
            )
        };
        if result == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    #[cfg(linux_pid1_safety_smoke)]
    fn spawn_pid1_orphan_fixture(descendant_delay_seconds: i64) -> io::Result<u32> {
        let child = unsafe { syscall5(NR_CLONE, SIGCHLD, 0, 0, 0, 0) };
        if child == 0 {
            let groups = unsafe { syscall2(NR_SETGROUPS, 0, 0) };
            let group_ids = unsafe {
                syscall3(
                    NR_SETRESGID,
                    SERVICE_SANDBOX_ID,
                    SERVICE_SANDBOX_ID,
                    SERVICE_SANDBOX_ID,
                )
            };
            let user_ids = unsafe {
                syscall3(
                    NR_SETRESUID,
                    SERVICE_SANDBOX_ID,
                    SERVICE_SANDBOX_ID,
                    SERVICE_SANDBOX_ID,
                )
            };
            let no_new_privs = unsafe { syscall5(NR_PRCTL, PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
            if groups < 0 || group_ids < 0 || user_ids < 0 || no_new_privs < 0 {
                exit_with_code(122);
            }

            let descendant = unsafe { syscall5(NR_CLONE, SIGCHLD, 0, 0, 0, 0) };
            if descendant == 0 {
                let delay = Timespec {
                    seconds: descendant_delay_seconds,
                    nanoseconds: 0,
                };
                let _ = unsafe { syscall2(NR_NANOSLEEP, &delay as *const Timespec as usize, 0) };
                exit_with_code(37);
            }
            if descendant < 0 {
                exit_with_code(123);
            }
            exit_with_code(0);
        }
        syscall_result(child).map(|pid| pid as u32)
    }

    #[cfg(linux_pid1_safety_smoke)]
    fn run_reaper_setup_failure_smoke() -> io::Result<()> {
        let parent = spawn_pid1_orphan_fixture(1)?;
        wait_for_child_bounded(parent, std::time::Duration::from_secs(5))?;

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            match reap_after_notification_failure(
                io::Error::from_raw_os_error(24),
                reap_available_children,
            ) {
                Ok(reaped) if reaped != 0 => return Ok(()),
                Ok(_) => return Err(io::Error::other("fallback returned without a reap")),
                Err(error) if error.raw_os_error() == Some(24) => {}
                Err(error) => return Err(error),
            }
            if std::time::Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "injected signalfd failure fallback did not reap within 5 seconds",
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
    }

    #[cfg(linux_pid1_safety_smoke)]
    fn run_pid_namespace_boundary_smoke() -> io::Result<()> {
        let helper_pid = spawn_unrelated_helper_fixture()?;
        let boundary_result = (|| {
            let (service_pid, _ready_read, descendant_hold_read) =
                spawn_pid_namespace_service_fixture()?;
            let mut service_pid_to_cleanup = Some(service_pid);
            let result = (|| {
                if wait_child_nonblocking(helper_pid)?.is_some() || !process_is_alive(helper_pid)? {
                    return Err(io::Error::other(
                        "unrelated helper exited before service shutdown",
                    ));
                }

                let mut process_os = LinuxProcessOs;
                let mut service = ProcessHandle::<Running, LinuxChildRaw>::new(
                    service_pid,
                    LinuxChildRaw {
                        gate: None,
                        exec_error: None,
                        executable: None,
                        spawn: None,
                        reap: ReapTicket::unverified(service_pid),
                        termination: TerminationState::NotAttempted,
                        descriptor_close: DescriptorCloseState::NotAttempted,
                    },
                );
                process_os.terminate(&mut service)?;
                wait_for_child_bounded(service_pid, std::time::Duration::from_secs(5))?;
                service_pid_to_cleanup = None;
                wait_for_pipe_eof(&descendant_hold_read, std::time::Duration::from_secs(5))?;

                if wait_child_nonblocking(helper_pid)?.is_some() || !process_is_alive(helper_pid)? {
                    return Err(io::Error::other(
                        "stopping the tracked service namespace also stopped the unrelated helper",
                    ));
                }
                Ok(())
            })();
            if let Some(service_pid) = service_pid_to_cleanup {
                let _ = terminate_and_reap(service_pid);
            }
            result
        })();

        let helper_cleanup = terminate_and_reap(helper_pid);
        match (boundary_result, helper_cleanup) {
            (Ok(()), Ok(_)) => Ok(()),
            (Err(error), _) => Err(error),
            (Ok(()), Err(error)) => Err(error),
        }
    }

    #[cfg(linux_pid1_safety_smoke)]
    fn spawn_unrelated_helper_fixture() -> io::Result<u32> {
        let child = unsafe { syscall5(NR_CLONE, SIGCHLD, 0, 0, 0, 0) };
        if child == 0 {
            let delay = Timespec {
                seconds: 30,
                nanoseconds: 0,
            };
            let _ = unsafe { syscall2(NR_NANOSLEEP, &delay as *const Timespec as usize, 0) };
            exit_with_code(0);
        }
        syscall_result(child).map(|pid| pid as u32)
    }

    #[cfg(linux_pid1_safety_smoke)]
    fn spawn_pid_namespace_service_fixture() -> io::Result<(u32, OwnedFd, OwnedFd)> {
        let (ready_read, ready_write) = create_pipe_pair()?;
        let (hold_read, hold_write) = create_pipe_pair()?;
        let child = unsafe { syscall5(NR_CLONE, SIGCHLD | CLONE_NEWPID, 0, 0, 0, 0) };
        if child == 0 {
            unsafe {
                syscall1(NR_CLOSE, ready_read.raw() as usize);
                syscall1(NR_CLOSE, hold_read.raw() as usize);
            }
            let unblocked_signals = 0u64;
            let signals_unblocked = unsafe {
                syscall4(
                    NR_RT_SIGPROCMASK,
                    SIG_SETMASK,
                    &unblocked_signals as *const u64 as usize,
                    0,
                    core::mem::size_of::<u64>(),
                )
            };
            let groups = unsafe { syscall2(NR_SETGROUPS, 0, 0) };
            let group_ids = unsafe {
                syscall3(
                    NR_SETRESGID,
                    SERVICE_SANDBOX_ID,
                    SERVICE_SANDBOX_ID,
                    SERVICE_SANDBOX_ID,
                )
            };
            let user_ids = unsafe {
                syscall3(
                    NR_SETRESUID,
                    SERVICE_SANDBOX_ID,
                    SERVICE_SANDBOX_ID,
                    SERVICE_SANDBOX_ID,
                )
            };
            let no_new_privs = unsafe { syscall5(NR_PRCTL, PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) };
            if signals_unblocked < 0
                || groups < 0
                || group_ids < 0
                || user_ids < 0
                || no_new_privs < 0
            {
                exit_with_code(122);
            }

            let descendant = unsafe { syscall5(NR_CLONE, SIGCHLD, 0, 0, 0, 0) };
            if descendant == 0 {
                unsafe { syscall1(NR_CLOSE, ready_write.raw() as usize) };
                let delay = Timespec {
                    seconds: 30,
                    nanoseconds: 0,
                };
                let _ = unsafe { syscall2(NR_NANOSLEEP, &delay as *const Timespec as usize, 0) };
                exit_with_code(37);
            }
            if descendant < 0 {
                exit_with_code(123);
            }

            // Only the descendant retains the hold-pipe writer. EOF in the
            // outer PID 1 proves the whole service namespace has terminated.
            unsafe {
                syscall1(NR_CLOSE, hold_write.raw() as usize);
                let ready_byte = [1u8];
                let _ = syscall3(
                    NR_WRITE,
                    ready_write.raw() as usize,
                    ready_byte.as_ptr() as usize,
                    ready_byte.len(),
                );
                syscall1(NR_CLOSE, ready_write.raw() as usize);
            }
            let delay = Timespec {
                seconds: 30,
                nanoseconds: 0,
            };
            loop {
                let _ = unsafe { syscall2(NR_NANOSLEEP, &delay as *const Timespec as usize, 0) };
            }
        }

        let service_pid = match syscall_result(child) {
            Ok(pid) => pid as u32,
            Err(error) => return Err(error),
        };
        drop(ready_write);
        drop(hold_write);
        if let Err(error) = set_nonblocking(hold_read.raw()) {
            let _ = terminate_and_reap(service_pid);
            return Err(error);
        }
        let mut ready = 0u8;
        let read = unsafe {
            syscall3(
                NR_READ,
                ready_read.raw() as usize,
                &mut ready as *mut u8 as usize,
                1,
            )
        };
        if read != 1 || ready != 1 {
            let _ = terminate_and_reap(service_pid);
            return Err(if read < 0 {
                io::Error::from_raw_os_error((-read) as i32)
            } else {
                io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "service namespace did not become ready",
                )
            });
        }
        Ok((service_pid, ready_read, hold_read))
    }

    #[cfg(linux_pid1_safety_smoke)]
    fn create_pipe_pair() -> io::Result<(OwnedFd, OwnedFd)> {
        let mut descriptors = [-1i32; 2];
        syscall_result(unsafe { syscall2(NR_PIPE2, descriptors.as_mut_ptr() as usize, 0) })?;
        Ok((OwnedFd(descriptors[0]), OwnedFd(descriptors[1])))
    }

    #[cfg(linux_pid1_safety_smoke)]
    fn process_is_alive(process_id: u32) -> io::Result<bool> {
        let result = unsafe { syscall2(NR_KILL, process_id as usize, 0) };
        if result == 0 {
            Ok(true)
        } else if result == -ESRCH {
            Ok(false)
        } else {
            Err(io::Error::from_raw_os_error((-result) as i32))
        }
    }

    #[cfg(linux_pid1_safety_smoke)]
    fn terminate_and_reap(process_id: u32) -> io::Result<ServiceExitStatus> {
        if let Some(status) = wait_child_nonblocking(process_id)? {
            return Ok(status);
        }
        let mut process_os = LinuxProcessOs;
        let mut child = ProcessHandle::<Running, LinuxChildRaw>::new(
            process_id,
            LinuxChildRaw {
                gate: None,
                exec_error: None,
                executable: None,
                spawn: None,
                reap: ReapTicket::unverified(process_id),
                termination: TerminationState::NotAttempted,
                descriptor_close: DescriptorCloseState::NotAttempted,
            },
        );
        process_os.terminate(&mut child)?;
        wait_for_child_bounded(process_id, std::time::Duration::from_secs(5))
    }

    #[cfg(linux_pid1_safety_smoke)]
    fn wait_for_child_bounded(
        process_id: u32,
        budget: std::time::Duration,
    ) -> io::Result<ServiceExitStatus> {
        let deadline = std::time::Instant::now() + budget;
        loop {
            if let Some(status) = wait_child_nonblocking(process_id)? {
                return Ok(status);
            }
            if std::time::Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "tracked child did not exit before the smoke deadline",
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[cfg(linux_pid1_safety_smoke)]
    fn wait_for_pipe_eof(fd: &OwnedFd, budget: std::time::Duration) -> io::Result<()> {
        let deadline = std::time::Instant::now() + budget;
        let mut byte = 0u8;
        loop {
            let result =
                unsafe { syscall3(NR_READ, fd.raw() as usize, &mut byte as *mut u8 as usize, 1) };
            if result == 0 {
                return Ok(());
            }
            if result == -(EINTR) {
                continue;
            }
            if result < 0 && result != -EAGAIN {
                return Err(io::Error::from_raw_os_error((-result) as i32));
            }
            if std::time::Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "service namespace descendant retained the pipe past the deadline",
                ));
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    #[cfg(linux_pid1_safety_smoke)]
    #[repr(C)]
    struct Timespec {
        seconds: i64,
        nanoseconds: i64,
    }

    #[cfg(linux_pid1_safety_smoke)]
    fn exit_with_code(code: usize) -> ! {
        unsafe {
            loop {
                syscall1(NR_EXIT_GROUP, code);
            }
        }
    }

    #[inline(always)]
    unsafe fn syscall1(number: usize, a1: usize) -> isize {
        let result: isize;
        unsafe {
            asm!(
                "syscall",
                inlateout("rax") number as isize => result,
                in("rdi") a1,
                lateout("rcx") _,
                lateout("r11") _,
                options(nostack)
            );
        }
        result
    }

    #[inline(always)]
    unsafe fn syscall2(number: usize, a1: usize, a2: usize) -> isize {
        let result: isize;
        unsafe {
            asm!(
                "syscall",
                inlateout("rax") number as isize => result,
                in("rdi") a1,
                in("rsi") a2,
                lateout("rcx") _,
                lateout("r11") _,
                options(nostack)
            );
        }
        result
    }

    #[inline(always)]
    unsafe fn syscall3(number: usize, a1: usize, a2: usize, a3: usize) -> isize {
        let result: isize;
        unsafe {
            asm!(
                "syscall",
                inlateout("rax") number as isize => result,
                in("rdi") a1,
                in("rsi") a2,
                in("rdx") a3,
                lateout("rcx") _,
                lateout("r11") _,
                options(nostack)
            );
        }
        result
    }

    #[inline(always)]
    unsafe fn syscall4(number: usize, a1: usize, a2: usize, a3: usize, a4: usize) -> isize {
        let result: isize;
        unsafe {
            asm!(
                "syscall",
                inlateout("rax") number as isize => result,
                in("rdi") a1,
                in("rsi") a2,
                in("rdx") a3,
                in("r10") a4,
                lateout("rcx") _,
                lateout("r11") _,
                options(nostack)
            );
        }
        result
    }

    #[inline(always)]
    unsafe fn syscall5(
        number: usize,
        a1: usize,
        a2: usize,
        a3: usize,
        a4: usize,
        a5: usize,
    ) -> isize {
        let result: isize;
        unsafe {
            asm!(
                "syscall",
                inlateout("rax") number as isize => result,
                in("rdi") a1,
                in("rsi") a2,
                in("rdx") a3,
                in("r10") a4,
                in("r8") a5,
                lateout("rcx") _,
                lateout("r11") _,
                options(nostack)
            );
        }
        result
    }

    #[inline(always)]
    unsafe fn syscall6(
        number: usize,
        a1: usize,
        a2: usize,
        a3: usize,
        a4: usize,
        a5: usize,
        a6: usize,
    ) -> isize {
        let result: isize;
        unsafe {
            asm!(
                "syscall",
                inlateout("rax") number as isize => result,
                in("rdi") a1,
                in("rsi") a2,
                in("rdx") a3,
                in("r10") a4,
                in("r8") a5,
                in("r9") a6,
                lateout("rcx") _,
                lateout("r11") _,
                options(nostack)
            );
        }
        result
    }

    fn syscall_result(result: isize) -> io::Result<usize> {
        if result < 0 {
            Err(io::Error::from_raw_os_error((-result) as i32))
        } else {
            Ok(result as usize)
        }
    }

    struct OwnedFd(i32);

    impl OwnedFd {
        fn from_result(result: isize) -> io::Result<Self> {
            syscall_result(result).map(|fd| Self(fd as i32))
        }

        fn raw(&self) -> i32 {
            self.0
        }

        fn close_now(&mut self) -> io::Result<()> {
            if self.0 < 0 {
                return Ok(());
            }
            let fd = self.0;
            self.0 = -1;
            unsafe { syscall_result(syscall1(NR_CLOSE, fd as usize)).map(|_| ()) }
        }
    }

    impl Drop for OwnedFd {
        fn drop(&mut self) {
            let _ = self.close_now();
        }
    }

    #[repr(C)]
    struct LandlockRulesetAttr {
        handled_access_fs: u64,
    }

    #[repr(C)]
    struct LandlockPathBeneathAttr {
        allowed_access: u64,
        parent_fd: i32,
    }

    fn add_landlock_path_rule(
        ruleset: &OwnedFd,
        parent_fd: i32,
        allowed_access: u64,
    ) -> io::Result<()> {
        let rule = LandlockPathBeneathAttr {
            allowed_access,
            parent_fd,
        };
        syscall_result(unsafe {
            syscall4(
                NR_LANDLOCK_ADD_RULE,
                ruleset.raw() as usize,
                LANDLOCK_RULE_PATH_BENEATH,
                &rule as *const LandlockPathBeneathAttr as usize,
                0,
            )
        })
        .map(|_| ())
    }

    /// Build a deny-by-default filesystem ruleset before clone. The roots are
    /// exact resource descriptors and per-resource rights from the C3
    /// resolver; this function does not resolve names or infer Session
    /// authorization. It also grants EXECUTE to
    /// only the already-sealed executable memfd, while Notes roots cannot run
    /// code. ABI < 5 is rejected because safe rename, O_TRUNC, and per-resource
    /// device ioctl decisions need REFER, TRUNCATE, and IOCTL_DEV governed.
    fn create_notes_landlock_ruleset(
        scope: &NotesResourceScope<'_>,
        executable_fd: i32,
    ) -> io::Result<OwnedFd> {
        if scope.rules.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Notes resource scope has no authorized resource rules",
            ));
        }
        for rule in scope.rules {
            if rule.allowed_access_fs == 0
                || rule.allowed_access_fs & !LANDLOCK_NOTES_ALLOWED_FS != 0
            {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "C3 Notes resource rights exceed the Linux Notes allowlist",
                ));
            }
            let fd = std::os::fd::AsRawFd::as_raw_fd(&rule.path);
            let status_flags = syscall_result(unsafe {
                syscall3(NR_FCNTL, fd as usize, F_GETFL, 0)
            })?;
            if status_flags & O_PATH == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "C3 Notes resources must be passed as O_PATH descriptors",
                ));
            }
        }
        let abi = syscall_result(unsafe {
            syscall3(
                NR_LANDLOCK_CREATE_RULESET,
                0,
                0,
                LANDLOCK_CREATE_RULESET_VERSION,
            )
        })?;
        if abi < LANDLOCK_ABI_NOTES_MIN {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "Landlock ABI 5 is required for Notes filesystem policy",
            ));
        }

        let attr = LandlockRulesetAttr {
            handled_access_fs: LANDLOCK_HANDLED_FS_NOTES,
        };
        let ruleset = promote_fd(OwnedFd::from_result(unsafe {
            syscall3(
                NR_LANDLOCK_CREATE_RULESET,
                &attr as *const LandlockRulesetAttr as usize,
                core::mem::size_of::<LandlockRulesetAttr>(),
                0,
            )
        })?)?;

        for rule in scope.rules {
            let fd = std::os::fd::AsRawFd::as_raw_fd(&rule.path);
            add_landlock_path_rule(&ruleset, fd, rule.allowed_access_fs)?;
        }
        add_landlock_path_rule(&ruleset, executable_fd, LANDLOCK_ACCESS_FS_EXECUTE)?;
        Ok(ruleset)
    }

    fn promote_fd(mut fd: OwnedFd) -> io::Result<OwnedFd> {
        if fd.raw() >= 3 {
            return Ok(fd);
        }
        let duplicated = unsafe { syscall3(NR_FCNTL, fd.raw() as usize, F_DUPFD_CLOEXEC, 3) };
        let promoted = OwnedFd::from_result(duplicated)?;
        fd.close_now()?;
        Ok(promoted)
    }

    struct LinuxSealedExecutableOps;

    impl SealedExecutableOps for LinuxSealedExecutableOps {
        type Handle = OwnedFd;

        fn create(&mut self) -> io::Result<Self::Handle> {
            let name = b"nagi-package-executable\0";
            let flags = MFD_CLOEXEC | MFD_ALLOW_SEALING | MFD_EXEC;
            let fd = unsafe { syscall2(NR_MEMFD_CREATE, name.as_ptr() as usize, flags) };
            promote_fd(OwnedFd::from_result(fd)?)
        }

        fn write(&mut self, handle: &Self::Handle, bytes: &[u8]) -> io::Result<usize> {
            syscall_result(unsafe {
                syscall3(
                    NR_WRITE,
                    handle.raw() as usize,
                    bytes.as_ptr() as usize,
                    bytes.len(),
                )
            })
        }

        fn seal(&mut self, handle: &Self::Handle) -> io::Result<()> {
            let seals = F_SEAL_WRITE | F_SEAL_GROW | F_SEAL_SHRINK | F_SEAL_SEAL;
            syscall_result(unsafe { syscall3(NR_FCNTL, handle.raw() as usize, F_ADD_SEALS, seals) })
                .map(|_| ())
        }
    }

    fn set_nonblocking(fd: i32) -> io::Result<()> {
        syscall_result(unsafe { syscall3(NR_FCNTL, fd as usize, F_SETFL, O_NONBLOCK) }).map(|_| ())
    }

    fn open_null() -> io::Result<OwnedFd> {
        let path = b"/dev/null\0";
        let opened = unsafe {
            syscall4(
                NR_OPENAT,
                AT_FDCWD,
                path.as_ptr() as usize,
                O_RDWR | O_CLOEXEC,
                0,
            )
        };
        promote_fd(OwnedFd::from_result(opened)?)
    }

    fn decode_wait_status(status: i32) -> ServiceExitStatus {
        let terminating_signal = status & 0x7f;
        if terminating_signal == 0 {
            ServiceExitStatus {
                exit_code: Some((status >> 8) & 0xff),
                signal: None,
            }
        } else {
            ServiceExitStatus {
                exit_code: None,
                signal: Some(terminating_signal),
            }
        }
    }

    #[repr(C)]
    struct LinuxSigInfo {
        signo: i32,
        errno: i32,
        code: i32,
        _pad0: i32,
        pid: i32,
        uid: u32,
        status: i32,
        _rest: [u8; 100],
    }

    impl Default for LinuxSigInfo {
        fn default() -> Self {
            Self {
                signo: 0,
                errno: 0,
                code: 0,
                _pad0: 0,
                pid: 0,
                uid: 0,
                status: 0,
                _rest: [0; 100],
            }
        }
    }

    const _: [(); LINUX_SIGNAL_INFO_BYTES] = [(); core::mem::size_of::<LinuxSigInfo>()];

    /// Safety-smoke-only exact numeric observer. Production uses P_ALL plus
    /// the shared registry coordinator and verifies tracked children by pidfd.
    #[cfg(linux_pid1_safety_smoke)]
    fn observe_child_exit_nonconsuming(pid: u32) -> io::Result<bool> {
        let mut info = LinuxSigInfo::default();
        let result = unsafe {
            syscall5(
                NR_WAITID, P_PID, pid as usize, &mut info as *mut LinuxSigInfo as usize,
                WEXITED | WNOHANG | WNOWAIT, 0,
            )
        };
        syscall_result(result)?;
        if info.pid == 0 {
            return Ok(false);
        }
        if info.pid as u32 != pid {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "waitid(P_PID) returned a different child PID",
            ));
        }
        Ok(true)
    }

    struct LinuxReapKernel;

    impl ReapKernel for LinuxReapKernel {
        fn observe_any_exit_nonconsuming(&mut self) -> io::Result<Option<u32>> {
            let mut info = LinuxSigInfo::default();
            let result = unsafe {
                syscall5(
                    NR_WAITID,
                    P_ALL,
                    0,
                    &mut info as *mut LinuxSigInfo as usize,
                    WEXITED | WNOHANG | WNOWAIT,
                    0,
                )
            };
            syscall_result(result)?;
            if info.pid == 0 {
                return Ok(None);
            }
            if info.pid <= 1 {
                return Err(io::Error::other("waitid(P_ALL) returned an invalid child PID"));
            }
            Ok(Some(info.pid as u32))
        }

        fn observe_tracked_exit_nonconsuming(
            &mut self,
            process_id: u32,
            pidfd: i32,
        ) -> io::Result<bool> {
            if pidfd < 0 {
                return Err(io::Error::other("tracked process has no pidfd"));
            }
            let mut info = LinuxSigInfo::default();
            let result = unsafe {
                syscall5(
                    NR_WAITID,
                    P_PIDFD,
                    pidfd as usize,
                    &mut info as *mut LinuxSigInfo as usize,
                    WEXITED | WNOHANG | WNOWAIT,
                    0,
                )
            };
            syscall_result(result)?;
            if info.pid == 0 {
                return Ok(false);
            }
            if info.pid as u32 != process_id {
                return Err(io::Error::other("waitid(P_PIDFD) returned a different child PID"));
            }
            Ok(true)
        }

        fn reap_exact(&mut self, process_id: u32) -> io::Result<Option<ServiceExitStatus>> {
            if process_id <= 1 {
                return Err(io::Error::other("refusing to reap a non-child PID"));
            }
            let mut status = 0i32;
            let result = unsafe {
                syscall4(
                    NR_WAIT4,
                    process_id as usize,
                    &mut status as *mut i32 as usize,
                    WNOHANG,
                    0,
                )
            };
            let waited_pid = syscall_result(result)? as u32;
            if waited_pid == 0 {
                return Ok(None);
            }
            if waited_pid != process_id {
                return Err(io::Error::other("wait4 returned a different child PID"));
            }
            Ok(Some(decode_wait_status(status)))
        }
    }

    #[cfg(linux_pid1_safety_smoke)]
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum SmokeReapState {
        Unobserved,
        Observed,
        Attempted,
        Reaped(ServiceExitStatus),
        Unknown,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum TerminationState {
        NotAttempted,
        Attempted,
        Sent,
        Unknown,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum DescriptorCloseState {
        NotAttempted,
        Attempted,
        Complete,
        Unknown,
    }

    fn terminate_child_once(
        pidfd: Option<&SpawnedProcessPidfd>,
        state: &mut TerminationState,
    ) -> io::Result<()> {
        match *state {
            TerminationState::Sent => return Ok(()),
            TerminationState::Attempted | TerminationState::Unknown => {
                return Err(io::Error::other("child termination outcome is unknown"));
            }
            TerminationState::NotAttempted => {}
        }
        *state = TerminationState::Attempted;
        let Some(pidfd) = pidfd else {
            *state = TerminationState::Unknown;
            return Err(io::Error::other("child termination has no retained pidfd"));
        };
        let pidfd_raw = match pidfd.as_raw_fd() {
            Ok(fd) => fd,
            Err(error) => {
                *state = TerminationState::Unknown;
                return Err(error);
            }
        };
        match syscall_result(unsafe {
            syscall4(
                NR_PIDFD_SEND_SIGNAL,
                pidfd_raw as usize,
                SIGKILL,
                0,
                0,
            )
        }) {
            Ok(_) => {
                *state = TerminationState::Sent;
                Ok(())
            }
            Err(error) => {
                *state = TerminationState::Unknown;
                Err(error)
            }
        }
    }

    /// A LinuxChildRaw is the one consuming-wait owner for its exact PID.
    /// `waitid(WNOWAIT)` may be repeated; `wait4(pid)` is attempted once only.
    #[cfg(linux_pid1_safety_smoke)]
    fn reap_observed_child(
        pid: u32,
        state: &mut SmokeReapState,
    ) -> io::Result<Option<ServiceExitStatus>> {
        match *state {
            SmokeReapState::Reaped(status) => return Ok(Some(status)),
            SmokeReapState::Unknown | SmokeReapState::Attempted => {
                return Err(io::Error::other("child reap outcome is already unknown"));
            }
            SmokeReapState::Unobserved => {
                if !observe_child_exit_nonconsuming(pid)? {
                    return Ok(None);
                }
                *state = SmokeReapState::Observed;
            }
            SmokeReapState::Observed => {}
        }

        // Publish before the consuming wait. An error or unwind cannot cause a
        // second owner to repeat wait4 for this child.
        *state = SmokeReapState::Attempted;
        let mut status = 0i32;
        let result = unsafe {
            syscall4(
                NR_WAIT4, pid as usize, &mut status as *mut i32 as usize, WNOHANG, 0,
            )
        };
        let waited_pid = match syscall_result(result) {
            Ok(waited_pid) => waited_pid as u32,
            Err(error) => {
                *state = SmokeReapState::Unknown;
                return Err(error);
            }
        };
        if waited_pid != pid {
            *state = SmokeReapState::Unknown;
            return Err(io::Error::other(
                "wait4 did not consume the previously observed child",
            ));
        }
        let exit = decode_wait_status(status);
        *state = SmokeReapState::Reaped(exit);
        Ok(Some(exit))
    }

    #[cfg(linux_pid1_safety_smoke)]
    fn wait_child_nonblocking(pid: u32) -> io::Result<Option<ServiceExitStatus>> {
        // The smoke fixtures use this direct wait only for PIDs they created
        // and own locally; production ProcessLifecycle uses ChildReapState.
        let mut state = SmokeReapState::Unobserved;
        reap_observed_child(pid, &mut state)
    }

    fn send_child_error(error_fd: i32, errno: usize) -> ! {
        let bytes = (errno as u32).to_ne_bytes();
        let mut written = 0usize;
        while written < bytes.len() {
            let result = unsafe {
                syscall3(
                    NR_WRITE,
                    error_fd as usize,
                    bytes.as_ptr().add(written) as usize,
                    bytes.len() - written,
                )
            };
            if result == -(EINTR) {
                continue;
            }
            if result <= 0 {
                break;
            }
            written += result as usize;
        }
        unsafe {
            loop {
                syscall1(NR_EXIT_GROUP, 127);
            }
        }
    }

    unsafe fn child_wait_and_exec(
        gate_parent: i32,
        gate_child: i32,
        error_read: i32,
        error_write: i32,
        null_fd: i32,
        executable_fd: i32,
        landlock_ruleset_fd: i32,
        argv: *const *const i8,
        envp: *const *const i8,
        execution_identity: LinuxExecutionIdentity,
    ) -> ! {
        unsafe {
            syscall1(NR_CLOSE, gate_parent as usize);
            syscall1(NR_CLOSE, error_read as usize);
        }

        let mut command = 0u8;
        loop {
            let result = unsafe {
                syscall3(
                    NR_READ,
                    gate_child as usize,
                    &mut command as *mut u8 as usize,
                    1,
                )
            };
            if result == -(EINTR) {
                continue;
            }
            if result != 1 || command != RELEASE_BYTE {
                unsafe {
                    loop {
                        syscall1(NR_EXIT_GROUP, 125);
                    }
                }
            }
            break;
        }

        unsafe {
            syscall1(NR_CLOSE, gate_child as usize);
            for target in 0..3usize {
                // The dedicated Notes development smoke writes its actual
                // startup and input errors to the inherited QEMU console.
                // Keep stdin detached and leave every other process on the
                // normal null-stdio path. Production builds never inherit it.
                if target != 0 && inherit_child_console_output(execution_identity) {
                    continue;
                }
                let result = syscall2(NR_DUP2, null_fd as usize, target);
                if result < 0 {
                    send_child_error(error_write, (-result) as usize);
                }
            }
            syscall1(NR_CLOSE, null_fd as usize);
            let closed = syscall3(NR_CLOSE_RANGE, 3, u32::MAX as usize, CLOSE_RANGE_CLOEXEC);
            if closed < 0 {
                send_child_error(error_write, (-closed) as usize);
            }
            // PID 1 blocks SIGCHLD for signalfd-based reaping. Services inherit
            // that signal mask across clone, so restore an empty mask before
            // exec in the private service PID namespace.
            let unblocked_signals = 0u64;
            let signals_unblocked = syscall4(
                NR_RT_SIGPROCMASK,
                SIG_SETMASK,
                &unblocked_signals as *const u64 as usize,
                0,
                core::mem::size_of::<u64>(),
            );
            if signals_unblocked < 0 {
                send_child_error(error_write, (-signals_unblocked) as usize);
            }
            // PID 1 owns Linux-side setup, but service ELFs never inherit its
            // host root identity. These numeric credentials are an OS
            // sandbox detail only; they do not create Nagi identity or grant
            // consent. Drop supplementary groups before the primary IDs, then
            // prohibit privilege-gaining exec transitions.
            let groups_dropped = syscall2(NR_SETGROUPS, 0, 0);
            if groups_dropped < 0 {
                send_child_error(error_write, (-groups_dropped) as usize);
            }
            let group_ids_dropped = syscall3(
                NR_SETRESGID,
                execution_identity.gid as usize,
                execution_identity.gid as usize,
                execution_identity.gid as usize,
            );
            if group_ids_dropped < 0 {
                send_child_error(error_write, (-group_ids_dropped) as usize);
            }
            let user_ids_dropped = syscall3(
                NR_SETRESUID,
                execution_identity.uid as usize,
                execution_identity.uid as usize,
                execution_identity.uid as usize,
            );
            if user_ids_dropped < 0 {
                send_child_error(error_write, (-user_ids_dropped) as usize);
            }
            let no_new_privileges = syscall5(NR_PRCTL, PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0);
            if no_new_privileges < 0 {
                send_child_error(error_write, (-no_new_privileges) as usize);
            }
            let landlocked = syscall3(
                NR_LANDLOCK_RESTRICT_SELF,
                landlock_ruleset_fd as usize,
                0,
                0,
            );
            if landlocked < 0 {
                send_child_error(error_write, (-landlocked) as usize);
            }
            let empty_path = b"\0";
            let executed = syscall5(
                NR_EXECVEAT,
                executable_fd as usize,
                empty_path.as_ptr() as usize,
                argv as usize,
                envp as usize,
                AT_EMPTY_PATH,
            );
            let errno = if executed < 0 {
                (-executed) as usize
            } else {
                5
            };
            send_child_error(error_write, errno);
        }
    }

    pub(super) struct LinuxChildRaw {
        gate: Option<OwnedFd>,
        exec_error: Option<OwnedFd>,
        executable: Option<OwnedFd>,
        spawn: Option<SpawnedProcessPidfd>,
        reap: ReapTicket,
        termination: TerminationState,
        descriptor_close: DescriptorCloseState,
    }

    impl LinuxChildRaw {
        fn duplicate_spawn_proof(&self) -> io::Result<SpawnedProcessPidfd> {
            self.spawn
                .as_ref()
                .ok_or_else(|| io::Error::other("atomic clone3 pidfd proof is unavailable"))?
                .duplicate_for_authority()
        }

        fn close_gate_to_fail_closed(&mut self) -> io::Result<()> {
            match self.gate.as_mut() {
                Some(gate) => gate.close_now(),
                None => Ok(()),
            }
        }
    }

    #[repr(C)]
    struct PollFd {
        fd: i32,
        events: i16,
        revents: i16,
    }

    struct LinuxExecStatusWait<'a> {
        status_fd: i32,
        reap: &'a ReapTicket,
    }

    impl ExecStatusWait for LinuxExecStatusWait<'_> {
        fn read_status_nonblocking(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let result = unsafe {
                syscall3(
                    NR_READ,
                    self.status_fd as usize,
                    buffer.as_mut_ptr() as usize,
                    buffer.len(),
                )
            };
            if result < 0 {
                Err(io::Error::from_raw_os_error((-result) as i32))
            } else {
                Ok(result as usize)
            }
        }

        fn wait_readable(&mut self, timeout: std::time::Duration) -> io::Result<bool> {
            const POLLIN: i16 = 0x001;
            const POLLERR: i16 = 0x008;
            const POLLHUP: i16 = 0x010;
            const POLLNVAL: i16 = 0x020;

            let whole_ms = timeout.as_millis();
            let fractional_ms = if timeout.subsec_nanos() % 1_000_000 == 0 {
                0
            } else {
                1
            };
            let timeout_ms = whole_ms
                .saturating_add(fractional_ms)
                .max(1)
                .min(i32::MAX as u128) as i32;
            let mut poll_fd = PollFd {
                fd: self.status_fd,
                events: POLLIN,
                revents: 0,
            };
            let result = unsafe {
                syscall3(
                    NR_POLL,
                    &mut poll_fd as *mut PollFd as usize,
                    1,
                    timeout_ms as usize,
                )
            };
            if result < 0 {
                return Err(io::Error::from_raw_os_error((-result) as i32));
            }
            if result == 0 {
                return Ok(false);
            }
            if poll_fd.revents & POLLNVAL != 0 {
                return Err(io::Error::from_raw_os_error(9));
            }
            Ok(poll_fd.revents & (POLLIN | POLLERR | POLLHUP) != 0)
        }

        fn child_exit_status(&mut self) -> io::Result<Option<ServiceExitStatus>> {
            reap_tracked_child(self.reap)
        }
    }

    impl LinuxChildRaw {
        fn write_release_gate(
            &mut self,
            budget: std::time::Duration,
        ) -> Result<std::time::Instant, ReleaseFailureReason> {
            let started = std::time::Instant::now();
            if started.elapsed() >= budget {
                return Err(ReleaseFailureReason::TimedOut);
            }
            let gate = self.gate.as_ref().ok_or(ReleaseFailureReason::Io)?.raw();
            let byte = RELEASE_BYTE;
            // The parent end is nonblocking. Treat EINTR and a partial/error
            // result as a failed commit so the caller can clean the uncertain
            // child without holding the Supervisor lock for a retry loop.
            let result = unsafe {
                syscall6(
                    NR_SENDTO,
                    gate as usize,
                    &byte as *const u8 as usize,
                    1,
                    MSG_NOSIGNAL,
                    0,
                    0,
                )
            };
            if result != 1 {
                return Err(ReleaseFailureReason::Io);
            }
            Ok(started)
        }

        fn close_resources_once(&mut self) -> io::Result<()> {
            match self.descriptor_close {
                DescriptorCloseState::Complete => return Ok(()),
                DescriptorCloseState::Attempted | DescriptorCloseState::Unknown => {
                    return Err(io::Error::other("descriptor close outcome is unknown"));
                }
                DescriptorCloseState::NotAttempted => {}
            }
            self.descriptor_close = DescriptorCloseState::Attempted;
            let mut first_error = None;
            for slot in [
                &mut self.gate,
                &mut self.exec_error,
                &mut self.executable,
            ] {
                if let Some(fd) = slot.as_mut() {
                    if let Err(error) = fd.close_now() {
                        if first_error.is_none() {
                            first_error = Some(error);
                        }
                    }
                }
                if slot.as_ref().is_some_and(|fd| fd.raw() < 0) {
                    slot.take();
                }
            }
            if let Some(spawn) = self.spawn.as_mut() {
                if let Err(error) = spawn.close_now() {
                    if first_error.is_none() {
                        first_error = Some(error);
                    }
                }
            }
            if let Some(error) = first_error {
                self.descriptor_close = DescriptorCloseState::Unknown;
                Err(error)
            } else {
                self.descriptor_close = DescriptorCloseState::Complete;
                Ok(())
            }
        }

        fn wait_exec_ack(
            &mut self,
            started: std::time::Instant,
            budget: std::time::Duration,
            cancel_check_interval: std::time::Duration,
            cancelled: &mut dyn FnMut() -> bool,
        ) -> Result<(), ReleaseFailureReason> {
            let status_fd = self
                .exec_error
                .as_ref()
                .ok_or(ReleaseFailureReason::Io)?
                .raw();
            let mut wait = LinuxExecStatusWait {
                status_fd,
                reap: &self.reap,
            };
            let mut elapsed = || started.elapsed();
            match wait_for_exec_ack(
                &mut wait,
                budget,
                cancel_check_interval,
                &mut elapsed,
                cancelled,
            ) {
                Ok(()) => self
                    .close_resources_once()
                    .map_err(|_| ReleaseFailureReason::Io),
                Err(ReleaseFailureReason::ChildExited(_)) => {
                    self.close_resources_once()
                        .map_err(|_| ReleaseFailureReason::Io)
                        .and_then(|_| Err(ReleaseFailureReason::ChildExited(
                            match self.reap.exit_status() {
                                Ok(Some(status)) => status,
                                _ => return Err(ReleaseFailureReason::Io),
                            },
                        )))
                }
                Err(reason) => Err(reason),
            }
        }
    }

    pub(super) struct LinuxProcessOs;

    impl ProcessOs for LinuxProcessOs {
        type Raw = LinuxChildRaw;

        fn pidfd_handoff(
            &self,
            child: &ProcessHandle<Prepared, Self::Raw>,
        ) -> io::Result<SpawnedProcessPidfd> {
            child.raw.duplicate_spawn_proof()
        }

        fn requires_spawn_proof_handoff(&self) -> bool {
            true
        }

        fn requires_launch_lease(&self) -> bool {
            true
        }

        fn same_process_instance<Stage: ProcessStage>(
            &self,
            child: &ProcessHandle<Stage, Self::Raw>,
            identity: &SpawnedProcessIdentity,
        ) -> bool {
            child
                .raw
                .spawn
                .as_ref()
                .is_some_and(|proof| proof.identity().same_instance(identity))
        }

        fn process_instance_identity<Stage: ProcessStage>(
            &self,
            child: &ProcessHandle<Stage, Self::Raw>,
        ) -> Option<SpawnedProcessIdentity> {
            child.raw.spawn.as_ref().map(SpawnedProcessPidfd::identity)
        }

        fn termination_audit_state<Stage: ProcessStage>(
            &self,
            child: &ProcessHandle<Stage, Self::Raw>,
        ) -> TerminationAuditState {
            match child.raw.termination {
                TerminationState::NotAttempted => TerminationAuditState::NotAttempted,
                TerminationState::Sent => TerminationAuditState::Acknowledged,
                TerminationState::Attempted | TerminationState::Unknown => {
                    TerminationAuditState::Unknown
                }
            }
        }

        fn prepare(
            &mut self,
            executable: ExecutablePayload<'_>,
            arguments: &[&str],
            execution_identity: LinuxExecutionIdentity,
        ) -> io::Result<ProcessHandle<Prepared, Self::Raw>> {
            let mut owner = None;
            self.prepare_into(executable, arguments, execution_identity, &mut owner)?;
            owner.ok_or_else(|| io::Error::other("Linux prepare returned without owning its child"))
        }

        fn prepare_into(
            &mut self,
            executable: ExecutablePayload<'_>,
            arguments: &[&str],
            execution_identity: LinuxExecutionIdentity,
            owner: &mut Option<ProcessHandle<Prepared, Self::Raw>>,
        ) -> io::Result<()> {
            self.prepare_into_with_notes_scope(
                executable,
                arguments,
                execution_identity,
                None,
                owner,
            )
        }

        fn requires_notes_resource_scope(&self) -> bool {
            true
        }

        fn prepare_into_with_notes_scope(
            &mut self,
            executable: ExecutablePayload<'_>,
            arguments: &[&str],
            execution_identity: LinuxExecutionIdentity,
            notes_scope: Option<&NotesResourceScope<'_>>,
            owner: &mut Option<ProcessHandle<Prepared, Self::Raw>>,
        ) -> io::Result<()> {
            if owner.is_some() {
                return Err(io::Error::other("process acquisition slot is occupied"));
            }
            let notes_scope = notes_scope.ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "canonical C3 Notes resource roots were not supplied",
                )
            })?;
            if execution_identity.uid == 0 || execution_identity.gid == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Linux process identity must not retain root credentials",
                ));
            }
            // Serialize the clone3 handoff with every PID-1 wait owner. If
            // clone3 cannot provide CLONE_PIDFD, no numeric-PID fallback is
            // permitted; the launch fails closed before a child is created.
            let mut reaper = child_reaper()
                .lock()
                .map_err(|_| io::Error::other("PID-1 child reaper lock is poisoned"))?;
            // Reserve both the ticket state and registry capacity before the
            // irreversible clone3 side effect.
            let ticket_reservation: ReapTicketReservation = reaper.reserve_ticket()?;
            // Copy the exact PackageView executable slice into a private memfd and
            // seal it before creating the gated child. No pathname is resolved.
            let executable_fd =
                create_sealed_executable(&mut LinuxSealedExecutableOps, executable.bytes())?;
            let landlock_ruleset =
                create_notes_landlock_ruleset(notes_scope, executable_fd.raw())?;
            let argv_storage = process_argv_storage(arguments)?;
            let mut argv = argv_storage
                .iter()
                .map(|argument| argument.as_ptr())
                .collect::<Vec<_>>();
            argv.push(ptr::null());
            let envp = [ptr::null()];

            let null_fd = open_null()?;
            let mut gate_fds = [-1i32; 2];
            syscall_result(unsafe {
                syscall4(
                    NR_SOCKETPAIR,
                    AF_UNIX,
                    SOCK_STREAM | SOCK_CLOEXEC,
                    0,
                    gate_fds.as_mut_ptr() as usize,
                )
            })?;
            let gate_first = OwnedFd(gate_fds[0]);
            let gate_second = OwnedFd(gate_fds[1]);
            let gate_parent = promote_fd(gate_first)?;
            let gate_child = promote_fd(gate_second)?;
            set_nonblocking(gate_parent.raw())?;

            let mut error_fds = [-1i32; 2];
            syscall_result(unsafe {
                syscall2(NR_PIPE2, error_fds.as_mut_ptr() as usize, O_CLOEXEC)
            })?;
            let error_first = OwnedFd(error_fds[0]);
            let error_second = OwnedFd(error_fds[1]);
            let error_read = promote_fd(error_first)?;
            let error_write = promote_fd(error_second)?;
            set_nonblocking(error_read.raw())?;

            // This shared-contract function is the only proof issuer. It
            // always requests CLONE_NEWPID|CLONE_PIDFD in one clone3 syscall;
            // unsupported kernels fail without clone/pidfd_open fallback.
            let spawned = unsafe { clone3_pid1_service_child()? };
            match spawned {
                Clone3Pid1Child::Child => {
                    // The child must not unlock an inherited process-wide
                    // mutex. It immediately enters the raw syscall-only gate.
                    std::mem::forget(reaper);
                    unsafe {
                        child_wait_and_exec(
                            gate_parent.raw(),
                            gate_child.raw(),
                            error_read.raw(),
                            error_write.raw(),
                            null_fd.raw(),
                            executable_fd.raw(),
                            landlock_ruleset.raw(),
                            argv.as_ptr(),
                            envp.as_ptr(),
                            execution_identity,
                        )
                    }
                }
                Clone3Pid1Child::Parent(spawn) => {
                    let process_id = spawn.process_id();
                    let identity = spawn.identity();
                    let pidfd = spawn.as_raw_fd().unwrap_or(-1);
                    let ticket = ticket_reservation.bind(identity, pidfd);

                    // Move the actual child, pidfd, and reap ticket into the
                    // lifecycle owner's persistent slot before another
                    // fallible operation or before the exec gate is released.
                    *owner = Some(ProcessHandle::new(
                        process_id,
                        LinuxChildRaw {
                            gate: Some(gate_parent),
                            exec_error: Some(error_read),
                            executable: Some(executable_fd),
                            spawn: Some(spawn),
                            reap: ticket.clone(),
                            termination: TerminationState::NotAttempted,
                            descriptor_close: DescriptorCloseState::NotAttempted,
                        },
                    ));

                    // Capacity and ticket memory were reserved before clone3.
                    // A failed registration closes the gate; the sole P_ALL
                    // reaper can then own the now-untracked exit.
                    if let Err(error) = reaper.register(ticket) {
                        if let Some(child) = owner.as_mut() {
                            let _ = child.raw.close_gate_to_fail_closed();
                        }
                        drop(reaper);
                        drop(gate_child);
                        drop(error_write);
                        drop(null_fd);
                        return Err(error);
                    }
                    drop(reaper);
                    drop(gate_child);
                    drop(error_write);
                    drop(null_fd);
                    Ok(())
                }
                Clone3Pid1Child::ParentWithoutPidfd { process_id } => {
                    let ticket = ticket_reservation.bind_unverified(process_id);
                    *owner = Some(ProcessHandle::new(
                        process_id,
                        LinuxChildRaw {
                            gate: Some(gate_parent),
                            exec_error: Some(error_read),
                            executable: Some(executable_fd),
                            spawn: None,
                            reap: ticket,
                            termination: TerminationState::Unknown,
                            descriptor_close: DescriptorCloseState::NotAttempted,
                        },
                    ));
                    let close_result = owner
                        .as_mut()
                        .map(|child| child.raw.close_gate_to_fail_closed())
                        .unwrap_or_else(|| Err(io::Error::other("spawn owner slot was lost")));
                    drop(reaper);
                    drop(gate_child);
                    drop(error_write);
                    drop(null_fd);
                    close_result?;
                    Err(io::Error::other(
                        "clone3 created a child without its required atomic pidfd; exec gate stayed closed",
                    ))
                }
            }
        }

        fn commit_release_gate<LaunchRecord>(
            &mut self,
            child: &mut ProcessHandle<Prepared, Self::Raw>,
            _registered: &RegisteredLaunch<LaunchRecord>,
            wait_budget: std::time::Duration,
        ) -> Result<std::time::Instant, ReleaseFailureReason> {
            child.raw.write_release_gate(wait_budget)
        }

        fn wait_exec_ack(
            &mut self,
            child: &mut ProcessHandle<ReleaseUncertain, Self::Raw>,
            started: std::time::Instant,
            wait_budget: std::time::Duration,
            cancel_check_interval: std::time::Duration,
            cancelled: &mut dyn FnMut() -> bool,
        ) -> Result<(), ReleaseFailureInfo> {
            match child.raw.wait_exec_ack(
                started,
                wait_budget,
                cancel_check_interval,
                cancelled,
            ) {
                Ok(()) => Ok(()),
                Err(reason) => {
                    let reaped = match reason {
                        ReleaseFailureReason::ChildExited(status) => Some(status),
                        _ => None,
                    };
                    Err(ReleaseFailureInfo { reason, reaped })
                }
            }
        }

        fn try_wait<Stage: ProcessStage>(
            &mut self,
            child: &mut ProcessHandle<Stage, Self::Raw>,
        ) -> io::Result<Option<ServiceExitStatus>> {
            reap_tracked_child(&child.raw.reap)
        }

        fn close_child<Stage: ProcessStage>(
            &mut self,
            child: &mut ProcessHandle<Stage, Self::Raw>,
        ) -> io::Result<()> {
            child.raw.close_resources_once()
        }

        fn terminate<Stage: ProcessStage>(
            &mut self,
            child: &mut ProcessHandle<Stage, Self::Raw>,
        ) -> io::Result<()> {
            terminate_child_once(child.raw.spawn.as_ref(), &mut child.raw.termination)
        }
    }

    #[cfg(all(test, target_os = "linux"))]
    mod ws04_landlock_integration_tests {
        use super::*;
        use std::ffi::CString;
        use std::fs::{self, File};
        use std::io::Write;
        use std::os::fd::{AsFd, AsRawFd, FromRawFd};
        use std::os::unix::fs::PermissionsExt;
        use std::path::{Path, PathBuf};
        use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

        const TEST_UID: u32 = 65_533;
        const TEST_GID: u32 = 65_533;
        const O_PATH: usize = 0x0020_0000;
        const O_DIRECTORY: usize = 0x0001_0000;

        struct RemoveTree(PathBuf);

        impl Drop for RemoveTree {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.0);
            }
        }

        enum TestChildOwner {
            Prepared(ProcessHandle<Prepared, LinuxChildRaw>),
            ReleaseUncertain(ProcessHandle<ReleaseUncertain, LinuxChildRaw>),
        }

        impl TestChildOwner {
            fn try_wait(
                &mut self,
                os: &mut LinuxProcessOs,
            ) -> io::Result<Option<super::super::ServiceExitStatus>> {
                match self {
                    Self::Prepared(child) => ProcessOs::try_wait(os, child),
                    Self::ReleaseUncertain(child) => ProcessOs::try_wait(os, child),
                }
            }

            fn terminate(&mut self, os: &mut LinuxProcessOs) -> io::Result<()> {
                match self {
                    Self::Prepared(child) => ProcessOs::terminate(os, child),
                    Self::ReleaseUncertain(child) => ProcessOs::terminate(os, child),
                }
            }

            fn close(&mut self, os: &mut LinuxProcessOs) -> io::Result<()> {
                match self {
                    Self::Prepared(child) => ProcessOs::close_child(os, child),
                    Self::ReleaseUncertain(child) => ProcessOs::close_child(os, child),
                }
            }
        }

        fn set_mode(path: &Path, mode: u32) {
            let mut permissions = fs::metadata(path).unwrap().permissions();
            permissions.set_mode(mode);
            fs::set_permissions(path, permissions).unwrap();
        }

        fn write_fixture(path: &Path, bytes: &[u8]) {
            let mut file = File::create(path).unwrap();
            file.write_all(bytes).unwrap();
            set_mode(path, 0o666);
        }

        fn open_path_directory(path: &Path) -> std::os::fd::OwnedFd {
            let path = CString::new(path.to_str().unwrap()).unwrap();
            let fd = syscall_result(unsafe {
                syscall4(
                    NR_OPENAT,
                    AT_FDCWD,
                    path.as_ptr() as usize,
                    O_PATH | O_DIRECTORY | O_CLOEXEC,
                    0,
                )
            })
            .unwrap() as i32;
            unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) }
        }

        fn open_read_without_cloexec(path: &Path) -> std::os::fd::OwnedFd {
            let path = CString::new(path.to_str().unwrap()).unwrap();
            let fd = syscall_result(unsafe {
                syscall4(NR_OPENAT, AT_FDCWD, path.as_ptr() as usize, 0, 0)
            })
            .unwrap() as i32;
            // Deliberately omit O_CLOEXEC so the fixture proves the actual
            // close_range(CLOSE_RANGE_CLOEXEC) path closes an inherited FD.
            unsafe { std::os::fd::OwnedFd::from_raw_fd(fd) }
        }

        fn cleanup_child(
            os: &mut LinuxProcessOs,
            child: &mut TestChildOwner,
            reaped: &mut Option<super::super::ServiceExitStatus>,
        ) -> io::Result<()> {
            if reaped.is_none() {
                match child.try_wait(os)? {
                    Some(status) => *reaped = Some(status),
                    None => {
                        // The test owns the exact clone3 pidfd and never uses a
                        // numeric-PID kill fallback. Continue polling after the
                        // one terminate attempt so ambiguous results stay visible.
                        let _ = child.terminate(os);
                        let deadline = Instant::now() + Duration::from_secs(5);
                        loop {
                            match child.try_wait(os)? {
                                Some(status) => {
                                    *reaped = Some(status);
                                    break;
                                }
                                None if Instant::now() < deadline => {
                                    std::thread::sleep(Duration::from_millis(5));
                                }
                                None => {
                                    return Err(io::Error::new(
                                        io::ErrorKind::TimedOut,
                                        "test child was not reaped during cleanup",
                                    ));
                                }
                            }
                        }
                    }
                }
            }
            child.close(os)
        }

        /// Linux-target negative integration: the real LinuxProcessOs child
        /// receives only the Notes App data root; raw open/write/create and
        /// cross-App rename to another same-UID App/Profile must fail. The
        /// held parent FD must be gone after exec, while the authorized Notes
        /// data file remains writable. Requires a Linux test kernel permitting
        /// clone3(CLONE_NEWPID|CLONE_PIDFD), setresuid, and Landlock ABI >= 5.
        #[test]
        fn notes_child_denies_same_uid_cross_app_profile_and_closes_held_fd() {
            let nonce = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let base = std::env::temp_dir().join(format!(
                "nagi-ws04-landlock-{}-{nonce}",
                std::process::id()
            ));
            let _remove_tree = RemoveTree(base.clone());
            let app = base.join("notes-app-data");
            let other_app = base.join("other-app-data");
            let other_profile = base.join("different-profile-data");
            fs::create_dir_all(&app).unwrap();
            fs::create_dir_all(&other_app).unwrap();
            fs::create_dir_all(&other_profile).unwrap();
            set_mode(&base, 0o777);
            set_mode(&app, 0o777);
            set_mode(&other_app, 0o777);
            set_mode(&other_profile, 0o777);

            let foreign_read = other_app.join("read-me");
            let foreign_create = other_app.join("created-by-probe");
            let foreign_rename_source = other_app.join("move-into-notes");
            let allowed_rename_dest = app.join("moved-from-other-app");
            let allowed_rename_source = app.join("move-out-of-notes");
            let foreign_rename_dest = other_app.join("moved-from-notes");
            let profile_secret = other_profile.join("profile-secret");
            let allowed_write = app.join("notes-entry");
            write_fixture(&foreign_read, b"foreign-read");
            write_fixture(&foreign_rename_source, b"foreign-rename");
            write_fixture(&allowed_rename_source, b"allowed-rename");
            write_fixture(&profile_secret, b"other-profile");
            write_fixture(&allowed_write, b"notes-write");

            // The fixtures are DAC-accessible to the child's single UID. A
            // Landlock denial therefore cannot be explained by file mode.
            let held_file = open_read_without_cloexec(&foreign_read);
            let allowed_root = open_path_directory(&app);
            let rules = [NotesResourceRule {
                path: allowed_root.as_fd(),
                allowed_access_fs: LANDLOCK_NOTES_DATA_ACCESS,
            }];
            let scope = NotesResourceScope { rules: &rules };

            let arguments = [
                foreign_read.to_str().unwrap().to_owned(),
                foreign_create.to_str().unwrap().to_owned(),
                foreign_rename_source.to_str().unwrap().to_owned(),
                allowed_rename_dest.to_str().unwrap().to_owned(),
                allowed_rename_source.to_str().unwrap().to_owned(),
                foreign_rename_dest.to_str().unwrap().to_owned(),
                profile_secret.to_str().unwrap().to_owned(),
                held_file.as_raw_fd().to_string(),
                allowed_write.to_str().unwrap().to_owned(),
            ];
            let arguments = arguments.iter().map(String::as_str).collect::<Vec<_>>();
            let executable = super::super::ExecutablePayload::from_package_executable(
                include_bytes!("landlock_raw_io_probe.x86_64"),
            )
            .expect("static test probe is a valid x86_64 ET_EXEC ELF");

            let mut os = LinuxProcessOs;
            let mut raw_owner = None;
            let prepare_result = ProcessOs::prepare_into_with_notes_scope(
                &mut os,
                executable,
                &arguments,
                super::super::LinuxExecutionIdentity {
                    uid: TEST_UID,
                    gid: TEST_GID,
                },
                Some(&scope),
                &mut raw_owner,
            );
            let mut child = raw_owner.map(TestChildOwner::Prepared);
            let mut run_error = prepare_result.err().map(|error| error.to_string());
            if run_error.is_none() && child.is_none() {
                run_error = Some("prepare returned without retaining its child".to_owned());
            }
            let mut observed_exit = None;

            if run_error.is_none() {
                if let Some(TestChildOwner::Prepared(mut prepared)) = child.take() {
                    let registered = super::super::RegisteredLaunch { record: 1u8 };
                    match ProcessOs::commit_release_gate(
                        &mut os,
                        &mut prepared,
                        &registered,
                        Duration::from_secs(5),
                    ) {
                        Ok(started) => {
                            child = Some(TestChildOwner::ReleaseUncertain(
                                prepared.transition::<ReleaseUncertain>(),
                            ));
                            let mut cancelled = || false;
                            if let Some(TestChildOwner::ReleaseUncertain(released)) = child.as_mut()
                            {
                                if let Err(error) = ProcessOs::wait_exec_ack(
                                    &mut os,
                                    released,
                                    started,
                                    Duration::from_secs(5),
                                    Duration::from_millis(10),
                                    &mut cancelled,
                                ) {
                                    run_error = Some(format!("exec setup failed: {:?}", error.reason));
                                } else {
                                    let deadline = Instant::now() + Duration::from_secs(5);
                                    loop {
                                        match ProcessOs::try_wait(&mut os, released) {
                                            Ok(Some(status)) => {
                                                observed_exit = Some(status);
                                                break;
                                            }
                                            Ok(None) if Instant::now() < deadline => {
                                                std::thread::sleep(Duration::from_millis(5));
                                            }
                                            Ok(None) => {
                                                run_error = Some(
                                                    "Linux probe did not exit before its deadline"
                                                        .to_owned(),
                                                );
                                                break;
                                            }
                                            Err(error) => {
                                                run_error = Some(format!(
                                                    "exact child wait failed: {error}"
                                                ));
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Err(reason) => {
                            child = Some(TestChildOwner::Prepared(prepared));
                            run_error = Some(format!("exec gate failed: {reason:?}"));
                        }
                    }
                }
            }

            let cleanup_result = match child.as_mut() {
                Some(child) => cleanup_child(&mut os, child, &mut observed_exit),
                None => Ok(()),
            };
            assert!(cleanup_result.is_ok(), "exact child cleanup failed: {cleanup_result:?}");
            assert!(run_error.is_none(), "Linux Landlock probe failed: {run_error:?}");
            let status = observed_exit.expect("probe exit was not observed");
            assert_eq!(status.exit_code, Some(0), "negative syscall probe bitmask");
            assert_eq!(status.signal, None, "probe should exit normally");
            assert!(!foreign_create.exists(), "Landlock allowed foreign file creation");
            assert!(allowed_write.exists(), "authorized Notes file disappeared");
        }

        #[test]
        fn lifecycle_fails_unavailable_before_clone_without_c3_scope() {
            let mut lifecycle = super::super::ProcessLifecycle::<
                LinuxProcessOs,
                &'static str,
                u64,
            >::new(LinuxProcessOs);
            let launch = super::super::ProcessLaunch {
                service: "notes",
                executable: super::super::ExecutablePayload {
                    bytes: include_bytes!("landlock_raw_io_probe.x86_64"),
                },
                arguments: &[],
                execution_identity: super::super::LinuxExecutionIdentity {
                    uid: TEST_UID,
                    gid: TEST_GID,
                },
                notes_scope: None,
                required: true,
                release_wait_budget: Duration::from_secs(1),
                cancel_check_interval: Duration::from_millis(10),
            };
            let result = lifecycle.spawn(
                launch,
                || false,
                |_pid| Ok(1u64),
                |_pid, _record| Ok(()),
            );
            assert_eq!(result, Err(super::super::BackendError::Unavailable));
            assert!(lifecycle.acquiring.is_none());
        }
    }
}
