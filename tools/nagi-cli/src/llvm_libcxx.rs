//! Pinned LLVM libc++ source for the Nagi target C++ runtime (ADR 0058).
//!
//! `third_party/sources.lock` pins the official LLVM release source tarball
//! by URL and SHA-256. Commands that link target C++ code call
//! [`ensure_libcxx_source`], which downloads the tarball into `out/cache/`
//! when it is missing or mismatched, verifies it, and extracts only the
//! directories the libc++ runtimes build reads into the lock's
//! `vendored_path`. Reuse checks a generated-tree digest under an OS lock;
//! inconsistent or legacy extractions are rejected intact.

use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

const SOURCES_LOCK: &str = "third_party/sources.lock";
const SECTION: &str = "[sources.llvm_libcxx]";
const URL_PREFIX: &str = "https://github.com/llvm/llvm-project/releases/download/llvmorg-";
const EXTRACTED_MARKER: &str = ".nagi-libcxx-source";

/// Top-level tarball directories the `runtimes` build of libc++ reads.
const EXTRACTED_SUBTREES: &[&str] = &[
    "cmake",
    "libc/hdr",
    "libcxx",
    "libcxxabi",
    "llvm/cmake",
    "llvm/utils/llvm-lit",
    "runtimes",
];

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LibcxxPin {
    pub(crate) version: String,
    pub(crate) url: String,
    pub(crate) sha256: String,
    pub(crate) vendored_path: String,
}

impl LibcxxPin {
    /// The directory name every tarball entry starts with.
    pub(crate) fn top_directory(&self) -> String {
        format!("llvm-project-{}.src", self.version)
    }

    fn tarball_name(&self) -> String {
        format!("{}.tar.xz", self.top_directory())
    }
}

fn field(section: &str, name: &str) -> Option<String> {
    section.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        if key.trim() != name {
            return None;
        }
        let value = value.trim();
        Some(
            value
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
                .unwrap_or(value)
                .to_owned(),
        )
    })
}

fn is_release_version(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

/// Parse and validate the `[sources.llvm_libcxx]` entry.
pub(crate) fn parse_libcxx_pin(lock: &str) -> Result<LibcxxPin, String> {
    let (_, rest) = lock
        .split_once(SECTION)
        .ok_or_else(|| format!("{SOURCES_LOCK} has no {SECTION} entry"))?;
    let body = rest.split("\n[").next().unwrap_or(rest);
    let require = |name: &str| {
        field(body, name).ok_or_else(|| format!("{SOURCES_LOCK} {SECTION} is missing {name}"))
    };
    let version = require("version")?;
    if !is_release_version(&version) {
        return Err(format!(
            "{SOURCES_LOCK} {SECTION} version `{version}` is not an exact release"
        ));
    }
    let url = require("repository")?;
    let expected_url = format!("{URL_PREFIX}{version}/llvm-project-{version}.src.tar.xz");
    if url != expected_url {
        return Err(format!(
            "{SOURCES_LOCK} {SECTION} repository must be the official release tarball {expected_url}"
        ));
    }
    let source_hash = require("source_hash")?;
    let sha256 = source_hash
        .strip_prefix("sha256:")
        .filter(|digest| {
            digest.len() == 64
                && digest
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
        .ok_or_else(|| {
            format!("{SOURCES_LOCK} {SECTION} source_hash must be sha256:<64 lowercase hex>")
        })?
        .to_owned();
    let vendored_path = require("vendored_path")?;
    if !vendored_path.starts_with("out/cache/")
        || vendored_path.contains("..")
        || vendored_path.ends_with('/')
    {
        return Err(format!(
            "{SOURCES_LOCK} {SECTION} vendored_path must be a directory under out/cache/"
        ));
    }
    Ok(LibcxxPin {
        version,
        url,
        sha256,
        vendored_path,
    })
}

fn file_sha256(path: &Path) -> Result<String, String> {
    let mut file =
        fs::File::open(path).map_err(|error| format!("cannot open {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn verify_tarball(path: &Path, pin: &LibcxxPin) -> Result<(), String> {
    let digest = file_sha256(path)?;
    if digest == pin.sha256 {
        Ok(())
    } else {
        Err(format!(
            "{} has SHA-256 {digest}, expected {}",
            path.display(),
            pin.sha256
        ))
    }
}

fn run(command: &mut ProcessCommand, what: &str) -> Result<(), String> {
    let output = command
        .output()
        .map_err(|error| format!("cannot start {what}: {error}"))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "{what} failed with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn marker_contents(pin: &LibcxxPin) -> String {
    format!(
        "version={}\nsha256={}\nsubtrees={}\n",
        pin.version,
        pin.sha256,
        EXTRACTED_SUBTREES.join(",")
    )
}

// Locks coordinate cooperating preparers only. Consumers must not mutate the
// returned source; this is not a lease preventing edits after this call returns.
const PREPARATION_LOCK: &str = ".llvm-libcxx-source.lock";
static NEXT_STAGING: AtomicU64 = AtomicU64::new(0);

fn real_directory(path: &Path) -> Result<(), String> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(format!(
            "libc++ cache requires a real directory: {}",
            path.display()
        ));
    }
    Ok(())
}

fn cache_directory(root: &Path) -> Result<PathBuf, String> {
    real_directory(root)?;
    let mut current = root.to_path_buf();
    for part in ["out", "cache"] {
        current.push(part);
        match fs::create_dir(&current) {
            Ok(()) => (),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => (),
            Err(error) => return Err(format!("cannot create {}: {error}", current.display())),
        }
        real_directory(&current)?;
    }
    Ok(current)
}

fn preparation_lock(cache: &Path) -> Result<File, String> {
    let path = cache.join(PREPARATION_LOCK);
    match fs::symlink_metadata(&path) {
        Ok(metadata) if !metadata.file_type().is_file() => {
            return Err(format!(
                "libc++ preparation lock is not a regular file: {}",
                path.display()
            ));
        }
        Ok(_) => (),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(format!("cannot inspect {}: {error}", path.display())),
    }
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&path)
        .map_err(|error| format!("cannot open {}: {error}", path.display()))?;
    lock.lock()
        .map_err(|error| format!("cannot lock {}: {error}", path.display()))?;
    Ok(lock)
}

struct Staging(PathBuf);
impl Staging {
    fn new(cache: &Path) -> Result<Self, String> {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| format!("cannot read libc++ preparation clock: {error}"))?
            .as_nanos();
        let path = cache.join(format!(
            ".libcxx-{}-{nonce}-{}.extracting",
            std::process::id(),
            NEXT_STAGING.fetch_add(1, Ordering::Relaxed)
        ));
        // Exclusive creation: never clear a leftover or another preparer's work.
        fs::create_dir(&path)
            .map_err(|error| format!("cannot create {}: {error}", path.display()))?;
        Ok(Self(path))
    }
}
impl Drop for Staging {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn tree_sha256(source: &Path) -> Result<String, String> {
    real_directory(source)?;
    for subtree in EXTRACTED_SUBTREES {
        real_directory(&source.join(subtree))?;
    }
    source_tree_content_sha256(source)
}

// Shared observation only: fingerprinting records available bytes and link text;
// preparation separately validates required subtrees, markers and archive pins.
pub(crate) fn source_tree_content_sha256(source: &Path) -> Result<String, String> {
    fn field(hasher: &mut Sha256, bytes: &[u8]) {
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    }
    fn visit(source: &Path, directory: &Path, hasher: &mut Sha256) -> Result<(), String> {
        let mut entries = fs::read_dir(directory)
            .map_err(|error| format!("cannot list {}: {error}", directory.display()))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("cannot enumerate {}: {error}", directory.display()))?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let path = entry.path();
            if path == source.join(EXTRACTED_MARKER) {
                continue;
            }
            let relative = path
                .strip_prefix(source)
                .map_err(|error| error.to_string())?;
            let name = relative
                .to_str()
                .ok_or_else(|| format!("non-UTF-8 libc++ path: {}", path.display()))?;
            let metadata = fs::symlink_metadata(&path)
                .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?;
            field(hasher, name.as_bytes());
            if metadata.is_dir() {
                field(hasher, b"directory");
                visit(source, &path, hasher)?;
            } else if metadata.is_file() {
                field(hasher, b"file");
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    field(
                        hasher,
                        &(metadata.permissions().mode() & 0o111).to_le_bytes(),
                    );
                }
                field(hasher, file_sha256(&path)?.as_bytes());
            } else if metadata.file_type().is_symlink() {
                // Hash link text without traversing it (including dangling links
                // present in a release subtree). The pinned archive is the origin.
                field(hasher, b"symlink");
                let target = fs::read_link(&path)
                    .map_err(|error| format!("cannot read link {}: {error}", path.display()))?;
                field(
                    hasher,
                    target
                        .to_str()
                        .ok_or_else(|| format!("non-UTF-8 libc++ link: {}", path.display()))?
                        .as_bytes(),
                );
            } else {
                return Err(format!(
                    "unsupported libc++ source entry: {}",
                    path.display()
                ));
            }
        }
        Ok(())
    }
    real_directory(source)?;
    let mut hasher = Sha256::new();
    hasher.update(b"nagi-libcxx-tree-v1\0");
    visit(source, source, &mut hasher)?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn verified_marker(pin: &LibcxxPin, digest: &str) -> String {
    format!("{}tree_sha256={digest}\n", marker_contents(pin))
}

/// Ensure the pinned libc++ source subset is extracted and verified.
/// Reuse rehashes the complete tree; legacy/inconsistent caches are rejected
/// intact and require an explicit rebuild. The consumer shell's marker-only
/// check does not independently provide this guarantee.
pub(crate) fn ensure_libcxx_source(root: &Path) -> Result<PathBuf, String> {
    let lock = fs::read_to_string(root.join(SOURCES_LOCK))
        .map_err(|error| format!("cannot read {SOURCES_LOCK}: {error}"))?;
    let pin = parse_libcxx_pin(&lock)?;
    let cache = cache_directory(root)?;
    let _lock = preparation_lock(&cache)?;
    let source = root.join(&pin.vendored_path);
    // Check each ancestor rather than traversing a redirected source parent.
    let mut parent = root.to_path_buf();
    let relative = Path::new(&pin.vendored_path);
    for part in relative
        .parent()
        .ok_or("missing libc++ source parent")?
        .components()
    {
        let std::path::Component::Normal(name) = part else {
            return Err("invalid libc++ source parent".into());
        };
        parent.push(name);
        real_directory(&parent)?;
    }
    match fs::symlink_metadata(&source) {
        Ok(_) => {
            real_directory(&source)?;
            let marker = source.join(EXTRACTED_MARKER);
            if !fs::symlink_metadata(&marker)
                .map_err(|error| format!("unverified libc++ cache {}: {error}", source.display()))?
                .file_type()
                .is_file()
            {
                return Err(format!(
                    "libc++ source marker is not a regular file: {}",
                    marker.display()
                ));
            }
            let saved = fs::read_to_string(&marker)
                .map_err(|error| format!("cannot read {}: {error}", marker.display()))?;
            let digest = tree_sha256(&source)?;
            if saved != verified_marker(&pin, &digest) {
                return Err(format!(
                    "libc++ cache tree or marker mismatch at {}; preserved for explicit rebuild",
                    source.display()
                ));
            }
            return Ok(source);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => (),
        Err(error) => return Err(format!("cannot inspect {}: {error}", source.display())),
    }
    let staging = Staging::new(&cache)?;
    let tarball = cache.join(pin.tarball_name());
    if verify_tarball(&tarball, &pin).is_err() {
        let partial = staging.0.join("source.tar.xz.partial");
        run(
            ProcessCommand::new("curl")
                .args(["--fail", "--silent", "--show-error", "--location"])
                .args(["--retry", "3", "--proto", "=https", "--output"])
                .arg(&partial)
                .arg(&pin.url),
            &format!("curl {}", pin.url),
        )?;
        verify_tarball(&partial, &pin)
            .map_err(|error| format!("downloaded LLVM source rejected: {error}"))?;
        fs::rename(&partial, &tarball)
            .map_err(|error| format!("cannot install {}: {error}", tarball.display()))?;
    }
    let top = pin.top_directory();
    let mut extract = ProcessCommand::new("tar");
    extract.arg("-xJf").arg(&tarball).arg("-C").arg(&staging.0);
    for subtree in EXTRACTED_SUBTREES {
        extract.arg(format!("{top}/{subtree}"));
    }
    run(&mut extract, "tar extraction of the pinned LLVM source")?;
    let extracted = staging.0.join(&top);
    let digest = tree_sha256(&extracted)?;
    fs::write(
        extracted.join(EXTRACTED_MARKER),
        verified_marker(&pin, &digest),
    )
    .map_err(|error| format!("cannot write libc++ source marker: {error}"))?;
    if fs::symlink_metadata(&source).is_ok() {
        return Err(format!(
            "libc++ source destination appeared during preparation: {}",
            source.display()
        ));
    }
    fs::rename(&extracted, &source)
        .map_err(|error| format!("cannot install {}: {error}", source.display()))?;
    Ok(source)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENTRY: &str = "[sources.llvm_libcxx]\n\
component = \"llvm-libcxx\"\n\
repository = \"https://github.com/llvm/llvm-project/releases/download/llvmorg-19.1.7/llvm-project-19.1.7.src.tar.xz\"\n\
version = \"19.1.7\"\n\
source_hash = \"sha256:82401fea7b79d0078043f7598b835284d6650a75b93e64b6f761ea7b63097501\"\n\
license = \"Apache-2.0 WITH LLVM-exception\"\n\
vendored_path = \"out/cache/llvm-libcxx-19.1.7\"\n";

    #[test]
    fn repository_lock_pins_the_libcxx_source() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let lock = fs::read_to_string(root.join(SOURCES_LOCK)).expect("sources.lock");
        let pin = parse_libcxx_pin(&lock).expect("valid libc++ pin");
        assert_eq!(pin.version, "19.1.7");
        assert_eq!(pin.top_directory(), "llvm-project-19.1.7.src");
        assert_eq!(pin.vendored_path, "out/cache/llvm-libcxx-19.1.7");
    }

    #[test]
    fn parses_an_exact_entry_followed_by_other_sections() {
        let lock = format!("format_version = 1\n\n{ENTRY}\n[sources.other]\nversion = \"1\"\n");
        let pin = parse_libcxx_pin(&lock).expect("pin");
        assert_eq!(
            pin.sha256,
            "82401fea7b79d0078043f7598b835284d6650a75b93e64b6f761ea7b63097501"
        );
    }

    #[test]
    fn rejects_unpinned_or_unsafe_entries() {
        for (from, to) in [
            ("version = \"19.1.7\"", "version = \"19.1\""),
            ("llvmorg-19.1.7/", "llvmorg-main/"),
            ("https://github.com/llvm", "http://example.com/llvm"),
            ("sha256:82401fea", "sha256:82401FEA"),
            ("sha256:82401fea", "md5:82401fea"),
            ("out/cache/llvm-libcxx-19.1.7", "out/cache/../escape"),
            ("out/cache/llvm-libcxx-19.1.7", "third_party/llvm"),
        ] {
            let entry = ENTRY.replacen(from, to, 1);
            assert_ne!(entry, ENTRY, "replacement {from} applies");
            assert!(parse_libcxx_pin(&entry).is_err(), "{to} must be rejected");
        }
        assert!(parse_libcxx_pin("format_version = 1\n").is_err());
    }

    #[test]
    fn verifies_the_tarball_digest() {
        let directory = std::env::temp_dir().join(format!(
            "nagi-libcxx-digest-{}-{}",
            std::process::id(),
            line!()
        ));
        fs::create_dir_all(&directory).expect("temp dir");
        let path = directory.join("source.tar.xz");
        fs::write(&path, b"abc").expect("write");
        let mut pin = parse_libcxx_pin(ENTRY).expect("pin");
        assert!(verify_tarball(&path, &pin).is_err());
        pin.sha256 = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into();
        assert!(verify_tarball(&path, &pin).is_ok());
        let _ = fs::remove_dir_all(&directory);
    }
}

#[cfg(test)]
#[path = "../../../tests/libcxx-source-cache-host.rs"]
mod cache_host_tests;
