//! Host-only compiler discovery regressions; fake tools exist only in child fixtures.
use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "nagi-std-discovery-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("other-cwd")).unwrap();
        fs::create_dir_all(root.join("bin")).unwrap();
        let patch = root.join("third_party/rust-std/patches/0001-nagi-target-support.patch");
        fs::create_dir_all(patch.parent().unwrap()).unwrap();
        fs::write(patch, "diff --git a/library/test.rs b/library/test.rs\n--- a/library/test.rs\n+++ b/library/test.rs\n@@ -1 +1 @@\n-before\n+after\n").unwrap();
        assert!(ProcessCommand::new("git")
            .args(["init", "--quiet"])
            .current_dir(&root)
            .status()
            .unwrap()
            .success());
        fs::write(root.join(".gitattributes"), "* text eol=lf\n").unwrap();
        Self(root)
    }

    #[cfg(unix)]
    fn compiler(&self, name: &str, executable: &Path) {
        use std::os::unix::fs::PermissionsExt;
        let sysroot = self.0.join(name);
        let library = sysroot.join("lib/rustlib/src/rust/library");
        fs::create_dir_all(&library).unwrap();
        fs::write(
            library.join("Cargo.toml"),
            "[workspace]\nmembers=[]\n[patch.crates-io]\n",
        )
        .unwrap();
        fs::write(library.join("test.rs"), "before\n").unwrap();
        fs::write(library.join("identity"), name).unwrap();
        fs::write(executable, format!(
            "#!/bin/sh\nprintf '%s\\n' \"$PWD\" >> '{}/{}-queries'\nif [ \"$1\" = --print ]; then printf '%s\\n' '{}'; else printf '%s\\n' 'rustc {}'; fi\n",
            self.0.display(), name, sysroot.display(), name
        )).unwrap();
        fs::set_permissions(executable, fs::Permissions::from_mode(0o700)).unwrap();
    }

    fn child(&self) -> ProcessCommand {
        let mut child = ProcessCommand::new(std::env::current_exe().unwrap());
        child
            .args([
                "--exact",
                "commands::rust_std_source_boundary_tests::discovery_child",
                "--ignored",
                "--nocapture",
            ])
            .current_dir(self.0.join("other-cwd"))
            .env("NAGI_STD_DISCOVERY_ROOT", &self.0);
        let mut path = vec![self.0.join("bin")];
        path.extend(std::env::split_paths(
            &std::env::var_os("PATH").unwrap_or_default(),
        ));
        child.env("PATH", std::env::join_paths(path).unwrap());
        child
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
#[ignore = "child of isolated compiler discovery host fixture"]
fn discovery_child() {
    let root = PathBuf::from(std::env::var_os("NAGI_STD_DISCOVERY_ROOT").unwrap());
    let prepared = prepare_nagi_rust_std_source(&root).unwrap();
    let library = &prepared.library;
    let identity = fs::read_to_string(library.join("identity")).unwrap();
    assert_eq!(
        identity,
        std::env::var("NAGI_STD_EXPECTED_IDENTITY").unwrap()
    );
}

#[cfg(unix)]
#[test]
fn compiler_discovery_honors_rustc_over_path() {
    let f = Fixture::new();
    f.compiler("path", &f.0.join("bin/rustc"));
    let selected = f.0.join("selected-rustc");
    f.compiler("selected", &selected);
    let output = f
        .child()
        .env("RUSTC", selected)
        .env("NAGI_STD_EXPECTED_IDENTITY", "selected")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!f.0.join("path-queries").exists());
    let queries = fs::read_to_string(f.0.join("selected-queries")).unwrap();
    assert!(queries.lines().all(|line| Path::new(line) == f.0));
}

#[cfg(unix)]
#[test]
fn compiler_discovery_uses_repo_cwd_with_path_fallback() {
    let f = Fixture::new();
    f.compiler("path", &f.0.join("bin/rustc"));
    let output = f
        .child()
        .env_remove("RUSTC")
        .env("NAGI_STD_EXPECTED_IDENTITY", "path")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let queries = fs::read_to_string(f.0.join("path-queries")).unwrap();
    assert_eq!(queries.lines().count(), 2);
    assert!(
        queries.lines().all(|line| Path::new(line) == f.0),
        "{queries}"
    );
}

#[cfg(unix)]
#[test]
fn compiler_discovery_resolves_relative_rustc_in_repo_cwd() {
    let f = Fixture::new();
    f.compiler("path", &f.0.join("bin/rustc"));
    f.compiler("selected", &f.0.join("selected-rustc"));
    let output = f
        .child()
        .env("RUSTC", "./selected-rustc")
        .env("NAGI_STD_EXPECTED_IDENTITY", "selected")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!f.0.join("path-queries").exists());
}

#[cfg(unix)]
#[test]
fn an_invalid_explicit_rustc_does_not_fall_back_to_path() {
    let f = Fixture::new();
    f.compiler("path", &f.0.join("bin/rustc"));
    let output = f
        .child()
        .env("RUSTC", f.0.join("absent-rustc"))
        .env("NAGI_STD_EXPECTED_IDENTITY", "path")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("cannot query rustc sysroot"));
    assert!(!f.0.join("path-queries").exists());
}
