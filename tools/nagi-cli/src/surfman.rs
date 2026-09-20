use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};

use crate::paths::external_command_path;

const SURFMAN_SECTION: &str = "sources.surfman";
const SURFMAN_REVISION: &str = "205778f497327c573929c7b471194390e15f331d";
const SURFMAN_REPOSITORY: &str = "https://github.com/servo/surfman.git";
const SURFMAN_LICENSE: &str = "MIT OR Apache-2.0 OR MPL-2.0";
const GENERATED_MARKER: &str = ".nagi-surfman-checkout";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SurfmanSourceSpec {
    pub(crate) repository: String,
    pub(crate) revision: String,
    pub(crate) source_hash: String,
    pub(crate) license: String,
    pub(crate) vendored_path: PathBuf,
    pub(crate) patch_path: PathBuf,
}

pub(crate) fn load_surfman_source_spec(root: &Path) -> Result<SurfmanSourceSpec, String> {
    let lock = fs::read_to_string(root.join("third_party").join("sources.lock"))
        .map_err(|error| format!("cannot read third_party/sources.lock: {error}"))?;
    let required = |key: &str| {
        lock_value(&lock, SURFMAN_SECTION, key)
            .ok_or_else(|| format!("third_party/sources.lock is missing Surfman field `{key}`"))
    };
    Ok(SurfmanSourceSpec {
        repository: required("repository")?,
        revision: required("revision")?,
        source_hash: required("source_hash")?,
        license: required("license")?,
        vendored_path: relative_path(&required("vendored_path")?, "vendored_path")?,
        patch_path: relative_path(&required("nagi_patch")?, "nagi_patch")?,
    })
}

pub(crate) fn validate_surfman_source_lock(root: &Path) -> Result<SurfmanSourceSpec, String> {
    let spec = load_surfman_source_spec(root)?;
    if spec.repository != SURFMAN_REPOSITORY {
        return Err(format!(
            "third_party/sources.lock Surfman repository is `{}`, expected `{SURFMAN_REPOSITORY}`",
            spec.repository
        ));
    }
    if spec.revision != SURFMAN_REVISION {
        return Err(format!(
            "third_party/sources.lock Surfman revision is `{}`, expected `{SURFMAN_REVISION}`",
            spec.revision
        ));
    }
    if spec.source_hash != format!("git:{SURFMAN_REVISION}") {
        return Err(format!(
            "third_party/sources.lock Surfman source_hash is `{}`, expected `git:{SURFMAN_REVISION}`",
            spec.source_hash
        ));
    }
    if spec.license != SURFMAN_LICENSE {
        return Err(format!(
            "third_party/sources.lock Surfman license is `{}`, expected `{SURFMAN_LICENSE}`",
            spec.license
        ));
    }
    if spec.vendored_path != Path::new("third_party/surfman") {
        return Err(format!(
            "third_party/sources.lock Surfman vendored_path is `{}`, expected `third_party/surfman`",
            spec.vendored_path.display()
        ));
    }
    if spec.patch_path != Path::new("third_party/surfman-patches") {
        return Err(format!(
            "third_party/sources.lock Surfman nagi_patch is `{}`, expected `third_party/surfman-patches`",
            spec.patch_path.display()
        ));
    }
    Ok(spec)
}

/// Bootstrap the pinned Servo Surfman source and its Nagi static-EGL adapter.
/// Existing destinations are never repaired or overwritten in place.
pub(crate) fn ensure_surfman_checkout(root: &Path) -> Result<PathBuf, String> {
    let spec = validate_surfman_source_lock(root)?;
    let checkout = root.join(&spec.vendored_path);
    if checkout.exists() {
        return validate_surfman_checkout(root, &spec);
    }

    let patch_files = surfman_patch_files(root, &spec)?;
    let parent = checkout
        .parent()
        .ok_or_else(|| format!("Surfman checkout has no parent: {}", checkout.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    let temporary = parent.join(format!(
        ".surfman-bootstrap-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| format!("cannot read clock for Surfman bootstrap: {error}"))?
            .as_nanos()
    ));
    if temporary.exists() {
        return Err(format!(
            "refusing to reuse unexpected Surfman bootstrap directory {}",
            temporary.display()
        ));
    }
    fs::create_dir_all(&temporary).map_err(|error| {
        format!(
            "cannot create temporary Surfman checkout {}: {error}",
            temporary.display()
        )
    })?;

    let result = (|| {
        git(
            &temporary,
            ["init", "--quiet"],
            "initialize Surfman checkout",
        )?;
        git(
            &temporary,
            ["config", "core.autocrlf", "false"],
            "configure Surfman line endings",
        )?;
        git(
            &temporary,
            ["remote", "add", "origin", SURFMAN_REPOSITORY],
            "configure Surfman remote",
        )?;
        git(
            &temporary,
            [
                "fetch",
                "--depth",
                "1",
                "--filter=blob:none",
                "origin",
                SURFMAN_REVISION,
            ],
            "fetch pinned Surfman revision",
        )?;
        git(
            &temporary,
            ["checkout", "--detach", "FETCH_HEAD"],
            "checkout pinned Surfman revision",
        )?;
        exclude_marker(&temporary)?;
        apply_surfman_patches(&temporary, &patch_files)?;
        write_generated_marker(root, &temporary, &spec)?;
        validate_surfman_checkout_at(root, &temporary, &spec)
    })();

    match result {
        Ok(()) => {
            if checkout.exists() {
                let _ = fs::remove_dir_all(&temporary);
                return Err(format!(
                    "cannot install generated Surfman checkout at {}; destination was created concurrently and was preserved",
                    checkout.display()
                ));
            }
            fs::rename(&temporary, &checkout).map_err(|error| {
                let _ = fs::remove_dir_all(&temporary);
                format!(
                    "cannot install generated Surfman checkout {} at {}: {error}",
                    temporary.display(),
                    checkout.display()
                )
            })?;
            validate_surfman_checkout(root, &spec)
        }
        Err(error) => {
            let _ = fs::remove_dir_all(&temporary);
            Err(error)
        }
    }
}

fn validate_surfman_checkout(root: &Path, spec: &SurfmanSourceSpec) -> Result<PathBuf, String> {
    let checkout = root.join(&spec.vendored_path);
    validate_surfman_checkout_at(root, &checkout, spec).map(|()| checkout)
}

fn validate_surfman_checkout_at(
    root: &Path,
    checkout: &Path,
    spec: &SurfmanSourceSpec,
) -> Result<(), String> {
    if !checkout.is_dir() {
        return Err(format!(
            "Surfman checkout is not a directory: {}",
            checkout.display()
        ));
    }
    let head = git(checkout, ["rev-parse", "HEAD"], "read Surfman revision")?;
    if head.trim() != spec.revision {
        return Err(format!(
            "Surfman checkout revision is `{}`, expected `{}`",
            head.trim(),
            spec.revision
        ));
    }
    let marker = checkout.join(GENERATED_MARKER);
    let contents = fs::read_to_string(&marker).map_err(|error| {
        format!(
            "generated Surfman checkout marker is missing {}; refusing to modify {}: {error}",
            marker.display(),
            checkout.display()
        )
    })?;
    let expected_patch = surfman_patch_fingerprint(root, spec)?;
    if marker_value(&contents, "revision") != Some(spec.revision.as_str())
        || marker_value(&contents, "patch_fingerprint") != Some(expected_patch.as_str())
    {
        return Err(format!(
            "generated Surfman checkout marker does not match pinned revision or patch fingerprint: {}",
            marker.display()
        ));
    }
    let status = git(checkout, ["status", "--short"], "validate Surfman worktree")?;
    let actual = checkout_state_fingerprint(checkout, &status)?;
    let expected = marker_value(&contents, "checkout_fingerprint").ok_or_else(|| {
        format!(
            "generated Surfman checkout marker is missing checkout_fingerprint: {}",
            marker.display()
        )
    })?;
    if actual != expected {
        return Err(format!(
            "pinned Surfman checkout state does not match generated patch result; refusing to modify {}",
            checkout.display()
        ));
    }
    Ok(())
}

fn surfman_patch_files(root: &Path, spec: &SurfmanSourceSpec) -> Result<Vec<PathBuf>, String> {
    let directory = root.join(&spec.patch_path);
    if !directory.is_dir() {
        return Err(format!(
            "Surfman patch directory is missing: {}",
            directory.display()
        ));
    }
    let mut patches = Vec::new();
    for entry in fs::read_dir(&directory)
        .map_err(|error| format!("cannot read Surfman patch directory: {error}"))?
    {
        let path = entry
            .map_err(|error| format!("cannot inspect Surfman patch entry: {error}"))?
            .path();
        if path.extension() != Some(OsStr::new("patch")) {
            continue;
        }
        let name = path
            .file_name()
            .and_then(OsStr::to_str)
            .ok_or_else(|| format!("Surfman patch has a non-UTF-8 filename: {}", path.display()))?;
        if name
            .split_once('-')
            .and_then(|(number, _)| number.parse::<u32>().ok())
            .is_none()
        {
            return Err(format!(
                "Surfman patch `{name}` must start with a numeric prefix"
            ));
        }
        patches.push(path);
    }
    patches.sort();
    Ok(patches)
}

fn apply_surfman_patches(checkout: &Path, patches: &[PathBuf]) -> Result<(), String> {
    for patch in patches {
        let name = patch.display().to_string();
        let check = Command::new("git")
            .args([
                "-C",
                checkout
                    .to_str()
                    .ok_or_else(|| format!("invalid checkout path: {}", checkout.display()))?,
                "apply",
                "--check",
                "--unidiff-zero",
                "--ignore-space-change",
            ])
            .arg(external_command_path(patch))
            .output()
            .map_err(|error| format!("cannot check Surfman patch {name}: {error}"))?;
        if !check.status.success() {
            return Err(format!(
                "Surfman patch check failed for {name}: {}",
                command_output(&check)
            ));
        }
        let applied = Command::new("git")
            .args([
                "-C",
                checkout
                    .to_str()
                    .ok_or_else(|| format!("invalid checkout path: {}", checkout.display()))?,
                "apply",
                "--unidiff-zero",
                "--ignore-space-change",
            ])
            .arg(external_command_path(patch))
            .output()
            .map_err(|error| format!("cannot apply Surfman patch {name}: {error}"))?;
        if !applied.status.success() {
            return Err(format!(
                "Surfman patch application failed for {name}: {}",
                command_output(&applied)
            ));
        }
    }
    Ok(())
}

fn exclude_marker(checkout: &Path) -> Result<(), String> {
    let exclude = checkout.join(".git").join("info").join("exclude");
    let marker = format!("/{GENERATED_MARKER}");
    let mut contents = fs::read_to_string(&exclude).unwrap_or_default();
    if !contents.lines().any(|line| line.trim() == marker) {
        if !contents.is_empty() && !contents.ends_with('\n') {
            contents.push('\n');
        }
        contents.push_str(&marker);
        contents.push('\n');
        fs::write(&exclude, contents)
            .map_err(|error| format!("cannot update generated Surfman Git excludes: {error}"))?;
    }
    Ok(())
}

fn write_generated_marker(
    root: &Path,
    checkout: &Path,
    spec: &SurfmanSourceSpec,
) -> Result<(), String> {
    let status = git(checkout, ["status", "--short"], "read Surfman worktree")?;
    let patch_fingerprint = surfman_patch_fingerprint(root, spec)?;
    let checkout_fingerprint = checkout_state_fingerprint(checkout, &status)?;
    fs::write(
        checkout.join(GENERATED_MARKER),
        format!(
            "format_version = 1\nrevision = {}\npatch_fingerprint = {}\ncheckout_fingerprint = {}\n",
            spec.revision, patch_fingerprint, checkout_fingerprint
        ),
    )
    .map_err(|error| format!("cannot write generated Surfman marker: {error}"))
}

fn surfman_patch_fingerprint(root: &Path, spec: &SurfmanSourceSpec) -> Result<String, String> {
    let mut fingerprint = Fnv1a::new();
    for patch in surfman_patch_files(root, spec)? {
        let name = patch.file_name().and_then(OsStr::to_str).ok_or_else(|| {
            format!(
                "Surfman patch has a non-UTF-8 filename: {}",
                patch.display()
            )
        })?;
        fingerprint.update(name.as_bytes());
        fingerprint.update(&[0]);
        fingerprint.update(
            &fs::read(&patch).map_err(|error| {
                format!("cannot read Surfman patch {}: {error}", patch.display())
            })?,
        );
        fingerprint.update(&[0]);
    }
    Ok(fingerprint.finish())
}

fn checkout_state_fingerprint(checkout: &Path, status: &str) -> Result<String, String> {
    let diff = git(
        checkout,
        ["diff", "--binary", "--no-ext-diff"],
        "read Surfman generated diff",
    )?;
    let mut fingerprint = Fnv1a::new();
    fingerprint.update(status.as_bytes());
    fingerprint.update(&[0]);
    fingerprint.update(diff.as_bytes());
    Ok(fingerprint.finish())
}

fn git<I, S>(directory: &Path, args: I, action: &str) -> Result<String, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(args)
        .output()
        .map_err(|error| format!("cannot {action}: {error}"))?;
    if !output.status.success() {
        return Err(format!("cannot {action}: {}", command_output(&output)));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
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
            if candidate.trim() != key {
                continue;
            }
            return Some(value.trim().trim_matches('"').to_owned());
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
        return Err(format!("Surfman {field} must be a relative normal path"));
    }
    Ok(path)
}

fn marker_value<'a>(contents: &'a str, key: &str) -> Option<&'a str> {
    contents
        .lines()
        .find_map(|line| line.strip_prefix(&format!("{key} = ")))
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

    use super::{load_surfman_source_spec, validate_surfman_source_lock, SurfmanSourceSpec};

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("nagi-surfman-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("third_party")).expect("temp root");
        root
    }

    fn expected_spec() -> SurfmanSourceSpec {
        SurfmanSourceSpec {
            repository: "https://github.com/servo/surfman.git".into(),
            revision: "205778f497327c573929c7b471194390e15f331d".into(),
            source_hash: "git:205778f497327c573929c7b471194390e15f331d".into(),
            license: "MIT OR Apache-2.0 OR MPL-2.0".into(),
            vendored_path: PathBuf::from("third_party/surfman"),
            patch_path: PathBuf::from("third_party/surfman-patches"),
        }
    }

    #[test]
    fn surfman_lock_metadata_is_pinned_without_a_checkout() {
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
        assert_eq!(validate_surfman_source_lock(&root), Ok(expected_spec()));
        assert_eq!(load_surfman_source_spec(&root), Ok(expected_spec()));
        assert!(!root.join("third_party/surfman").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn surfman_lock_rejects_a_changed_revision() {
        let root = temp_root("wrong-revision");
        fs::write(
            root.join("third_party/sources.lock"),
            "[sources.surfman]\nrepository = \"https://github.com/servo/surfman.git\"\nrevision = \"wrong\"\nsource_hash = \"git:wrong\"\nlicense = \"MIT OR Apache-2.0 OR MPL-2.0\"\nvendored_path = \"third_party/surfman\"\nnagi_patch = \"third_party/surfman-patches\"\n",
        )
        .expect("lock");
        let error = validate_surfman_source_lock(&root).unwrap_err();
        assert!(error.contains("revision"), "{error}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn nagi_workspace_binds_surfman_to_the_pinned_checkout() {
        let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root");
        let cargo_toml = fs::read_to_string(workspace.join("Cargo.toml")).expect("root manifest");
        assert!(
            cargo_toml.contains("surfman = { path = \"third_party/surfman\" }"),
            "the Nagi workspace must bind Servo's Surfman dependency to its pinned patched checkout"
        );
    }
}
