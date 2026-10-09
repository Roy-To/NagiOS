use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

static NEXT: AtomicUsize = AtomicUsize::new(0);

struct Fixture {
    root: PathBuf,
    source: PathBuf,
    patch: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "nagi-rust-source-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("installed/library")).unwrap();
        let source = root.join("installed");
        fs::write(
            source.join("library/Cargo.toml"),
            "[workspace]\nmembers=[]\n[patch.crates-io]\n",
        )
        .unwrap();
        fs::write(source.join("library/test.rs"), "before\n").unwrap();
        fs::write(source.join("library/untouched.rs"), "one\n").unwrap();
        let patch = root.join("nagi.patch");
        fs::write(&patch, "diff --git a/library/test.rs b/library/test.rs\n--- a/library/test.rs\n+++ b/library/test.rs\n@@ -1 +1 @@\n-before\n+after\n").unwrap();
        Self {
            root,
            source,
            patch,
        }
    }
    fn prepare(&self, compiler: &str) -> Result<PreparedSource, String> {
        prepare(&self.root, &self.source, compiler, &self.patch)
    }
    fn generated(&self) -> PathBuf {
        self.root.join("out/rust-src")
    }
    fn state(&self) -> PathBuf {
        self.root.join("out/rust-src-state.json")
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn identical_inputs_reuse_bytes_and_preserve_source_timestamps() {
    let f = Fixture::new();
    let first = f.prepare("compiler-1").unwrap();
    assert!(!first.reused);
    let file = first.library.join("test.rs");
    let modified = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1000);
    OpenOptions::new()
        .write(true)
        .open(&file)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified))
        .unwrap();
    let before = tree_hash(&f.generated()).unwrap();
    let second = f.prepare("compiler-1").unwrap();
    assert!(second.reused);
    assert_eq!(first.library, second.library);
    assert_eq!(tree_hash(&f.generated()).unwrap(), before);
    assert_eq!(fs::metadata(file).unwrap().modified().unwrap(), modified);
}

#[test]
fn changed_toolchain_rebuilds_even_with_identical_sources() {
    let f = Fixture::new();
    f.prepare("compiler-1").unwrap();
    assert!(!f.prepare("compiler-2").unwrap().reused);
    assert!(f.prepare("compiler-2").unwrap().reused);
}

#[test]
fn input_content_change_is_detected_even_if_size_and_timestamp_are_unchanged() {
    let f = Fixture::new();
    f.prepare("compiler").unwrap();
    let path = f.source.join("library/untouched.rs");
    let modified = fs::metadata(&path).unwrap().modified().unwrap();
    fs::write(&path, "two\n").unwrap();
    OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified))
        .unwrap();
    let prepared = f.prepare("compiler").unwrap();
    assert!(!prepared.reused);
    assert_eq!(
        fs::read_to_string(prepared.library.join("untouched.rs")).unwrap(),
        "two\n"
    );
}

#[test]
fn changed_patch_rebuilds_the_expected_new_output() {
    let f = Fixture::new();
    f.prepare("compiler").unwrap();
    let patch = fs::read_to_string(&f.patch)
        .unwrap()
        .replace("+after", "+other");
    fs::write(&f.patch, patch).unwrap();
    let prepared = f.prepare("compiler").unwrap();
    assert!(!prepared.reused);
    assert_eq!(
        fs::read_to_string(prepared.library.join("test.rs")).unwrap(),
        "other\n"
    );
}

#[test]
fn output_corruption_missing_files_and_extra_files_force_fresh_preparation() {
    let f = Fixture::new();
    f.prepare("compiler").unwrap();
    fs::write(f.generated().join("library/test.rs"), "wrong\n").unwrap();
    assert!(!f.prepare("compiler").unwrap().reused);
    assert_eq!(
        fs::read_to_string(f.generated().join("library/test.rs")).unwrap(),
        "after\n"
    );
    fs::remove_file(f.generated().join("library/test.rs")).unwrap();
    assert!(!f.prepare("compiler").unwrap().reused);
    fs::write(f.generated().join("unexpected"), "stale").unwrap();
    assert!(!f.prepare("compiler").unwrap().reused);
    assert!(!f.generated().join("unexpected").exists());
}

#[test]
fn interrupted_preparation_and_missing_or_malformed_stamps_are_rebuilt() {
    let f = Fixture::new();
    f.prepare("compiler").unwrap();
    let staging = f.root.join("out/rust-src-preparing");
    fs::create_dir_all(&staging).unwrap();
    fs::write(staging.join("partial"), "interrupted").unwrap();
    assert!(!f.prepare("compiler").unwrap().reused);
    assert!(!staging.exists());
    fs::remove_file(f.state()).unwrap();
    assert!(!f.prepare("compiler").unwrap().reused);
    fs::write(f.state(), "{\"inputs\":").unwrap();
    assert!(!f.prepare("compiler").unwrap().reused);
    let mut stamp: serde_json::Value =
        serde_json::from_slice(&fs::read(f.state()).unwrap()).unwrap();
    stamp["inputs"]["recipe"] = 0.into();
    fs::write(f.state(), serde_json::to_vec(&stamp).unwrap()).unwrap();
    assert!(!f.prepare("compiler").unwrap().reused);
}

#[test]
fn missing_or_failed_patch_never_accepts_old_cache_and_next_attempt_recovers() {
    let f = Fixture::new();
    f.prepare("compiler").unwrap();
    let original = fs::read(&f.patch).unwrap();
    fs::remove_file(&f.patch).unwrap();
    assert!(f.prepare("compiler").is_err());
    fs::write(&f.patch, b"invalid patch\n").unwrap();
    assert!(f.prepare("compiler").is_err());
    fs::write(&f.patch, original).unwrap();
    assert!(!f.prepare("compiler").unwrap().reused);
    assert!(f.prepare("compiler").unwrap().reused);
}

#[cfg(unix)]
#[test]
fn a_backslash_filename_cannot_impersonate_a_missing_nested_source() {
    let f = Fixture::new();
    f.prepare("compiler").unwrap();
    let impostor = f.generated().join("library\\untouched.rs");
    fs::rename(f.generated().join("library/untouched.rs"), &impostor).unwrap();
    assert!(!f.prepare("compiler").unwrap().reused);
    assert!(!impostor.exists());
    assert_eq!(
        fs::read_to_string(f.generated().join("library/untouched.rs")).unwrap(),
        "one\n"
    );
}

#[cfg(unix)]
#[test]
fn symlink_corruption_is_replaced_without_changing_its_target() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    f.prepare("compiler").unwrap();
    let outside = f.root.join("kept");
    fs::write(&outside, "neighbor").unwrap();
    let file = f.generated().join("library/test.rs");
    fs::remove_file(&file).unwrap();
    symlink(&outside, &file).unwrap();
    assert!(!f.prepare("compiler").unwrap().reused);
    assert_eq!(fs::read_to_string(&outside).unwrap(), "neighbor");
    assert!(!fs::symlink_metadata(file).unwrap().file_type().is_symlink());
}

#[test]
fn an_os_lock_is_released_with_its_handle_despite_the_leftover_file() {
    let f = Fixture::new();
    fs::create_dir_all(f.root.join("out")).unwrap();
    let path = f.root.join("out/rust-src.lock");
    let first = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    first.lock().unwrap();
    let second = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap();
    assert!(second.try_lock().is_err());
    drop(first);
    second.try_lock().unwrap();
    drop(second);
    assert!(!f.prepare("compiler").unwrap().reused);
}
