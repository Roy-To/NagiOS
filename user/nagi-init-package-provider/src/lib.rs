#![no_std]

extern crate alloc;

#[cfg(unix)]
extern crate std;

#[cfg(unix)]
pub mod linux_package_persistence;
pub mod package_trust;
