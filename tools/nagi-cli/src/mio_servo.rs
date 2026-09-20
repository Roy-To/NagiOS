use std::path::{Path, PathBuf};

use crate::registry_source::{ensure_registry_checkout, RegistrySourceSpec};

const SPEC: RegistrySourceSpec = RegistrySourceSpec {
    section: "sources.mio_servo",
    component: "mio-servo",
    package: "mio",
    version: "1.2.3",
    repository: "https://crates.io/crates/mio/1.2.3",
    registry_archive: "https://crates.io/api/v1/crates/mio/1.2.3/download",
    source_hash: "sha256:4b18443e9c262bfe8fa82f51666e2642c53393f7e5c27b3e1aeab922cff5b9d8",
    license: "MIT",
    vendored_path: "third_party/mio-servo",
    patch_path: "third_party/mio-servo-patches",
};

pub(crate) fn ensure_mio_servo_checkout(root: &Path) -> Result<PathBuf, String> {
    ensure_registry_checkout(root, &SPEC)
}

#[cfg(test)]
mod tests {
    use super::SPEC;
    use crate::registry_source::validate_source_lock;

    #[test]
    fn mio_servo_lock_is_pinned() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("workspace root");
        validate_source_lock(root, &SPEC).expect("pinned mio source lock");
    }
}
