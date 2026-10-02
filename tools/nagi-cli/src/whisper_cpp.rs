use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::paths::external_command_path;

const SOURCE_SECTION: &str = "sources.whisper_cpp";
const SOURCE_REPOSITORY: &str = "https://github.com/ggml-org/whisper.cpp.git";
const SOURCE_REVISION: &str = "927cfce34f31707e17f2bff35c349632fb9e2c3a";
const SOURCE_LICENSE: &str = "MIT";
const SOURCE_PATH: &str = "third_party/whisper.cpp";
const SOURCE_PATCH_DIR: &str = "third_party/whisper-cpp-patches";
const GENERATED_MARKER: &str = ".nagi-whisper-cpp-checkout";

const MODEL_SECTION: &str = "models.whisper_small_multilingual";
const MODEL_COMPONENT: &str = "whisper-small-multilingual";
const MODEL_ID: &str = "openai.whisper-small-multilingual";
const MODEL_REPOSITORY: &str = "https://huggingface.co/ggerganov/whisper.cpp";
const MODEL_REVISION: &str = "5359861c739e955e79d9a303bcbc70fb988958b1";
const MODEL_FILE: &str = "ggml-small.bin";
const MODEL_FORMAT: &str = "ggml";
const MODEL_SIZE_BYTES: u64 = 487_601_967;
const MODEL_SHA256: &str = "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b";
const MODEL_LICENSE: &str = "MIT";
const MODEL_ARTIFACT_ID: &str = "openai.whisper-small-multilingual";
const MODEL_STORAGE: &str = "model_store";

#[derive(Debug, Clone, PartialEq, Eq)]
struct WhisperCppSourceSpec {
    repository: String,
    revision: String,
    source_hash: String,
    license: String,
    vendored_path: PathBuf,
    patch_path: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WhisperModelArtifactSpec {
    pub(crate) component: String,
    pub(crate) model_id: String,
    pub(crate) repository: String,
    pub(crate) revision: String,
    pub(crate) file_name: String,
    pub(crate) format: String,
    pub(crate) size_bytes: u64,
    pub(crate) sha256: String,
    pub(crate) license: String,
    pub(crate) license_reference: String,
    pub(crate) artifact_id: String,
    pub(crate) storage: String,
}

pub(crate) fn ensure_whisper_cpp_checkout(root: &Path) -> Result<PathBuf, String> {
    let spec = validate_whisper_cpp_source_lock(root)?;
    validate_whisper_model_artifact_lock(root)?;
    let checkout = root.join(&spec.vendored_path);
    let source = if checkout.exists() {
        validate_checkout(&checkout, &spec)?
    } else {
        fetch_upstream_checkout(root, &checkout, &spec)?
    };
    ensure_patched_checkout(root, &source, &spec)
}

fn validate_whisper_cpp_source_lock(root: &Path) -> Result<WhisperCppSourceSpec, String> {
    let lock = fs::read_to_string(root.join("third_party/sources.lock"))
        .map_err(|error| format!("cannot read third_party/sources.lock: {error}"))?;
    let required = |key: &str| {
        lock_value(&lock, SOURCE_SECTION, key)
            .ok_or_else(|| format!("third_party/sources.lock is missing whisper.cpp field `{key}`"))
    };
    let spec = WhisperCppSourceSpec {
        repository: required("repository")?,
        revision: required("revision")?,
        source_hash: required("source_hash")?,
        license: required("license")?,
        vendored_path: PathBuf::from(required("vendored_path")?),
        patch_path: PathBuf::from(required("nagi_patch")?),
    };
    if spec.repository != SOURCE_REPOSITORY
        || spec.revision != SOURCE_REVISION
        || spec.source_hash != format!("git:{SOURCE_REVISION}")
        || spec.license != SOURCE_LICENSE
        || spec.vendored_path != Path::new(SOURCE_PATH)
        || spec.patch_path != Path::new(SOURCE_PATCH_DIR)
    {
        return Err(format!(
            "third_party/sources.lock whisper.cpp entry does not match the reviewed pin `{SOURCE_REVISION}`"
        ));
    }
    Ok(spec)
}

pub(crate) fn validate_whisper_model_artifact_lock(
    root: &Path,
) -> Result<WhisperModelArtifactSpec, String> {
    let lock = fs::read_to_string(root.join("third_party/models.lock"))
        .map_err(|error| format!("cannot read third_party/models.lock: {error}"))?;
    let required = |key: &str| {
        lock_value(&lock, MODEL_SECTION, key).ok_or_else(|| {
            format!("third_party/models.lock is missing Whisper small field `{key}`")
        })
    };
    let size_bytes = required("size_bytes")?
        .parse::<u64>()
        .map_err(|_| "Whisper small size_bytes must be an unsigned integer".to_owned())?;
    let spec = WhisperModelArtifactSpec {
        component: required("component")?,
        model_id: required("model_id")?,
        repository: required("repository")?,
        revision: required("revision")?,
        file_name: required("file_name")?,
        format: required("format")?,
        size_bytes,
        sha256: required("sha256")?,
        license: required("license")?,
        license_reference: required("license_reference")?,
        artifact_id: required("artifact_id")?,
        storage: required("storage")?,
    };
    if spec.component != MODEL_COMPONENT
        || spec.model_id != MODEL_ID
        || spec.repository != MODEL_REPOSITORY
        || spec.revision != MODEL_REVISION
        || spec.file_name != MODEL_FILE
        || spec.format != MODEL_FORMAT
        || spec.size_bytes != MODEL_SIZE_BYTES
        || spec.sha256 != MODEL_SHA256
        || spec.license != MODEL_LICENSE
        || spec.license_reference != format!("{MODEL_REPOSITORY}/tree/{MODEL_REVISION}")
        || spec.artifact_id != MODEL_ARTIFACT_ID
        || spec.storage != MODEL_STORAGE
    {
        return Err(format!(
            "third_party/models.lock Whisper small entry does not match the reviewed artifact pin `{MODEL_REVISION}/{MODEL_FILE}`"
        ));
    }
    Ok(spec)
}

fn fetch_upstream_checkout(
    root: &Path,
    checkout: &Path,
    spec: &WhisperCppSourceSpec,
) -> Result<PathBuf, String> {
    let parent = checkout
        .parent()
        .ok_or_else(|| format!("whisper.cpp checkout has no parent: {}", checkout.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    let cache = root.join("out/cache");
    fs::create_dir_all(&cache)
        .map_err(|error| format!("cannot create {}: {error}", cache.display()))?;
    let temporary = cache.join(format!("whisper-cpp-fetch-{}", std::process::id()));
    if temporary.exists() {
        return Err(format!(
            "refusing to reuse unexpected whisper.cpp fetch directory {}",
            temporary.display()
        ));
    }
    fs::create_dir(&temporary)
        .map_err(|error| format!("cannot create {}: {error}", temporary.display()))?;

    let result = (|| {
        git_output(&temporary, ["init", "--quiet"])?;
        git_output(&temporary, ["config", "core.autocrlf", "false"])?;
        git_output(
            &temporary,
            ["remote", "add", "origin", spec.repository.as_str()],
        )?;
        git_output(
            &temporary,
            [
                "fetch",
                "--depth",
                "1",
                "--filter=blob:none",
                "origin",
                spec.revision.as_str(),
            ],
        )?;
        git_output(&temporary, ["checkout", "--detach", "FETCH_HEAD"])?;
        validate_checkout(&temporary, spec)?;
        if checkout.exists() {
            return Err(format!(
                "cannot install whisper.cpp checkout at {}; destination was created concurrently and was preserved",
                checkout.display()
            ));
        }
        fs::rename(&temporary, checkout).map_err(|error| {
            format!(
                "cannot install whisper.cpp checkout {} at {}: {error}",
                temporary.display(),
                checkout.display()
            )
        })?;
        validate_checkout(checkout, spec)
    })();
    if result.is_err() && temporary.exists() {
        let _ = fs::remove_dir_all(&temporary);
    }
    result
}

fn ensure_patched_checkout(
    root: &Path,
    source: &Path,
    spec: &WhisperCppSourceSpec,
) -> Result<PathBuf, String> {
    if whisper_cpp_patch_files(root, spec)?.is_empty() {
        return Err(
            "Nagi whisper.cpp patch boundary contains no Nagi whisper.cpp patches".to_owned(),
        );
    }
    validate_checkout(source, spec)?;
    let checkout = root.join("out/cache/whisper-cpp-nagi");
    if checkout.exists() {
        return validate_patched_checkout(root, &checkout, spec);
    }
    let cache = checkout.parent().ok_or_else(|| {
        format!(
            "whisper.cpp generated checkout has no parent: {}",
            checkout.display()
        )
    })?;
    fs::create_dir_all(cache)
        .map_err(|error| format!("cannot create {}: {error}", cache.display()))?;
    let temporary = cache.join(format!("whisper-cpp-nagi-{}", std::process::id()));
    if temporary.exists() {
        return Err(format!(
            "refusing to reuse unexpected whisper.cpp patch directory {}",
            temporary.display()
        ));
    }

    let result = (|| {
        let source_path = external_command_path(source);
        let source_path = source_path.to_str().ok_or_else(|| {
            format!(
                "whisper.cpp source path is not valid UTF-8: {}",
                source_path.display()
            )
        })?;
        let temporary_path = external_command_path(&temporary);
        let temporary_path = temporary_path.to_str().ok_or_else(|| {
            format!(
                "whisper.cpp patch path is not valid UTF-8: {}",
                temporary_path.display()
            )
        })?;
        git_output(
            root,
            [
                "clone",
                "--quiet",
                "--local",
                "--no-hardlinks",
                "--no-checkout",
                source_path,
                temporary_path,
            ],
        )?;
        git_output(&temporary, ["checkout", "--detach", spec.revision.as_str()])?;
        apply_whisper_cpp_patches(root, &temporary, spec)?;
        write_generated_marker(root, &temporary, spec)?;
        validate_patched_checkout(root, &temporary, spec)?;
        if checkout.exists() {
            return Err(format!(
                "cannot install generated whisper.cpp checkout at {}; destination was created concurrently and was preserved",
                checkout.display()
            ));
        }
        fs::rename(&temporary, &checkout).map_err(|error| {
            format!(
                "cannot install generated whisper.cpp checkout {} at {}: {error}",
                temporary.display(),
                checkout.display()
            )
        })?;
        validate_patched_checkout(root, &checkout, spec)
    })();
    if result.is_err() && temporary.exists() {
        let _ = fs::remove_dir_all(&temporary);
    }
    result
}

fn whisper_cpp_patch_files(
    root: &Path,
    spec: &WhisperCppSourceSpec,
) -> Result<Vec<PathBuf>, String> {
    let patch_dir = root.join(&spec.patch_path);
    let mut patches = Vec::new();
    for entry in fs::read_dir(&patch_dir).map_err(|error| {
        format!(
            "cannot read Nagi whisper.cpp patch boundary {}: {error}",
            patch_dir.display()
        )
    })? {
        let entry = entry.map_err(|error| {
            format!(
                "cannot read Nagi whisper.cpp patch entry {}: {error}",
                patch_dir.display()
            )
        })?;
        let path = entry.path();
        if path.extension().and_then(OsStr::to_str) != Some("patch") {
            continue;
        }
        if !entry
            .file_type()
            .map_err(|error| format!("cannot inspect patch {}: {error}", path.display()))?
            .is_file()
        {
            return Err(format!(
                "Nagi whisper.cpp patch must be a regular file: {}",
                path.display()
            ));
        }
        let name = path.file_name().and_then(OsStr::to_str).ok_or_else(|| {
            format!(
                "Nagi whisper.cpp patch path is not valid UTF-8: {}",
                path.display()
            )
        })?;
        if name
            .chars()
            .next()
            .is_none_or(|character| !character.is_ascii_digit())
        {
            return Err(format!(
                "Nagi whisper.cpp patch must have a numeric prefix: {}",
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

fn apply_whisper_cpp_patches(
    root: &Path,
    checkout: &Path,
    spec: &WhisperCppSourceSpec,
) -> Result<(), String> {
    for patch in whisper_cpp_patch_files(root, spec)? {
        let patch_arg = external_command_path(&patch).into_os_string();
        let check_args = vec![
            OsString::from("apply"),
            OsString::from("--check"),
            OsString::from("--whitespace=nowarn"),
            patch_arg.clone(),
        ];
        git_output(checkout, check_args).map_err(|error| {
            format!(
                "cannot apply Nagi whisper.cpp patch {} (check): {error}",
                patch.display()
            )
        })?;
        let apply_args = vec![
            OsString::from("apply"),
            OsString::from("--whitespace=nowarn"),
            patch_arg,
        ];
        git_output(checkout, apply_args).map_err(|error| {
            format!(
                "cannot apply Nagi whisper.cpp patch {}: {error}",
                patch.display()
            )
        })?;
    }
    Ok(())
}

fn write_generated_marker(
    root: &Path,
    checkout: &Path,
    spec: &WhisperCppSourceSpec,
) -> Result<(), String> {
    let patch_fingerprint = whisper_cpp_patch_fingerprint(root, spec)?;
    add_git_exclude(checkout, GENERATED_MARKER)?;
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
    .map_err(|error| {
        format!(
            "cannot write generated whisper.cpp marker {}: {error}",
            marker.display()
        )
    })
}

fn validate_patched_checkout(
    root: &Path,
    checkout: &Path,
    spec: &WhisperCppSourceSpec,
) -> Result<PathBuf, String> {
    if !checkout.join("CMakeLists.txt").is_file() || !checkout.join("LICENSE").is_file() {
        return Err(format!(
            "generated whisper.cpp checkout is missing build or license metadata: {}",
            checkout.display()
        ));
    }
    let head = git_output(checkout, ["rev-parse", "HEAD"])?;
    if head.trim() != spec.revision {
        return Err(format!(
            "generated whisper.cpp checkout HEAD is `{}`, expected `{}`",
            head.trim(),
            spec.revision
        ));
    }
    let license = fs::read_to_string(checkout.join("LICENSE")).map_err(|error| {
        format!(
            "cannot read generated whisper.cpp license {}: {error}",
            checkout.join("LICENSE").display()
        )
    })?;
    if !license.starts_with("MIT License") {
        return Err(format!(
            "generated whisper.cpp LICENSE does not match the locked MIT declaration: {}",
            checkout.join("LICENSE").display()
        ));
    }
    let marker = checkout.join(GENERATED_MARKER);
    let marker_contents = fs::read_to_string(&marker).map_err(|error| {
        format!(
            "generated whisper.cpp marker is missing {}; refusing to modify {}: {error}",
            marker.display(),
            checkout.display()
        )
    })?;
    let expected_prefix = format!(
        "format_version = 1\nrevision = {}\npatch_fingerprint = {}\n",
        spec.revision,
        whisper_cpp_patch_fingerprint(root, spec)?
    );
    if !marker_contents.starts_with(&expected_prefix) {
        return Err(format!(
            "generated whisper.cpp marker does not match the pinned revision or patch fingerprint: {}",
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
                "generated whisper.cpp marker is missing checkout_fingerprint: {}",
                marker.display()
            )
        })?;
    if checkout_fingerprint != expected_checkout_fingerprint {
        return Err(format!(
            "generated whisper.cpp checkout state does not match the patch result; refusing to modify {} ({})",
            checkout.display(),
            status.trim().replace('\n', "; ")
        ));
    }
    Ok(checkout.to_path_buf())
}

fn whisper_cpp_patch_fingerprint(
    root: &Path,
    spec: &WhisperCppSourceSpec,
) -> Result<String, String> {
    let mut fingerprint = Fnv1a::new();
    for patch in whisper_cpp_patch_files(root, spec)? {
        let name = patch.file_name().and_then(OsStr::to_str).ok_or_else(|| {
            format!(
                "Nagi whisper.cpp patch path is not valid UTF-8: {}",
                patch.display()
            )
        })?;
        let contents = fs::read(&patch).map_err(|error| {
            format!(
                "cannot read Nagi whisper.cpp patch {}: {error}",
                patch.display()
            )
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
        .filter_map(|line| line.strip_prefix("?? ").map(str::to_owned))
        .filter(|path| path != GENERATED_MARKER)
        .collect::<Vec<_>>();
    untracked.sort();
    for path in untracked {
        let file = checkout.join(&path);
        let contents = fs::read(&file).map_err(|error| {
            format!(
                "cannot read generated whisper.cpp change {}: {error}",
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

fn add_git_exclude(checkout: &Path, entry: &str) -> Result<(), String> {
    let exclude = checkout.join(".git/info/exclude");
    let mut contents = match fs::read_to_string(&exclude) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(format!(
                "cannot read Git exclude {}: {error}",
                exclude.display()
            ));
        }
    };
    let line = format!("/{entry}");
    if !contents.lines().any(|candidate| candidate.trim() == line) {
        if !contents.is_empty() && !contents.ends_with('\n') {
            contents.push('\n');
        }
        contents.push_str(&line);
        contents.push('\n');
        fs::write(&exclude, contents)
            .map_err(|error| format!("cannot update Git exclude {}: {error}", exclude.display()))?;
    }
    Ok(())
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

fn validate_checkout(checkout: &Path, spec: &WhisperCppSourceSpec) -> Result<PathBuf, String> {
    if !checkout.join("CMakeLists.txt").is_file() || !checkout.join("LICENSE").is_file() {
        return Err(format!(
            "pinned whisper.cpp checkout is missing build or license metadata: {}",
            checkout.display()
        ));
    }
    let head = git_output(checkout, ["rev-parse", "HEAD"])?;
    if head.trim() != spec.revision {
        return Err(format!(
            "whisper.cpp checkout HEAD is `{}`, expected `{}`",
            head.trim(),
            spec.revision
        ));
    }
    let status = git_output(checkout, ["status", "--porcelain", "--untracked-files=all"])?;
    if !status.trim().is_empty() {
        return Err(format!(
            "pinned whisper.cpp checkout is dirty; refusing to modify {} ({})",
            checkout.display(),
            status.trim().replace('\n', "; ")
        ));
    }
    let license = fs::read_to_string(checkout.join("LICENSE")).map_err(|error| {
        format!(
            "cannot read whisper.cpp license {}: {error}",
            checkout.join("LICENSE").display()
        )
    })?;
    if !license.starts_with("MIT License") {
        return Err(format!(
            "pinned whisper.cpp LICENSE does not match the locked MIT declaration: {}",
            checkout.display()
        ));
    }
    Ok(checkout.to_path_buf())
}

fn git_output<I, S>(directory: &Path, args: I) -> Result<String, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let directory = external_command_path(directory);
    let output = Command::new("git")
        .arg("-C")
        .arg(&directory)
        .args(args)
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

fn lock_value(lock: &str, section: &str, key: &str) -> Option<String> {
    let mut active = false;
    for line in lock.lines().map(str::trim) {
        if line.starts_with('[') && line.ends_with(']') {
            active = line
                .strip_prefix('[')
                .and_then(|value| value.strip_suffix(']'))
                == Some(section);
            continue;
        }
        if !active {
            continue;
        }
        let Some((candidate, value)) = line.split_once('=') else {
            continue;
        };
        if candidate.trim() == key {
            return Some(value.trim().trim_matches('"').to_owned());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::{
        ensure_patched_checkout, fetch_upstream_checkout, validate_checkout,
        validate_patched_checkout, validate_whisper_cpp_source_lock,
        validate_whisper_model_artifact_lock, whisper_cpp_patch_files, WhisperCppSourceSpec,
    };
    use std::ffi::OsStr;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_root(name: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "nagi-whisper-cpp-{name}-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("temporary root");
        root
    }

    fn git(directory: &Path, args: &[&str]) {
        let output = Command::new("git")
            .arg("-C")
            .arg(directory)
            .args(args)
            .output()
            .expect("run git");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn fixture_spec(repository: String, revision: String) -> WhisperCppSourceSpec {
        WhisperCppSourceSpec {
            repository,
            source_hash: format!("git:{revision}"),
            revision,
            license: "MIT".to_owned(),
            vendored_path: PathBuf::from("third_party/whisper.cpp"),
            patch_path: PathBuf::from("third_party/whisper-cpp-patches"),
        }
    }

    #[test]
    fn whisper_cpp_and_small_multilingual_artifacts_are_exactly_pinned() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source = validate_whisper_cpp_source_lock(&root).expect("whisper.cpp source pin");
        assert_eq!(
            source.repository,
            "https://github.com/ggml-org/whisper.cpp.git"
        );
        assert_eq!(source.revision, "927cfce34f31707e17f2bff35c349632fb9e2c3a");
        assert_eq!(source.source_hash, format!("git:{}", source.revision));
        assert_eq!(source.license, "MIT");
        assert_eq!(source.vendored_path, Path::new("third_party/whisper.cpp"));
        assert_eq!(
            source.patch_path,
            Path::new("third_party/whisper-cpp-patches")
        );

        let model = validate_whisper_model_artifact_lock(&root).expect("Whisper model pin");
        assert_eq!(model.component, "whisper-small-multilingual");
        assert_eq!(model.model_id, "openai.whisper-small-multilingual");
        assert_eq!(
            model.repository,
            "https://huggingface.co/ggerganov/whisper.cpp"
        );
        assert_eq!(model.revision, "5359861c739e955e79d9a303bcbc70fb988958b1");
        assert_eq!(model.file_name, "ggml-small.bin");
        assert_eq!(model.format, "ggml");
        assert_eq!(model.size_bytes, 487_601_967);
        assert_eq!(
            model.sha256,
            "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b"
        );
        assert_eq!(model.license, "MIT");
        assert_eq!(
            model.license_reference,
            "https://huggingface.co/ggerganov/whisper.cpp/tree/5359861c739e955e79d9a303bcbc70fb988958b1"
        );
        assert_eq!(model.artifact_id, "openai.whisper-small-multilingual");
        assert_eq!(model.storage, "model_store");
    }

    #[test]
    fn standard_model_loader_read_count_hardening_is_a_numbered_patch() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source = validate_whisper_cpp_source_lock(&root).expect("whisper.cpp source pin");
        let patches = whisper_cpp_patch_files(&root, &source).expect("ordered Whisper patches");
        let patch_name = "0002-nagi-whisper-model-read-counts.patch";
        let patch_path = root.join(&source.patch_path).join(patch_name);
        let patch_position = patches
            .iter()
            .position(|path| path.file_name().and_then(OsStr::to_str) == Some(patch_name))
            .expect("numbered read-count patch is in the patch boundary");
        let noexceptions_position = patches
            .iter()
            .position(|path| {
                path.file_name().and_then(OsStr::to_str)
                    == Some("0001-nagi-whisper-target-noexceptions.patch")
            })
            .expect("base Nagi target patch is in the patch boundary");
        assert!(noexceptions_position < patch_position);

        let patch = fs::read_to_string(patch_path).expect("read-count patch contents");
        assert!(patch.contains("src/whisper.cpp"));
        assert!(patch.contains("read_model_data"));
        assert!(patch.contains("header_bytes == 0"));
        assert!(patch.contains("tests/test-whisper-buffer-loader.cpp"));
        assert!(patch.contains("partial_size < sizeof(tensor_header)"));
        assert!(patch.contains("fin->gcount()"));
    }

    #[test]
    fn fetches_exact_source_revision_and_rejects_modified_checkout() {
        let root = temporary_root("fetch");
        let source = root.join("fixture-source");
        fs::create_dir_all(&source).expect("fixture source");
        git(&source, &["init", "--quiet"]);
        git(&source, &["config", "user.email", "nagi@example.invalid"]);
        git(&source, &["config", "user.name", "Nagi Test"]);
        fs::write(source.join("CMakeLists.txt"), "project(whisper_fixture)\n")
            .expect("CMake fixture");
        fs::write(source.join("LICENSE"), "MIT License\n").expect("MIT fixture");
        git(&source, &["add", "."]);
        git(&source, &["commit", "--quiet", "-m", "fixture"]);
        let revision = String::from_utf8(
            Command::new("git")
                .arg("-C")
                .arg(&source)
                .args(["rev-parse", "HEAD"])
                .output()
                .expect("git rev-parse")
                .stdout,
        )
        .expect("UTF-8 revision")
        .trim()
        .to_owned();
        let spec = fixture_spec(source.display().to_string(), revision);

        // Exercise the production fetch/validation path with a local repository
        // without contacting the network or fetching model bytes.
        let checkout = root.join("third_party/whisper.cpp");
        fs::create_dir_all(checkout.parent().unwrap()).expect("checkout parent");
        let installed =
            fetch_upstream_checkout(&root, &checkout, &spec).expect("fetch exact fixture commit");
        assert_eq!(
            String::from_utf8(
                Command::new("git")
                    .arg("-C")
                    .arg(&installed)
                    .args(["rev-parse", "HEAD"])
                    .output()
                    .expect("installed revision")
                    .stdout
            )
            .unwrap()
            .trim(),
            spec.revision
        );
        fs::write(installed.join("untracked.txt"), "modified\n").expect("dirty marker");
        assert!(validate_checkout(&installed, &spec).is_err());
        fs::remove_dir_all(root).expect("remove fixture");
    }

    #[test]
    fn applies_ordered_patches_to_a_separate_generated_checkout_and_rejects_tampering() {
        let root = temporary_root("patch-apply");
        let source = root.join("fixture-source");
        let patches = root.join("third_party/whisper-cpp-patches");
        fs::create_dir_all(&source).expect("fixture source");
        fs::create_dir_all(&patches).expect("patch boundary");
        git(&source, &["init", "--quiet"]);
        git(&source, &["config", "user.email", "nagi@example.invalid"]);
        git(&source, &["config", "user.name", "Nagi Test"]);
        fs::write(source.join("CMakeLists.txt"), "project(whisper_fixture)\n")
            .expect("CMake fixture");
        fs::write(source.join("LICENSE"), "MIT License\n").expect("MIT fixture");
        fs::write(source.join("sample.txt"), "base\n").expect("source fixture");
        git(&source, &["add", "."]);
        git(&source, &["commit", "--quiet", "-m", "fixture"]);
        let revision = String::from_utf8(
            Command::new("git")
                .arg("-C")
                .arg(&source)
                .args(["rev-parse", "HEAD"])
                .output()
                .expect("git rev-parse")
                .stdout,
        )
        .expect("UTF-8 revision")
        .trim()
        .to_owned();
        let spec = fixture_spec(source.display().to_string(), revision);

        fs::write(
            patches.join("0002-second.patch"),
            "diff --git a/sample.txt b/sample.txt\n--- a/sample.txt\n+++ b/sample.txt\n@@ -1 +1 @@\n-first\n+second\n",
        )
        .expect("second patch");
        fs::write(
            patches.join("0001-first.patch"),
            "diff --git a/sample.txt b/sample.txt\n--- a/sample.txt\n+++ b/sample.txt\n@@ -1 +1 @@\n-base\n+first\n",
        )
        .expect("first patch");

        let ordered = whisper_cpp_patch_files(&root, &spec).expect("ordered patch list");
        assert_eq!(ordered[0].file_name().unwrap(), "0001-first.patch");
        assert_eq!(ordered[1].file_name().unwrap(), "0002-second.patch");

        let generated = ensure_patched_checkout(&root, &source, &spec)
            .expect("generate patched whisper.cpp checkout");
        validate_patched_checkout(&root, &generated, &spec)
            .expect("validate generated whisper.cpp checkout");
        assert_eq!(
            fs::read_to_string(source.join("sample.txt")).unwrap(),
            "base\n"
        );
        assert_eq!(
            fs::read_to_string(generated.join("sample.txt"))
                .unwrap()
                .replace("\r\n", "\n"),
            "second\n"
        );

        fs::write(generated.join("sample.txt"), "tampered\n").expect("tamper generated source");
        assert!(validate_patched_checkout(&root, &generated, &spec).is_err());
        fs::remove_dir_all(root).expect("remove fixture");
    }

    #[test]
    fn generated_checkout_requires_at_least_one_patch() {
        let root = temporary_root("empty-patch");
        let source = root.join("fixture-source");
        let patches = root.join("third_party/whisper-cpp-patches");
        fs::create_dir_all(&source).expect("fixture source");
        fs::create_dir_all(&patches).expect("patch boundary");
        git(&source, &["init", "--quiet"]);
        git(&source, &["config", "user.email", "nagi@example.invalid"]);
        git(&source, &["config", "user.name", "Nagi Test"]);
        fs::write(source.join("CMakeLists.txt"), "project(whisper_fixture)\n")
            .expect("CMake fixture");
        fs::write(source.join("LICENSE"), "MIT License\n").expect("MIT fixture");
        git(&source, &["add", "."]);
        git(&source, &["commit", "--quiet", "-m", "fixture"]);
        let revision = String::from_utf8(
            Command::new("git")
                .arg("-C")
                .arg(&source)
                .args(["rev-parse", "HEAD"])
                .output()
                .expect("git rev-parse")
                .stdout,
        )
        .expect("UTF-8 revision")
        .trim()
        .to_owned();
        let spec = fixture_spec(source.display().to_string(), revision);

        let error = match ensure_patched_checkout(&root, &source, &spec) {
            Ok(checkout) => {
                fs::remove_dir_all(root).expect("remove unexpected generated checkout");
                panic!("generated unpatched checkout unexpectedly succeeded: {checkout:?}");
            }
            Err(error) => error,
        };
        assert!(error.contains("no Nagi whisper.cpp patches"), "{error}");
        fs::remove_dir_all(root).expect("remove fixture");
    }

    #[test]
    fn whisper_cpp_patch_files_require_numeric_prefixes() {
        let root = temporary_root("patch-name");
        let spec = fixture_spec("fixture".to_owned(), "revision".to_owned());
        let patch_dir = root.join(&spec.patch_path);
        fs::create_dir_all(&patch_dir).expect("patch directory");
        fs::write(patch_dir.join("nagi-fix.patch"), "diff --git a/a b/a\n").expect("patch file");
        let error = whisper_cpp_patch_files(&root, &spec).expect_err("reject unordered patch");
        assert!(error.contains("numeric prefix"));
        fs::remove_dir_all(root).expect("remove fixture");
    }
}
