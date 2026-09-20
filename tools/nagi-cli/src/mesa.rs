use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use crate::paths::external_command_path;

const MESA_SECTION: &str = "sources.mesa";
const MESA_REVISION: &str = "f1f246cfda65eff82fba3be1caf2d23bdeda60cc";
const MESA_REPOSITORY: &str = "https://gitlab.freedesktop.org/mesa/mesa.git";
const MESA_LICENSE: &str = "MIT (core/Gallium); component notices required";
const GENERATED_MARKER: &str = ".nagi-mesa-checkout";
const MESA_CLONE_ATTEMPTS: usize = 3;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MesaSourceSpec {
    pub(crate) repository: String,
    pub(crate) revision: String,
    pub(crate) source_hash: String,
    pub(crate) license: String,
    pub(crate) vendored_path: PathBuf,
    pub(crate) patch_path: PathBuf,
}

pub(crate) fn load_mesa_source_spec(root: &Path) -> Result<MesaSourceSpec, String> {
    let lock = fs::read_to_string(root.join("third_party").join("sources.lock"))
        .map_err(|error| format!("cannot read third_party/sources.lock: {error}"))?;
    let required = |key: &str| {
        lock_value(&lock, MESA_SECTION, key)
            .ok_or_else(|| format!("third_party/sources.lock is missing Mesa field `{key}`"))
    };
    Ok(MesaSourceSpec {
        repository: required("repository")?,
        revision: required("revision")?,
        source_hash: required("source_hash")?,
        license: required("license")?,
        vendored_path: relative_path(&required("vendored_path")?, "vendored_path")?,
        patch_path: relative_path(&required("nagi_patch")?, "nagi_patch")?,
    })
}

pub(crate) fn validate_mesa_source_lock(root: &Path) -> Result<MesaSourceSpec, String> {
    let spec = load_mesa_source_spec(root)?;
    if spec.repository != MESA_REPOSITORY {
        return Err(format!(
            "third_party/sources.lock Mesa repository is `{}`, expected `{MESA_REPOSITORY}`",
            spec.repository
        ));
    }
    if spec.revision != MESA_REVISION {
        return Err(format!(
            "third_party/sources.lock Mesa revision is `{}`, expected `{MESA_REVISION}`",
            spec.revision
        ));
    }
    if spec.source_hash != format!("git:{MESA_REVISION}") {
        return Err(format!(
            "third_party/sources.lock Mesa source_hash is `{}`, expected `git:{MESA_REVISION}`",
            spec.source_hash
        ));
    }
    if spec.license != MESA_LICENSE {
        return Err(format!(
            "third_party/sources.lock Mesa license is `{}`, expected `{MESA_LICENSE}`",
            spec.license
        ));
    }
    if spec.vendored_path != Path::new("third_party/mesa") {
        return Err(format!(
            "third_party/sources.lock Mesa vendored_path is `{}`, expected `third_party/mesa`",
            spec.vendored_path.display()
        ));
    }
    if spec.patch_path != Path::new("third_party/mesa-patches") {
        return Err(format!(
            "third_party/sources.lock Mesa nagi_patch is `{}`, expected `third_party/mesa-patches`",
            spec.patch_path.display()
        ));
    }
    Ok(spec)
}

/// Ensure the exact pinned Mesa source is available for the guest Softpipe
/// build. The checkout is generated and never repaired in place: an existing
/// checkout must carry our marker and match its recorded patch/worktree state.
pub(crate) fn ensure_mesa_checkout(root: &Path) -> Result<PathBuf, String> {
    let spec = validate_mesa_source_lock(root)?;
    let checkout = root.join(&spec.vendored_path);
    if checkout.exists() {
        return validate_mesa_checkout(root, &spec);
    }

    let patch_files = mesa_patch_files(root, &spec)?;
    let parent = checkout
        .parent()
        .ok_or_else(|| format!("Mesa checkout has no parent: {}", checkout.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    let temporary = parent.join(format!(
        ".mesa-bootstrap-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| format!("cannot read clock for Mesa bootstrap: {error}"))?
            .as_nanos()
    ));

    run_git_clone(parent, &temporary)?;
    if let Err(error) = run_git(
        root,
        ["checkout", "--detach", MESA_REVISION],
        &temporary,
        "checkout pinned Mesa revision",
    )
    .and_then(|_| exclude_marker(&temporary))
    .and_then(|_| apply_mesa_patches(&temporary, &patch_files))
    .and_then(|_| write_generated_marker(root, &temporary, &spec))
    .and_then(|_| validate_mesa_checkout_at(root, &temporary, &spec))
    {
        let _ = fs::remove_dir_all(&temporary);
        return Err(error);
    }

    if checkout.exists() {
        let _ = fs::remove_dir_all(&temporary);
        return Err(format!(
            "cannot install generated Mesa checkout at {}; destination was created concurrently and was preserved",
            checkout.display()
        ));
    }
    fs::rename(&temporary, &checkout).map_err(|error| {
        let _ = fs::remove_dir_all(&temporary);
        format!(
            "cannot install generated Mesa checkout {} at {}: {error}",
            temporary.display(),
            checkout.display()
        )
    })?;
    validate_mesa_checkout(root, &spec)
}

fn validate_mesa_checkout(root: &Path, spec: &MesaSourceSpec) -> Result<PathBuf, String> {
    let checkout = root.join(&spec.vendored_path);
    validate_mesa_checkout_at(root, &checkout, spec)
}

fn validate_mesa_checkout_at(
    root: &Path,
    checkout: &Path,
    spec: &MesaSourceSpec,
) -> Result<PathBuf, String> {
    if !checkout.is_dir() {
        return Err(format!(
            "Mesa checkout is not a directory: {}",
            checkout.display()
        ));
    }
    let head = run_git(root, ["rev-parse", "HEAD"], checkout, "read Mesa revision")?;
    if head.trim() != spec.revision {
        return Err(format!(
            "Mesa checkout revision is `{}`, expected `{}`",
            head.trim(),
            spec.revision
        ));
    }
    let marker = checkout.join(GENERATED_MARKER);
    let marker_contents = fs::read_to_string(&marker).map_err(|error| {
        format!(
            "generated Mesa checkout marker is missing {}; refusing to modify {}: {error}",
            marker.display(),
            checkout.display()
        )
    })?;
    let expected_patch = mesa_patch_fingerprint(root, spec)?;
    let marker_revision = marker_value(&marker_contents, "revision").ok_or_else(|| {
        format!(
            "generated Mesa checkout marker is missing revision: {}",
            marker.display()
        )
    })?;
    let marker_patch = marker_value(&marker_contents, "patch_fingerprint").ok_or_else(|| {
        format!(
            "generated Mesa checkout marker is missing patch_fingerprint: {}",
            marker.display()
        )
    })?;
    if marker_revision != spec.revision || marker_patch != expected_patch {
        return Err(format!(
            "generated Mesa checkout marker does not match pinned revision or patch fingerprint: {}",
            marker.display()
        ));
    }
    let status = run_git(
        root,
        ["status", "--short"],
        checkout,
        "validate Mesa worktree",
    )?;
    let actual = checkout_state_fingerprint(checkout, &status)?;
    let expected = marker_value(&marker_contents, "checkout_fingerprint").ok_or_else(|| {
        format!(
            "generated Mesa checkout marker is missing checkout_fingerprint: {}",
            marker.display()
        )
    })?;
    if actual != expected {
        return Err(format!(
            "pinned Mesa checkout state does not match generated patch result; refusing to modify {}",
            checkout.display()
        ));
    }
    Ok(checkout.to_path_buf())
}

fn mesa_patch_files(root: &Path, spec: &MesaSourceSpec) -> Result<Vec<PathBuf>, String> {
    let directory = root.join(&spec.patch_path);
    if !directory.exists() {
        return Err(format!(
            "Mesa patch directory is missing: {}",
            directory.display()
        ));
    }
    let mut patches = Vec::new();
    for entry in fs::read_dir(&directory)
        .map_err(|error| format!("cannot read Mesa patch directory: {error}"))?
    {
        let path = entry
            .map_err(|error| format!("cannot inspect Mesa patch entry: {error}"))?
            .path();
        if path.extension() != Some(OsStr::new("patch")) {
            continue;
        }
        let name = path
            .file_name()
            .and_then(OsStr::to_str)
            .ok_or_else(|| format!("Mesa patch has a non-UTF-8 filename: {}", path.display()))?;
        if name
            .split_once('-')
            .and_then(|(number, _)| number.parse::<u32>().ok())
            .is_none()
        {
            return Err(format!(
                "Mesa patch `{name}` must start with a numeric prefix"
            ));
        }
        patches.push(path);
    }
    patches.sort();
    Ok(patches)
}

fn apply_mesa_patches(checkout: &Path, patches: &[PathBuf]) -> Result<(), String> {
    for patch in patches {
        let name = patch.display().to_string();
        let check = Command::new("git")
            .arg("-C")
            .arg(checkout)
            .args(["apply", "--check", "--unidiff-zero"])
            .arg(external_command_path(patch))
            .output()
            .map_err(|error| format!("cannot check Mesa patch {name}: {error}"))?;
        if !check.status.success() {
            return Err(format!(
                "Mesa patch check failed for {name}: {}",
                command_output(&check)
            ));
        }
        let applied = Command::new("git")
            .arg("-C")
            .arg(checkout)
            .args(["apply", "--unidiff-zero"])
            .arg(external_command_path(patch))
            .output()
            .map_err(|error| format!("cannot apply Mesa patch {name}: {error}"))?;
        if !applied.status.success() {
            return Err(format!(
                "Mesa patch application failed for {name}: {}",
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
            .map_err(|error| format!("cannot update generated Mesa Git excludes: {error}"))?;
    }
    Ok(())
}

fn write_generated_marker(
    root: &Path,
    checkout: &Path,
    spec: &MesaSourceSpec,
) -> Result<(), String> {
    let status = run_git(root, ["status", "--short"], checkout, "read Mesa worktree")?;
    let patch_fingerprint = mesa_patch_fingerprint(root, spec)?;
    let checkout_fingerprint = checkout_state_fingerprint(checkout, &status)?;
    fs::write(
        checkout.join(GENERATED_MARKER),
        format!(
            "format_version = 1\nrevision = {}\npatch_fingerprint = {}\ncheckout_fingerprint = {}\n",
            spec.revision, patch_fingerprint, checkout_fingerprint
        ),
    )
    .map_err(|error| format!("cannot write generated Mesa marker: {error}"))
}

fn mesa_patch_fingerprint(root: &Path, spec: &MesaSourceSpec) -> Result<String, String> {
    let mut fingerprint = Fnv1a::new();
    for patch in mesa_patch_files(root, spec)? {
        let name = patch
            .file_name()
            .and_then(OsStr::to_str)
            .ok_or_else(|| format!("Mesa patch has a non-UTF-8 filename: {}", patch.display()))?;
        fingerprint.update(name.as_bytes());
        fingerprint.update(&[0]);
        fingerprint.update(
            &fs::read(&patch)
                .map_err(|error| format!("cannot read Mesa patch {}: {error}", patch.display()))?,
        );
        fingerprint.update(&[0]);
    }
    Ok(fingerprint.finish())
}

fn checkout_state_fingerprint(checkout: &Path, status: &str) -> Result<String, String> {
    let diff = run_git(
        checkout,
        ["diff", "--binary", "--no-ext-diff"],
        checkout,
        "read Mesa generated diff",
    )?;
    let mut fingerprint = Fnv1a::new();
    fingerprint.update(status.as_bytes());
    fingerprint.update(&[0]);
    fingerprint.update(diff.as_bytes());
    Ok(fingerprint.finish())
}

fn run_git<I, S>(_root: &Path, args: I, directory: &Path, action: &str) -> Result<String, String>
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

fn run_git_clone(parent: &Path, destination: &Path) -> Result<(), String> {
    for attempt in 1..=MESA_CLONE_ATTEMPTS {
        if attempt > 1 {
            std::thread::sleep(std::time::Duration::from_secs((attempt - 1) as u64 * 2));
        }

        let output = Command::new("git")
            .current_dir(parent)
            .args([
                "clone",
                "-c",
                "core.autocrlf=false",
                "--filter=blob:none",
                "--no-checkout",
                "--no-single-branch",
                MESA_REPOSITORY,
            ])
            .arg(external_command_path(destination))
            .output()
            .map_err(|error| format!("cannot clone pinned Mesa source: {error}"))?;
        if output.status.success() {
            return Ok(());
        }

        let detail = command_output(&output);
        if attempt == MESA_CLONE_ATTEMPTS || !is_retryable_clone_error(&detail) {
            return Err(format!("cannot clone pinned Mesa source: {detail}"));
        }

        // The destination is a generated, unique temporary path owned by this
        // invocation. Remove a partial clone before retrying so a later Git
        // invocation cannot accidentally reuse an unverified repository.
        let _ = fs::remove_dir_all(destination);
    }

    unreachable!("Mesa clone attempts must return from the loop");
}

fn is_retryable_clone_error(detail: &str) -> bool {
    let detail = detail.to_ascii_lowercase();
    [
        "connection reset",
        "connection timed out",
        "failed to connect",
        "could not resolve host",
        "network is unreachable",
        "early eof",
        "unexpected disconnect",
        "remote end hung up",
        "http 502",
        "http 503",
        "http 504",
        "curl 5",
        "curl 6",
        "curl 7",
        "curl 18",
        "curl 28",
        "tls connection was non-properly terminated",
    ]
    .iter()
    .any(|marker| detail.contains(marker))
}

fn command_output(output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    format!("{}{}", stdout, stderr).trim().to_owned()
}

fn lock_value(contents: &str, section: &str, key: &str) -> Option<String> {
    let mut current = "";
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            current = trimmed.trim_matches(['[', ']']);
            continue;
        }
        if current != section {
            continue;
        }
        let (candidate, value) = trimmed.split_once('=')?;
        if candidate.trim() == key {
            return Some(value.trim().trim_matches('"').to_owned());
        }
    }
    None
}

fn relative_path(value: &str, field: &str) -> Result<PathBuf, String> {
    let path = PathBuf::from(value);
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(format!(
            "third_party/sources.lock Mesa {field} must be a relative safe path"
        ));
    }
    Ok(path)
}

fn marker_value<'a>(contents: &'a str, key: &str) -> Option<&'a str> {
    contents.lines().find_map(|line| {
        let (candidate, value) = line.split_once('=')?;
        (candidate.trim() == key).then(|| value.trim())
    })
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
        is_retryable_clone_error, load_mesa_source_spec, validate_mesa_source_lock, MesaSourceSpec,
    };

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("nagi-mesa-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("third_party")).expect("temp root");
        root
    }

    fn expected_spec() -> MesaSourceSpec {
        MesaSourceSpec {
            repository: "https://gitlab.freedesktop.org/mesa/mesa.git".into(),
            revision: "f1f246cfda65eff82fba3be1caf2d23bdeda60cc".into(),
            source_hash: "git:f1f246cfda65eff82fba3be1caf2d23bdeda60cc".into(),
            license: "MIT (core/Gallium); component notices required".into(),
            vendored_path: PathBuf::from("third_party/mesa"),
            patch_path: PathBuf::from("third_party/mesa-patches"),
        }
    }

    #[test]
    fn mesa_lock_metadata_is_pinned_without_a_checkout() {
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

        assert_eq!(validate_mesa_source_lock(&root), Ok(expected_spec()));
        assert_eq!(load_mesa_source_spec(&root), Ok(expected_spec()));
        assert!(!root.join("third_party/mesa").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn mesa_lock_rejects_a_changed_revision() {
        let root = temp_root("wrong-revision");
        fs::write(
            root.join("third_party/sources.lock"),
            "[sources.mesa]\nrepository = \"https://gitlab.freedesktop.org/mesa/mesa.git\"\nrevision = \"wrong\"\nsource_hash = \"git:wrong\"\nlicense = \"MIT (core/Gallium); component notices required\"\nvendored_path = \"third_party/mesa\"\nnagi_patch = \"third_party/mesa-patches\"\n",
        )
        .expect("lock");

        let error = validate_mesa_source_lock(&root).unwrap_err();
        assert!(error.contains("revision"), "{error}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn mesa_clone_retries_only_transient_transport_failures() {
        assert!(is_retryable_clone_error(
            "fatal: unable to access 'https://gitlab.freedesktop.org/mesa/mesa.git/': Send failure: Connection reset by peer"
        ));
        assert!(is_retryable_clone_error("fatal: early EOF"));
        assert!(!is_retryable_clone_error(
            "fatal: could not read Username for 'https://gitlab.freedesktop.org': terminal prompts disabled"
        ));
        assert!(!is_retryable_clone_error(
            "fatal: repository 'https://gitlab.freedesktop.org/mesa/mesa.git/' not found"
        ));
    }
}
