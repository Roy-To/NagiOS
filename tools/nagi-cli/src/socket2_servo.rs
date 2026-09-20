use std::path::{Path, PathBuf};

use crate::registry_source::{ensure_registry_checkout, RegistrySourceSpec};

const SPEC: RegistrySourceSpec = RegistrySourceSpec {
    section: "sources.socket2_servo",
    component: "socket2-servo",
    package: "socket2",
    version: "0.6.5",
    repository: "https://crates.io/crates/socket2/0.6.5",
    registry_archive: "https://crates.io/api/v1/crates/socket2/0.6.5/download",
    source_hash: "sha256:c3d1e2c7f27f8d4cb10542a02c49005dbd6e93095799d6f3be745fae9f8fedd4",
    license: "MIT OR Apache-2.0",
    vendored_path: "third_party/socket2-servo",
    patch_path: "third_party/socket2-servo-patches",
};

pub(crate) fn ensure_socket2_servo_checkout(root: &Path) -> Result<PathBuf, String> {
    ensure_registry_checkout(root, &SPEC)
}

#[cfg(test)]
mod tests {
    use super::SPEC;
    use crate::registry_source::validate_source_lock;

    #[test]
    fn socket2_servo_lock_is_pinned() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("workspace root");
        validate_source_lock(root, &SPEC).expect("pinned socket2 source lock");
    }
}
