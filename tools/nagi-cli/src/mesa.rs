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

    #[test]
    fn m17_mesa_link_does_not_force_duplicate_archive_members() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root");
        let build_script = fs::read_to_string(root.join("user/nagi-init/build.rs"))
            .expect("Nagi init build script");
        assert!(build_script.contains("static=nagi_mesa"));
        assert!(build_script.contains("static=nagi_mesa_roots"));
        assert!(!build_script.contains("static:+whole-archive=nagi_mesa"));
        assert!(build_script.contains(
            "cargo:rustc-link-arg-bin=nagi-init=--undefined=nagi_mesa_glthread_finish_link_anchor"
        ));
        assert!(build_script
            .contains("cargo:rustc-link-arg-bin=nagi-init=--undefined=st_context_flush"));
        assert!(build_script.contains(
            "--undefined=_ZN2JS26NewArrayBufferWithContentsEP9JSContextmSt10unique_ptrIvNS_10FreePolicyEE"
        ));
        for symbol in [
            "glcpp_preprocess",
            "spirv_to_nir",
            "spirv_verify_gl_specialization_constants",
            "strpbrk",
            "pthread_barrier_init",
            "pthread_barrier_destroy",
            "pthread_barrier_wait",
            "fsync",
            "ftruncate",
            "fchmod",
            "fchown",
            "utimes",
            "dlopen",
            "dlerror",
            "dlclose",
        ] {
            assert!(
                build_script.contains(symbol),
                "missing target link root {symbol}"
            );
        }
        let cxx_abi = fs::read_to_string(root.join("tools/mesa/nagi-libcpp-abi.cpp"))
            .expect("target libc++ ABI providers");
        let cxx_sort = fs::read_to_string(root.join("tools/mesa/nagi-libcpp-sort.cpp"))
            .expect("target libc++ sort providers");
        let relibc_nagi = fs::read_to_string(root.join("third_party/relibc/src/nagi.rs"))
            .expect("Nagi relibc backend");
        let target_cc = fs::read_to_string(root.join("tools/nagi-target-cc.sh"))
            .expect("target C compiler wrapper");
        let relibc_backend = fs::read_to_string(root.join("third_party/relibc/src/nagi.rs"))
            .expect("Nagi relibc backend");
        let mesa_build =
            fs::read_to_string(root.join("tools/mesa/build.sh")).expect("Nagi Mesa build script");
        let relibc_portability_patch = fs::read_to_string(
            root.join("third_party/relibc-patches/0001-nagi-portable-header-find.patch"),
        )
        .expect("relibc host portability patch");
        assert!(build_script.contains("nagi-libcpp-abi.cpp"));
        assert!(build_script.contains("nagi-libcpp-sort.cpp"));
        assert!(build_script.contains("_LIBCPP_HAS_THREAD_API_PTHREAD=1"));
        assert!(build_script.contains("_LIBCPP_PROVIDES_DEFAULT_RUNE_TABLE=1"));
        assert!(cxx_abi.contains("nagi_posix_sleep_ns"));
        assert!(cxx_abi.contains("this_thread") && cxx_abi.contains("sleep_for"));
        assert!(cxx_abi.contains("basic_string<char>::append"));
        assert!(cxx_abi.contains("basic_string<char>::__grow_by"));
        for method in ["assign", "resize", "append", "replace"] {
            assert!(
                cxx_abi.contains(&format!("basic_string<char>::{method}")),
                "missing target libc++ string provider {method}"
            );
        }
        for symbol in ["ntohs", "ntohl", "htons", "htonl"] {
            assert!(
                relibc_nagi.contains(&format!("fn {symbol}(")),
                "missing Nagi relibc provider {symbol}"
            );
            assert!(
                build_script.contains(&format!("\"{symbol}\"")),
                "missing Nagi relibc link root {symbol}"
            );
        }
        assert!(relibc_nagi.contains("fn strpbrk("));
        for type_name in [
            "signed char",
            "int",
            "long",
            "short",
            "unsigned short",
            "unsigned char",
            "unsigned int",
            "unsigned long",
        ] {
            assert!(
                cxx_sort.contains(type_name),
                "missing libc++ sort type {type_name}"
            );
        }
        assert!(target_cc.contains("SQLITE_OMIT_LOAD_EXTENSION=1"));
        assert!(target_cc.contains("*/libsqlite3-sys-*/sqlite3/sqlite3.c"));
        for symbol in ["fn dlopen(", "fn dlsym(", "fn dlerror(", "fn dlclose("] {
            assert!(
                relibc_backend.contains(symbol),
                "missing fail-closed dynamic-loader symbol {symbol}"
            );
        }
        assert!(mesa_build.contains("relibc_header_patch"));
        assert!(mesa_build.contains("mesa_glcpp_target"));
        assert!(mesa_build.contains("libglcpp\\.a"));
        assert!(mesa_build.contains("mesa_vtn_target"));
        assert!(mesa_build.contains("libvtn\\.a"));
        assert!(relibc_portability_patch.contains("-exec basename {}"));
        assert!(relibc_portability_patch.contains("-printf"));
        assert!(mesa_build.contains("_mesa_glthread_finish"));
        assert!(mesa_build.contains("libnagi_mesa_roots.a"));
        assert!(mesa_build.contains("libgallium\\.a"));
        assert!(mesa_build.contains("libglsl\\.a"));
    }

    #[test]
    fn m17_posix_abort_is_a_weak_fallback_for_relibc() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root");
        let abi =
            fs::read_to_string(root.join("user/nagi-posix/src/abi.rs")).expect("Nagi POSIX ABI");
        let abi = abi.replace("\r\n", "\n");
        let abort = abi
            .find("pub unsafe extern \"C\" fn abort() -> !")
            .expect("abort");
        let prefix = &abi[..abort];
        assert!(prefix.ends_with(
            "#[cfg_attr(target_os = \"nagi\", linkage = \"weak\")]\n#[unsafe(no_mangle)]\n"
        ));
    }

    #[test]
    fn m17_target_cxx_runtime_is_linked_from_nagi_allocator_boundary() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root");
        let build_script = fs::read_to_string(root.join("user/nagi-init/build.rs"))
            .expect("Nagi init build script");
        let runtime = fs::read_to_string(root.join("tools/mesa/nagi-cxx-runtime.cpp"))
            .expect("Nagi C++ runtime shim");
        let relibc = fs::read_to_string(root.join("third_party/relibc/src/nagi.rs"))
            .expect("Nagi relibc backend");
        assert!(build_script.contains("nagi-cxx-runtime.cpp"));
        assert!(build_script.contains("-x") && build_script.contains("c++"));
        assert!(runtime.contains("operator delete"));
        assert!(runtime.contains("__stack_chk_guard"));
        assert!(runtime.contains("nagi_posix_malloc"));
        assert!(runtime.contains("nagi_posix_free"));
        assert!(runtime.contains("_ZNSt3__122__libcpp_verbose_abortEPKcz"));
        assert!(runtime.contains("__cxa_guard_acquire"));
        assert!(runtime.contains("__cxa_guard_release"));
        assert!(runtime.contains("__cxa_guard_abort"));
        assert!(runtime.contains("__cxa_atexit"));
        assert!(runtime.contains("__cxa_finalize"));
        assert!(runtime.contains("nagi_cxx_finalize"));
        assert!(runtime.contains("_ZNSt3__16locale7classicEv"));
        assert!(runtime.contains("_ZNSt3__15ctypeIcE2idE"));
        assert!(runtime.contains("_ZSt7nothrow"));
        assert!(runtime.contains("_ZSt20__throw_length_errorPKc"));
        assert!(runtime.contains("_ZSt28__throw_bad_array_new_lengthv"));
        assert!(runtime.contains("__cxa_begin_catch"));
        assert!(runtime.contains("__cxa_rethrow"));
        assert!(runtime.contains("__cxa_pure_virtual"));
        assert!(runtime.contains("_ZSt17__throw_bad_allocv"));
        assert!(runtime.contains("_ZNSt9bad_allocC1Ev"));
        assert!(runtime.contains("_ZNSt9bad_allocC2Ev"));
        assert!(runtime.contains("_ZNKSt9bad_alloc4whatEv"));
        assert!(runtime.contains("_ZNSt3__112__next_primeEm"));
        assert!(runtime.contains("_ZNKSt3__16locale9use_facetERNS0_2idE"));
        assert!(runtime.contains("__cxa_bad_typeid"));
        assert!(runtime.contains("_ZNSt3__15mutex4lockEv"));
        assert!(runtime.contains("_ZNSt3__15mutex8try_lockEv"));
        assert!(runtime.contains("_ZNSt3__15mutex6unlockEv"));
        assert!(runtime.contains("_ZNSt3__15mutexD1Ev"));
        assert!(runtime.contains("_ZNSt3__15mutexD0Ev"));
        assert!(runtime.contains("_ZNSt3__111__call_onceERVmPvPFvS2_E"));
        assert!(runtime.contains("_ZNSt3__118condition_variable10notify_oneEv"));
        assert!(runtime.contains("_ZNSt3__118condition_variable10notify_allEv"));
        assert!(
            runtime.contains("_ZNSt3__118condition_variable4waitERNS_11unique_lockINS_5mutexEEE")
        );
        assert!(runtime.contains("_ZNSt3__118condition_variableD1Ev"));
        assert!(runtime.contains("_ZNSt3__118condition_variableD0Ev"));
        assert!(runtime.contains("pthread_cond_wait"));
        assert!(runtime.contains("pthread_cond_broadcast"));
        assert!(runtime.contains("pthread_mutex_destroy"));
        for symbol in [
            "pthread_mutex_lock",
            "pthread_mutex_trylock",
            "pthread_mutex_unlock",
            "pthread_mutex_destroy",
            "pthread_cond_signal",
            "pthread_cond_broadcast",
            "pthread_cond_wait",
            "pthread_cond_destroy",
            "pthread_getattr_np",
            "pthread_attr_getstack",
        ] {
            assert!(build_script.contains(symbol));
        }
        assert!(build_script.contains("_ZNSt3__111__call_onceERVmPvPFvS2_E"));
        assert!(runtime.contains("__cxa_end_catch"));
        assert!(runtime.contains("nagi_mesa_glthread_finish_link_anchor"));
        assert!(runtime.contains("__dynamic_cast"));
        assert!(runtime.contains("_ZSt18_Rb_tree_incrementPSt18_Rb_tree_node_base"));
        assert!(runtime.contains("_ZSt18_Rb_tree_incrementPKSt18_Rb_tree_node_base"));
        assert!(
            runtime.contains("_ZSt29_Rb_tree_insert_and_rebalancebPSt18_Rb_tree_node_baseS0_RS_")
        );
        assert!(relibc.contains("pub unsafe extern \"C\" fn __fprintf_chk("));
        assert!(relibc.contains("pub unsafe extern \"C\" fn __vfprintf_chk("));
        assert!(relibc.contains("pub unsafe extern \"C\" fn strnlen("));
        assert!(relibc.contains("pub unsafe extern \"C\" fn div("));
        assert!(relibc.contains("pub unsafe extern \"C\" fn openlog("));
        assert!(relibc.contains("pub unsafe extern \"C\" fn syslog("));
        assert!(relibc.contains("pub unsafe extern \"C\" fn __memset_chk("));
        assert!(relibc.contains("pub unsafe extern \"C\" fn __memmove_chk("));
        assert!(relibc.contains(".globl fabsl"));
        assert!(relibc.contains("pub unsafe extern \"C\" fn sincosf("));
        assert!(relibc.contains("pub unsafe extern \"C\" fn __isnormal("));
        assert!(relibc.contains("pub unsafe extern \"C\" fn __isnormalf("));
        assert!(relibc.contains("pub unsafe extern \"C\" fn frexp("));
        assert!(relibc.contains("pub unsafe extern \"C\" fn scalbn("));
        assert!(relibc.contains("pub unsafe extern \"C\" fn scalbnf("));
        assert!(relibc.contains("pub unsafe extern \"C\" fn lrint("));
        assert!(relibc.contains("pub unsafe extern \"C\" fn llrint("));
        assert!(relibc.contains("pub unsafe extern \"C\" fn localtime_r("));
        assert!(relibc.contains("pub unsafe extern \"C\" fn setlocale("));
        assert!(relibc.contains("pub unsafe extern \"C\" fn islower("));
        assert!(relibc.contains("pub unsafe extern \"C\" fn nearbyint("));
        assert!(relibc.contains("pub unsafe extern \"C\" fn nearbyintf("));
        assert!(build_script.contains("\"nearbyintf\""));
        assert!(relibc.contains("pub unsafe extern \"C\" fn mktime("));
        assert!(relibc.contains("pub unsafe extern \"C\" fn gmtime_r("));
        assert!(relibc.contains("pub unsafe extern \"C\" fn readlink("));
        assert!(relibc.contains("pub static mut tzname"));
        assert!(runtime.contains("_ZSt18_Rb_tree_decrementPSt18_Rb_tree_node_base"));
        assert!(runtime.contains("_ZSt18_Rb_tree_decrementPKSt18_Rb_tree_node_base"));
        assert!(runtime.contains("_ZSt28_Rb_tree_rebalance_for_erasePSt18_Rb_tree_node_baseRS_"));
        assert!(runtime.contains("__popcountdi2"));
        assert!(
            runtime.contains("_ZNSt7__cxx1112basic_stringIcSt11char_traitsIcESaIcEE10_M_disposeEv")
        );
        assert!(runtime
            .contains("_ZNSt7__cxx1112basic_stringIcSt11char_traitsIcESaIcEE9_M_appendEPKcm"));
        assert!(
            runtime.contains("_ZNKSt7__cxx1112basic_stringIcSt11char_traitsIcESaIcEE4findEPKcmm")
        );
        assert!(
            runtime.contains("_ZNSt7__cxx1112basic_stringIcSt11char_traitsIcESaIcEE9_M_createERmm")
        );
        assert!(runtime
            .contains("_ZNSt7__cxx1112basic_stringIcSt11char_traitsIcESaIcEE10_M_replaceEmmPKcm"));
        assert!(runtime.contains("_ZNKSt8__detail20_Prime_rehash_policy14_M_need_rehashEmmm"));
        assert!(runtime.contains("_ZNSt7__cxx1112basic_stringIcSt11char_traitsIcESaIcEE6resizeEmc"));
        assert!(runtime
            .contains("_ZNSt7__cxx1112basic_stringIcSt11char_traitsIcESaIcEE12_M_constructEmc"));
        assert!(runtime.contains("_ZSt24__throw_out_of_range_fmtPKcz"));
        assert!(runtime.contains("__gxx_personality_v0"));
        assert!(runtime.contains(
            "_ZNSt7__cxx1112basic_stringIcSt11char_traitsIcESaIcEE14_M_replace_auxEmmmc"
        ));
        assert!(runtime.contains("_Unwind_Resume"));
        assert!(runtime.contains("_Unwind_GetCFA"));
        assert!(runtime.contains("_Unwind_FindEnclosingFunction"));
        assert!(runtime.contains("nagi_gnu_basic_string_layout"));
        assert!(runtime.contains("class __class_type_info"));
        assert!(runtime.contains("class __si_class_type_info"));
        assert!(runtime.contains("class __vmi_class_type_info"));
        assert!(runtime.contains("-fno-rtti") || build_script.contains("-fno-rtti"));
        assert!(runtime.contains("-fno-exceptions") || build_script.contains("-fno-exceptions"));
    }

    #[test]
    fn m17_target_process_abi_uses_nagi_spawn_and_exit_boundaries() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root");
        let relibc = fs::read_to_string(root.join("third_party/relibc/src/nagi.rs"))
            .expect("Nagi relibc backend");
        let abi =
            fs::read_to_string(root.join("user/nagi-posix/src/abi.rs")).expect("Nagi POSIX ABI");
        assert!(relibc.contains("pub unsafe extern \"C\" fn _exit("));
        assert!(relibc.contains("pub unsafe extern \"C\" fn exit("));
        assert!(relibc.contains("pub unsafe extern \"C\" fn dup2("));
        assert!(relibc.contains("pub unsafe extern \"C\" fn setgid("));
        assert!(relibc.contains("pub unsafe extern \"C\" fn setuid("));
        assert!(relibc.contains("pub unsafe extern \"C\" fn getpid("));
        assert!(relibc.contains("pub unsafe extern \"C\" fn getuid("));
        assert!(relibc.contains("pub unsafe extern \"C\" fn geteuid("));
        assert!(relibc.contains("pub unsafe extern \"C\" fn getgid("));
        assert!(relibc.contains("pub unsafe extern \"C\" fn getegid("));
        assert!(relibc.contains("pub unsafe extern \"C\" fn printf("));
        assert!(relibc.contains("pub unsafe extern \"C\" fn chdir("));
        assert!(relibc.contains("pub unsafe extern \"C\" fn chroot("));
        assert!(relibc.contains("pub unsafe extern \"C\" fn setpgid("));
        assert!(relibc.contains("pub unsafe extern \"C\" fn setsid("));
        assert!(relibc.contains("pub unsafe extern \"C\" fn signal("));
        assert!(relibc.contains("pub unsafe extern \"C\" fn waitpid("));
        assert!(relibc.contains("nagi_posix_dup2"));
        assert!(relibc.contains("nagi_posix_chdir"));
        assert!(relibc.contains("nagi_posix_chroot"));
        assert!(relibc.contains("nagi_posix_exit"));
        assert!(relibc.contains("nagi_posix_setpgid"));
        assert!(relibc.contains("nagi_posix_setgid"));
        assert!(relibc.contains("nagi_posix_setuid"));
        assert!(relibc.contains("nagi_posix_getpid"));
        assert!(relibc.contains("nagi_posix_getuid"));
        assert!(relibc.contains("nagi_posix_geteuid"));
        assert!(relibc.contains("nagi_posix_getgid"));
        assert!(relibc.contains("nagi_posix_getegid"));
        assert!(relibc.contains("nagi_posix_setsid"));
        assert!(relibc.contains("nagi_posix_signal"));
        assert!(relibc.contains("nagi_posix_waitpid"));
        assert!(abi.contains("crate::process::native_wait"));
        assert!(abi.contains("pub unsafe extern \"C\" fn nagi_posix_dup2("));
        assert!(abi.contains("nagi_posix_setpgid"));
        assert!(abi.contains("nagi_posix_setsid"));
        assert!(abi.contains("nagi_posix_signal"));
        assert!(abi.contains("pub unsafe extern \"C\" fn nagi_posix_getuid("));
        assert!(abi.contains("pub unsafe extern \"C\" fn nagi_posix_geteuid("));
        assert!(abi.contains("pub unsafe extern \"C\" fn nagi_posix_getgid("));
        assert!(abi.contains("pub unsafe extern \"C\" fn nagi_posix_getegid("));
        assert!(abi.contains("libnagi::exit((code as u8) as u64)"));
    }

    #[test]
    fn m17_target_memory_abi_has_checked_copy_boundary() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root");
        let libnagi =
            fs::read_to_string(root.join("user/libnagi/src/lib.rs")).expect("Nagi user ABI");
        assert!(libnagi.contains("pub unsafe extern \"C\" fn __memcpy_chk("));
        assert!(libnagi.contains("checked_add(count)"));
    }

    #[test]
    fn m17_mesa_freestanding_cpp_and_dri2_boundaries_are_pinned() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root");
        let build =
            fs::read_to_string(root.join("tools/mesa/build.sh")).expect("Mesa build script");
        assert!(build.contains("-fno-exceptions"));
        assert!(build.contains("-fno-rtti"));
        let patch = fs::read_to_string(
            root.join("third_party/mesa-patches/0018-nagi-enable-dri2-frontend.patch"),
        )
        .expect("Mesa DRI2 patch");
        assert!(patch.contains("host_machine.system() == 'nagi'"));
        assert!(patch.contains("with_dri2"));
    }

    #[test]
    fn m17_posix_thread_abi_covers_servo_runtime_symbols() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root");
        let abi =
            fs::read_to_string(root.join("user/nagi-posix/src/abi.rs")).expect("Nagi POSIX ABI");
        for symbol in ["fn pthread_equal(", "fn pthread_setname_np("] {
            assert!(
                abi.contains(symbol),
                "missing target thread ABI symbol: {symbol}"
            );
        }
    }

    #[test]
    fn m17_target_relibc_backend_covers_basic_c_runtime_symbols() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root");
        let relibc = fs::read_to_string(root.join("third_party/relibc/src/nagi.rs"))
            .expect("Nagi relibc backend");
        for symbol in [
            "pub unsafe extern \"C\" fn strcmp(",
            "pub unsafe extern \"C\" fn strchr(",
            "pub unsafe extern \"C\" fn strrchr(",
            "pub unsafe extern \"C\" fn strcpy(",
            "pub unsafe extern \"C\" fn memchr(",
            "pub unsafe extern \"C\" fn qsort(",
            "pub unsafe extern \"C\" fn atoi(",
            "pub static mut stderr:",
            "pub unsafe extern \"C\" fn gai_strerror(",
            "pub unsafe extern \"C\" fn ioctl(",
            "pub unsafe extern \"C\" fn accept(",
            "pub unsafe extern \"C\" fn getsockopt(",
            "pub unsafe extern \"C\" fn lstat(",
            "pub unsafe extern \"C\" fn isatty(",
            "pub unsafe extern \"C\" fn strncmp(",
            "pub unsafe extern \"C\" fn snprintf(",
            "pub unsafe extern \"C\" fn openat(",
            "pub unsafe extern \"C\" fn unlink(",
            "pub unsafe extern \"C\" fn unlinkat(",
            "pub unsafe extern \"C\" fn mkdir(",
            "pub unsafe extern \"C\" fn rmdir(",
            "pub unsafe extern \"C\" fn opendir(",
            "pub unsafe extern \"C\" fn readdir(",
            "pub unsafe extern \"C\" fn readdir_r(",
            "pub unsafe extern \"C\" fn closedir(",
            "pub unsafe extern \"C\" fn sinf(",
            "pub unsafe extern \"C\" fn cosf(",
            "pub unsafe extern \"C\" fn tan(",
            "pub unsafe extern \"C\" fn tanf(",
            "pub unsafe extern \"C\" fn tanh(",
            "pub unsafe extern \"C\" fn tanhf(",
            "pub unsafe extern \"C\" fn log(",
            "pub unsafe extern \"C\" fn logf(",
            "pub unsafe extern \"C\" fn log2(",
            "pub unsafe extern \"C\" fn log2f(",
            "pub unsafe extern \"C\" fn exp2f(",
            "pub unsafe extern \"C\" fn fread(",
            "pub unsafe extern \"C\" fn fopen(",
            "pub unsafe extern \"C\" fn fprintf(",
            "pub unsafe extern \"C\" fn vfprintf(",
            "pub unsafe extern \"C\" fn vasprintf(",
            "pub unsafe extern \"C\" fn asprintf(",
            "pub unsafe extern \"C\" fn __vsnprintf_chk(",
            "pub unsafe extern \"C\" fn atexit(",
            "pub unsafe extern \"C\" fn ldexp(",
            "pub unsafe extern \"C\" fn __isfinite(",
            "pub unsafe extern \"C\" fn regcomp(",
            "pub unsafe extern \"C\" fn regexec(",
            "pub unsafe extern \"C\" fn regfree(",
            "pub unsafe extern \"C\" fn fseek(",
            "pub unsafe extern \"C\" fn ftell(",
            "pub unsafe extern \"C\" fn strstr(",
            "pub unsafe extern \"C\" fn strncpy(",
            "pub unsafe extern \"C\" fn asin(",
            "pub unsafe extern \"C\" fn asinf(",
            "pub unsafe extern \"C\" fn atan(",
            "pub unsafe extern \"C\" fn atanf(",
            "pub unsafe extern \"C\" fn atan2(",
            "pub unsafe extern \"C\" fn atan2f(",
            "pub unsafe extern \"C\" fn acos(",
            "pub unsafe extern \"C\" fn acosf(",
            "pub unsafe extern \"C\" fn exp(",
            "pub unsafe extern \"C\" fn expf(",
            "pub unsafe extern \"C\" fn hypot(",
            "pub unsafe extern \"C\" fn hypotf(",
            "pub unsafe extern \"C\" fn abs(",
            "pub unsafe extern \"C\" fn fdopendir(",
            "pub unsafe extern \"C\" fn pow(",
            "pub unsafe extern \"C\" fn powf(",
            "pub unsafe extern \"C\" fn __assert_fail(",
            "pub unsafe extern \"C\" fn dlsym(",
            "pub unsafe extern \"C\" fn pthread_once(",
            "pub unsafe extern \"C\" fn perror(",
            "pub unsafe extern \"C\" fn __errno_location(",
            "pub unsafe extern \"C\" fn sscanf(",
            "pub unsafe extern \"C\" fn strdup(",
            "pub unsafe extern \"C\" fn getaddrinfo(",
            "pub unsafe extern \"C\" fn freeaddrinfo(",
            "pub unsafe extern \"C\" fn execvp(",
            "pub unsafe extern \"C\" fn fork(",
            "pub unsafe extern \"C\" fn strcat(",
            "pub unsafe extern \"C\" fn strncat(",
            "pub unsafe extern \"C\" fn islower(",
            "pub unsafe extern \"C\" fn nearbyint(",
            "pub unsafe extern \"C\" fn mktime(",
            "pub unsafe extern \"C\" fn gmtime_r(",
            "pub unsafe extern \"C\" fn readlink(",
            "pub unsafe extern \"C\" fn bsearch(",
            "pub unsafe extern \"C\" fn pthread_rwlock_init(",
            "pub unsafe extern \"C\" fn pthread_rwlock_rdlock(",
            "pub unsafe extern \"C\" fn pthread_rwlock_wrlock(",
            "pub unsafe extern \"C\" fn pthread_rwlock_unlock(",
        ] {
            assert!(
                relibc.contains(symbol),
                "missing target relibc C runtime symbol: {symbol}"
            );
        }
        assert!(
            relibc.contains("const NAGI_FILE_FD: u32") && relibc.contains("nagi_posix_write_fd"),
            "stderr must use the Nagi descriptor-backed stream path"
        );
        assert!(relibc.contains("pub static mut environ:"));
        assert!(relibc.contains("core::arch::global_asm!"));
        assert!(relibc.contains(".globl setjmp") && relibc.contains(".globl longjmp"));
        let abi =
            fs::read_to_string(root.join("user/nagi-posix/src/abi.rs")).expect("Nagi POSIX ABI");
        for symbol in [
            "pub unsafe extern \"C\" fn pthread_cond_timedwait(",
            "pub unsafe extern \"C\" fn nagi_posix_ioctl(",
            "pub unsafe extern \"C\" fn nagi_posix_accept(",
            "pub unsafe extern \"C\" fn nagi_posix_getsockopt(",
            "pub unsafe extern \"C\" fn nagi_posix_getsockname(",
            "pub unsafe extern \"C\" fn nagi_posix_lstat(",
            "pub unsafe extern \"C\" fn nagi_posix_isatty(",
            "pub unsafe extern \"C\" fn nagi_posix_openat(",
            "pub unsafe extern \"C\" fn nagi_posix_unlink(",
            "pub unsafe extern \"C\" fn nagi_posix_unlinkat(",
            "pub unsafe extern \"C\" fn nagi_posix_mkdir(",
            "pub unsafe extern \"C\" fn nagi_posix_rmdir(",
            "pub unsafe extern \"C\" fn nagi_posix_opendir(",
            "pub unsafe extern \"C\" fn nagi_posix_readdir(",
            "pub unsafe extern \"C\" fn nagi_posix_readdir_r(",
            "pub unsafe extern \"C\" fn nagi_posix_closedir(",
            "pub unsafe extern \"C\" fn nagi_posix_fdopendir(",
            "pub unsafe extern \"C\" fn nagi_posix_dirfd(",
            "pub unsafe extern \"C\" fn pthread_detach(",
            "pub unsafe extern \"C\" fn pthread_getattr_np(",
            "pub unsafe extern \"C\" fn pthread_attr_getstack(",
            "pub unsafe extern \"C\" fn gettimeofday(",
        ] {
            assert!(
                abi.contains(symbol),
                "missing Nagi POSIX runtime symbol: {symbol}"
            );
        }
    }

    #[test]
    fn m17_posix_network_abi_has_real_nagi_net_backends() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root");
        let abi =
            fs::read_to_string(root.join("user/nagi-posix/src/abi.rs")).expect("Nagi POSIX ABI");
        let runtime = fs::read_to_string(root.join("user/nagi-posix/src/runtime.rs"))
            .expect("Nagi POSIX runtime");
        assert!(abi.contains("fn getpeername("));
        assert!(abi.contains("fn bind("));
        assert!(abi.contains("fn listen("));
        let network = fs::read_to_string(root.join("user/nagi-net/src/smoltcp_stack.rs"))
            .expect("Nagi smoltcp adapter");
        for symbol in [
            "fn readv(",
            "fn shutdown(",
            "fn setsockopt(",
            "fn getsockname(",
            "fn dirfd(",
            "fn pthread_detach(",
        ] {
            assert!(abi.contains(symbol), "missing target ABI symbol: {symbol}");
        }
        for operation in [
            "pub fn shutdown(",
            "pub fn set_tcp_nodelay(",
            "pub fn set_socket_timeout(",
            "pub fn peer_name(",
        ] {
            assert!(
                runtime.contains(operation),
                "missing runtime operation: {operation}"
            );
        }
        for operation in [
            "pub fn tcp_shutdown_write(",
            "pub fn tcp_set_nagle(",
            "pub fn tcp_set_timeout(",
            "pub fn tcp_local_name(",
        ] {
            assert!(
                network.contains(operation),
                "missing smoltcp operation: {operation}"
            );
        }
    }
}
