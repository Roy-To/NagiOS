use std::{
    ffi::OsStr,
    format, fs,
    path::{Path, PathBuf},
    string::String,
};

/// Resolve the repository root from the two supported consumers of this shared
/// build script instead of searching arbitrary parent directories for a marker.
pub fn expected_repository_root(manifest_dir: &Path) -> Result<PathBuf, String> {
    let manifest_dir = fs::canonicalize(manifest_dir)
        .map_err(|error| format!("canonicalize package manifest directory: {error}"))?;
    if !manifest_dir.join("Cargo.toml").is_file() {
        return Err("package manifest directory has no Cargo.toml".into());
    }

    let package_name = manifest_dir
        .file_name()
        .ok_or_else(|| String::from("package manifest directory has no final path component"))?;
    let tests_dir = if package_name == OsStr::new("session-ui") {
        let tests_dir = manifest_dir
            .parent()
            .ok_or_else(|| String::from("session-ui package has no tests parent"))?;
        if tests_dir.file_name() != Some(OsStr::new("tests")) {
            return Err("session-ui package is not under the expected tests directory".into());
        }
        tests_dir
    } else if package_name == OsStr::new("desktop-host") {
        let session_ui_dir = manifest_dir
            .parent()
            .ok_or_else(|| String::from("desktop-host package has no session-ui parent"))?;
        let tests_dir = session_ui_dir
            .parent()
            .ok_or_else(|| String::from("session-ui package has no tests parent"))?;
        if session_ui_dir.file_name() != Some(OsStr::new("session-ui"))
            || tests_dir.file_name() != Some(OsStr::new("tests"))
        {
            return Err(
                "desktop-host package is not under the expected tests/session-ui layout".into(),
            );
        }
        tests_dir
    } else {
        return Err("shared build script used by an unsupported package layout".into());
    };

    tests_dir
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| String::from("tests directory has no repository root parent"))
}

/// Identify the Desktop host test consumer after validating its supported
/// manifest layout. Its test-only Adapter hooks are not used by session-ui.
pub fn is_desktop_host_test_package(manifest_dir: &Path) -> Result<bool, String> {
    let manifest_dir = fs::canonicalize(manifest_dir)
        .map_err(|error| format!("canonicalize package manifest directory: {error}"))?;
    expected_repository_root(&manifest_dir)?;
    Ok(manifest_dir.file_name() == Some(OsStr::new("desktop-host")))
}

/// Canonicalize a required repository file and reject symlinks that escape root.
pub fn canonical_file_within_root(
    repository_root: &Path,
    candidate: &Path,
    description: &str,
) -> Result<PathBuf, String> {
    let repository_root = fs::canonicalize(repository_root)
        .map_err(|error| format!("canonicalize expected repository root: {error}"))?;
    let candidate = fs::canonicalize(candidate)
        .map_err(|error| format!("canonicalize {description}: {error}"))?;
    if !candidate.starts_with(&repository_root) {
        return Err(format!(
            "{description} resolves outside the expected repository root"
        ));
    }
    if !candidate.is_file() {
        return Err(format!("{description} is not a regular file"));
    }
    Ok(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        format, fs,
        sync::atomic::{AtomicU64, Ordering},
        time::{SystemTime, UNIX_EPOCH},
    };

    struct TestTree(PathBuf);

    impl TestTree {
        fn new() -> Self {
            static NEXT_ID: AtomicU64 = AtomicU64::new(0);
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock is after Unix epoch")
                .as_nanos();
            let id = NEXT_ID.fetch_add(1, Ordering::Relaxed);
            let root = std::env::temp_dir().join(format!(
                "nagi-session-ui-path-{}-{now}-{id}",
                std::process::id()
            ));
            fs::create_dir(&root).expect("create unique path-guard test tree");
            Self(root)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestTree {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn create_manifest(directory: &Path) {
        fs::create_dir_all(directory).expect("create package directory");
        fs::write(
            directory.join("Cargo.toml"),
            "[package]\nname = \"fixture\"\n",
        )
        .expect("write package manifest");
    }

    #[test]
    fn resolves_both_shared_build_script_package_layouts() {
        let tree = TestTree::new();
        let repository_root = tree.path().join("checkout");
        let session_ui = repository_root.join("tests/session-ui");
        let desktop_host = session_ui.join("desktop-host");
        create_manifest(&session_ui);
        create_manifest(&desktop_host);

        let expected = fs::canonicalize(&repository_root).expect("canonicalize fixture root");
        assert_eq!(expected_repository_root(&session_ui), Ok(expected.clone()));
        assert_eq!(expected_repository_root(&desktop_host), Ok(expected));
        assert_eq!(is_desktop_host_test_package(&session_ui), Ok(false));
        assert_eq!(is_desktop_host_test_package(&desktop_host), Ok(true));
    }

    #[test]
    fn nested_checkout_does_not_select_an_outer_source_marker() {
        let tree = TestTree::new();
        let outer_root = tree.path().join("outer");
        let nested_root = outer_root.join("nested-checkout");
        let nested_manifest = nested_root.join("tests/session-ui");
        create_manifest(&nested_manifest);

        let outer_source = outer_root.join("user/nagi-init/src/session_services.rs");
        let outer_fixture =
            outer_root.join("user/nagi-model-manager/tests/fixtures/granite-4.2-3b.json");
        fs::create_dir_all(outer_source.parent().unwrap()).expect("create outer source parent");
        fs::create_dir_all(outer_fixture.parent().unwrap()).expect("create outer fixture parent");
        fs::write(&outer_source, "outer source marker").expect("write outer source marker");
        fs::write(&outer_fixture, "outer fixture").expect("write outer fixture");

        let expected = fs::canonicalize(&nested_root).expect("canonicalize nested root");
        assert_eq!(
            expected_repository_root(&nested_manifest),
            Ok(expected.clone())
        );
        assert!(canonical_file_within_root(&expected, &outer_source, "source").is_err());
        assert!(canonical_file_within_root(&expected, &outer_fixture, "fixture").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_fixture_symlink_that_resolves_outside_repository_root() {
        use std::os::unix::fs::symlink;

        let tree = TestTree::new();
        let repository_root = tree.path().join("checkout");
        let fixture =
            repository_root.join("user/nagi-model-manager/tests/fixtures/granite-4.2-3b.json");
        let outside_fixture = tree.path().join("outside.json");
        fs::create_dir_all(fixture.parent().unwrap()).expect("create fixture directory");
        fs::write(&outside_fixture, "outside fixture").expect("write outside fixture");
        symlink(&outside_fixture, &fixture).expect("create escaping fixture symlink");

        assert!(
            canonical_file_within_root(&repository_root, &fixture, "fixture")
                .unwrap_err()
                .contains("outside the expected repository root")
        );
    }
}
