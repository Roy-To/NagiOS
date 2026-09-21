use std::path::{Path, PathBuf};

use crate::registry_source::{ensure_registry_checkout, RegistrySourceSpec};

const SPEC: RegistrySourceSpec = RegistrySourceSpec {
    section: "sources.cc_nagi",
    component: "cc-nagi",
    package: "cc",
    version: "1.4.6",
    repository: "https://crates.io/crates/cc/1.4.6",
    registry_archive: "https://crates.io/api/v1/crates/cc/1.4.6/download",
    source_hash: "sha256:a3eb0f42d6c360dc3f8a821f6bf2fdea7f72bfd36b3076eb0e6d1e9e0752fff4",
    license: "MIT OR Apache-2.0",
    vendored_path: "third_party/cc-nagi",
    patch_path: "third_party/cc-nagi-patches",
};

pub(crate) fn ensure_cc_nagi_checkout(root: &Path) -> Result<PathBuf, String> {
    ensure_registry_checkout(root, &SPEC)
}

#[cfg(test)]
mod tests {
    use super::SPEC;
    use crate::registry_source::validate_source_lock;

    #[test]
    fn cc_nagi_lock_is_pinned() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("workspace root");
        validate_source_lock(root, &SPEC).expect("pinned cc source lock");
    }

    #[test]
    fn cc_nagi_patch_targets_nagi_without_host_cxx_runtime() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("workspace root");
        let patch = std::fs::read_to_string(
            root.join("third_party/cc-nagi-patches/0001-nagi-no-host-cxx-runtime.patch"),
        )
        .expect("cc Nagi patch");
        assert!(patch.contains("target.os == \"nagi\""));
        assert!(patch.contains("Ok(None)"));
    }
}
