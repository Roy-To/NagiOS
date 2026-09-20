use std::path::{Path, PathBuf};

use crate::registry_source::{ensure_registry_checkout, RegistrySourceSpec};

const SPEC: RegistrySourceSpec = RegistrySourceSpec {
    section: "sources.freetype_sys",
    component: "freetype-sys",
    package: "freetype-sys",
    version: "0.23.0",
    repository: "https://crates.io/crates/freetype-sys/0.23.0",
    registry_archive: "https://crates.io/api/v1/crates/freetype-sys/0.23.0/download",
    source_hash: "sha256:eab537ce43cab850c64b4cdc390ce7e4f47f877485ddc323208e268280c308ae",
    license: "MIT",
    vendored_path: "third_party/freetype-sys",
    patch_path: "third_party/freetype-sys-patches",
};

pub(crate) fn ensure_freetype_sys_checkout(root: &Path) -> Result<PathBuf, String> {
    ensure_registry_checkout(root, &SPEC)
}

#[cfg(test)]
mod tests {
    use super::SPEC;
    use crate::registry_source::validate_source_lock;

    #[test]
    fn freetype_sys_lock_is_pinned() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(std::path::Path::parent)
            .expect("workspace root");
        validate_source_lock(root, &SPEC).expect("pinned freetype-sys source lock");
    }
}
