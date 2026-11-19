#![no_std]

extern crate alloc;

#[cfg(any(target_os = "linux", target_os = "macos"))]
extern crate std;

#[cfg(any(target_os = "linux", target_os = "macos"))]
pub mod linux_package_persistence;
pub mod package_trust;
