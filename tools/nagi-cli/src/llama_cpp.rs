use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const SECTION: &str = "sources.llama_cpp";
const REPOSITORY: &str = "https://github.com/ggml-org/llama.cpp.git";
const REVISION: &str = "c85b92c69c955961621193cd51da194f3cbcedf3";
const LICENSE: &str = "MIT";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LlamaCppSourceSpec {
    repository: String,
    revision: String,
    source_hash: String,
    license: String,
    vendored_path: PathBuf,
    nagi_patch: String,
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
        nagi_patch: required("nagi_patch")?,
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
    if spec.nagi_patch != "none" {
        return Err(format!(
            "third_party/sources.lock llama.cpp nagi_patch is `{}`, expected `none` until a reproducible adapter patch is added",
            spec.nagi_patch
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
    if checkout.exists() {
        return validate_checkout(&checkout, &spec);
    }

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
        validate_checkout(&temporary, &spec)?;
        if checkout.exists() {
            return Err(format!(
                "cannot install llama.cpp checkout at {}; destination was created concurrently and was preserved",
                checkout.display()
            ));
        }
        fs::rename(&temporary, &checkout).map_err(|error| {
            format!(
                "cannot install llama.cpp checkout {} at {}: {error}",
                temporary.display(),
                checkout.display()
            )
        })?;
        validate_checkout(&checkout, &spec)
    })();
    if result.is_err() && temporary.exists() {
        let _ = fs::remove_dir_all(&temporary);
    }
    result
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
    use super::validate_llama_cpp_source_lock;
    use std::path::Path;

    #[test]
    fn llama_cpp_source_lock_pins_revision_license_and_patch_boundary() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let spec = validate_llama_cpp_source_lock(&root).expect("pinned source lock");
        assert_eq!(spec.repository, "https://github.com/ggml-org/llama.cpp.git");
        assert_eq!(spec.revision, "c85b92c69c955961621193cd51da194f3cbcedf3");
        assert_eq!(spec.source_hash, format!("git:{}", spec.revision));
        assert_eq!(spec.license, "MIT");
        assert_eq!(spec.vendored_path, Path::new("third_party/llama.cpp"));
        assert_eq!(spec.nagi_patch, "none");
    }
}
