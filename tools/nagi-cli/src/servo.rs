use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use crate::freetype_sys::ensure_freetype_sys_checkout;
use crate::hyper_util_servo::ensure_hyper_util_servo_checkout;
use crate::libc_servo::ensure_libc_servo_checkout;
use crate::mio_servo::ensure_mio_servo_checkout;
use crate::paths::external_command_path;
use crate::socket2_servo::ensure_socket2_servo_checkout;
use crate::tokio_servo::ensure_tokio_servo_checkout;

const SERVO_SECTION: &str = "sources.servo";
const SERVO_REVISION: &str = "b820a9679a784877f91b4acc90c2c6e849f18d3b";
const SERVO_REPOSITORY: &str = "https://github.com/servo/servo.git";
const SERVO_LICENSE: &str = "MPL-2.0";
const GENERATED_MARKER: &str = ".nagi-servo-checkout";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ServoSourceSpec {
    pub(crate) repository: String,
    pub(crate) revision: String,
    pub(crate) source_hash: String,
    pub(crate) license: String,
    pub(crate) vendored_path: PathBuf,
    pub(crate) patch_path: PathBuf,
}

pub(crate) fn load_servo_source_spec(root: &Path) -> Result<ServoSourceSpec, String> {
    let lock = fs::read_to_string(root.join("third_party").join("sources.lock"))
        .map_err(|error| format!("cannot read third_party/sources.lock: {error}"))?;
    let required = |key: &str| {
        lock_value(&lock, SERVO_SECTION, key)
            .ok_or_else(|| format!("third_party/sources.lock is missing Servo field `{key}`"))
    };
    let repository = required("repository")?;
    let revision = required("revision")?;
    let source_hash = required("source_hash")?;
    let license = required("license")?;
    let vendored_path = relative_path(&required("vendored_path")?, "vendored_path")?;
    let patch_path = relative_path(&required("nagi_patch")?, "nagi_patch")?;
    Ok(ServoSourceSpec {
        repository,
        revision,
        source_hash,
        license,
        vendored_path,
        patch_path,
    })
}

pub(crate) fn validate_servo_source_lock(root: &Path) -> Result<ServoSourceSpec, String> {
    let spec = load_servo_source_spec(root)?;
    if spec.repository != SERVO_REPOSITORY {
        return Err(format!(
            "third_party/sources.lock Servo repository is `{}`, expected `{SERVO_REPOSITORY}`",
            spec.repository
        ));
    }
    if spec.revision != SERVO_REVISION {
        return Err(format!(
            "third_party/sources.lock Servo revision is `{}`, expected `{SERVO_REVISION}`",
            spec.revision
        ));
    }
    if spec.source_hash != format!("git:{SERVO_REVISION}") {
        return Err(format!(
            "third_party/sources.lock Servo source_hash is `{}`, expected `git:{SERVO_REVISION}`",
            spec.source_hash
        ));
    }
    if spec.license != SERVO_LICENSE {
        return Err(format!(
            "third_party/sources.lock Servo license is `{}`, expected `{SERVO_LICENSE}`",
            spec.license
        ));
    }
    if spec.vendored_path != Path::new("third_party/servo") {
        return Err(format!(
            "third_party/sources.lock Servo vendored_path is `{}`, expected `third_party/servo`",
            spec.vendored_path.display()
        ));
    }
    if spec.patch_path != Path::new("third_party/servo-patches") {
        return Err(format!(
            "third_party/sources.lock Servo nagi_patch is `{}`, expected `third_party/servo-patches`",
            spec.patch_path.display()
        ));
    }
    Ok(spec)
}

pub(crate) fn validate_servo_checkout(
    root: &Path,
    spec: &ServoSourceSpec,
) -> Result<PathBuf, String> {
    let checkout = root.join(&spec.vendored_path);
    validate_servo_checkout_at(root, &checkout, spec)
}

fn validate_servo_checkout_at(
    root: &Path,
    checkout: &Path,
    spec: &ServoSourceSpec,
) -> Result<PathBuf, String> {
    if !checkout.is_dir() {
        return Err(format!(
            "pinned Servo checkout is missing {}",
            checkout.display()
        ));
    }
    if !checkout.join("Cargo.toml").is_file() {
        return Err(format!(
            "pinned Servo checkout is missing {}",
            checkout.join("Cargo.toml").display()
        ));
    }
    let revision = fs::read_to_string(checkout.join("REVISION")).map_err(|error| {
        format!(
            "cannot read pinned Servo revision {}: {error}",
            checkout.join("REVISION").display()
        )
    })?;
    if revision.trim() != spec.revision {
        return Err(format!(
            "third_party/servo/REVISION is `{}`, expected `{}`",
            revision.trim(),
            spec.revision
        ));
    }
    let patch_path = root.join(&spec.patch_path);
    if !patch_path.is_dir() {
        return Err(format!(
            "Nagi Servo patch boundary is missing {}",
            patch_path.display()
        ));
    }
    let head = git_output(checkout, ["rev-parse", "HEAD"])?;
    if head.trim() != spec.revision {
        return Err(format!(
            "pinned Servo checkout HEAD is `{}`, expected `{}`",
            head.trim(),
            spec.revision
        ));
    }
    let patch_fingerprint = servo_patch_fingerprint(root, spec)?;
    let marker = checkout.join(GENERATED_MARKER);
    let expected_prefix = format!(
        "format_version = 1\nrevision = {}\npatch_fingerprint = {}\n",
        spec.revision, patch_fingerprint
    );
    let marker_contents = fs::read_to_string(&marker).map_err(|error| {
        format!(
            "generated Servo checkout marker is missing {}; refusing to modify {}: {error}",
            marker.display(),
            checkout.display()
        )
    })?;
    if !marker_contents.starts_with(&expected_prefix) {
        return Err(format!(
            "generated Servo checkout marker does not match pinned revision or patch fingerprint: {}",
            marker.display()
        ));
    }
    let status = git_output(checkout, ["status", "--porcelain", "--untracked-files=all"])?;
    let checkout_fingerprint = checkout_state_fingerprint(checkout, &status)?;
    let expected_checkout_fingerprint = marker_contents
        .lines()
        .find_map(|line| line.strip_prefix("checkout_fingerprint = "))
        .ok_or_else(|| {
            format!(
                "generated Servo checkout marker is missing checkout_fingerprint: {}",
                marker.display()
            )
        })?;
    if checkout_fingerprint != expected_checkout_fingerprint {
        return Err(format!(
            "pinned Servo checkout state does not match generated patch result; refusing to modify {} ({})",
            checkout.display(),
            status.trim().replace('\n', "; "),
        ));
    }
    Ok(checkout.to_path_buf())
}

pub(crate) fn ensure_servo_checkout(root: &Path) -> Result<PathBuf, String> {
    let spec = validate_servo_source_lock(root)?;
    let checkout = root.join(&spec.vendored_path);
    if checkout.exists() {
        let validated = validate_servo_checkout(root, &spec)?;
        ensure_freetype_sys_checkout(root)?;
        ensure_hyper_util_servo_checkout(root)?;
        return Ok(validated);
    }

    let cache = root.join("out").join("cache");
    fs::create_dir_all(&cache).map_err(|error| {
        format!(
            "cannot create Servo fetch cache {}: {error}",
            cache.display()
        )
    })?;
    let temporary = cache.join(format!("servo-fetch-{}", std::process::id()));
    if temporary.exists() {
        return Err(format!(
            "refusing to reuse unexpected Servo fetch directory {}",
            temporary.display()
        ));
    }
    fs::create_dir_all(&temporary).map_err(|error| {
        format!(
            "cannot create temporary Servo checkout {}: {error}",
            temporary.display()
        )
    })?;

    let result = (|| {
        git_output(&temporary, ["init", "--quiet"])?;
        git_output(&temporary, ["config", "core.autocrlf", "false"])?;
        git_output(&temporary, ["config", "core.longpaths", "true"])?;
        git_output(
            &temporary,
            ["remote", "add", "origin", spec.repository.as_str()],
        )?;
        let fetch_args = vec![
            "fetch".to_owned(),
            "--depth".to_owned(),
            "1".to_owned(),
            "--filter=blob:none".to_owned(),
            "origin".to_owned(),
            spec.revision.clone(),
        ];
        git_output(&temporary, fetch_args)?;
        git_output(&temporary, ["checkout", "--detach", "FETCH_HEAD"])?;
        fetch_unpatched_dependencies(&temporary)?;
        ensure_mio_servo_checkout(root)?;
        ensure_socket2_servo_checkout(root)?;
        ensure_tokio_servo_checkout(root)?;
        ensure_libc_servo_checkout(root)?;
        ensure_freetype_sys_checkout(root)?;
        ensure_hyper_util_servo_checkout(root)?;
        fs::write(temporary.join("REVISION"), format!("{}\n", spec.revision)).map_err(|error| {
            format!(
                "cannot write generated Servo revision marker {}: {error}",
                temporary.join("REVISION").display()
            )
        })?;
        let exclude = temporary.join(".git").join("info").join("exclude");
        let mut excludes = fs::read_to_string(&exclude).unwrap_or_default();
        if !excludes.lines().any(|line| line.trim() == "/REVISION") {
            if !excludes.is_empty() && !excludes.ends_with('\n') {
                excludes.push('\n');
            }
            excludes.push_str("/REVISION\n");
            fs::write(&exclude, excludes).map_err(|error| {
                format!(
                    "cannot update generated Servo Git excludes {}: {error}",
                    exclude.display()
                )
            })?;
        }
        apply_servo_patches(root, &temporary, &spec)?;
        write_generated_marker(root, &temporary, &spec)?;
        validate_servo_checkout_at(root, &temporary, &spec)
            .map_err(|error| format!("temporary Servo checkout validation failed: {error}"))
    })();

    match result {
        Ok(_) => match fs::rename(&temporary, &checkout) {
            Ok(()) => Ok(()),
            Err(error) => {
                let concurrent_checkout = checkout.exists();
                let _ = fs::remove_dir_all(&temporary);
                if concurrent_checkout {
                    Err(format!(
                        "cannot install generated Servo checkout at {}; destination was created concurrently and was preserved: {error}",
                        checkout.display()
                    ))
                } else {
                    Err(format!(
                        "cannot install generated Servo checkout {} at {}: {error}",
                        temporary.display(),
                        checkout.display()
                    ))
                }
            }
        },
        Err(error) => {
            let _ = fs::remove_dir_all(&temporary);
            Err(error)
        }
    }?;
    validate_servo_checkout(root, &spec)
}

fn fetch_unpatched_dependencies(checkout: &Path) -> Result<(), String> {
    let output = Command::new("cargo")
        .args(["fetch", "--locked"])
        .current_dir(checkout)
        .output()
        .map_err(|error| {
            format!(
                "cannot start Cargo while fetching unpatched Servo dependencies in {}: {error}",
                checkout.display()
            )
        })?;
    if !output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        let detail = match (stdout.is_empty(), stderr.is_empty()) {
            (true, true) => format!("exit status {}", output.status),
            (false, true) => stdout,
            (true, false) => stderr,
            (false, false) => format!("{stdout}; {stderr}"),
        };
        return Err(format!(
            "Cargo could not fetch the locked unpatched Servo dependencies in {}: {detail}",
            checkout.display()
        ));
    }
    Ok(())
}

pub(crate) fn apply_servo_patches(
    root: &Path,
    checkout: &Path,
    spec: &ServoSourceSpec,
) -> Result<(), String> {
    for patch in servo_patch_files(root, spec)? {
        let patch_arg = external_command_path(&patch).into_os_string();
        let check_args = vec![
            OsString::from("apply"),
            OsString::from("--check"),
            OsString::from("--unidiff-zero"),
            OsString::from("--ignore-space-change"),
            patch_arg.clone(),
        ];
        git_output(checkout, check_args).map_err(|error| {
            format!(
                "cannot apply Nagi Servo patch {} (check): {error}",
                patch.display()
            )
        })?;
        let apply_args = vec![
            OsString::from("apply"),
            OsString::from("--unidiff-zero"),
            OsString::from("--ignore-space-change"),
            patch_arg,
        ];
        git_output(checkout, apply_args).map_err(|error| {
            format!("cannot apply Nagi Servo patch {}: {error}", patch.display())
        })?;
    }
    Ok(())
}

fn write_generated_marker(
    root: &Path,
    checkout: &Path,
    spec: &ServoSourceSpec,
) -> Result<(), String> {
    let patch_fingerprint = servo_patch_fingerprint(root, spec)?;
    let status = git_output(checkout, ["status", "--porcelain", "--untracked-files=all"])?;
    let checkout_fingerprint = checkout_state_fingerprint(checkout, &status)?;
    let marker = checkout.join(GENERATED_MARKER);
    fs::write(
        &marker,
        format!(
            "format_version = 1\nrevision = {}\npatch_fingerprint = {}\ncheckout_fingerprint = {}\n",
            spec.revision, patch_fingerprint, checkout_fingerprint
        ),
    )
    .map_err(|error| format!("cannot write generated Servo marker {}: {error}", marker.display()))?;
    let exclude = checkout.join(".git").join("info").join("exclude");
    let mut excludes = fs::read_to_string(&exclude).unwrap_or_default();
    for entry in ["/REVISION", "/.nagi-servo-checkout"] {
        if !excludes.lines().any(|line| line.trim() == entry) {
            if !excludes.is_empty() && !excludes.ends_with('\n') {
                excludes.push('\n');
            }
            excludes.push_str(entry);
            excludes.push('\n');
        }
    }
    fs::write(&exclude, excludes).map_err(|error| {
        format!(
            "cannot update generated Servo Git excludes {}: {error}",
            exclude.display()
        )
    })?;
    Ok(())
}

fn servo_patch_files(root: &Path, spec: &ServoSourceSpec) -> Result<Vec<PathBuf>, String> {
    let patch_path = root.join(&spec.patch_path);
    let mut patches = Vec::new();
    for entry in fs::read_dir(&patch_path).map_err(|error| {
        format!(
            "cannot read Nagi Servo patch boundary {}: {error}",
            patch_path.display()
        )
    })? {
        let entry = entry.map_err(|error| {
            format!(
                "cannot read Nagi Servo patch entry {}: {error}",
                patch_path.display()
            )
        })?;
        let path = entry.path();
        if path.extension().and_then(OsStr::to_str) != Some("patch") {
            continue;
        }
        let name = path.file_name().and_then(OsStr::to_str).ok_or_else(|| {
            format!(
                "Nagi Servo patch path is not valid UTF-8: {}",
                path.display()
            )
        })?;
        if name
            .chars()
            .next()
            .is_none_or(|character| !character.is_ascii_digit())
        {
            return Err(format!(
                "Nagi Servo patch must have a numeric prefix: {}",
                path.display()
            ));
        }
        patches.push(path);
    }
    patches.sort_by(|left, right| {
        left.file_name()
            .unwrap_or_default()
            .cmp(right.file_name().unwrap_or_default())
    });
    Ok(patches)
}

fn servo_patch_fingerprint(root: &Path, spec: &ServoSourceSpec) -> Result<String, String> {
    let mut fingerprint = Fnv1a::new();
    for patch in servo_patch_files(root, spec)? {
        let name = patch.file_name().and_then(OsStr::to_str).ok_or_else(|| {
            format!(
                "Nagi Servo patch path is not valid UTF-8: {}",
                patch.display()
            )
        })?;
        let contents = fs::read(&patch).map_err(|error| {
            format!("cannot read Nagi Servo patch {}: {error}", patch.display())
        })?;
        fingerprint.update(name.as_bytes());
        fingerprint.update(&[0]);
        fingerprint.update(&contents);
        fingerprint.update(&[0]);
    }
    Ok(fingerprint.finish())
}

fn checkout_state_fingerprint(checkout: &Path, status: &str) -> Result<String, String> {
    let mut fingerprint = Fnv1a::new();
    let diff = git_output(checkout, ["diff", "HEAD", "--binary", "--no-ext-diff"])?;
    fingerprint.update(diff.as_bytes());
    let mut untracked = status
        .lines()
        .filter_map(|line| {
            if !line.starts_with("?? ") {
                return None;
            }
            let path = line.get(3..)?.trim();
            if path == "REVISION" || path == GENERATED_MARKER {
                None
            } else {
                Some(path.to_owned())
            }
        })
        .collect::<Vec<_>>();
    untracked.sort();
    for path in untracked {
        let file = checkout.join(&path);
        let contents = fs::read(&file).map_err(|error| {
            format!(
                "cannot read generated Servo change {}: {error}",
                file.display()
            )
        })?;
        fingerprint.update(path.as_bytes());
        fingerprint.update(&[0]);
        fingerprint.update(&contents);
        fingerprint.update(&[0]);
    }
    Ok(fingerprint.finish())
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

fn lock_value(contents: &str, section: &str, key: &str) -> Option<String> {
    let mut current_section = "";
    for line in contents.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            current_section = trimmed.trim_matches(['[', ']']);
            continue;
        }
        if current_section != section {
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
        || path
            .components()
            .any(|component| component == Component::ParentDir)
    {
        return Err(format!(
            "third_party/sources.lock Servo {field} must be a relative safe path"
        ));
    }
    Ok(path)
}

fn git_output<I, S>(directory: &Path, args: I) -> Result<String, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let args: Vec<_> = args
        .into_iter()
        .map(|argument| argument.as_ref().to_os_string())
        .collect();
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
        .args(&args)
        .output()
        .map_err(|error| format!("cannot start git in {}: {error}", directory.display()))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(format!(
            "git command in {} failed{}",
            directory.display(),
            if detail.is_empty() {
                String::new()
            } else {
                format!(": {detail}")
            }
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::{
        apply_servo_patches, validate_servo_checkout, validate_servo_source_lock,
        write_generated_marker, ServoSourceSpec,
    };
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    fn temp_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("nagi-servo-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("third_party")).expect("temp root");
        root
    }

    fn expected_spec() -> ServoSourceSpec {
        ServoSourceSpec {
            repository: "https://github.com/servo/servo.git".into(),
            revision: "b820a9679a784877f91b4acc90c2c6e849f18d3b".into(),
            source_hash: "git:b820a9679a784877f91b4acc90c2c6e849f18d3b".into(),
            license: "MPL-2.0".into(),
            vendored_path: PathBuf::from("third_party/servo"),
            patch_path: PathBuf::from("third_party/servo-patches"),
        }
    }

    #[test]
    fn lock_metadata_is_valid_without_a_checkout() {
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
        assert_eq!(validate_servo_source_lock(&root), Ok(expected_spec()));
        assert!(!root.join("third_party/servo").exists());
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn checkout_validation_rejects_wrong_revision_marker() {
        let root = temp_root("wrong-revision");
        let checkout = root.join("third_party/servo");
        fs::create_dir_all(&checkout).expect("checkout");
        fs::create_dir_all(root.join("third_party/servo-patches")).expect("patches");
        fs::write(
            checkout.join("Cargo.toml"),
            b"[package]\nname = \"servo\"\n",
        )
        .expect("manifest");
        fs::write(checkout.join("REVISION"), b"wrong\n").expect("revision");
        let error = validate_servo_checkout(&root, &expected_spec()).unwrap_err();
        assert!(error.contains("REVISION"));
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn checkout_validation_rejects_dirty_git_checkout() {
        let root = temp_root("dirty");
        let checkout = root.join("third_party/servo");
        fs::create_dir_all(root.join("third_party/servo-patches")).expect("patches");
        fs::create_dir_all(&checkout).expect("checkout");
        fs::write(
            checkout.join("Cargo.toml"),
            b"[package]\nname = \"servo\"\n",
        )
        .expect("manifest");
        let run = |args: &[&str]| {
            let output = Command::new("git")
                .args(["-C", checkout.to_str().expect("utf8")])
                .args(args)
                .output()
                .expect("git");
            assert!(output.status.success(), "git failed: {output:?}");
        };
        run(&["init", "--quiet"]);
        run(&["config", "user.email", "nagi-test@example.invalid"]);
        run(&["config", "user.name", "Nagi Test"]);
        run(&["add", "Cargo.toml"]);
        run(&["commit", "--quiet", "-m", "initial"]);
        let head = String::from_utf8_lossy(
            &Command::new("git")
                .args(["-C", checkout.to_str().expect("utf8"), "rev-parse", "HEAD"])
                .output()
                .expect("git head")
                .stdout,
        )
        .trim()
        .to_owned();
        let mut spec = expected_spec();
        spec.revision = head.clone();
        spec.source_hash = format!("git:{head}");
        fs::write(checkout.join("REVISION"), format!("{head}\n")).expect("revision");
        write_generated_marker(&root, &checkout, &spec).expect("generated marker");
        fs::write(
            checkout.join("Cargo.toml"),
            b"[package]\nname = \"dirty\"\n",
        )
        .expect("dirty");
        let error = validate_servo_checkout(&root, &spec).unwrap_err();
        assert!(error.contains("state does not match"), "{error}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn patch_files_are_sorted_and_ignore_non_patch_files() {
        let root = temp_root("patch-order");
        let patches = root.join("third_party/servo-patches");
        fs::create_dir_all(&patches).expect("patches");
        fs::write(patches.join("README.md"), b"documentation").expect("readme");
        fs::write(patches.join("02-later.patch"), b"later").expect("later");
        fs::write(patches.join("01-first.patch"), b"first").expect("first");
        let files = super::servo_patch_files(&root, &expected_spec()).expect("patch files");
        let names = files
            .iter()
            .map(|path| {
                path.file_name()
                    .expect("name")
                    .to_string_lossy()
                    .into_owned()
            })
            .collect::<Vec<_>>();
        assert_eq!(names, ["01-first.patch", "02-later.patch"]);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn patch_files_reject_non_numeric_patch_names() {
        let root = temp_root("patch-name");
        let patches = root.join("third_party/servo-patches");
        fs::create_dir_all(&patches).expect("patches");
        fs::write(patches.join("nagi.patch"), b"invalid name").expect("patch");
        let error = super::servo_patch_files(&root, &expected_spec()).unwrap_err();
        assert!(error.contains("numeric prefix"), "{error}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn servo_patch_boundary_disables_webdriver_server_for_embedded_target() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root");
        let patch = fs::read_to_string(
            root.join("third_party/servo-patches/0005-nagi-split-webdriver-server-feature.patch"),
        )
        .expect("WebDriver feature boundary patch");
        assert!(patch.contains(
            "-webdriver = { version = \"0.54.0\" }\n+webdriver = { version = \"0.54.0\", default-features = false }"
        ));
        assert!(patch.contains(
            "-webdriver = { workspace = true }\n+webdriver = { workspace = true, features = [\"server\"] }"
        ));
    }

    #[test]
    fn servo_patch_boundary_defines_nagi_navigator_platform() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root");
        let patch = fs::read_to_string(
            root.join("third_party/servo-patches/0007-nagi-navigator-platform.patch"),
        )
        .expect("Nagi navigator platform patch");
        assert!(patch.contains("#[cfg(target_os = \"nagi\")]"));
        assert!(patch.contains("DOMString::from_static(\"Nagi\")"));
        assert!(patch.contains("navigatorinfo.rs"));
    }

    #[test]
    fn patch_application_uses_numeric_order() {
        let root = temp_root("patch-apply");
        let checkout = root.join("third_party/servo");
        let patches = root.join("third_party/servo-patches");
        fs::create_dir_all(&checkout).expect("checkout");
        fs::create_dir_all(&patches).expect("patches");
        fs::write(checkout.join("Cargo.toml"), b"value = 0\n").expect("source");
        init_git(&checkout);
        fs::write(
            patches.join("01-first.patch"),
            b"diff --git a/Cargo.toml b/Cargo.toml\n--- a/Cargo.toml\n+++ b/Cargo.toml\n@@ -1 +1 @@\n-value = 0\n+value = 1\n",
        )
        .expect("first patch");
        fs::write(
            patches.join("02-second.patch"),
            b"diff --git a/Cargo.toml b/Cargo.toml\n--- a/Cargo.toml\n+++ b/Cargo.toml\n@@ -1 +1 @@\n-value = 1\n+value = 2\n",
        )
        .expect("second patch");
        apply_servo_patches(&root, &checkout, &expected_spec()).expect("apply patches");
        assert_eq!(
            fs::read_to_string(checkout.join("Cargo.toml")).unwrap(),
            "value = 2\n"
        );
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn checkout_validation_rejects_missing_generated_marker() {
        let root = temp_root("missing-marker");
        let checkout = root.join("third_party/servo");
        fs::create_dir_all(root.join("third_party/servo-patches")).expect("patches");
        fs::create_dir_all(&checkout).expect("checkout");
        fs::write(
            checkout.join("Cargo.toml"),
            b"[package]\nname = \"servo\"\n",
        )
        .expect("manifest");
        init_git(&checkout);
        let head = git_head(&checkout);
        let mut spec = expected_spec();
        spec.revision = head.clone();
        spec.source_hash = format!("git:{head}");
        fs::write(checkout.join("REVISION"), format!("{head}\n")).expect("revision");
        let error = validate_servo_checkout(&root, &spec).unwrap_err();
        assert!(error.contains("marker"), "{error}");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn checkout_validation_reuses_matching_generated_checkout() {
        let root = temp_root("reuse");
        let checkout = root.join("third_party/servo");
        fs::create_dir_all(root.join("third_party/servo-patches")).expect("patches");
        fs::create_dir_all(&checkout).expect("checkout");
        fs::write(
            checkout.join("Cargo.toml"),
            b"[package]\nname = \"servo\"\n",
        )
        .expect("manifest");
        init_git(&checkout);
        let head = git_head(&checkout);
        let mut spec = expected_spec();
        spec.revision = head.clone();
        spec.source_hash = format!("git:{head}");
        fs::write(checkout.join("REVISION"), format!("{head}\n")).expect("revision");
        write_generated_marker(&root, &checkout, &spec).expect("marker");
        assert_eq!(validate_servo_checkout(&root, &spec).unwrap(), checkout);
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn checkout_validation_rejects_patch_fingerprint_change() {
        let root = temp_root("fingerprint-change");
        let checkout = root.join("third_party/servo");
        let patches = root.join("third_party/servo-patches");
        fs::create_dir_all(&patches).expect("patches");
        fs::create_dir_all(&checkout).expect("checkout");
        fs::write(
            checkout.join("Cargo.toml"),
            b"[package]\nname = \"servo\"\n",
        )
        .expect("manifest");
        init_git(&checkout);
        let head = git_head(&checkout);
        let mut spec = expected_spec();
        spec.revision = head.clone();
        spec.source_hash = format!("git:{head}");
        fs::write(checkout.join("REVISION"), format!("{head}\n")).expect("revision");
        write_generated_marker(&root, &checkout, &spec).expect("marker");
        fs::write(patches.join("01-new.patch"), b"not applied").expect("patch");
        let error = validate_servo_checkout(&root, &spec).unwrap_err();
        assert!(error.contains("patch fingerprint"), "{error}");
        let _ = fs::remove_dir_all(root);
    }

    fn init_git(checkout: &Path) {
        run_git(checkout, &["init", "--quiet"]);
        run_git(
            checkout,
            &["config", "user.email", "nagi-test@example.invalid"],
        );
        run_git(checkout, &["config", "user.name", "Nagi Test"]);
        run_git(checkout, &["config", "core.autocrlf", "false"]);
        run_git(checkout, &["add", "Cargo.toml"]);
        run_git(checkout, &["commit", "--quiet", "-m", "initial"]);
    }

    fn git_head(checkout: &Path) -> String {
        run_git(checkout, &["rev-parse", "HEAD"])
    }

    fn run_git(checkout: &Path, args: &[&str]) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(checkout)
            .args(args)
            .output()
            .expect("git");
        assert!(output.status.success(), "git failed: {output:?}");
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }
}
