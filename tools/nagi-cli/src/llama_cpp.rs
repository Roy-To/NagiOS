use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::paths::external_command_path;

const SECTION: &str = "sources.llama_cpp";
const REPOSITORY: &str = "https://github.com/ggml-org/llama.cpp.git";
const REVISION: &str = "c85b92c69c955961621193cd51da194f3cbcedf3";
const LICENSE: &str = "MIT";
const PATCH_DIRECTORY: &str = "third_party/llama-cpp-patches";
const GENERATED_MARKER: &str = ".nagi-llama-cpp-checkout";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LlamaCppSourceSpec {
    repository: String,
    revision: String,
    source_hash: String,
    license: String,
    vendored_path: PathBuf,
    patch_path: PathBuf,
}

pub(crate) fn validate_llama_cpp_source_lock(root: &Path) -> Result<LlamaCppSourceSpec, String> {
    let lock = fs::read_to_string(root.join("third_party").join("sources.lock"))
        .map_err(|error| format!("cannot read third_party/sources.lock: {error}"))?;
    let required = |key: &str| {
        lock_value(&lock, SECTION, key)
            .ok_or_else(|| format!("third_party/sources.lock is missing llama.cpp field `{key}`"))
    };
    let spec = LlamaCppSourceSpec {
        repository: required("repository")?,
        revision: required("revision")?,
        source_hash: required("source_hash")?,
        license: required("license")?,
        vendored_path: PathBuf::from(required("vendored_path")?),
        patch_path: PathBuf::from(required("nagi_patch")?),
    };
    if spec.repository != REPOSITORY {
        return Err(format!(
            "third_party/sources.lock llama.cpp repository is `{}`, expected `{REPOSITORY}`",
            spec.repository
        ));
    }
    if spec.revision != REVISION {
        return Err(format!(
            "third_party/sources.lock llama.cpp revision is `{}`, expected `{REVISION}`",
            spec.revision
        ));
    }
    if spec.source_hash != format!("git:{REVISION}") {
        return Err(format!(
            "third_party/sources.lock llama.cpp source_hash is `{}`, expected `git:{REVISION}`",
            spec.source_hash
        ));
    }
    if spec.license != LICENSE {
        return Err(format!(
            "third_party/sources.lock llama.cpp license is `{}`, expected `{LICENSE}`",
            spec.license
        ));
    }
    if spec.vendored_path != Path::new("third_party/llama.cpp") {
        return Err(format!(
            "third_party/sources.lock llama.cpp vendored_path is `{}`, expected `third_party/llama.cpp`",
            spec.vendored_path.display()
        ));
    }
    if spec.patch_path != Path::new(PATCH_DIRECTORY) {
        return Err(format!(
            "third_party/sources.lock llama.cpp nagi_patch is `{}`, expected `{PATCH_DIRECTORY}`",
            spec.patch_path.display()
        ));
    }
    Ok(spec)
}

/// Fetches the exact upstream commit into an ignored source checkout. Existing
/// source is only read: a mismatched or dirty checkout is preserved and
/// rejected for explicit recovery.
pub(crate) fn ensure_llama_cpp_checkout(root: &Path) -> Result<PathBuf, String> {
    let spec = validate_llama_cpp_source_lock(root)?;
    let checkout = root.join(&spec.vendored_path);
    let source = if checkout.exists() {
        validate_checkout(&checkout, &spec)?
    } else {
        fetch_upstream_checkout(root, &checkout, &spec)?
    };
    if llama_cpp_patch_files(root, &spec)?.is_empty() {
        return Ok(source);
    }
    ensure_patched_checkout(root, &source, &spec)
}

fn fetch_upstream_checkout(
    root: &Path,
    checkout: &Path,
    spec: &LlamaCppSourceSpec,
) -> Result<PathBuf, String> {
    let parent = checkout
        .parent()
        .ok_or_else(|| format!("llama.cpp checkout has no parent: {}", checkout.display()))?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    let cache = root.join("out").join("cache");
    fs::create_dir_all(&cache)
        .map_err(|error| format!("cannot create {}: {error}", cache.display()))?;
    let temporary = cache.join(format!("llama-cpp-fetch-{}", std::process::id()));
    if temporary.exists() {
        return Err(format!(
            "refusing to reuse unexpected llama.cpp fetch directory {}",
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
                "cannot install llama.cpp checkout at {}; destination was created concurrently and was preserved",
                checkout.display()
            ));
        }
        fs::rename(&temporary, checkout).map_err(|error| {
            format!(
                "cannot install llama.cpp checkout {} at {}: {error}",
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
    spec: &LlamaCppSourceSpec,
) -> Result<PathBuf, String> {
    let checkout = root.join("out/cache/llama-cpp-nagi");
    if checkout.exists() {
        return validate_patched_checkout(root, &checkout, spec);
    }
    let cache = checkout.parent().ok_or_else(|| {
        format!(
            "llama.cpp generated checkout has no parent: {}",
            checkout.display()
        )
    })?;
    fs::create_dir_all(cache)
        .map_err(|error| format!("cannot create {}: {error}", cache.display()))?;
    let temporary = cache.join(format!("llama-cpp-nagi-{}", std::process::id()));
    if temporary.exists() {
        return Err(format!(
            "refusing to reuse unexpected llama.cpp patch directory {}",
            temporary.display()
        ));
    }

    let result = (|| {
        let source_path = external_command_path(source);
        let source = source_path.to_str().ok_or_else(|| {
            format!(
                "llama.cpp source path is not valid UTF-8: {}",
                source_path.display()
            )
        })?;
        let temporary_path = external_command_path(&temporary);
        let temporary_path = temporary_path.to_str().ok_or_else(|| {
            format!(
                "llama.cpp patch path is not valid UTF-8: {}",
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
                source,
                temporary_path,
            ],
        )?;
        git_output(&temporary, ["checkout", "--detach", spec.revision.as_str()])?;
        apply_llama_cpp_patches(root, &temporary, spec)?;
        write_generated_marker(root, &temporary, spec)?;
        validate_patched_checkout(root, &temporary, spec)?;
        if checkout.exists() {
            return Err(format!(
                "cannot install generated llama.cpp checkout at {}; destination was created concurrently and was preserved",
                checkout.display()
            ));
        }
        fs::rename(&temporary, &checkout).map_err(|error| {
            format!(
                "cannot install generated llama.cpp checkout {} at {}: {error}",
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

fn llama_cpp_patch_files(root: &Path, spec: &LlamaCppSourceSpec) -> Result<Vec<PathBuf>, String> {
    let patch_dir = root.join(&spec.patch_path);
    let mut patches = Vec::new();
    for entry in fs::read_dir(&patch_dir).map_err(|error| {
        format!(
            "cannot read Nagi llama.cpp patch boundary {}: {error}",
            patch_dir.display()
        )
    })? {
        let entry = entry.map_err(|error| {
            format!(
                "cannot read Nagi llama.cpp patch entry {}: {error}",
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
                "Nagi llama.cpp patch must be a regular file: {}",
                path.display()
            ));
        }
        let name = path.file_name().and_then(OsStr::to_str).ok_or_else(|| {
            format!(
                "Nagi llama.cpp patch path is not valid UTF-8: {}",
                path.display()
            )
        })?;
        if name
            .chars()
            .next()
            .is_none_or(|character| !character.is_ascii_digit())
        {
            return Err(format!(
                "Nagi llama.cpp patch must have a numeric prefix: {}",
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

fn apply_llama_cpp_patches(
    root: &Path,
    checkout: &Path,
    spec: &LlamaCppSourceSpec,
) -> Result<(), String> {
    for patch in llama_cpp_patch_files(root, spec)? {
        let patch_arg = external_command_path(&patch).into_os_string();
        let check_args = vec![
            OsString::from("apply"),
            OsString::from("--check"),
            OsString::from("--whitespace=nowarn"),
            patch_arg.clone(),
        ];
        git_output(checkout, check_args).map_err(|error| {
            format!(
                "cannot apply Nagi llama.cpp patch {} (check): {error}",
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
                "cannot apply Nagi llama.cpp patch {}: {error}",
                patch.display()
            )
        })?;
    }
    Ok(())
}

fn write_generated_marker(
    root: &Path,
    checkout: &Path,
    spec: &LlamaCppSourceSpec,
) -> Result<(), String> {
    let patch_fingerprint = llama_cpp_patch_fingerprint(root, spec)?;
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
    .map_err(|error| format!("cannot write generated llama.cpp marker {}: {error}", marker.display()))
}

fn validate_patched_checkout(
    root: &Path,
    checkout: &Path,
    spec: &LlamaCppSourceSpec,
) -> Result<PathBuf, String> {
    if !checkout.join("CMakeLists.txt").is_file() || !checkout.join("LICENSE").is_file() {
        return Err(format!(
            "generated llama.cpp checkout is missing build or license metadata: {}",
            checkout.display()
        ));
    }
    let head = git_output(checkout, ["rev-parse", "HEAD"])?;
    if head.trim() != spec.revision {
        return Err(format!(
            "generated llama.cpp checkout HEAD is `{}`, expected `{}`",
            head.trim(),
            spec.revision
        ));
    }
    let license = fs::read_to_string(checkout.join("LICENSE")).map_err(|error| {
        format!(
            "cannot read generated llama.cpp license {}: {error}",
            checkout.join("LICENSE").display()
        )
    })?;
    if !license.starts_with("MIT License") {
        return Err(format!(
            "generated llama.cpp LICENSE does not match the locked MIT declaration: {}",
            checkout.join("LICENSE").display()
        ));
    }
    let marker = checkout.join(GENERATED_MARKER);
    let marker_contents = fs::read_to_string(&marker).map_err(|error| {
        format!(
            "generated llama.cpp marker is missing {}; refusing to modify {}: {error}",
            marker.display(),
            checkout.display()
        )
    })?;
    let expected_prefix = format!(
        "format_version = 1\nrevision = {}\npatch_fingerprint = {}\n",
        spec.revision,
        llama_cpp_patch_fingerprint(root, spec)?
    );
    if !marker_contents.starts_with(&expected_prefix) {
        return Err(format!(
            "generated llama.cpp marker does not match the pinned revision or patch fingerprint: {}",
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
                "generated llama.cpp marker is missing checkout_fingerprint: {}",
                marker.display()
            )
        })?;
    if checkout_fingerprint != expected_checkout_fingerprint {
        return Err(format!(
            "generated llama.cpp checkout state does not match the patch result; refusing to modify {} ({})",
            checkout.display(),
            status.trim().replace('\n', "; ")
        ));
    }
    Ok(checkout.to_path_buf())
}

fn llama_cpp_patch_fingerprint(root: &Path, spec: &LlamaCppSourceSpec) -> Result<String, String> {
    let mut fingerprint = Fnv1a::new();
    for patch in llama_cpp_patch_files(root, spec)? {
        let name = patch.file_name().and_then(OsStr::to_str).ok_or_else(|| {
            format!(
                "Nagi llama.cpp patch path is not valid UTF-8: {}",
                patch.display()
            )
        })?;
        let contents = fs::read(&patch).map_err(|error| {
            format!(
                "cannot read Nagi llama.cpp patch {}: {error}",
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
                "cannot read generated llama.cpp change {}: {error}",
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

fn validate_checkout(checkout: &Path, spec: &LlamaCppSourceSpec) -> Result<PathBuf, String> {
    if !checkout.join("CMakeLists.txt").is_file() || !checkout.join("LICENSE").is_file() {
        return Err(format!(
            "pinned llama.cpp checkout is missing build or license metadata: {}",
            checkout.display()
        ));
    }
    let head = git_output(checkout, ["rev-parse", "HEAD"])?;
    if head.trim() != spec.revision {
        return Err(format!(
            "llama.cpp checkout HEAD is `{}`, expected `{}`",
            head.trim(),
            spec.revision
        ));
    }
    let status = git_output(checkout, ["status", "--porcelain", "--untracked-files=all"])?;
    if !status.trim().is_empty() {
        return Err(format!(
            "pinned llama.cpp checkout is dirty; refusing to modify {} ({})",
            checkout.display(),
            status.trim().replace('\n', "; ")
        ));
    }
    let license = fs::read_to_string(checkout.join("LICENSE")).map_err(|error| {
        format!(
            "cannot read llama.cpp license {}: {error}",
            checkout.join("LICENSE").display()
        )
    })?;
    if !license.starts_with("MIT License") {
        return Err(format!(
            "pinned llama.cpp LICENSE does not match the locked MIT declaration: {}",
            checkout.join("LICENSE").display()
        ));
    }
    Ok(checkout.to_path_buf())
}

fn git_output<I, S>(directory: &Path, args: I) -> Result<String, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let output = Command::new("git")
        .arg("-C")
        .arg(directory)
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

pub(crate) fn lock_value(lock: &str, section: &str, key: &str) -> Option<String> {
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
        ensure_patched_checkout, llama_cpp_patch_files, lock_value, validate_llama_cpp_source_lock,
        validate_patched_checkout, LlamaCppSourceSpec,
    };
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    fn temporary_root(name: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("nagi-llama-cpp-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("third_party/llama-cpp-patches")).expect("temp root");
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

    fn fixture_spec(revision: String) -> LlamaCppSourceSpec {
        LlamaCppSourceSpec {
            repository: "fixture".to_owned(),
            revision,
            source_hash: "fixture".to_owned(),
            license: "MIT".to_owned(),
            vendored_path: PathBuf::from("third_party/llama.cpp"),
            patch_path: PathBuf::from("third_party/llama-cpp-patches"),
        }
    }

    #[test]
    fn llama_cpp_source_lock_pins_revision_license_and_patch_boundary() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let spec = validate_llama_cpp_source_lock(&root).expect("pinned source lock");
        assert_eq!(spec.repository, "https://github.com/ggml-org/llama.cpp.git");
        assert_eq!(spec.revision, "c85b92c69c955961621193cd51da194f3cbcedf3");
        assert_eq!(spec.source_hash, format!("git:{}", spec.revision));
        assert_eq!(spec.license, "MIT");
        assert_eq!(spec.vendored_path, Path::new("third_party/llama.cpp"));
        assert_eq!(spec.patch_path, Path::new("third_party/llama-cpp-patches"));
    }

    fn assert_model_artifact_lock_matches_manifest_fixture(section: &str, fixture: &str) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let lock =
            fs::read_to_string(root.join("third_party/models.lock")).expect("model artifact pins");
        let manifest_bytes = fs::read(root.join(fixture)).expect("model manifest fixture");
        let manifest = nagi_model_manager::ModelManifest::parse_json(&manifest_bytes)
            .expect("pinned model manifest fixture");
        let locked = |key: &str| {
            lock_value(&lock, section, key)
                .unwrap_or_else(|| panic!("model section `{section}` is missing `{key}`"))
        };
        let source = manifest.source.as_ref().expect("model source metadata");
        let integrity = manifest.artifact.integrity.as_ref().expect("model digest");
        let artifact_id = match &manifest.artifact.reference {
            nagi_model_manager::ArtifactReference::ModelStore { artifact_id } => {
                artifact_id.as_str()
            }
        };

        assert_eq!(locked("model_id"), manifest.model_id.as_str());
        assert_eq!(locked("repository"), source.uri);
        assert_eq!(locked("revision"), source.revision);
        assert_eq!(locked("file_name"), source.file_name);
        assert_eq!(locked("format"), manifest.artifact.format.as_str());
        assert_eq!(
            locked("size_bytes").parse::<u64>().expect("locked size"),
            manifest.artifact.size_bytes.expect("model artifact size")
        );
        assert_eq!(locked("sha256"), integrity.digest);
        assert_eq!(locked("license"), manifest.license.identifier);
        assert_eq!(
            locked("license_reference"),
            manifest
                .license
                .terms_reference
                .as_deref()
                .expect("model license terms")
        );
        assert_eq!(locked("notice_id"), manifest.license.notices[0].notice_id);
        assert_eq!(
            locked("notice_reference"),
            manifest.license.notices[0].reference
        );
        assert_eq!(
            locked("acknowledgement_required"),
            manifest.license.acknowledgement_required.to_string()
        );
        assert_eq!(locked("artifact_id"), artifact_id);
        assert_eq!(locked("storage"), "model_store");
    }

    #[test]
    fn model_artifact_locks_match_manifest_fixtures() {
        for (section, fixture) in [
            (
                "models.granite_4_2_3b",
                "user/nagi-model-manager/tests/fixtures/granite-4.2-3b.json",
            ),
            (
                "models.qwen3_4b",
                "user/nagi-model-manager/tests/fixtures/qwen3-4b.json",
            ),
            (
                "models.gemma_3_1b",
                "user/nagi-model-manager/tests/fixtures/gemma-3-1b.json",
            ),
        ] {
            assert_model_artifact_lock_matches_manifest_fixture(section, fixture);
        }
    }

    #[test]
    fn llama_cpp_nagi_boundary_patch_avoids_rtti_and_matches_path_capacity() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let patch = fs::read_to_string(
            root.join("third_party/llama-cpp-patches/0003-nagi-model-boundaries.patch"),
        )
        .expect("Nagi C++ boundary patch");
        let storage = fs::read_to_string(root.join("user/libnagi/src/storage.rs"))
            .expect("Nagi storage path contract");

        assert!(patch.contains("model_ptr ? model_ptr->as_model_base() : nullptr"));
        assert!(patch.contains("return 257;"));
        assert!(patch.contains("Nagi paths allow 256 bytes"));
        assert!(storage.contains("MAX_PATH_LENGTH: usize = 256"));
    }

    #[test]
    fn llama_cpp_nagi_chat_template_patch_returns_explicit_unknown_status() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let patch = fs::read_to_string(
            root.join("third_party/llama-cpp-patches/0004-nagi-chat-template-status.patch"),
        )
        .expect("Nagi chat template status patch");

        assert!(patch.contains("#ifdef __NAGI__"));
        assert!(patch.contains("LLM_CHAT_TEMPLATES.find(name)"));
        assert!(patch.contains("LLM_CHAT_TEMPLATES.find(tmpl)"));
        assert!(patch.contains("LLM_CHAT_TEMPLATE_UNKNOWN"));
        assert!(patch.contains("#else\n     return LLM_CHAT_TEMPLATES.at(name);"));
        assert!(patch.contains("catch (const std::out_of_range &)"));
    }

    #[test]
    fn llama_cpp_nagi_grammar_patch_propagates_parse_and_runtime_failures() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let patch = fs::read_to_string(
            root.join("third_party/llama-cpp-patches/0005-nagi-grammar-status.patch"),
        )
        .expect("Nagi grammar status patch");

        assert!(patch.contains("parse_uint64"));
        assert!(patch.contains("maximum number of repetitions is less than the minimum"));
        assert!(patch.contains("grammar.failed = true"));
        assert!(patch.contains("LLAMA_TOKEN_NULL"));
        assert!(patch.contains("-INFINITY"));
        assert!(!patch.contains("Nagi does not support regex-triggered lazy grammars"));
    }

    #[test]
    fn llama_cpp_sampler_null_patch_documents_and_guards_direct_consumers() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let patch = fs::read_to_string(
            root.join("third_party/llama-cpp-patches/0006-nagi-sampler-null-consumers.patch"),
        )
        .expect("Nagi sampler null-consumer patch");

        assert!(
            patch.contains("Returns the sampled token, or LLAMA_TOKEN_NULL if sampling failed.")
        );
        for path in [
            "examples/simple-chat/simple-chat.cpp",
            "examples/simple/simple.cpp",
            "examples/batched/batched.cpp",
            "examples/passkey/passkey.cpp",
            "examples/llama.swiftui/llama.cpp.swift/LibLlama.swift",
            "examples/batched.swift/Sources/main.swift",
        ] {
            let header = format!("diff --git a/{path} b/{path}\n");
            let consumer_patch = patch
                .split_once(&header)
                .unwrap_or_else(|| panic!("missing {path} hunk"))
                .1
                .split("\ndiff --git ")
                .next()
                .expect("consumer patch hunk");
            assert!(
                consumer_patch.contains("LLAMA_TOKEN_NULL"),
                "{path} does not guard the failure sentinel"
            );
            assert!(
                consumer_patch.contains("sampler failed to produce a token"),
                "{path} does not report the sampling failure"
            );
        }
    }

    #[test]
    fn llama_cpp_hybrid_restore_patch_covers_recurrent_suffix_failure() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let patch =
            fs::read_to_string(root.join(
                "third_party/llama-cpp-patches/0007-nagi-hybrid-state-restore-rollback.patch",
            ))
            .expect("Nagi hybrid state-restore rollback patch");

        assert!(patch.contains("llama_model_is_hybrid(model)"));
        assert!(patch.contains("truncated recurrent suffix"));
        assert!(patch.contains("state.size() - 1"));
        assert!(patch.contains("empty_state_size"));
        assert!(patch.contains("hybrid sequence retained state after failed restore"));
    }

    #[test]
    fn llama_cpp_tensor_weight_patch_returns_status_before_model_creation() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let patch = fs::read_to_string(
            root.join("third_party/llama-cpp-patches/0008-nagi-tensor-weight-status.patch"),
        )
        .expect("Nagi tensor weight status patch");
        let loader = patch
            .split_once("diff --git a/src/llama-model-loader.h")
            .expect("loader header patch")
            .1
            .split_once("diff --git a/src/llama.cpp")
            .expect("model-load status patch")
            .0;
        let model_load = patch
            .split_once("diff --git a/src/llama.cpp")
            .expect("model-load status patch")
            .1
            .split("\ndiff --git ")
            .next()
            .expect("model-load patch section");

        assert!(loader.contains("tensor_idx < 0"));
        assert!(loader.contains("data_offset > file_size"));
        assert!(loader.contains("tensor_offset > file_size - data_offset"));
        assert!(loader.contains("tensor_size > file_size - tensor_data_offset"));
        assert_eq!(loader.matches("if (!add_tensor_weight(").count(), 3);
        assert!(loader.contains("tensor_weights_valid = false"));
        assert!(model_load.contains("if (!ml.tensor_weights_valid)"));
        assert!(
            model_load.find("if (!ml.tensor_weights_valid)") < model_load.find("ml.print_info()")
        );
    }

    #[test]
    fn llama_cpp_split_paths_reject_truncation_and_fail_loader_initialization() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let patch = fs::read_to_string(
            root.join("third_party/llama-cpp-patches/0009-nagi-llama-split-path-status.patch"),
        )
        .expect("Nagi llama split path status patch");
        let split_api = patch
            .split_once("diff --git a/src/llama.cpp")
            .expect("split API patch")
            .1
            .split("\ndiff --git ")
            .next()
            .expect("split API patch section");
        let loader = patch
            .split_once("diff --git a/src/llama-model-loader.cpp")
            .expect("model loader patch")
            .1
            .split("\ndiff --git ")
            .next()
            .expect("model loader patch section");
        let model_load = split_api;
        let test = patch
            .split_once("diff --git a/tests/test-model-loader-bounds.cpp")
            .expect("split path test patch")
            .1
            .split("\ndiff --git ")
            .next()
            .expect("split path test patch section");

        assert!(split_api.contains("split_no >= 0 && split_count > 0 && split_no < split_count"));
        assert!(split_api.contains("written >= maxlen"));
        assert!(split_api.contains("size_prefix >= maxlen"));
        assert!(split_api.contains("memcpy(split_prefix, split_path, size_prefix)"));
        assert!(loader.contains("candidate_paths.size() != static_cast<size_t>(n_split)"));
        assert!(loader.contains("paths.swap(candidate_paths)"));
        assert!(loader.contains("if (!make_split_paths(fname, idx, n_split, splits))"));
        assert!(model_load.contains("!ml.tensor_weights_valid || !ml.split_paths_valid"));
        let status_check = model_load
            .find("!ml.tensor_weights_valid || !ml.split_paths_valid")
            .expect("loader status check");
        let failure_return = model_load
            .find("return {-1, nullptr};")
            .expect("loader failure return");
        assert!(status_check < failure_return);
        assert!(test.contains("exact-fit split path"));
        assert!(test.contains("short split prefix buffer"));
        assert!(test.contains("malformed split suffix"));
        assert!(test.contains("truncated generated shard path"));
        assert!(patch.contains("target_link_libraries(test-model-loader-bounds PRIVATE llama)"));
    }

    #[test]
    fn llama_cpp_patches_apply_in_numeric_order_and_validate_the_generated_tree() {
        let root = fs::canonicalize(temporary_root("patch-apply")).expect("canonical root");
        let source = root.join("source");
        fs::create_dir_all(&source).expect("source directory");
        git(&source, &["init", "--quiet"]);
        git(&source, &["config", "user.email", "nagi@example.invalid"]);
        git(&source, &["config", "user.name", "Nagi Test"]);
        fs::write(source.join("sample.txt"), "base\n").expect("source file");
        fs::write(source.join("CMakeLists.txt"), "project(fixture)\n").expect("cmake file");
        fs::write(source.join("LICENSE"), "MIT License\n").expect("license file");
        git(&source, &["add", "."]);
        git(&source, &["commit", "--quiet", "-m", "base"]);
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
        let spec = fixture_spec(revision);
        let patches = root.join(&spec.patch_path);
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
        let ordered = llama_cpp_patch_files(&root, &spec).expect("patch list");
        assert_eq!(ordered[0].file_name().unwrap(), "0001-first.patch");
        assert_eq!(ordered[1].file_name().unwrap(), "0002-second.patch");

        let checkout =
            ensure_patched_checkout(&root, &source, &spec).expect("generate patched checkout");
        validate_patched_checkout(&root, &checkout, &spec).expect("validate generated source");
        assert_eq!(
            fs::read_to_string(source.join("sample.txt")).unwrap(),
            "base\n"
        );
        assert_eq!(
            fs::read_to_string(checkout.join("sample.txt"))
                .unwrap()
                .replace("\r\n", "\n"),
            "second\n"
        );

        fs::write(checkout.join("sample.txt"), "tampered\n").expect("tamper generated source");
        assert!(validate_patched_checkout(&root, &checkout, &spec).is_err());
        fs::remove_dir_all(root).expect("remove fixture");
    }

    #[test]
    fn llama_cpp_patch_files_require_numeric_prefixes() {
        let root = temporary_root("patch-name");
        let spec = fixture_spec("revision".to_owned());
        fs::write(
            root.join(&spec.patch_path).join("nagi-fix.patch"),
            "diff --git a/a b/a\n",
        )
        .expect("patch file");
        let error = llama_cpp_patch_files(&root, &spec).expect_err("reject unordered patch");
        assert!(error.contains("numeric prefix"));
        fs::remove_dir_all(root).expect("remove fixture");
    }
}
