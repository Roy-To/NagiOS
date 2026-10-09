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
    100
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
