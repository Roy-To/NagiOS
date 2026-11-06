//! Ordinary host-only cache fixtures. Never use a product cache or network.
use super::*;
use std::process::Child;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

const FIXTURE_ENTRY: &str = include_str!("../third_party/sources.lock");
static NEXT: AtomicU64 = AtomicU64::new(0);
struct Fixture {
    root: PathBuf,
    tar: PathBuf,
    worker: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "libcxx-cache-host-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&root).unwrap();
        let pin = parse_libcxx_pin(FIXTURE_ENTRY).unwrap();
        let top = root.join("input").join(pin.top_directory());
        for subtree in EXTRACTED_SUBTREES {
            fs::create_dir_all(top.join(subtree)).unwrap();
        }
        fs::write(top.join("libcxx/header"), b"original source\n").unwrap();
        fs::create_dir_all(root.join("out/cache")).unwrap();
        let archive = root.join("out/cache").join(pin.tarball_name());
        let tar = host_tar();
        create_fixture_archive(&root, &pin, &archive);
        let digest = file_sha256(&archive).unwrap();
        fs::create_dir(root.join("third_party")).unwrap();
        fs::write(
            root.join(SOURCES_LOCK),
            FIXTURE_ENTRY.replace(&pin.sha256, &digest),
        )
        .unwrap();
        fs::create_dir(root.join("bin")).unwrap();
        // Compile a native executable: Windows cannot execute a /bin/sh script.
        // Only isolated workers receive this bin directory in their PATH.
        let shim_source = root.join("cache-tool.rs");
        fs::write(&shim_source, include_str!("fixtures/libcxx-cache-tool.rs")).unwrap();
        let shim = root
            .join("bin")
            .join(format!("tar{}", std::env::consts::EXE_SUFFIX));
        let output =
            ProcessCommand::new(std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into()))
                .arg("--edition=2021")
                .arg(&shim_source)
                .arg("-o")
                .arg(&shim)
                .output()
                .expect("compile native fixture tool with installed rustc");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        fs::copy(
            &shim,
            root.join("bin")
                .join(format!("curl{}", std::env::consts::EXE_SUFFIX)),
        )
        .unwrap();
        // Windows searches the application's directory before System32 and the
        // inherited PATH. Run a private worker copy beside the native shims so
        // unmodified production Command::new("tar"/"curl") cannot bypass them.
        let worker = root
            .join("bin")
            .join(format!("cache-worker{}", std::env::consts::EXE_SUFFIX));
        fs::copy(std::env::current_exe().unwrap(), &worker).unwrap();
        Self { root, tar, worker }
    }
    fn source(&self) -> PathBuf {
        self.root.join("out/cache/llvm-libcxx-19.1.7")
    }
    fn spawn(&self, name: &str) -> Child {
        let path = std::env::join_paths(std::iter::once(self.root.join("bin")).chain(
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
        ))
        .unwrap();
        let out = fs::File::create(self.root.join(format!("{name}.log"))).unwrap();
        ProcessCommand::new(&self.worker)
            .args([
                "--exact",
                &format!("{}::worker", module_path!().split_once("::").unwrap().1),
                "--ignored",
                "--nocapture",
            ])
            .env("FIXTURE_ROOT", &self.root)
            .env("FIXTURE_REAL_TAR", &self.tar)
            .env("FIXTURE_WORKER", name)
            .env("PATH", path)
            .stdout(out.try_clone().unwrap())
            .stderr(out)
            .spawn()
            .unwrap()
    }
    fn prepare(&self) -> bool {
        let mut child = self.spawn("prepare");
        child.wait().unwrap().success()
    }
    fn log(&self, name: &str) -> String {
        fs::read_to_string(self.root.join(format!("{name}.log"))).unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        // The interrupted worker's shim writes its marker just before exiting.
        // Windows may still have its executable mapped for that brief interval.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            match fs::remove_dir_all(&self.root) {
                Ok(()) => break,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
                Err(error)
                    if error.kind() == std::io::ErrorKind::PermissionDenied
                        && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(error) => {
                    if std::thread::panicking() {
                        eprintln!("fixture cleanup failed: {error}");
                    } else {
                        panic!("fixture cleanup failed: {error}");
                    }
                    break;
                }
            }
        }
    }
}
// Windows' inbox tar can read archives without supporting xz creation.
// Generate only these tiny private inputs using Python's stdlib lzma support.
// Extraction still executes the unmodified production tar -xJf path.
fn create_fixture_archive(root: &Path, pin: &LibcxxPin, archive: &Path) {
    let output = ProcessCommand::new("python3")
        .args([
            "-c",
            r#"
import pathlib, sys, tarfile
source, top, destination = sys.argv[1:]
with tarfile.open(destination, "w:xz", dereference=False) as archive:
    archive.add(pathlib.Path(source) / top, arcname=top)
"#,
        ])
        .arg(root.join("input"))
        .arg(pin.top_directory())
        .arg(archive)
        .output()
        .expect("Python 3 with stdlib lzma is required for host cache fixtures");
    assert!(
        output.status.success(),
        "fixture archive creation failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let bytes = fs::read(archive).unwrap();
    assert!(
        bytes.starts_with(b"\xfd7zXZ\0"),
        "fixture must remain a real xz archive"
    );
}

// Resolve before adding the fixture shims; never delegate to the shim itself.
fn host_tar() -> PathBuf {
    let name = format!("tar{}", std::env::consts::EXE_SUFFIX);
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|directory| directory.join(&name))
        .find(|path| path.is_file())
        .map(|path| fs::canonicalize(path).expect("resolve host tar"))
        .expect("host tar executable in PATH (tar.exe on Windows)")
}

fn until_file(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !path.exists() {
        assert!(Instant::now() < deadline, "missing {}", path.display());
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
#[ignore = "subprocess worker; called only with an isolated fixture root"]
fn worker() {
    let root = PathBuf::from(std::env::var_os("FIXTURE_ROOT").unwrap());
    let name = std::env::var("FIXTURE_WORKER").unwrap();
    fs::write(root.join(format!("started-{name}")), b"started").unwrap();
    if name == "hold-lock" {
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(root.join("out/cache/.llvm-libcxx-source.lock"))
            .unwrap();
        lock.lock().unwrap();
        fs::write(root.join("lock-held"), b"held").unwrap();
        loop {
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    let result = ensure_libcxx_source(&root);
    println!("result={result:?}");
    assert!(result.is_ok(), "cache rejected: {result:?}");
}

#[test]
fn valid_cache_reuses_without_extraction() {
    let f = Fixture::new();
    assert!(f.prepare(), "{}", f.log("prepare"));
    fs::write(f.root.join("fail-tar"), b"fail").unwrap();
    assert!(f.prepare(), "{}", f.log("prepare"));
}
#[test]
fn missing_archive_uses_fixture_curl_without_network() {
    let f = Fixture::new();
    let pin = parse_libcxx_pin(&fs::read_to_string(f.root.join(SOURCES_LOCK)).unwrap()).unwrap();
    fs::remove_file(f.root.join("out/cache").join(pin.tarball_name())).unwrap();
    assert!(!f.prepare());
    assert!(f.log("prepare").contains("fixture-curl-blocked"));
    assert!(!f.source().exists());
}
#[test]
fn correct_marker_does_not_authorize_one_byte_mutation() {
    let f = Fixture::new();
    assert!(f.prepare());
    fs::write(f.source().join("libcxx/header"), b"Original source\n").unwrap();
    assert!(!f.prepare(), "baseline incorrectly reused mutated tree");
    assert_eq!(
        fs::read(f.source().join("libcxx/header")).unwrap(),
        b"Original source\n"
    );
}
#[test]
fn correct_marker_does_not_authorize_missing_file() {
    let f = Fixture::new();
    assert!(f.prepare());
    fs::remove_file(f.source().join("libcxx/header")).unwrap();
    assert!(!f.prepare(), "baseline reused missing file");
    assert!(!f.source().join("libcxx/header").exists());
}
#[test]
fn correct_marker_does_not_authorize_missing_subtree() {
    let f = Fixture::new();
    assert!(f.prepare());
    fs::remove_dir(f.source().join("runtimes")).unwrap();
    assert!(!f.prepare(), "baseline reused missing subtree");
    assert!(!f.source().join("runtimes").exists());
}
#[test]
fn simultaneous_preparations_are_serialized() {
    let f = Fixture::new();
    fs::write(f.root.join("block-tar"), b"block").unwrap();
    let mut first = f.spawn("first");
    until_file(&f.root.join("tar-entered"));
    let mut second = f.spawn("second");
    until_file(&f.root.join("started-second"));
    let deadline = Instant::now() + Duration::from_millis(150);
    while Instant::now() < deadline && second.try_wait().unwrap().is_none() {
        std::thread::sleep(Duration::from_millis(10));
    }
    let second_finished_early = second.try_wait().unwrap().is_some();
    fs::write(f.root.join("release-tar"), b"release").unwrap();
    let a = first.wait().unwrap();
    let b = second.wait().unwrap();
    assert!(
        !second_finished_early && a.success() && b.success(),
        "first={a} second={b} early={second_finished_early}\n{}\n{}",
        f.log("first"),
        f.log("second")
    );
}
#[test]
fn stale_staging_is_preserved_and_not_reused() {
    let f = Fixture::new();
    let stale = f.root.join("out/cache/llvm-project-19.1.7.src.extracting");
    fs::create_dir(&stale).unwrap();
    fs::write(stale.join("interrupted"), b"keep").unwrap();
    assert!(f.prepare(), "{}", f.log("prepare"));
    assert_eq!(fs::read(stale.join("interrupted")).unwrap(), b"keep");
}
#[test]
fn failed_extraction_publishes_nothing_and_next_attempt_is_safe() {
    let f = Fixture::new();
    fs::write(f.root.join("fail-tar"), b"fail").unwrap();
    assert!(!f.prepare());
    assert!(f.log("prepare").contains("fixture-tar-failed"));
    assert!(!f.source().exists());
    fs::remove_file(f.root.join("fail-tar")).unwrap();
    assert!(f.prepare());
}
#[test]
fn process_death_releases_os_lock() {
    let f = Fixture::new();
    let mut holder = f.spawn("hold-lock");
    until_file(&f.root.join("lock-held"));
    let mut waiter = f.spawn("waiter");
    until_file(&f.root.join("started-waiter"));
    let deadline = Instant::now() + Duration::from_millis(150);
    while Instant::now() < deadline && waiter.try_wait().unwrap().is_none() {
        std::thread::sleep(Duration::from_millis(10));
    }
    let early = waiter.try_wait().unwrap().is_some();
    holder.kill().unwrap();
    holder.wait().unwrap();
    assert!(waiter.wait().unwrap().success(), "{}", f.log("waiter"));
    assert!(!early);
}
#[test]
fn legacy_marker_without_tree_digest_is_rejected_without_replacement() {
    let f = Fixture::new();
    assert!(f.prepare());
    let pin = parse_libcxx_pin(&fs::read_to_string(f.root.join(SOURCES_LOCK)).unwrap()).unwrap();
    fs::write(f.source().join(EXTRACTED_MARKER), marker_contents(&pin)).unwrap();
    assert!(!f.prepare());
    assert!(f.source().join("libcxx/header").exists());
}
#[test]
fn external_symlink_instead_of_source_root_is_rejected() {
    #[cfg(unix)]
    {
        let f = Fixture::new();
        assert!(f.prepare());
        let outside = f.root.join("saved-source");
        fs::rename(f.source(), &outside).unwrap();
        std::os::unix::fs::symlink(&outside, f.source()).unwrap();
        assert!(!f.prepare());
        assert!(outside.join("libcxx/header").exists());
    }
}

#[test]
fn link_text_and_executable_mode_changes_are_rejected() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let f = Fixture::new();
        // An ordinary release link may dangle outside the selected subtree.
        symlink(
            "missing",
            f.root.join("input/llvm-project-19.1.7.src/libcxx/link"),
        )
        .unwrap();
        let pin =
            parse_libcxx_pin(&fs::read_to_string(f.root.join(SOURCES_LOCK)).unwrap()).unwrap();
        let archive = f.root.join("out/cache").join(pin.tarball_name());
        create_fixture_archive(&f.root, &pin, &archive);
        let lock = fs::read_to_string(f.root.join(SOURCES_LOCK))
            .unwrap()
            .replace(&pin.sha256, &file_sha256(&archive).unwrap());
        fs::write(f.root.join(SOURCES_LOCK), lock).unwrap();
        assert!(f.prepare());
        assert!(f.prepare());
        fs::remove_file(f.source().join("libcxx/link")).unwrap();
        symlink("different", f.source().join("libcxx/link")).unwrap();
        assert!(!f.prepare());
        let g = Fixture::new();
        assert!(g.prepare());
        fs::set_permissions(
            g.source().join("libcxx/header"),
            fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        assert!(!g.prepare());
    }
}

#[test]
fn interrupted_extractor_leaves_only_its_unique_staging() {
    let f = Fixture::new();
    fs::write(f.root.join("block-tar"), b"block").unwrap();
    let mut interrupted = f.spawn("interrupted");
    until_file(&f.root.join("tar-entered"));
    let staging = fs::read_dir(f.root.join("out/cache"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| {
            p.file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with(".extracting")
        })
        .unwrap();
    interrupted.kill().unwrap();
    interrupted.wait().unwrap();
    let mut recovery = f.spawn("after-interruption");
    let recovered = recovery.wait().unwrap();
    let stale_preserved = staging.exists();
    fs::write(f.root.join("release-tar"), b"release").unwrap();
    // The orphaned fixture tar child exits before Fixture removes its own root.
    until_file(&f.root.join("tar-finished-interrupted"));
    assert!(recovered.success(), "{}", f.log("after-interruption"));
    assert!(stale_preserved, "a later preparer removed interrupted work");
    assert!(!staging
        .join("llvm-project-19.1.7.src")
        .join(EXTRACTED_MARKER)
        .exists());
    assert!(f.prepare());
}
