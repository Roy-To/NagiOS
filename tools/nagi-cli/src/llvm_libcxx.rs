//! Pinned LLVM libc++ source for the Nagi target C++ runtime (ADR 0058).
//!
//! `third_party/sources.lock` pins the official LLVM release source tarball
//! by URL and SHA-256. Commands that link target C++ code call
//! [`ensure_libcxx_source`], which downloads the tarball into `out/cache/`
//! when it is missing or mismatched, verifies it, and extracts only the
//! directories the libc++ runtimes build reads into the lock's
//! `vendored_path`. A marker records the verified digest so a stale or
//! partial extraction is replaced instead of reused.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

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

/// Ensure the pinned libc++ source subset is extracted and verified.
/// Returns the extracted source root (the `llvm-project-*.src` equivalent).
pub(crate) fn ensure_libcxx_source(root: &Path) -> Result<PathBuf, String> {
    let lock = fs::read_to_string(root.join(SOURCES_LOCK))
        .map_err(|error| format!("cannot read {SOURCES_LOCK}: {error}"))?;
    let pin = parse_libcxx_pin(&lock)?;
    let source = root.join(&pin.vendored_path);
    let marker = source.join(EXTRACTED_MARKER);
    if fs::read_to_string(&marker).ok().as_deref() == Some(marker_contents(&pin).as_str()) {
        return Ok(source);
    }

    let cache = root.join("out").join("cache");
    fs::create_dir_all(&cache)
        .map_err(|error| format!("cannot create {}: {error}", cache.display()))?;
    let tarball = cache.join(pin.tarball_name());
    if verify_tarball(&tarball, &pin).is_err() {
        let partial = cache.join(format!("{}.partial", pin.tarball_name()));
        let _ = fs::remove_file(&partial);
        run(
            ProcessCommand::new("curl")
                .args(["--fail", "--silent", "--show-error", "--location"])
                .args(["--retry", "3", "--proto", "=https", "--output"])
                .arg(&partial)
                .arg(&pin.url),
            &format!("curl {}", pin.url),
        )?;
        if let Err(error) = verify_tarball(&partial, &pin) {
            let _ = fs::remove_file(&partial);
            return Err(format!("downloaded LLVM source rejected: {error}"));
        }
        fs::rename(&partial, &tarball)
            .map_err(|error| format!("cannot install {}: {error}", tarball.display()))?;
    }

    // Extract into a fresh staging directory, then replace the vendored path,
    // so an interrupted extraction never looks complete.
    let staging = cache.join(format!("{}.extracting", pin.top_directory()));
    if staging.exists() {
        fs::remove_dir_all(&staging)
            .map_err(|error| format!("cannot clear {}: {error}", staging.display()))?;
    }
    fs::create_dir_all(&staging)
        .map_err(|error| format!("cannot create {}: {error}", staging.display()))?;
    let top = pin.top_directory();
    let mut extract = ProcessCommand::new("tar");
    extract.arg("-xJf").arg(&tarball).arg("-C").arg(&staging);
    for subtree in EXTRACTED_SUBTREES {
        extract.arg(format!("{top}/{subtree}"));
    }
    run(&mut extract, "tar extraction of the pinned LLVM source")?;
    let extracted = staging.join(&top);
    for subtree in EXTRACTED_SUBTREES {
        if !extracted.join(subtree).is_dir() {
            return Err(format!(
                "pinned LLVM source is missing {top}/{subtree} after extraction"
            ));
        }
    }
    fs::write(extracted.join(EXTRACTED_MARKER), marker_contents(&pin))
        .map_err(|error| format!("cannot write libc++ source marker: {error}"))?;
    if source.exists() {
        fs::remove_dir_all(&source)
            .map_err(|error| format!("cannot replace {}: {error}", source.display()))?;
    }
    fs::rename(&extracted, &source)
        .map_err(|error| format!("cannot install {}: {error}", source.display()))?;
    let _ = fs::remove_dir_all(&staging);
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
