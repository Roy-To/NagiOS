use std::path::{Path, PathBuf};

use crate::registry_source::{ensure_registry_checkout, RegistrySourceSpec};

const SPEC: RegistrySourceSpec = RegistrySourceSpec {
    section: "sources.hyper_util_servo",
    component: "hyper-util-servo",
    package: "hyper-util",
    version: "0.1.20",
    repository: "https://crates.io/crates/hyper-util/0.1.20",
    registry_archive: "https://crates.io/api/v1/crates/hyper-util/0.1.20/download",
    source_hash: "sha256:96547c2556ec9d12fb1578c4eaf448b04993e7fb79cbaad930a656880a6bdfa0",
    license: "MIT",
    vendored_path: "third_party/hyper-util-servo",
    patch_path: "third_party/hyper-util-servo-patches",
};

pub(crate) fn ensure_hyper_util_servo_checkout(root: &Path) -> Result<PathBuf, String> {
    ensure_registry_checkout(root, &SPEC)
}

#[cfg(test)]
mod tests {
    use super::SPEC;
    use crate::registry_source::validate_source_lock;

    #[test]
    fn hyper_util_servo_lock_is_pinned() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("workspace root");
        validate_source_lock(root, &SPEC).expect("pinned hyper-util source lock");
    }
}
