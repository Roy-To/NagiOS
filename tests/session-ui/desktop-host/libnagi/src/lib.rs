//! HOST-ONLY Desktop syscall boundary. Session, credential, VFS and renderer
//! implementations are production code. This shim never provides inference.
//! Nagi-target checks use the real libnagi dependency, never this crate.
#[cfg(target_os = "nagi")]
compile_error!("the Desktop host-test syscall boundary must never run on Nagi");
pub use real_libnagi::*;
use std::cell::RefCell;

#[derive(Default)]
struct Platform {
    readiness: bool,
    readiness_calls: usize,
    console: Vec<u8>,
    ticks: u64,
    fail_writes: bool,
}
thread_local! { static PLATFORM: RefCell<Platform> = RefCell::new(Platform::default()); }
pub fn set_readiness(ready: bool) {
    PLATFORM.with(|state| {
        *state.borrow_mut() = Platform {
            readiness: ready,
            ..Platform::default()
        }
    });
}
pub fn readiness_calls() -> usize {
    PLATFORM.with(|state| state.borrow().readiness_calls)
}
pub fn report_boot_ready() -> bool {
    PLATFORM.with(|state| {
        let mut state = state.borrow_mut();
        state.readiness_calls += 1;
        state.readiness
    })
}
pub fn console_write(bytes: &[u8]) -> usize {
    PLATFORM.with(|state| state.borrow_mut().console.extend_from_slice(bytes));
    bytes.len()
}
pub fn time_ticks() -> u64 {
    PLATFORM.with(|state| 100 + state.borrow().ticks)
}
pub fn set_ticks(ticks: u64) {
    PLATFORM.with(|state| state.borrow_mut().ticks = ticks);
}
pub fn fail_writes(fail: bool) {
    PLATFORM.with(|state| state.borrow_mut().fail_writes = fail);
}
pub fn console_text() -> String {
    PLATFORM.with(|state| String::from_utf8(state.borrow().console.clone()).unwrap())
}
pub fn random_fill(_: &mut [u8]) -> bool {
    panic!("existing-account regression must not request entropy or create credentials")
}
pub mod storage {
    pub use real_libnagi::storage::*;
    /// In-memory block transport only. The actual Nagi VFS performs all account
    /// loading, decoding, writes, checksums and login-throttle persistence.
    #[derive(Clone, Debug)]
    pub struct SyscallBlockDevice {
        sectors: Vec<[u8; SECTOR_SIZE]>,
    }
    impl SyscallBlockDevice {
        pub fn new(_: u64) -> Self {
            Self {
                sectors: vec![[0; SECTOR_SIZE]; 16384],
            }
        }
    }
    impl ReadOnlyBlockDevice for SyscallBlockDevice {
        fn read_sector(
            &mut self,
            sector: u64,
            destination: &mut [u8; SECTOR_SIZE],
        ) -> Result<(), StorageError> {
            destination.copy_from_slice(
                self.sectors
                    .get(sector as usize)
                    .ok_or(StorageError::Block)?,
            );
            Ok(())
        }
    }
    impl BlockDevice for SyscallBlockDevice {
        fn write_sector(
            &mut self,
            sector: u64,
            source: &[u8; SECTOR_SIZE],
        ) -> Result<(), StorageError> {
            if super::PLATFORM.with(|state| state.borrow().fail_writes) {
                return Err(StorageError::Block);
            }
            self.sectors
                .get_mut(sector as usize)
                .ok_or(StorageError::Block)?
                .copy_from_slice(source);
            Ok(())
        }
        fn flush(&mut self) -> Result<(), StorageError> {
            Ok(())
        }
    }
}

// Finite syscall script for the exact Files modal wait. Exhaustion fails the
// test instead of allowing the historical pause-only loop to hang the host.
#[derive(Default)]
struct ModalScript {
    input: std::collections::VecDeque<Option<InputEvent>>,
    yields: usize,
    reads: usize,
    presents: usize,
    fail_present: bool,
}
thread_local! { static MODAL: RefCell<ModalScript> = RefCell::new(ModalScript::default()); }
pub fn script_modal(input: Vec<Option<InputEvent>>, fail_present: bool) {
    MODAL.with(|state| {
        *state.borrow_mut() = ModalScript {
            input: input.into(),
            fail_present,
            ..ModalScript::default()
        };
    });
}
pub fn modal_counts() -> (usize, usize, usize) {
    MODAL.with(|state| {
        let state = state.borrow();
        (state.yields, state.reads, state.presents)
    })
}
pub fn thread_yield() -> bool {
    MODAL.with(|state| state.borrow_mut().yields += 1);
    true
}
pub fn input_read(_: u64, event: &mut InputEvent) -> bool {
    MODAL.with(|state| {
        let mut state = state.borrow_mut();
        state.reads += 1;
        match state
            .input
            .pop_front()
            .expect("modal input script exhausted")
        {
            Some(next) => {
                *event = next;
                true
            }
            None => false,
        }
    })
}
pub fn display_present(_: u64) -> bool {
    MODAL.with(|state| {
        let mut state = state.borrow_mut();
        state.presents += 1;
        !state.fail_present
    })
}

// Explicit orchestration-only Channel/process script. No kernel execution.
#[derive(Default)]
struct FilesScript {
    incoming: std::collections::VecDeque<Option<ChannelReceiveResult>>,
    exits: std::collections::VecDeque<Option<ProcessExitStatus>>,
    sent: Vec<ChannelSendRequest>,
    closed: Vec<u64>,
    peer_closed: bool,
    receives: usize,
    polls: usize,
}
thread_local! { static FILES: RefCell<FilesScript> = RefCell::new(FilesScript::default()); }
pub fn script_files(
    incoming: Vec<Option<ChannelReceiveResult>>,
    exits: Vec<Option<ProcessExitStatus>>,
) {
    FILES.with(|s| {
        *s.borrow_mut() = FilesScript {
            incoming: incoming.into(),
            exits: exits.into(),
            ..FilesScript::default()
        }
    });
}
pub fn files_counts() -> (usize, usize, usize, Vec<u64>) {
    FILES.with(|s| {
        let s = s.borrow();
        (s.receives, s.polls, s.sent.len(), s.closed.clone())
    })
}
pub fn channel_try_receive(_: u64, message: &mut ChannelReceiveResult) -> Option<bool> {
    FILES.with(|s| {
        let mut s = s.borrow_mut();
        s.receives += 1;
        match s
            .incoming
            .pop_front()
            .expect("Files receive script exhausted")
        {
            Some(next) => {
                *message = next;
                Some(true)
            }
            None => Some(false),
        }
    })
}
pub fn channel_send(_: u64, request: &ChannelSendRequest) -> bool {
    FILES.with(|s| {
        let mut s = s.borrow_mut();
        s.sent.push(*request);
        !s.peer_closed
    })
}
pub fn process_try_wait(_: u32) -> Option<Option<ProcessExitStatus>> {
    FILES.with(|s| {
        let mut s = s.borrow_mut();
        s.polls += 1;
        let next = s.exits.pop_front().expect("Files exit script exhausted");
        if next.is_some() {
            s.peer_closed = true;
        }
        Some(next)
    })
}
pub fn handle_close(handle: u64) -> bool {
    FILES.with(|s| s.borrow_mut().closed.push(handle));
    true
}
pub fn channel_receive(_: u64, _: &mut ChannelReceiveResult) -> Option<()> {
    panic!("blocking Channel receive must not run on the Desktop foreground")
}
pub fn process_wait(_: u32) -> Option<ProcessExitStatus> {
    panic!("blocking process wait must not run on the Desktop foreground")
}
