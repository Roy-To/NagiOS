use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};

use crate::paths::external_command_path;

const SECTION: &str = "sources.libc_servo";
const VERSION: &str = "0.2.189";
const ARCHIVE_HASH: &str = "3eaf3ede3fee6db1a4c2ee091bf8a8b4dccdc6d17f656fb07896ee72867612f2";
const GENERATED_MARKER: &str = ".nagi-libc-servo-checkout";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LibcServoSourceSpec {
    pub(crate) version: String,
    pub(crate) repository: String,
    pub(crate) registry_archive: String,
    pub(crate) source_hash: String,
    pub(crate) license: String,
    pub(crate) vendored_path: PathBuf,
    pub(crate) patch_path: PathBuf,
}

pub(crate) fn load_libc_servo_source_spec(root: &Path) -> Result<LibcServoSourceSpec, String> {
    let lock = fs::read_to_string(root.join("third_party").join("sources.lock"))
        .map_err(|error| format!("cannot read third_party/sources.lock: {error}"))?;
    let required = |key: &str| {
        lock_value(&lock, SECTION, key)
            .ok_or_else(|| format!("third_party/sources.lock is missing libc-servo field `{key}`"))
    };
    Ok(LibcServoSourceSpec {
        version: required("version")?,
        repository: required("repository")?,
        registry_archive: required("registry_archive")?,
        source_hash: required("source_hash")?,
        license: required("license")?,
        vendored_path: relative_path(&required("vendored_path")?, "vendored_path")?,
        patch_path: relative_path(&required("nagi_patch")?, "nagi_patch")?,
    })
}

pub(crate) fn validate_libc_servo_source_lock(root: &Path) -> Result<LibcServoSourceSpec, String> {
    let spec = load_libc_servo_source_spec(root)?;
    if spec.version != VERSION {
        return Err(format!(
            "third_party/sources.lock libc-servo version is `{}`, expected `{VERSION}`",
            spec.version
        ));
    }
    if spec.repository != "https://crates.io/crates/libc/0.2.189" {
        return Err(format!(
            "third_party/sources.lock libc-servo repository is `{}`, expected crates.io 0.2.189",
            spec.repository
        ));
    }
    if spec.registry_archive != "https://crates.io/api/v1/crates/libc/0.2.189/download" {
        return Err(format!(
            "third_party/sources.lock libc-servo archive is `{}`, expected pinned 0.2.189 archive",
            spec.registry_archive
        ));
    }
    if spec.source_hash != format!("sha256:{ARCHIVE_HASH}") {
        return Err(format!(
            "third_party/sources.lock libc-servo source_hash is `{}`, expected sha256:{ARCHIVE_HASH}",
            spec.source_hash
        ));
    }
    if spec.license != "MIT OR Apache-2.0" {
        return Err(format!(
            "third_party/sources.lock libc-servo license is `{}`, expected MIT OR Apache-2.0",
            spec.license
        ));
    }
    if spec.vendored_path != Path::new("third_party/libc-servo") {
        return Err(format!(
            "third_party/sources.lock libc-servo vendored_path is `{}`, expected third_party/libc-servo",
            spec.vendored_path.display()
        ));
    }
    if spec.patch_path != Path::new("third_party/libc-servo-patches") {
        return Err(format!(
            "third_party/sources.lock libc-servo nagi_patch is `{}`, expected third_party/libc-servo-patches",
            spec.patch_path.display()
        ));
    }
    Ok(spec)
}

// Use the same authenticated archive snapshot/extraction boundary as the
// registry adapters. The independent libc source lock and Servo selection
// checks remain authoritative; libc still requires pre-fetched dependencies.
fn archive_spec(spec: &LibcServoSourceSpec) -> crate::registry_source::RegistrySourceSpec<'_> {
    crate::registry_source::RegistrySourceSpec {
        section: SECTION,
        component: "libc-servo",
        package: "libc",
        version: &spec.version,
        repository: &spec.repository,
        registry_archive: &spec.registry_archive,
        source_hash: &spec.source_hash,
        license: &spec.license,
        vendored_path: "third_party/libc-servo",
        patch_path: "third_party/libc-servo-patches",
    }
}

/// Materialize the exact libc archive already fetched by Cargo, then apply the
/// tracked Nagi ABI patch set. This keeps Servo's newer libc requirement
/// separate from the M13 libc source while preserving a reproducible boundary.
pub(crate) fn ensure_libc_servo_checkout(root: &Path) -> Result<PathBuf, String> {
    let spec = validate_libc_servo_source_lock(root)?;
    materialize_libc_servo_checkout(root, &spec)
}

fn materialize_libc_servo_checkout(
    root: &Path,
    spec: &LibcServoSourceSpec,
) -> Result<PathBuf, String> {
    let checkout = root.join(&spec.vendored_path);
    if checkout.exists() {
        return validate_checkout(root, spec);
    }
    let source = find_registry_source(root, spec)?;
    let patches = patch_files(root, spec)?;
    let parent = checkout
        .parent()
        .ok_or_else(|| format!("libc-servo checkout has no parent: {}", checkout.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    let temporary = parent.join(format!(
        ".libc-servo-bootstrap-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| format!("cannot read clock for libc-servo bootstrap: {error}"))?
            .as_nanos()
    ));
    if temporary.exists() {
        return Err(format!(
            "refusing to reuse unexpected libc-servo bootstrap directory {}",
            temporary.display()
        ));
    }
    let result = (|| {
        crate::registry_source::extract_verified_archive(&source, &temporary, &archive_spec(spec))?;
        for patch in &patches {
            apply_patch(root, &temporary, patch)?;
        }
        write_marker(root, &temporary, spec)
    })();
    match result {
        Ok(()) => {
            if checkout.exists() {
                let _ = fs::remove_dir_all(&temporary);
                return Err(format!(
                    "cannot install generated libc-servo checkout at {}; destination was created concurrently and was preserved",
                    checkout.display()
                ));
            }
            fs::rename(&temporary, &checkout).map_err(|error| {
                let _ = fs::remove_dir_all(&temporary);
                format!(
                    "cannot install generated libc-servo checkout {} at {}: {error}",
                    temporary.display(),
                    checkout.display()
                )
            })?;
            validate_checkout(root, spec)
        }
        Err(error) => {
            let _ = fs::remove_dir_all(&temporary);
            Err(error)
        }
    }
}

fn validate_checkout(root: &Path, spec: &LibcServoSourceSpec) -> Result<PathBuf, String> {
    let checkout = root.join(&spec.vendored_path);
    if !checkout.is_dir() {
        return Err(format!(
            "libc-servo checkout is not a directory: {}",
            checkout.display()
        ));
    }
    let manifest = fs::read_to_string(checkout.join("Cargo.toml"))
        .map_err(|error| format!("cannot read libc-servo manifest: {error}"))?;
    if !manifest
        .lines()
        .any(|line| line.trim() == "version = \"0.2.189\"")
    {
        return Err(format!(
            "libc-servo checkout does not contain version {VERSION}"
        ));
    }
    let marker = checkout.join(GENERATED_MARKER);
    let contents = fs::read_to_string(&marker).map_err(|error| {
        format!(
            "generated libc-servo checkout marker is missing {}; refusing to modify {}: {error}",
            marker.display(),
            checkout.display()
        )
    })?;
    if marker_value(&contents, "format_version") != Some("2") {
        return Err(format!("generated libc-servo checkout has no authenticated-archive provenance marker; refusing to modify {}", checkout.display()));
    }
    let expected_patch = patch_fingerprint(root, spec)?;
    if marker_value(&contents, "source_hash") != Some(spec.source_hash.as_str())
        || marker_value(&contents, "patch_fingerprint") != Some(expected_patch.as_str())
    {
        return Err(format!(
            "generated libc-servo checkout marker does not match pinned source or patch fingerprint: {}",
            marker.display()
        ));
    }
    let actual = directory_fingerprint(&checkout)?;
    if marker_value(&contents, "checkout_fingerprint") != Some(actual.as_str()) {
        return Err(format!(
            "generated libc-servo checkout state does not match its marker; refusing to modify {}",
            checkout.display()
        ));
    }
    Ok(checkout)
}

fn find_registry_source(root: &Path, spec: &LibcServoSourceSpec) -> Result<Vec<u8>, String> {
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE").map(|value| PathBuf::from(value).join(".cargo"))
        })
        .or_else(|| std::env::var_os("HOME").map(|value| PathBuf::from(value).join(".cargo")))
        .ok_or_else(|| "cannot locate Cargo home for the pinned libc-servo archive".to_owned())?;
    let expected_hash = spec
        .source_hash
        .strip_prefix("sha256:")
        .ok_or_else(|| format!("invalid libc-servo source hash `{}`", spec.source_hash))?;
    let servo_lock = root.join("third_party").join("servo").join("Cargo.lock");
    if servo_lock.is_file() && !servo_lock_has_pinned_libc(&servo_lock, spec, expected_hash)? {
        return Err(format!(
            "{} does not select pinned libc-servo {} with checksum {}",
            servo_lock.display(),
            spec.version,
            expected_hash
        ));
    }
    crate::registry_source::locate_registry_archive(&cargo_home, &archive_spec(spec))?
        .ok_or_else(|| format!(
            "pinned libc-servo {} archive is absent from Cargo's registry cache; fetch the locked Servo dependencies before bootstrapping",
            spec.version
        ))
}

fn servo_lock_has_pinned_libc(
    lock_path: &Path,
    spec: &LibcServoSourceSpec,
    expected_hash: &str,
) -> Result<bool, String> {
    let lock = fs::read_to_string(lock_path)
        .map_err(|error| format!("cannot read pinned Servo Cargo.lock: {error}"))?;
    let mut in_libc = false;
    let mut version = None;
    let mut checksum = None;
    for line in lock.lines() {
        if line == "[[package]]" {
            in_libc = false;
            version = None;
            checksum = None;
        } else if in_libc && line == format!("version = \"{}\"", spec.version) {
            version = Some(true);
        } else if in_libc {
            if let Some(value) = line
                .strip_prefix("checksum = \"")
                .and_then(|v| v.strip_suffix('\"'))
            {
                checksum = Some(value == expected_hash);
            }
        } else if line == "name = \"libc\"" {
            in_libc = true;
        }
        if in_libc && version == Some(true) && checksum == Some(true) {
            return Ok(true);
        }
    }
    Ok(false)
}

fn patch_files(root: &Path, spec: &LibcServoSourceSpec) -> Result<Vec<PathBuf>, String> {
    let directory = root.join(&spec.patch_path);
    if !directory.is_dir() {
        return Err(format!(
            "libc-servo patch directory is missing: {}",
            directory.display()
        ));
    }
    let mut patches = Vec::new();
    for entry in fs::read_dir(&directory)
        .map_err(|error| format!("cannot read libc-servo patch directory: {error}"))?
    {
        let path = entry
            .map_err(|error| format!("cannot inspect libc-servo patch entry: {error}"))?
            .path();
        if path.extension() != Some(OsStr::new("patch")) {
            continue;
        }
        let name = path.file_name().and_then(OsStr::to_str).ok_or_else(|| {
            format!(
                "libc-servo patch has a non-UTF-8 filename: {}",
                path.display()
            )
        })?;
        if name
            .split_once('-')
            .and_then(|(number, _)| number.parse::<u32>().ok())
            .is_none()
        {
            return Err(format!(
                "libc-servo patch `{name}` must start with a numeric prefix"
            ));
        }
        let bytes = fs::read(&path)
            .map_err(|error| format!("cannot read libc-servo patch {}: {error}", path.display()))?;
        if bytes.contains(&b'\r') {
            return Err(format!(
                "libc-servo patch `{name}` must use LF line endings; refusing a CRLF-converted patch"
            ));
        }
        patches.push(path);
    }
    patches.sort();
    Ok(patches)
}

fn apply_patch(root: &Path, checkout: &Path, patch: &Path) -> Result<(), String> {
    let relative_checkout = checkout.strip_prefix(root).map_err(|error| {
        format!(
            "libc-servo checkout {} is outside repository root {}: {error}",
            checkout.display(),
            root.display()
        )
    })?;
    if relative_checkout.as_os_str().is_empty()
        || relative_checkout
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "libc-servo checkout must be a normal path under repository root: {}",
            checkout.display()
        ));
    }
    // Git's path arguments use slash-separated paths even on Windows. Keep
    // the generated checkout under the repository root, but avoid handing
    // Git-for-Windows a native backslash path that can be interpreted
    // differently by `git apply` across runner versions.
    let git_directory = relative_checkout.to_string_lossy().replace('\\', "/");
    let check = Command::new("git")
        .args([
            "-c",
            "core.autocrlf=false",
            "-c",
            "core.eol=lf",
            "apply",
            "--check",
            "--unidiff-zero",
            "--unsafe-paths",
        ])
        .arg(format!("--directory={git_directory}"))
        .arg(external_command_path(patch))
        .current_dir(root)
        .output()
        .map_err(|error| format!("cannot check libc-servo patch {}: {error}", patch.display()))?;
    if !check.status.success() {
        return Err(format!(
            "libc-servo patch check failed for {}: {}",
            patch.display(),
            command_output(&check)
        ));
    }
    let applied = Command::new("git")
        .args([
            "-c",
            "core.autocrlf=false",
            "-c",
            "core.eol=lf",
            "apply",
            "--unidiff-zero",
            "--unsafe-paths",
        ])
        .arg(format!("--directory={git_directory}"))
        .arg(external_command_path(patch))
        .current_dir(root)
        .output()
        .map_err(|error| format!("cannot apply libc-servo patch {}: {error}", patch.display()))?;
    if !applied.status.success() {
        return Err(format!(
            "libc-servo patch application failed for {}: {}",
            patch.display(),
            command_output(&applied)
        ));
    }
    Ok(())
}

fn write_marker(root: &Path, checkout: &Path, spec: &LibcServoSourceSpec) -> Result<(), String> {
    let patch_fingerprint = patch_fingerprint(root, spec)?;
    let checkout_fingerprint = directory_fingerprint(checkout)?;
    fs::write(
        checkout.join(GENERATED_MARKER),
        format!(
            "format_version = 2\nsource_hash = {}\npatch_fingerprint = {}\ncheckout_fingerprint = {}\n",
            spec.source_hash, patch_fingerprint, checkout_fingerprint
        ),
    )
    .map_err(|error| format!("cannot write generated libc-servo marker: {error}"))
}

fn patch_fingerprint(root: &Path, spec: &LibcServoSourceSpec) -> Result<String, String> {
    let mut fingerprint = Fnv1a::new();
    for patch in patch_files(root, spec)? {
        fingerprint.update(
            patch
                .file_name()
                .and_then(OsStr::to_str)
                .ok_or_else(|| format!("invalid libc-servo patch name: {}", patch.display()))?
                .as_bytes(),
        );
        fingerprint.update(&[0]);
        fingerprint.update(
            &fs::read(&patch)
                .map_err(|error| format!("cannot read {}: {error}", patch.display()))?,
        );
        fingerprint.update(&[0]);
    }
    Ok(fingerprint.finish())
}

fn directory_fingerprint(directory: &Path) -> Result<String, String> {
    let mut files = Vec::new();
    collect_files(directory, directory, &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut fingerprint = Fnv1a::new();
    for (relative, path) in files {
        fingerprint.update(relative.to_string_lossy().as_bytes());
        fingerprint.update(&[0]);
        fingerprint.update(
            &fs::read(path)
                .map_err(|error| format!("cannot read generated libc-servo file: {error}"))?,
        );
        fingerprint.update(&[0]);
    }
    Ok(fingerprint.finish())
}

fn collect_files(
    root: &Path,
    current: &Path,
    files: &mut Vec<(PathBuf, PathBuf)>,
) -> Result<(), String> {
    for entry in fs::read_dir(current)
        .map_err(|error| format!("cannot enumerate {}: {error}", current.display()))?
    {
        let entry =
            entry.map_err(|error| format!("cannot inspect generated libc-servo entry: {error}"))?;
        let path = entry.path();
        if path.file_name() == Some(OsStr::new(GENERATED_MARKER)) {
            continue;
        }
        if entry
            .file_type()
            .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?
            .is_dir()
        {
            collect_files(root, &path, files)?;
        } else {
            files.push((path.strip_prefix(root).unwrap_or(&path).to_path_buf(), path));
        }
    }
    Ok(())
}

fn lock_value(lock: &str, section: &str, key: &str) -> Option<String> {
    let mut in_section = false;
    for line in lock.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_section = trimmed == format!("[{section}]");
            continue;
        }
        if in_section {
            let (candidate, value) = trimmed.split_once('=')?;
            if candidate.trim() == key {
                return Some(value.trim().trim_matches('"').to_owned());
            }
        }
    }
    None
}

fn relative_path(value: &str, field: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!("libc-servo {field} must be a relative normal path"));
    }
    Ok(path)
}

fn marker_value<'a>(contents: &'a str, key: &str) -> Option<&'a str> {
    contents
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{key} = ")))
}

fn command_output(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    match (stdout.is_empty(), stderr.is_empty()) {
        (true, true) => format!("exit status {}", output.status),
        (false, true) => stdout,
        (true, false) => stderr,
        (false, false) => format!("{stdout}; {stderr}"),
    }
}

struct Fnv1a(u64);

impl Fnv1a {
    fn new() -> Self {
        Self(0xcbf29ce484222325)
    }

    fn update(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }

    fn finish(self) -> String {
        format!("fnv1a64:{:016x}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use super::{
        apply_patch, load_libc_servo_source_spec, validate_libc_servo_source_lock,
        LibcServoSourceSpec,
    };

    fn temp_root(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("nagi-libc-servo-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("third_party")).expect("temp root");
        root
    }

    #[test]
    fn libc_servo_patch_applies_inside_parent_workspace() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root");
        let root = workspace
            .join("out")
            .join(format!("nagi-libc-servo-apply-{}", std::process::id()));
        let checkout = root.join("checkout");
        let patch = root.join("01-test.patch");
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&checkout).expect("checkout");
        fs::write(checkout.join("Cargo.toml"), b"value = 1\n").expect("manifest");
        fs::write(
            &patch,
            b"diff --git a/Cargo.toml b/Cargo.toml\n--- a/Cargo.toml\n+++ b/Cargo.toml\n@@ -1 +1 @@\n-value = 1\n+value = 2\n",
        )
        .expect("patch");

        apply_patch(workspace, &checkout, &patch).expect("apply patch");
        assert_eq!(
            fs::read_to_string(checkout.join("Cargo.toml")).expect("patched manifest"),
            "value = 2\n"
        );
        let _ = fs::remove_dir_all(root);
    }

    fn expected_spec() -> LibcServoSourceSpec {
        LibcServoSourceSpec {
            version: "0.2.189".into(),
            repository: "https://crates.io/crates/libc/0.2.189".into(),
            registry_archive: "https://crates.io/api/v1/crates/libc/0.2.189/download".into(),
            source_hash: "sha256:3eaf3ede3fee6db1a4c2ee091bf8a8b4dccdc6d17f656fb07896ee72867612f2"
                .into(),
            license: "MIT OR Apache-2.0".into(),
            vendored_path: PathBuf::from("third_party/libc-servo"),
            patch_path: PathBuf::from("third_party/libc-servo-patches"),
        }
    }

    #[test]
    fn libc_servo_lock_metadata_is_pinned_without_a_checkout() {
        let root = temp_root("lock");
        fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(Path::parent)
                .expect("workspace root")
                .join("third_party/sources.lock"),
            root.join("third_party/sources.lock"),
        )
        .expect("copy lock");
        assert_eq!(validate_libc_servo_source_lock(&root), Ok(expected_spec()));
        assert_eq!(load_libc_servo_source_spec(&root), Ok(expected_spec()));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn libc_servo_lock_rejects_a_changed_hash() {
        let root = temp_root("wrong-hash");
        fs::write(
            root.join("third_party/sources.lock"),
            "[sources.libc_servo]\nversion = \"0.2.189\"\nrepository = \"https://crates.io/crates/libc/0.2.189\"\nregistry_archive = \"https://crates.io/api/v1/crates/libc/0.2.189/download\"\nsource_hash = \"wrong\"\nlicense = \"MIT OR Apache-2.0\"\nvendored_path = \"third_party/libc-servo\"\nnagi_patch = \"third_party/libc-servo-patches\"\n",
        )
        .expect("lock");
        let error = validate_libc_servo_source_lock(&root).unwrap_err();
        assert!(error.contains("source_hash"), "{error}");
        let _ = fs::remove_dir_all(root);
    }

    fn fixture_archive() -> Vec<u8> {
        let output = std::process::Command::new("python3")
            .args([
                "-c",
                r#"
import io, sys, tarfile
result = io.BytesIO()
with tarfile.open(fileobj=result, mode='w:gz') as archive:
    for name, content in [('Cargo.toml', b'[package]\nname = "libc"\nversion = "0.2.189"\n'),
                          ('src/lib.rs', b'upstream libc fixture\n')]:
        entry = tarfile.TarInfo('libc-0.2.189/' + name)
        entry.size = len(content)
        archive.addfile(entry, io.BytesIO(content))
sys.stdout.buffer.write(result.getvalue())
"#,
            ])
            .output()
            .unwrap();
        assert!(output.status.success(), "{output:?}");
        output.stdout
    }

    fn run_source_fixture(root: &Path, expected: &str) {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "libc_servo::tests::libc_archive_fixture_child",
                "--nocapture",
            ])
            .env("CARGO_HOME", root.join("cargo"))
            .env("CARGO_NET_OFFLINE", "true")
            .env("NAGI_LIBC_FIXTURE_ROOT", root)
            .env("NAGI_LIBC_EXPECT", expected)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[test]
    #[ignore = "isolated fake-CARGO_HOME child invoked by libc archive fixture"]
    fn libc_archive_fixture_child() {
        let root = PathBuf::from(std::env::var_os("NAGI_LIBC_FIXTURE_ROOT").unwrap());
        let mut spec = expected_spec();
        spec.source_hash = fs::read_to_string(root.join("expected-hash")).unwrap();
        let result = super::materialize_libc_servo_checkout(&root, &spec);
        let expected = std::env::var("NAGI_LIBC_EXPECT").unwrap();
        if expected == "success" {
            let checkout = result.unwrap();
            assert_eq!(
                fs::read(checkout.join("src/lib.rs")).unwrap(),
                b"patched libc fixture\n"
            );
            assert!(super::materialize_libc_servo_checkout(&root, &spec).is_ok());
            let marker = checkout.join(super::GENERATED_MARKER);
            let current = fs::read_to_string(&marker).unwrap();
            assert!(current.starts_with("format_version = 2\n"));
            let legacy = current.replace("format_version = 2", "format_version = 1");
            fs::write(&marker, &legacy).unwrap();
            let error = super::materialize_libc_servo_checkout(&root, &spec).unwrap_err();
            assert!(
                error.contains("no authenticated-archive provenance"),
                "{error}"
            );
            assert_eq!(fs::read_to_string(&marker).unwrap(), legacy);
            fs::write(marker, current).unwrap();
            fs::write(checkout.join("src/lib.rs"), "tampered generated libc").unwrap();
            let error = super::materialize_libc_servo_checkout(&root, &spec).unwrap_err();
            assert!(error.contains("state does not match"), "{error}");
            assert_eq!(
                fs::read_to_string(checkout.join("src/lib.rs")).unwrap(),
                "tampered generated libc"
            );
        } else {
            let error = result.unwrap_err();
            assert!(error.contains(&expected), "{error}");
            assert!(!root.join("third_party/libc-servo").exists());
        }
    }

    #[test]
    fn libc_archive_binding_ignores_spoofed_expanded_cache_and_preserves_lock_check() {
        use sha2::{Digest, Sha256};
        let bytes = fixture_archive();
        let hash = format!("{:x}", Sha256::digest(&bytes));
        for case in [
            "cargo-ok",
            "substring-checksum",
            "correct-checksum",
            "wrong-archive",
            "archive",
            "wrong-servo-lock",
        ] {
            let root = temp_root(case);
            fs::write(root.join("expected-hash"), format!("sha256:{hash}")).unwrap();
            let patches = root.join("third_party/libc-servo-patches");
            fs::create_dir_all(&patches).unwrap();
            fs::write(patches.join("0001-fixture.patch"), "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-upstream libc fixture\n+patched libc fixture\n").unwrap();
            let expanded = root.join("cargo/registry/src/fixture-index/libc-0.2.189");
            fs::create_dir_all(&expanded).unwrap();
            fs::write(
                expanded.join("Cargo.toml"),
                "[package]\nname = \"libc\"\nversion = \"0.2.189\"\n",
            )
            .unwrap();
            fs::write(expanded.join("src.rs"), "spoofed libc").unwrap();
            fs::write(expanded.join(".cargo-ok"), "").unwrap();
            if case != "cargo-ok" {
                let package = if case == "substring-checksum" {
                    format!("prefix{hash}suffix")
                } else {
                    hash.clone()
                };
                fs::write(
                    expanded.join(".cargo-checksum.json"),
                    format!(r#"{{"package":"{package}","files":{{}}}}"#),
                )
                .unwrap();
            }
            if matches!(case, "archive" | "wrong-archive" | "wrong-servo-lock") {
                let archive = root.join("cargo/registry/cache/fixture-index/libc-0.2.189.crate");
                fs::create_dir_all(archive.parent().unwrap()).unwrap();
                fs::write(
                    archive,
                    if case == "wrong-archive" {
                        b"wrong libc archive"
                    } else {
                        &bytes[..]
                    },
                )
                .unwrap();
            }
            if matches!(case, "archive" | "wrong-servo-lock") {
                fs::create_dir_all(root.join("third_party/servo")).unwrap();
                let checksum = if case == "archive" { &hash } else { "wrong" };
                fs::write(root.join("third_party/servo/Cargo.lock"), format!("[[package]]\nname = \"libc\"\nversion = \"0.2.189\"\nchecksum = \"{checksum}\"\n")).unwrap();
            }
            run_source_fixture(
                &root,
                match case {
                    "archive" => "success",
                    "wrong-archive" => "SHA-256 mismatch",
                    "wrong-servo-lock" => "does not select pinned libc-servo",
                    _ => "archive is absent",
                },
            );
            assert_eq!(
                fs::read_to_string(expanded.join("src.rs")).unwrap(),
                "spoofed libc"
            );
            assert!(fs::read_dir(root.join("third_party")).unwrap().all(|e| !e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("bootstrap")));
            fs::remove_dir_all(root).unwrap();
        }
    }
}
