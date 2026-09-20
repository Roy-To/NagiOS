use std::ffi::OsStr;
use std::fs;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output};

use crate::paths::external_command_path;

pub(crate) struct RegistrySourceSpec {
    pub(crate) section: &'static str,
    pub(crate) component: &'static str,
    pub(crate) package: &'static str,
    pub(crate) version: &'static str,
    pub(crate) repository: &'static str,
    pub(crate) registry_archive: &'static str,
    pub(crate) source_hash: &'static str,
    pub(crate) license: &'static str,
    pub(crate) vendored_path: &'static str,
    pub(crate) patch_path: &'static str,
}

pub(crate) fn ensure_registry_checkout(
    root: &Path,
    spec: &RegistrySourceSpec,
) -> Result<PathBuf, String> {
    validate_source_lock(root, spec)?;
    let checkout = root.join(spec.vendored_path);
    if checkout.exists() {
        return validate_checkout(root, spec);
    }

    let source = find_registry_source(spec)?;
    let patches = patch_files(root, spec)?;
    let parent = checkout.parent().ok_or_else(|| {
        format!(
            "{} checkout has no parent: {}",
            spec.component,
            checkout.display()
        )
    })?;
    fs::create_dir_all(parent)
        .map_err(|error| format!("cannot create {}: {error}", parent.display()))?;
    let temporary = parent.join(format!(
        ".{}-bootstrap-{}-{}",
        spec.component,
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| format!(
                "cannot read clock for {} bootstrap: {error}",
                spec.component
            ))?
            .as_nanos()
    ));
    if temporary.exists() {
        return Err(format!(
            "refusing to reuse unexpected {} bootstrap directory {}",
            spec.component,
            temporary.display()
        ));
    }

    copy_directory(&source, &temporary, spec)?;
    let result = (|| {
        for patch in &patches {
            apply_patch(root, &temporary, patch, spec)?;
        }
        write_marker(root, &temporary, spec)
    })();
    match result {
        Ok(()) => {
            if checkout.exists() {
                let _ = fs::remove_dir_all(&temporary);
                return Err(format!(
                    "cannot install generated {} checkout at {}; destination was created concurrently and was preserved",
                    spec.component,
                    checkout.display()
                ));
            }
            fs::rename(&temporary, &checkout).map_err(|error| {
                let _ = fs::remove_dir_all(&temporary);
                format!(
                    "cannot install generated {} checkout {} at {}: {error}",
                    spec.component,
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

pub(crate) fn validate_source_lock(root: &Path, spec: &RegistrySourceSpec) -> Result<(), String> {
    let lock = fs::read_to_string(root.join("third_party").join("sources.lock"))
        .map_err(|error| format!("cannot read third_party/sources.lock: {error}"))?;
    let required = |key: &str| {
        lock_value(&lock, spec.section, key).ok_or_else(|| {
            format!(
                "third_party/sources.lock is missing {} field `{key}`",
                spec.component
            )
        })
    };
    let expected = [
        ("component", spec.component),
        ("version", spec.version),
        ("repository", spec.repository),
        ("registry_archive", spec.registry_archive),
        ("source_hash", spec.source_hash),
        ("license", spec.license),
        ("vendored_path", spec.vendored_path),
        ("nagi_patch", spec.patch_path),
    ];
    for (key, expected) in expected {
        let actual = required(key)?;
        if actual != expected {
            return Err(format!(
                "third_party/sources.lock {} field `{key}` is `{actual}`, expected `{expected}`",
                spec.component
            ));
        }
    }
    relative_path(spec.vendored_path, "vendored_path", spec)?;
    relative_path(spec.patch_path, "nagi_patch", spec)?;
    Ok(())
}

fn validate_checkout(root: &Path, spec: &RegistrySourceSpec) -> Result<PathBuf, String> {
    let checkout = root.join(spec.vendored_path);
    if !checkout.is_dir() {
        return Err(format!(
            "{} checkout is not a directory: {}",
            spec.component,
            checkout.display()
        ));
    }
    let manifest = fs::read_to_string(checkout.join("Cargo.toml"))
        .map_err(|error| format!("cannot read {} manifest: {error}", spec.component))?;
    if !manifest
        .lines()
        .any(|line| line.trim() == format!("name = \"{}\"", spec.package))
        || !manifest
            .lines()
            .any(|line| line.trim() == format!("version = \"{}\"", spec.version))
    {
        return Err(format!(
            "generated {} checkout does not contain package {} version {}",
            spec.component, spec.package, spec.version
        ));
    }
    let marker = checkout.join(marker_name(spec));
    let contents = fs::read_to_string(&marker).map_err(|error| {
        format!(
            "generated {} checkout marker is missing {}; refusing to modify {}: {error}",
            spec.component,
            marker.display(),
            checkout.display()
        )
    })?;
    let expected_patch = patch_fingerprint(root, spec)?;
    if marker_value(&contents, "source_hash") != Some(spec.source_hash)
        || marker_value(&contents, "patch_fingerprint") != Some(expected_patch.as_str())
    {
        return Err(format!(
            "generated {} checkout marker does not match pinned source or patch fingerprint: {}",
            spec.component,
            marker.display()
        ));
    }
    let actual = directory_fingerprint(&checkout, spec)?;
    if marker_value(&contents, "checkout_fingerprint") != Some(actual.as_str()) {
        return Err(format!(
            "generated {} checkout state does not match its marker; refusing to modify {}",
            spec.component,
            checkout.display()
        ));
    }
    Ok(checkout)
}

fn find_registry_source(spec: &RegistrySourceSpec) -> Result<PathBuf, String> {
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("USERPROFILE").map(|value| PathBuf::from(value).join(".cargo"))
        })
        .or_else(|| std::env::var_os("HOME").map(|value| PathBuf::from(value).join(".cargo")))
        .ok_or_else(|| {
            format!(
                "cannot locate Cargo home for pinned {} archive",
                spec.component
            )
        })?;
    let source_root = cargo_home.join("registry").join("src");
    let expected_hash = spec.source_hash.strip_prefix("sha256:").ok_or_else(|| {
        format!(
            "invalid {} source hash `{}`",
            spec.component, spec.source_hash
        )
    })?;
    for index in fs::read_dir(&source_root).map_err(|error| {
        format!(
            "cannot read Cargo registry source cache {}: {error}",
            source_root.display()
        )
    })? {
        let index = index
            .map_err(|error| format!("cannot inspect Cargo registry source cache: {error}"))?
            .path();
        let candidate = index.join(format!("{}-{}", spec.package, spec.version));
        if !candidate.is_dir() {
            continue;
        }
        let checksum = candidate.join(".cargo-checksum.json");
        match fs::read_to_string(&checksum) {
            Ok(checksum) if checksum.contains(expected_hash) => return Ok(candidate),
            Ok(_) => continue,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if registry_source_manifest_matches(&candidate, spec)
                    && candidate.join(".cargo-ok").is_file()
                {
                    return Ok(candidate);
                }
            }
            Err(error) => {
                return Err(format!(
                    "cannot read pinned {} checksum {}: {error}",
                    spec.component,
                    checksum.display()
                ));
            }
        }
    }
    Err(format!(
        "pinned {} {} source is absent from Cargo's registry cache; fetch the locked dependencies before bootstrapping",
        spec.package, spec.version
    ))
}

fn registry_source_manifest_matches(candidate: &Path, spec: &RegistrySourceSpec) -> bool {
    let Ok(manifest) = fs::read_to_string(candidate.join("Cargo.toml")) else {
        return false;
    };
    manifest
        .lines()
        .any(|line| line.trim() == format!("name = \"{}\"", spec.package))
        && manifest
            .lines()
            .any(|line| line.trim() == format!("version = \"{}\"", spec.version))
}

fn patch_files(root: &Path, spec: &RegistrySourceSpec) -> Result<Vec<PathBuf>, String> {
    let directory = root.join(spec.patch_path);
    if !directory.is_dir() {
        return Err(format!(
            "{} patch directory is missing: {}",
            spec.component,
            directory.display()
        ));
    }
    let mut patches = Vec::new();
    for entry in fs::read_dir(&directory)
        .map_err(|error| format!("cannot read {} patch directory: {error}", spec.component))?
    {
        let path = entry
            .map_err(|error| format!("cannot inspect {} patch entry: {error}", spec.component))?
            .path();
        if path.extension() != Some(OsStr::new("patch")) {
            continue;
        }
        let name = path.file_name().and_then(OsStr::to_str).ok_or_else(|| {
            format!(
                "{} patch has a non-UTF-8 filename: {}",
                spec.component,
                path.display()
            )
        })?;
        if name
            .split_once('-')
            .and_then(|(number, _)| number.parse::<u32>().ok())
            .is_none()
        {
            return Err(format!(
                "{} patch `{name}` must start with a numeric prefix",
                spec.component
            ));
        }
        let bytes = fs::read(&path).map_err(|error| {
            format!(
                "cannot read {} patch {}: {error}",
                spec.component,
                path.display()
            )
        })?;
        if bytes.contains(&b'\r') {
            return Err(format!(
                "{} patch `{name}` must use LF line endings; refusing CRLF conversion",
                spec.component
            ));
        }
        patches.push(path);
    }
    patches.sort();
    Ok(patches)
}

fn apply_patch(
    root: &Path,
    checkout: &Path,
    patch: &Path,
    spec: &RegistrySourceSpec,
) -> Result<(), String> {
    let relative_checkout = checkout.strip_prefix(root).map_err(|error| {
        format!(
            "{} checkout {} is outside repository root {}: {error}",
            spec.component,
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
            "{} checkout must be a normal path under repository root: {}",
            spec.component,
            checkout.display()
        ));
    }
    let git_directory = relative_checkout.to_string_lossy().replace('\\', "/");
    for check_only in [true, false] {
        let mut command = Command::new("git");
        command
            .args(["-c", "core.autocrlf=false", "-c", "core.eol=lf", "apply"])
            .args(check_only.then_some("--check"))
            .args(["--unidiff-zero", "--unsafe-paths"])
            .arg(format!("--directory={git_directory}"))
            .arg(external_command_path(patch))
            .current_dir(root);
        let output = command.output().map_err(|error| {
            format!(
                "cannot run git for {} patch {}: {error}",
                spec.component,
                patch.display()
            )
        })?;
        if !output.status.success() {
            return Err(format!(
                "{} patch {} failed: {}",
                spec.component,
                patch.display(),
                command_output(&output)
            ));
        }
    }
    Ok(())
}

fn write_marker(root: &Path, checkout: &Path, spec: &RegistrySourceSpec) -> Result<(), String> {
    let patch_fingerprint = patch_fingerprint(root, spec)?;
    let checkout_fingerprint = directory_fingerprint(checkout, spec)?;
    fs::write(
        checkout.join(marker_name(spec)),
        format!(
            "format_version = 1\nsource_hash = {}\npatch_fingerprint = {}\ncheckout_fingerprint = {}\n",
            spec.source_hash, patch_fingerprint, checkout_fingerprint
        ),
    )
    .map_err(|error| format!("cannot write generated {} marker: {error}", spec.component))
}

fn patch_fingerprint(root: &Path, spec: &RegistrySourceSpec) -> Result<String, String> {
    let mut fingerprint = Fnv1a::new();
    for patch in patch_files(root, spec)? {
        fingerprint.update(
            patch
                .file_name()
                .and_then(OsStr::to_str)
                .ok_or_else(|| {
                    format!("invalid {} patch name: {}", spec.component, patch.display())
                })?
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

fn directory_fingerprint(directory: &Path, spec: &RegistrySourceSpec) -> Result<String, String> {
    let mut files = Vec::new();
    collect_files(directory, directory, &mut files, spec)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut fingerprint = Fnv1a::new();
    for (relative, path) in files {
        fingerprint.update(relative.to_string_lossy().as_bytes());
        fingerprint.update(&[0]);
        fingerprint.update(
            &fs::read(path).map_err(|error| {
                format!("cannot read generated {} file: {error}", spec.component)
            })?,
        );
        fingerprint.update(&[0]);
    }
    Ok(fingerprint.finish())
}

fn collect_files(
    root: &Path,
    current: &Path,
    files: &mut Vec<(PathBuf, PathBuf)>,
    spec: &RegistrySourceSpec,
) -> Result<(), String> {
    for entry in fs::read_dir(current)
        .map_err(|error| format!("cannot enumerate {}: {error}", current.display()))?
    {
        let entry = entry.map_err(|error| {
            format!("cannot inspect generated {} entry: {error}", spec.component)
        })?;
        let path = entry.path();
        let marker = marker_name(spec);
        if path.file_name() == Some(OsStr::new(&marker)) {
            continue;
        }
        if entry
            .file_type()
            .map_err(|error| format!("cannot inspect {}: {error}", path.display()))?
            .is_dir()
        {
            collect_files(root, &path, files, spec)?;
        } else {
            files.push((path.strip_prefix(root).unwrap_or(&path).to_path_buf(), path));
        }
    }
    Ok(())
}

fn copy_directory(
    source: &Path,
    destination: &Path,
    spec: &RegistrySourceSpec,
) -> Result<(), String> {
    fs::create_dir_all(destination)
        .map_err(|error| format!("cannot create {}: {error}", destination.display()))?;
    for entry in fs::read_dir(source)
        .map_err(|error| format!("cannot enumerate {}: {error}", source.display()))?
    {
        let entry =
            entry.map_err(|error| format!("cannot inspect {} entry: {error}", spec.component))?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry
            .file_type()
            .map_err(|error| format!("cannot inspect {}: {error}", source_path.display()))?
            .is_dir()
        {
            copy_directory(&source_path, &destination_path, spec)?;
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

fn relative_path(value: &str, field: &str, spec: &RegistrySourceSpec) -> Result<(), String> {
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "{} {field} must be a relative normal path",
            spec.component
        ));
    }
    Ok(())
}

fn marker_name(spec: &RegistrySourceSpec) -> String {
    format!(".nagi-{}-checkout", spec.component)
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
