#![cfg_attr(target_os = "nagi", no_std)]

extern crate alloc;
#[path = "../../../user/nagi-init/src/m19_storage.rs"]
mod m19_storage;
mod desktop {
    #[cfg(test)]
    pub(crate) use crate::files_panel;
    pub(crate) use crate::m19_runtime::files;
}
#[path = "../../../user/nagi-init/src/m19_files_panel.rs"]
pub(crate) mod files_panel;
#[path = "../../../user/nagi-init/src/m19_runtime.rs"]
mod m19_runtime;
#[path = "../../../user/nagi-init/src/recovery.rs"]
#[cfg(any(feature = "m27-recovery", test))]
mod recovery;
#[path = "../../../user/nagi-init/src/vfs_recovery.rs"]
mod vfs_recovery;

#[cfg(test)]
mod recovery_tests;
