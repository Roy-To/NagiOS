//! POSIX C library, implemented in Rust.
//!
//! This crate exists to provide a standard libc as its public API. This is
//! largely provided by automatically generated bindings to the functions and
//! data structures in the [`header`] module.
//!
//! Currently, Linux and Redox syscall backends are supported.

#![no_std]
#![feature(alloc_error_handler)]
#![feature(c_variadic)]
#![feature(core_intrinsics)]
#![feature(lang_items)]
#![feature(linkage)]
#![feature(ptr_as_uninit)]
#![feature(slice_ptr_get)]
#![feature(stmt_expr_attributes)]
#![feature(sync_unsafe_cell)]
#![feature(thread_local)]
#![feature(negative_impls)]

#[cfg(not(target_os = "nagi"))]
#[macro_use]
extern crate alloc;
#[cfg(not(target_os = "nagi"))]
extern crate cbitset;
#[cfg(not(target_os = "nagi"))]
extern crate memchr;
#[cfg(not(target_os = "nagi"))]
extern crate posix_regex;
#[cfg(not(target_os = "nagi"))]
extern crate rand;

#[cfg(target_os = "linux")]
#[macro_use]
extern crate sc;

#[cfg(target_os = "redox")]
extern crate syscall;

#[cfg(target_os = "nagi")]
mod nagi;

#[cfg(not(target_os = "nagi"))]
#[macro_use]
mod macros;
#[cfg(not(target_os = "nagi"))]
pub mod c_str;
#[cfg(not(target_os = "nagi"))]
pub mod c_vec;
#[cfg(not(target_os = "nagi"))]
pub mod casting;
#[cfg(not(target_os = "nagi"))]
pub mod cxa;
#[cfg(not(target_os = "nagi"))]
pub mod db;
#[cfg(not(target_os = "nagi"))]
pub mod error;
#[cfg(not(target_os = "nagi"))]
pub mod fs;
#[cfg(not(target_os = "nagi"))]
pub mod header;
#[cfg(not(target_os = "nagi"))]
pub mod io;
#[cfg(not(target_os = "nagi"))]
pub mod iter;
#[cfg(not(target_os = "nagi"))]
pub mod ld_so;
#[cfg(not(target_os = "nagi"))]
pub mod out;
#[cfg(not(target_os = "nagi"))]
pub mod panic;
#[cfg(not(target_os = "nagi"))]
pub mod platform;
#[cfg(not(target_os = "nagi"))]
pub mod pthread;
#[cfg(not(target_os = "nagi"))]
pub mod raw_cell;
#[cfg(not(target_os = "nagi"))]
pub mod start;
#[cfg(not(target_os = "nagi"))]
pub mod sync;

#[cfg(not(target_os = "nagi"))]
use crate::platform::{Allocator, NEWALLOCATOR};

#[cfg(not(target_os = "nagi"))]
#[global_allocator]
static ALLOCATOR: Allocator = NEWALLOCATOR;

#[cfg(target_os = "nagi")]
pub use nagi::nagi_backend_probe;

#[cfg(all(not(test), not(target_os = "nagi")))]
#[panic_handler]
#[linkage = "weak"]
pub fn rust_begin_unwind(pi: &::core::panic::PanicInfo) -> ! {
    crate::panic::relibc_panic(pi)
}

#[cfg(all(not(test), not(target_os = "nagi")))]
#[lang = "eh_personality"]
#[linkage = "weak"]
pub extern "C" fn rust_eh_personality() {}

#[cfg(all(not(test), not(target_os = "nagi")))]
#[alloc_error_handler]
#[linkage = "weak"]
#[expect(improper_ctypes_definitions)]
#[unsafe(no_mangle)]
pub extern "C" fn rust_oom(layout: ::core::alloc::Layout) -> ! {
    panic!(
        "RELIBC OOM: {} bytes aligned to {} bytes",
        layout.size(),
        layout.align()
    );
}

#[cfg(all(not(test), not(target_os = "nagi")))]
#[allow(non_snake_case)]
#[linkage = "weak"]
#[unsafe(no_mangle)]
pub extern "C" fn _Unwind_Resume() -> ! {
    panic!("_Unwind_Resume")
}
