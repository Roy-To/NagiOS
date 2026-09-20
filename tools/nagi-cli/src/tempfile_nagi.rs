use std::path::{Path, PathBuf};

use crate::registry_source::{ensure_registry_checkout, RegistrySourceSpec};

const SPEC: RegistrySourceSpec = RegistrySourceSpec {
    section: "sources.tempfile_nagi",
    component: "tempfile-nagi",
    package: "tempfile",
    version: "3.27.0",
    repository: "https://crates.io/crates/tempfile/3.27.0",
    registry_archive: "https://crates.io/api/v1/crates/tempfile/3.27.0/download",
    source_hash: "sha256:32497e9a4c7b38532efcdebeef879707aa9f794296a4f0244f6f69e9bc8574bd",
    license: "MIT OR Apache-2.0",
    vendored_path: "third_party/tempfile-nagi",
    patch_path: "third_party/tempfile-nagi-patches",
};

pub(crate) fn ensure_tempfile_nagi_checkout(root: &Path) -> Result<PathBuf, String> {
    ensure_registry_checkout(root, &SPEC)
}

#[cfg(test)]
mod tests {
    use super::SPEC;
    use crate::registry_source::validate_source_lock;

    #[test]
    fn tempfile_nagi_lock_is_pinned() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("workspace root");
        validate_source_lock(root, &SPEC).expect("pinned tempfile source lock");
    }
}
