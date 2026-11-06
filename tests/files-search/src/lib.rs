#![cfg_attr(target_os = "nagi", no_std)]

extern crate alloc;

#[allow(dead_code)]
#[path = "../../../user/nagi-init/src/recovery.rs"]
#[cfg(any(feature = "m27-recovery", test))]
mod recovery;
#[allow(dead_code)]
#[path = "../../../user/nagi-init/src/vfs_recovery.rs"]
mod vfs_recovery;

#[cfg(test)]
mod recovery_tests;
