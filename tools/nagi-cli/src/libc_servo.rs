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

/// Materialize the exact libc archive already fetched by Cargo, then apply the
/// tracked Nagi ABI patch set. This keeps Servo's newer libc requirement
/// separate from the M13 libc source while preserving a reproducible boundary.
pub(crate) fn ensure_libc_servo_checkout(root: &Path) -> Result<PathBuf, String> {
    let spec = validate_libc_servo_source_lock(root)?;
    let checkout = root.join(&spec.vendored_path);
    if checkout.exists() {
        return validate_checkout(root, &spec);
    }
    let source = find_registry_source(root, &spec)?;
    let patches = patch_files(root, &spec)?;
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
    copy_directory(&source, &temporary)?;
    let result = (|| {
        for patch in &patches {
            apply_patch(root, &temporary, patch)?;
        }
        write_marker(root, &temporary, &spec)
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
            validate_checkout(root, &spec)
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

fn find_registry_source(root: &Path, spec: &LibcServoSourceSpec) -> Result<PathBuf, String> {
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE").map(|value| PathBuf::from(value).join(".cargo"))
        })
        .or_else(|| std::env::var_os("HOME").map(|value| PathBuf::from(value).join(".cargo")))
        .ok_or_else(|| "cannot locate Cargo home for the pinned libc-servo archive".to_owned())?;
    let source_root = cargo_home.join("registry").join("src");
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
    for index in fs::read_dir(&source_root).map_err(|error| {
        format!(
            "cannot read Cargo registry source cache {}: {error}",
            source_root.display()
        )
    })? {
        let index = index
            .map_err(|error| format!("cannot inspect Cargo registry source cache: {error}"))?
            .path();
        let candidate = index.join(format!("libc-{}", spec.version));
        if !candidate.is_dir() {
            continue;
        }
        let checksum = candidate.join(".cargo-checksum.json");
        match fs::read_to_string(&checksum) {
            Ok(checksum) if checksum.contains(expected_hash) => return Ok(candidate),
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // Some CI images pre-populate Cargo's registry source cache
                // from the exact registry package but omit Cargo's checksum
                // sidecar. The locked Servo package record plus Cargo's
                // `.cargo-ok` marker still identify the source Cargo selected;
                // require both before accepting this cache representation.
                if registry_source_manifest_matches(&candidate, spec)
                    && candidate.join(".cargo-ok").is_file()
                {
                    return Ok(candidate);
                }
            }
            Err(error) => {
                return Err(format!(
                    "cannot read pinned libc-servo checksum {}: {error}",
                    checksum.display()
                ));
            }
        }
    }
    Err(format!(
        "pinned libc-servo {VERSION} source is absent from Cargo's registry cache; fetch the locked Servo dependencies before bootstrapping"
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

fn registry_source_manifest_matches(candidate: &Path, spec: &LibcServoSourceSpec) -> bool {
    let Ok(manifest) = fs::read_to_string(candidate.join("Cargo.toml")) else {
        return false;
    };
    manifest
        .lines()
        .any(|line| line.trim() == "name = \"libc\"")
        && manifest
            .lines()
            .any(|line| line.trim() == format!("version = \"{}\"", spec.version))
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
            "format_version = 1\nsource_hash = {}\npatch_fingerprint = {}\ncheckout_fingerprint = {}\n",
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

fn copy_directory(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination)
        .map_err(|error| format!("cannot create {}: {error}", destination.display()))?;
    for entry in fs::read_dir(source)
        .map_err(|error| format!("cannot enumerate {}: {error}", source.display()))?
    {
        let entry =
            entry.map_err(|error| format!("cannot inspect {}: {error}", source.display()))?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry
            .file_type()
            .map_err(|error| format!("cannot inspect {}: {error}", source_path.display()))?
            .is_dir()
        {
            copy_directory(&source_path, &destination_path)?;
        } else {
            fs::copy(&source_path, &destination_path).map_err(|error| {
                format!(
                    "cannot copy {} to {}: {error}",
                    source_path.display(),
                    destination_path.display()
                )
            })?;
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
}
