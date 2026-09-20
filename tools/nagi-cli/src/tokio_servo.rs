use std::path::{Path, PathBuf};

use crate::registry_source::{ensure_registry_checkout, RegistrySourceSpec};

const SPEC: RegistrySourceSpec = RegistrySourceSpec {
    section: "sources.tokio_servo",
    component: "tokio-servo",
    package: "tokio",
    version: "1.53.1",
    repository: "https://crates.io/crates/tokio/1.53.1",
    registry_archive: "https://crates.io/api/v1/crates/tokio/1.53.1/download",
    source_hash: "sha256:202caea871b69668250d242070849eb495be178ed697a3e98aebce5bc81a0bed",
    license: "MIT",
    vendored_path: "third_party/tokio-servo",
    patch_path: "third_party/tokio-servo-patches",
};

pub(crate) fn ensure_tokio_servo_checkout(root: &Path) -> Result<PathBuf, String> {
    ensure_registry_checkout(root, &SPEC)
}

#[cfg(test)]
mod tests {
    use super::SPEC;
    use crate::registry_source::validate_source_lock;

    #[test]
    fn tokio_servo_lock_is_pinned() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("workspace root");
        validate_source_lock(root, &SPEC).expect("pinned tokio source lock");
    }
}
