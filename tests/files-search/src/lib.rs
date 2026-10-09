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
