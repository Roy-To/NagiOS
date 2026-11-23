use std::ffi::OsStr;
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output, Stdio};

use sha2::{Digest, Sha256};

use crate::paths::external_command_path;

pub(crate) struct RegistrySourceSpec<'a> {
    pub(crate) section: &'a str,
    pub(crate) component: &'a str,
    pub(crate) package: &'a str,
    pub(crate) version: &'a str,
    pub(crate) repository: &'a str,
    pub(crate) registry_archive: &'a str,
    pub(crate) source_hash: &'a str,
    pub(crate) license: &'a str,
    pub(crate) vendored_path: &'a str,
    pub(crate) patch_path: &'a str,
}

pub(crate) fn ensure_registry_checkout(
    root: &Path,
    spec: &RegistrySourceSpec,
) -> Result<PathBuf, String> {
    ensure_registry_checkout_using(root, spec, || find_registry_source(spec))
}

fn ensure_registry_checkout_using(
    root: &Path,
    spec: &RegistrySourceSpec,
    source: impl FnOnce() -> Result<Vec<u8>, String>,
) -> Result<PathBuf, String> {
    validate_source_lock(root, spec)?;
    let checkout = root.join(spec.vendored_path);
    if checkout.exists() {
        return validate_checkout(root, spec);
    }

    let source = source()?;
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

    let result = (|| {
        extract_verified_archive(&source, &temporary, spec)?;
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
    if marker_value(&contents, "format_version") != Some("2") {
        return Err(format!(
            "generated {} checkout has no authenticated-archive provenance marker; refusing to modify {}",
            spec.component, checkout.display()
        ));
    }
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

// Only the pinned compressed archive authenticates upstream bytes. Cargo's
// expanded tree, .cargo-ok and checksum text are mutable cache metadata.
fn find_registry_source(spec: &RegistrySourceSpec) -> Result<Vec<u8>, String> {
    let cargo_home = std::env::var_os("CARGO_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("USERPROFILE").map(|v| PathBuf::from(v).join(".cargo")))
        .or_else(|| std::env::var_os("HOME").map(|v| PathBuf::from(v).join(".cargo")))
        .ok_or_else(|| {
            format!(
                "cannot locate Cargo home for pinned {} archive",
                spec.component
            )
        })?;
    find_registry_source_with_fetch(spec, &cargo_home, fetch_registry_source)
}

fn find_registry_source_with_fetch(
    spec: &RegistrySourceSpec,
    cargo_home: &Path,
    fetch: impl FnOnce(&RegistrySourceSpec, &Path) -> Result<(), String>,
) -> Result<Vec<u8>, String> {
    if let Some(bytes) = locate_registry_archive(cargo_home, spec)? {
        return Ok(bytes);
    }
    // Preserve normal source preparation, but never trust its expanded tree.
    // Tests inject a local fixture here; the real fetch is not run by fixtures.
    fetch(spec, cargo_home)?;
    locate_registry_archive(cargo_home, spec)?.ok_or_else(|| format!(
        "pinned {} {} archive is absent from Cargo cache after source preparation; no source was generated",
        spec.package, spec.version
    ))
}

pub(crate) fn locate_registry_archive(
    cargo_home: &Path,
    spec: &RegistrySourceSpec,
) -> Result<Option<Vec<u8>>, String> {
    let expected = spec
        .source_hash
        .strip_prefix("sha256:")
        .filter(|hash| {
            hash.len() == 64
                && hash
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        })
        .ok_or_else(|| {
            format!(
                "invalid {} source hash `{}`",
                spec.component, spec.source_hash
            )
        })?;
    let cache = cargo_home.join("registry/cache");
    let mut indexes = match fs::read_dir(&cache) {
        Ok(entries) => entries
            .map(|entry| entry.map(|entry| entry.path()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("cannot inspect registry archive cache: {error}"))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => return Err(format!("cannot read registry archive cache: {error}")),
    };
    indexes.sort();
    for index in indexes {
        let archive = index.join(format!("{}-{}.crate", spec.package, spec.version));
        let bytes = match fs::read(&archive) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!(
                    "cannot read {} archive {}: {error}",
                    spec.component,
                    archive.display()
                ))
            }
        };
        let actual = format!("{:x}", Sha256::digest(&bytes));
        if actual != expected {
            return Err(format!(
                "pinned {} archive SHA-256 mismatch at {}: expected {expected}, got {actual}",
                spec.component,
                archive.display()
            ));
        }
        // Keep this authenticated snapshot; never reopen an archive or copy
        // registry/src after verification (including when it changes later).
        return Ok(Some(bytes));
    }
    Ok(None)
}

pub(crate) fn extract_verified_archive(
    bytes: &[u8],
    destination: &Path,
    spec: &RegistrySourceSpec,
) -> Result<(), String> {
    // Python is an existing host-tool prerequisite. Validate every member before
    // writing; tar extractall would permit links and traversal outside the tree.
    const EXTRACT: &str = r#"
import io, os, pathlib, sys, tarfile
root = pathlib.Path(sys.argv[1])
prefix = sys.argv[2]
with tarfile.open(fileobj=io.BytesIO(sys.stdin.buffer.read()), mode='r:gz') as archive:
    entries = []
    seen = set()
    for member in archive.getmembers():
        name = member.name.rstrip('/') if member.isdir() else member.name
        parts = name.split('/')
        if (parts[0] != prefix or any(p in ('', '.', '..') or '\\' in p or ':' in p for p in parts)
                or not (member.isfile() or member.isdir())):
            raise ValueError('unsafe registry archive member: ' + member.name)
        relative = tuple(parts[1:])
        if relative in seen or (not relative and not member.isdir()):
            raise ValueError('duplicate or invalid archive member: ' + member.name)
        seen.add(relative)
        entries.append((member, relative))
    root.mkdir(exist_ok=False)
    for member, relative in entries:
        target = root.joinpath(*relative)
        if member.isdir():
            target.mkdir(parents=True, exist_ok=True)
        else:
            target.parent.mkdir(parents=True, exist_ok=True)
            with archive.extractfile(member) as source, target.open('xb') as output:
                output.write(source.read())
            os.chmod(target, 0o755 if member.mode & 0o111 else 0o644)
"#;
    let mut child = Command::new("python3")
        .args(["-c", EXTRACT])
        .arg(destination)
        .arg(format!("{}-{}", spec.package, spec.version))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("cannot extract pinned {} archive: {error}", spec.component))?;
    let written = child
        .stdin
        .take()
        .expect("piped archive input")
        .write_all(bytes);
    let output = child.wait_with_output().map_err(|error| {
        format!(
            "cannot wait for {} archive extraction: {error}",
            spec.component
        )
    })?;
    written
        .map_err(|error| format!("cannot supply {} archive snapshot: {error}", spec.component))?;
    if !output.status.success() {
        return Err(format!(
            "cannot extract pinned {} archive: {}",
            spec.component,
            command_output(&output)
        ));
    }
    if !registry_source_manifest_matches(destination, spec) {
        return Err(format!(
            "pinned {} archive has a mismatched package manifest",
            spec.component
        ));
    }
    Ok(())
}

fn fetch_registry_source(spec: &RegistrySourceSpec, cargo_home: &Path) -> Result<(), String> {
    let temporary = std::env::temp_dir().join(format!(
        "nagi-registry-bootstrap-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|error| format!("cannot read clock for {} fetch: {error}", spec.component))?
            .as_nanos()
    ));
    if temporary.exists() {
        return Err(format!(
            "refusing to reuse unexpected registry bootstrap directory {}",
            temporary.display()
        ));
    }
    fs::create_dir_all(&temporary).map_err(|error| {
        format!(
            "cannot create temporary {} registry bootstrap directory: {error}",
            spec.component
        )
    })?;

    let result = (|| {
        let manifest = temporary.join("Cargo.toml");
        fs::write(&manifest, registry_fetch_manifest(spec)).map_err(|error| {
            format!(
                "cannot write temporary {} registry manifest {}: {error}",
                spec.component,
                manifest.display()
            )
        })?;
        fs::write(temporary.join("lib.rs"), "").map_err(|error| {
            format!(
                "cannot write temporary {} registry target: {error}",
                spec.component
            )
        })?;

        run_cargo(
            ["generate-lockfile", "--manifest-path"],
            &manifest,
            cargo_home,
            spec,
        )?;
        run_cargo(
            ["fetch", "--locked", "--manifest-path"],
            &manifest,
            cargo_home,
            spec,
        )
    })();
    let cleanup = fs::remove_dir_all(&temporary);
    match (result, cleanup) {
        (Err(error), _) => Err(error),
        (Ok(()), Err(error)) => Err(format!(
            "cannot remove temporary {} registry bootstrap directory {}: {error}",
            spec.component,
            temporary.display()
        )),
        (Ok(()), Ok(())) => Ok(()),
    }
}

fn registry_fetch_manifest(spec: &RegistrySourceSpec) -> String {
    format!(
        "[package]\nname = \"nagi-registry-bootstrap\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n[lib]\npath = \"lib.rs\"\n\n[dependencies]\n{} = \"={}\"\n",
        spec.package, spec.version
    )
}

fn run_cargo<const N: usize>(
    prefix: [&str; N],
    manifest: &Path,
    cargo_home: &Path,
    spec: &RegistrySourceSpec,
) -> Result<(), String> {
    let mut command = Command::new("cargo");
    command.args(prefix);
    command.arg(manifest);
    let output = command
        .env("CARGO_HOME", cargo_home)
        .current_dir(manifest.parent().unwrap_or_else(|| Path::new(".")))
        .output()
        .map_err(|error| {
            format!(
                "cannot run Cargo for {} source fetch: {error}",
                spec.component
            )
        })?;
    if !output.status.success() {
        return Err(format!(
            "Cargo {} failed for {}: {}",
            prefix.join(" "),
            spec.component,
            command_output(&output)
        ));
    }
    Ok(())
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
            "format_version = 2\nsource_hash = {}\npatch_fingerprint = {}\ncheckout_fingerprint = {}\n",
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

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(hash: String) -> RegistrySourceSpec<'static> {
        RegistrySourceSpec {
            section: "sources.example",
            component: "example",
            package: "example-package",
            version: "1.2.3",
            repository: "https://crates.io/crates/example-package/1.2.3",
            registry_archive: "https://crates.io/api/v1/crates/example-package/1.2.3/download",
            source_hash: Box::leak(hash.into_boxed_str()),
            license: "MIT",
            vendored_path: "third_party/example-package",
            patch_path: "third_party/example-package-patches",
        }
    }

    fn fixture_root(label: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "nagi-registry-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&root).unwrap();
        root
    }

    fn archive(kind: &str) -> Vec<u8> {
        let output = Command::new("python3").args(["-c", r#"
import io, sys, tarfile
kind = sys.argv[1]
result = io.BytesIO()
with tarfile.open(fileobj=result, mode='w:gz') as archive:
    for name, content in [('Cargo.toml', b'[package]\nname = "example-package"\nversion = "1.2.3"\n'),
                          ('src/lib.rs', b'upstream\n')]:
        entry = tarfile.TarInfo('example-package-1.2.3/' + name)
        if kind == 'manifest' and name == 'Cargo.toml':
            content = content.replace(b'1.2.3', b'9.9.9')
        entry.size = len(content)
        archive.addfile(entry, io.BytesIO(content))
    if kind in ('traversal', 'symlink', 'hardlink', 'duplicate', 'backslash', 'absolute'):
        name = {'traversal': '../outside', 'duplicate': 'src/lib.rs',
                'backslash': 'dir\\outside', 'absolute': '/outside'}.get(kind, 'link')
        entry = tarfile.TarInfo('example-package-1.2.3/' + name)
        if kind == 'symlink':
            entry.type = tarfile.SYMTYPE
            entry.linkname = '../../outside'
        if kind == 'hardlink':
            entry.type = tarfile.LNKTYPE
            entry.linkname = '/outside'
        archive.addfile(entry, io.BytesIO(b''))
sys.stdout.buffer.write(result.getvalue())
"#, kind]).output().unwrap();
        assert!(output.status.success(), "{output:?}");
        output.stdout
    }

    fn setup(root: &Path, bytes: &[u8]) -> RegistrySourceSpec<'static> {
        let spec = spec(format!("sha256:{:x}", Sha256::digest(bytes)));
        let lock = format!("[{}]\ncomponent = \"{}\"\nversion = \"{}\"\nrepository = \"{}\"\nregistry_archive = \"{}\"\nsource_hash = \"{}\"\nlicense = \"{}\"\nvendored_path = \"{}\"\nnagi_patch = \"{}\"\n",
            spec.section, spec.component, spec.version, spec.repository, spec.registry_archive,
            spec.source_hash, spec.license, spec.vendored_path, spec.patch_path);
        fs::create_dir_all(root.join(spec.patch_path)).unwrap();
        fs::write(root.join("third_party/sources.lock"), lock).unwrap();
        fs::write(root.join("expected-hash"), spec.source_hash).unwrap();
        // The actual patch stage must consume authenticated archive content.
        fs::write(root.join(spec.patch_path).join("0001-fixture.patch"),
            "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -1 +1 @@\n-upstream\n+patched upstream\n").unwrap();
        spec
    }

    fn cached_archive(root: &Path) -> PathBuf {
        root.join("cargo/registry/cache/fixture-index/example-package-1.2.3.crate")
    }

    fn run_child(root: &Path, expect: &str) {
        let output = Command::new(std::env::current_exe().unwrap())
            .args([
                "--ignored",
                "--exact",
                "registry_source::tests::registry_archive_fixture_child",
                "--nocapture",
            ])
            .env("CARGO_HOME", root.join("cargo"))
            .env("NAGI_REGISTRY_FIXTURE_ROOT", root)
            .env("NAGI_REGISTRY_EXPECT", expect)
            .env("CARGO_NET_OFFLINE", "true")
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
    #[ignore = "isolated fake-CARGO_HOME child, invoked by registry archive fixtures"]
    fn registry_archive_fixture_child() {
        let root = PathBuf::from(std::env::var_os("NAGI_REGISTRY_FIXTURE_ROOT").unwrap());
        let spec = spec(fs::read_to_string(root.join("expected-hash")).unwrap());
        let result = ensure_registry_checkout_using(&root, &spec, || {
            find_registry_source_with_fetch(&spec, &root.join("cargo"), |_, _| {
                Err("archive is absent; fixture forbids network source preparation".into())
            })
        });
        let expect = std::env::var("NAGI_REGISTRY_EXPECT").unwrap();
        if expect == "success" {
            let checkout = result.unwrap();
            assert_eq!(
                fs::read(checkout.join("src/lib.rs")).unwrap(),
                b"patched upstream\n"
            );
            assert!(!checkout.join(".cargo-ok").exists());
            assert!(!checkout.join(".cargo-checksum.json").exists());
            assert!(ensure_registry_checkout(&root, &spec).is_ok());
            let marker = checkout.join(marker_name(&spec));
            let current_marker = fs::read_to_string(&marker).unwrap();
            let legacy_marker = current_marker.replace("format_version = 2", "format_version = 1");
            fs::write(&marker, &legacy_marker).unwrap();
            let error = ensure_registry_checkout(&root, &spec).unwrap_err();
            assert!(
                error.contains("no authenticated-archive provenance"),
                "{error}"
            );
            assert_eq!(fs::read_to_string(&marker).unwrap(), legacy_marker);
            assert_eq!(
                fs::read(checkout.join("src/lib.rs")).unwrap(),
                b"patched upstream\n"
            );
            fs::write(&marker, current_marker).unwrap();
            fs::write(checkout.join("src/lib.rs"), "tampered generated source").unwrap();
            let error = ensure_registry_checkout(&root, &spec).unwrap_err();
            assert!(error.contains("state does not match"), "{error}");
            assert_eq!(
                fs::read_to_string(checkout.join("src/lib.rs")).unwrap(),
                "tampered generated source"
            );
        } else {
            let error = result.unwrap_err();
            assert!(error.contains(&expect), "{error}");
            assert!(!root.join(spec.vendored_path).exists());
            assert!(fs::read_dir(root.join("third_party")).unwrap().all(|p| !p
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("bootstrap")));
        }
    }

    #[test]
    fn expanded_cache_metadata_cannot_authenticate_source_without_archive() {
        let bytes = archive("valid");
        for checksum in [None, Some("substring"), Some("correct"), Some("malformed")] {
            let root = fixture_root("metadata");
            let spec = setup(&root, &bytes);
            let expanded = root.join("cargo/registry/src/fixture-index/example-package-1.2.3");
            fs::create_dir_all(expanded.join("src")).unwrap();
            fs::write(
                expanded.join("Cargo.toml"),
                "[package]\nname = \"example-package\"\nversion = \"1.2.3\"\n",
            )
            .unwrap();
            fs::write(expanded.join("src/lib.rs"), "tampered").unwrap();
            fs::write(expanded.join(".cargo-ok"), "").unwrap();
            if let Some(kind) = checksum {
                let hash = spec.source_hash.strip_prefix("sha256:").unwrap();
                let json = match kind {
                    "substring" => {
                        format!(r#"{{"package":"wrong","files":{{"junk":"prefix{hash}suffix"}}}}"#)
                    }
                    "correct" => format!(r#"{{"package":"{hash}","files":{{}}}}"#),
                    _ => format!("invalid json {hash}"),
                };
                fs::write(expanded.join(".cargo-checksum.json"), json).unwrap();
            }
            run_child(&root, "archive is absent");
            assert_eq!(
                fs::read_to_string(expanded.join("src/lib.rs")).unwrap(),
                "tampered"
            );
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn pinned_archive_bytes_override_tampered_expanded_tree() {
        let root = fixture_root("authenticated");
        let bytes = archive("valid");
        setup(&root, &bytes);
        let path = cached_archive(&root);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, bytes).unwrap();
        let expanded = root.join("cargo/registry/src/fixture-index/example-package-1.2.3");
        fs::create_dir_all(expanded.join("src")).unwrap();
        fs::write(expanded.join("src/lib.rs"), "injected source").unwrap();
        run_child(&root, "success");
        assert_eq!(
            fs::read_to_string(expanded.join("src/lib.rs")).unwrap(),
            "injected source"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn registry_fetch_manifest_pins_the_requested_package_version() {
        let manifest = registry_fetch_manifest(&spec("sha256:fixture".into()));
        assert!(manifest.contains("example-package = \"=1.2.3\""));
        assert!(!manifest.contains("example-package = \"1.2.3\""));
    }

    #[test]
    fn missing_archive_preserves_preparation_but_only_uses_authenticated_result() {
        for outcome in ["archive", "expanded-only", "wrong-archive", "failure"] {
            let root = fixture_root(outcome);
            let bytes = archive("valid");
            let spec = setup(&root, &bytes);
            let calls = std::cell::Cell::new(0);
            let result = ensure_registry_checkout_using(&root, &spec, || {
                find_registry_source_with_fetch(&spec, &root.join("cargo"), |_, _| {
                    calls.set(calls.get() + 1);
                    if outcome == "failure" {
                        return Err("fixture source preparation failed".into());
                    }
                    // This is a local stand-in for Cargo preparation. No child
                    // Cargo, network operation or dependency download runs.
                    let expanded =
                        root.join("cargo/registry/src/fixture-index/example-package-1.2.3");
                    fs::create_dir_all(&expanded).unwrap();
                    fs::write(expanded.join("src.rs"), "untrusted expanded content").unwrap();
                    if outcome != "expanded-only" {
                        let path = cached_archive(&root);
                        fs::create_dir_all(path.parent().unwrap()).unwrap();
                        fs::write(
                            path,
                            if outcome == "archive" {
                                &bytes[..]
                            } else {
                                b"wrong archive"
                            },
                        )
                        .unwrap();
                    }
                    Ok(())
                })
            });
            assert_eq!(calls.get(), 1);
            if outcome == "archive" {
                let checkout = result.unwrap();
                assert_eq!(
                    fs::read(checkout.join("src/lib.rs")).unwrap(),
                    b"patched upstream\n"
                );
                let cached = find_registry_source_with_fetch(&spec, &root.join("cargo"), |_, _| {
                    panic!("valid cached archive must not trigger preparation")
                })
                .unwrap();
                assert_eq!(cached, bytes);
            } else {
                let error = result.unwrap_err();
                assert!(
                    error.contains(match outcome {
                        "expanded-only" => "archive is absent",
                        "wrong-archive" => "SHA-256 mismatch",
                        _ => "fixture source preparation failed",
                    }),
                    "{outcome}: {error}"
                );
                assert!(!root.join(spec.vendored_path).exists());
            }
            fs::remove_dir_all(root).unwrap();
        }
    }

    #[test]
    fn wrong_archive_digest_fails_before_materialization() {
        let root = fixture_root("digest");
        setup(&root, &archive("valid"));
        let path = cached_archive(&root);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"wrong compressed bytes").unwrap();
        let error = find_registry_source_with_fetch(
            &spec(fs::read_to_string(root.join("expected-hash")).unwrap()),
            &root.join("cargo"),
            |_, _| panic!("bad cached archive must fail without refreshing it"),
        )
        .unwrap_err();
        assert!(error.contains("SHA-256 mismatch"), "{error}");
        run_child(&root, "SHA-256 mismatch");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn malformed_pins_and_archive_members_fail_closed() {
        for kind in [
            "manifest",
            "traversal",
            "symlink",
            "hardlink",
            "duplicate",
            "backslash",
            "absolute",
        ] {
            let root = fixture_root(kind);
            let bytes = archive(kind);
            setup(&root, &bytes);
            let path = cached_archive(&root);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, bytes).unwrap();
            run_child(
                &root,
                if kind == "manifest" {
                    "mismatched package manifest"
                } else {
                    "archive"
                },
            );
            assert!(!root.join("outside").exists());
            fs::remove_dir_all(root).unwrap();
        }
        let root = fixture_root("pin");
        let spec = spec("sha256:short".into());
        assert!(locate_registry_archive(&root, &spec)
            .unwrap_err()
            .contains("invalid"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn legacy_poisoned_checkout_without_archive_is_preserved_and_rejected() {
        let root = fixture_root("legacy");
        let spec = setup(&root, &archive("valid"));
        let checkout = root.join(spec.vendored_path);
        fs::create_dir_all(checkout.join("src")).unwrap();
        fs::write(
            checkout.join("Cargo.toml"),
            "[package]\nname = \"example-package\"\nversion = \"1.2.3\"\n",
        )
        .unwrap();
        fs::write(
            checkout.join("src/lib.rs"),
            "legacy unverified cache content",
        )
        .unwrap();
        write_marker(&root, &checkout, &spec).unwrap();
        let marker = checkout.join(marker_name(&spec));
        let legacy = fs::read_to_string(&marker)
            .unwrap()
            .replace("format_version = 2", "format_version = 1");
        fs::write(&marker, &legacy).unwrap();
        assert!(!cached_archive(&root).exists());
        let error = ensure_registry_checkout(&root, &spec).unwrap_err();
        assert!(
            error.contains("no authenticated-archive provenance"),
            "{error}"
        );
        assert_eq!(fs::read_to_string(&marker).unwrap(), legacy);
        assert_eq!(
            fs::read_to_string(checkout.join("src/lib.rs")).unwrap(),
            "legacy unverified cache content"
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn archive_replacement_after_verification_cannot_change_consumed_bytes() {
        let root = fixture_root("snapshot");
        let bytes = archive("valid");
        let spec = setup(&root, &bytes);
        let path = cached_archive(&root);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, bytes).unwrap();
        let verified = locate_registry_archive(&root.join("cargo"), &spec)
            .unwrap()
            .unwrap();
        fs::write(path, "replaced after validation").unwrap();
        let destination = root.join("snapshot");
        extract_verified_archive(&verified, &destination, &spec).unwrap();
        assert_eq!(
            fs::read(destination.join("src/lib.rs")).unwrap(),
            b"upstream\n"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
