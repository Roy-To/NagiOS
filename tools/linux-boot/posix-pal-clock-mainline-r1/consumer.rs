#![no_std]

extern crate self as nagi_pal;

#[path = "../../../user/nagi-pal/src/time.rs"]
pub mod time;
pub use time::{Clock, TimeError};

#[path = "../../../user/nagi-posix/src/errno.rs"]
pub mod errno;
#[path = "../../../user/nagi-posix/src/posix_time.rs"]
mod posix_time;

#[panic_handler]
fn panic(_information: &core::panic::PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}

// `panic=abort` needs no unwinding personality; this link-only symbol satisfies
// the compiler-builtins archive when the smoke-only staticlib is linked by Zig.
#[unsafe(no_mangle)]
pub extern "C" fn rust_eh_personality() {}

/// Smoke-only C entry that exercises the same shared POSIX/PAL implementation
/// called by `nagi-posix/src/abi.rs`; it is not part of the product ABI.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_pal_posix_clock_gettime(
    clock_id: i32,
    output: *mut posix_time::PosixTimespec,
) -> i32 {
    match unsafe { posix_time::clock_gettime(clock_id, output, &time::GuestClock) } {
        Ok(()) => 0,
        Err(error) => {
            errno::set_errno(error);
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn nagi_pal_posix_errno() -> i32 {
    errno::errno()
}
