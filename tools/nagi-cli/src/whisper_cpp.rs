use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::paths::external_command_path;

const SOURCE_SECTION: &str = "sources.whisper_cpp";
const SOURCE_REPOSITORY: &str = "https://github.com/ggml-org/whisper.cpp.git";
const SOURCE_REVISION: &str = "927cfce34f31707e17f2bff35c349632fb9e2c3a";
const SOURCE_LICENSE: &str = "MIT";
const SOURCE_PATH: &str = "third_party/whisper.cpp";
const SOURCE_PATCH: &str = "none";

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
    patch: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct WhisperModelArtifactSpec {
    component: String,
    model_id: String,
    repository: String,
    revision: String,
    file_name: String,
    format: String,
    size_bytes: u64,
    sha256: String,
    license: String,
    license_reference: String,
    artifact_id: String,
    storage: String,
}

pub(crate) fn ensure_whisper_cpp_checkout(root: &Path) -> Result<PathBuf, String> {
    let spec = validate_whisper_cpp_source_lock(root)?;
    validate_whisper_model_artifact_lock(root)?;
    let checkout = root.join(&spec.vendored_path);
    if checkout.exists() {
        return validate_checkout(&checkout, &spec);
    }
    fetch_upstream_checkout(root, &checkout, &spec)
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
        patch: required("nagi_patch")?,
    };
    if spec.repository != SOURCE_REPOSITORY
        || spec.revision != SOURCE_REVISION
        || spec.source_hash != format!("git:{SOURCE_REVISION}")
        || spec.license != SOURCE_LICENSE
        || spec.vendored_path != Path::new(SOURCE_PATH)
        || spec.patch != SOURCE_PATCH
    {
        return Err(format!(
            "third_party/sources.lock whisper.cpp entry does not match the reviewed pin `{SOURCE_REVISION}`"
        ));
    }
    Ok(spec)
}

fn validate_whisper_model_artifact_lock(root: &Path) -> Result<WhisperModelArtifactSpec, String> {
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
        fetch_upstream_checkout, validate_checkout, validate_whisper_cpp_source_lock,
        validate_whisper_model_artifact_lock, WhisperCppSourceSpec,
    };
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
            patch: "none".to_owned(),
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
        assert_eq!(source.patch, "none");

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
}
