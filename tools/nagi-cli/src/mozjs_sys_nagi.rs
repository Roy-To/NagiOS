use std::path::{Path, PathBuf};

use crate::registry_source::{ensure_registry_checkout, RegistrySourceSpec};

const SPEC: RegistrySourceSpec = RegistrySourceSpec {
    section: "sources.mozjs_sys_nagi",
    component: "mozjs-sys-nagi",
    package: "mozjs_sys",
    version: "153.0.0-2",
    repository: "https://crates.io/crates/mozjs_sys/153.0.0-2",
    registry_archive: "https://crates.io/api/v1/crates/mozjs_sys/153.0.0-2/download",
    source_hash: "sha256:28adaa4255fd0d42133b993ff81d391df41de1d718777f2e3f0aee5ba8636f10",
    license: "MPL-2.0",
    vendored_path: "third_party/mozjs-sys-nagi",
    patch_path: "third_party/mozjs-sys-nagi-patches",
};

pub(crate) fn ensure_mozjs_sys_nagi_checkout(root: &Path) -> Result<PathBuf, String> {
    ensure_registry_checkout(root, &SPEC)
}

#[cfg(test)]
mod tests {
    use super::SPEC;
    use crate::registry_source::validate_source_lock;

    #[test]
    fn mozjs_sys_nagi_lock_is_pinned() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("workspace root");
        validate_source_lock(root, &SPEC).expect("pinned mozjs_sys source lock");
    }

    #[test]
    fn mozjs_configure_uses_supported_freestanding_triplet() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("workspace root");
        let patch = std::fs::read_to_string(root.join(
            "third_party/mozjs-sys-nagi-patches/0001-nagi-freestanding-configure-target.patch",
        ))
        .expect("mozjs configure patch");
        assert!(patch.contains("--target=x86_64-unknown-nagi"));
        assert!(patch.contains("AR = llvm-ar"));
        assert!(!patch.contains("--target=x86_64-unknown-linux-gnu"));

        let native_os_patch = std::fs::read_to_string(
            root.join("third_party/mozjs-sys-nagi-patches/0002-nagi-native-configure-os.patch"),
        )
        .expect("mozjs native Nagi configure patch");
        assert!(native_os_patch.contains("canonical_os = canonical_kernel = \"Nagi\""));
        assert!(native_os_patch.contains("\"Nagi\": \"__NAGI__\""));

        let time_patch = std::fs::read_to_string(
            root.join("third_party/mozjs-sys-nagi-patches/0004-nagi-timestamp-platform.patch"),
        )
        .expect("mozjs Nagi timestamp patch");
        assert!(time_patch.contains("CONFIG[\"OS_TARGET\"] == \"Nagi\""));
        assert!(time_patch.contains("/mozglue/misc/TimeStamp_posix.cpp"));

        let compiler_wrapper = std::fs::read_to_string(root.join("tools/nagi-target-cc.sh"))
            .expect("Nagi target compiler wrapper");
        assert!(compiler_wrapper.contains("NAGI_CXX_HEADERS"));
        assert!(compiler_wrapper.contains("-isystem"));
        assert!(compiler_wrapper.contains("-idirafter"));
        assert!(compiler_wrapper.contains("cxx_include_args"));
    }
}
