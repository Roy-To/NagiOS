//! Content-validated preparation of the pinned Rust standard library source.
//! This caches source preparation only; Cargo builds and guest gates still run.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    time::Instant,
};

const RECIPE: u32 = 1;
const LIBC_PATCH: &str = "\nlibc = { path = \"../../../third_party/libc\" }\n";

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Inputs {
    recipe: u32,
    toolchain: String,
    installed_path: PathBuf,
    source_hash: String,
    patch_hash: String,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stamp {
    inputs: Inputs,
    prepared_hash: String,
}

pub(crate) struct PreparedSource {
    pub library: PathBuf,
    pub reused: bool,
}

fn timing(phase: &str, start: Instant) {
    eprintln!(
        "TIMING rust-std-source phase={phase} elapsed_ms={:.3}",
        start.elapsed().as_secs_f64() * 1000.0
    );
}

pub(crate) fn prepare(
    root: &Path,
    installed_source: &Path,
    toolchain: &str,
    patch: &Path,
) -> Result<PreparedSource, String> {
    let total = Instant::now();
    let out = root.join("out");
    fs::create_dir_all(&out).map_err(|e| format!("cannot create {}: {e}", out.display()))?;
    let lock_path = out.join("rust-src.lock");
    if fs::symlink_metadata(&lock_path).is_ok_and(|m| !m.file_type().is_file()) {
        return Err(format!(
            "Rust source lock is not a regular file: {}",
            lock_path.display()
        ));
    }
    let lock_start = Instant::now();
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|e| format!("cannot open Rust source lock: {e}"))?;
    // OS locks are released even when preparation is killed. A leftover lock
    // file is harmless; a leftover staging tree forces a fresh preparation.
    lock.lock()
        .map_err(|e| format!("cannot lock Rust source preparation: {e}"))?;
    timing("lock", lock_start);

    let generated = out.join("rust-src");
    let staging = out.join("rust-src-preparing");
    let state = out.join("rust-src-state.json");
    let interrupted = fs::symlink_metadata(&staging).is_ok();
    let input_start = Instant::now();
    let inputs = Inputs {
        recipe: RECIPE,
        toolchain: toolchain.to_owned(),
        installed_path: installed_source
            .canonicalize()
            .map_err(|e| format!("cannot resolve installed Rust source: {e}"))?,
        source_hash: tree_hash(installed_source)?,
        patch_hash: file_hash(patch)?,
    };
    timing("inputs", input_start);
    let validation_start = Instant::now();
    let valid = !interrupted
        && read_stamp(&state).is_some_and(|stamp| {
            stamp.inputs == inputs
                && generated.join("library").is_dir()
                && tree_hash(&generated).is_ok_and(|hash| hash == stamp.prepared_hash)
        });
    timing("validate", validation_start);
    if valid {
        // Do not rewrite any source or its timestamps on a verified hit.
        timing("total-reuse", total);
        return Ok(PreparedSource {
            library: generated.join("library"),
            reused: true,
        });
    }

    remove_path(&staging)?;
    let copy_start = Instant::now();
    copy_tree(installed_source, &staging)?;
    timing("copy", copy_start);
    let patch_start = Instant::now();
    let applied = Command::new("git")
        .args([
            "apply",
            "--unsafe-paths",
            "--whitespace=nowarn",
            "--directory=out/rust-src-preparing",
        ])
        .arg(patch)
        .current_dir(root)
        .output()
        .map_err(|e| format!("cannot apply Rust std patch: {e}"))?;
    if !applied.status.success() {
        return Err(format!(
            "Rust std patch failed: {}{}",
            String::from_utf8_lossy(&applied.stdout),
            String::from_utf8_lossy(&applied.stderr)
        ));
    }
    let manifest_path = staging.join("library/Cargo.toml");
    let mut manifest = fs::read_to_string(&manifest_path)
        .map_err(|e| format!("cannot read generated Rust library manifest: {e}"))?;
    if !manifest.contains("../../../third_party/libc") {
        manifest.push_str(LIBC_PATCH);
        fs::write(&manifest_path, manifest)
            .map_err(|e| format!("cannot add Nagi libc patch: {e}"))?;
    }
    timing("patch", patch_start);
    let verify_start = Instant::now();
    let prepared_hash = tree_hash(&staging)?;
    if tree_hash(installed_source)? != inputs.source_hash || file_hash(patch)? != inputs.patch_hash
    {
        return Err(
            "Rust source or patch changed during preparation; retry from fresh inputs".into(),
        );
    }
    timing("verify-prepared", verify_start);

    let publish_start = Instant::now();
    // A complete stamp is published last. Any crash between these operations
    // leaves either a staging tree, no stamp, or a mismatched content digest.
    remove_path(&state)?;
    remove_path(&generated)?;
    fs::rename(&staging, &generated)
        .map_err(|e| format!("cannot publish generated Rust source: {e}"))?;
    let temporary_state = out.join("rust-src-state.json.tmp");
    remove_path(&temporary_state)?;
    let bytes = serde_json::to_vec(&Stamp {
        inputs,
        prepared_hash,
    })
    .map_err(|e| format!("cannot encode Rust source state: {e}"))?;
    let mut stamp = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary_state)
        .map_err(|e| format!("cannot create Rust source state: {e}"))?;
    stamp
        .write_all(&bytes)
        .and_then(|()| stamp.sync_all())
        .map_err(|e| format!("cannot write Rust source state: {e}"))?;
    drop(stamp);
    fs::rename(&temporary_state, &state)
        .map_err(|e| format!("cannot publish Rust source state: {e}"))?;
    timing("publish", publish_start);
    timing("total-rebuild", total);
    Ok(PreparedSource {
        library: generated.join("library"),
        reused: false,
    })
}

fn read_stamp(path: &Path) -> Option<Stamp> {
    if !fs::symlink_metadata(path).ok()?.file_type().is_file() {
        return None;
    }
    serde_json::from_slice(&fs::read(path).ok()?).ok()
}

fn remove_path(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            let result = if meta.file_type().is_dir() {
                fs::remove_dir_all(path)
            } else {
                fs::remove_file(path)
            };
            result.map_err(|e| format!("cannot remove {}: {e}", path.display()))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("cannot inspect {}: {e}", path.display())),
    }
}

fn entries(path: &Path) -> Result<Vec<fs::DirEntry>, String> {
    let mut entries = fs::read_dir(path)
        .map_err(|e| format!("cannot enumerate {}: {e}", path.display()))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("cannot inspect Rust source entry: {e}"))?;
    entries.sort_by_key(|entry| entry.file_name());
    Ok(entries)
}

fn copy_tree(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination)
        .map_err(|e| format!("cannot create {}: {e}", destination.display()))?;
    for entry in entries(source)? {
        let source = entry.path();
        let destination = destination.join(entry.file_name());
        let kind = entry
            .file_type()
            .map_err(|e| format!("cannot inspect {}: {e}", source.display()))?;
        if kind.is_dir() {
            copy_tree(&source, &destination)?;
        } else if kind.is_file() {
            fs::copy(&source, &destination)
                .map_err(|e| format!("cannot copy {}: {e}", source.display()))?;
        } else {
            return Err(format!(
                "Rust source is not a regular file or directory: {}",
                source.display()
            ));
        }
    }
    Ok(())
}

fn file_hash(path: &Path) -> Result<String, String> {
    let mut hash = Sha256::new();
    hash_file(path, &mut hash)?;
    Ok(format!("{:x}", hash.finalize()))
}

fn hash_file(path: &Path, hash: &mut Sha256) -> Result<(), String> {
    let mut file = File::open(path).map_err(|e| format!("cannot read {}: {e}", path.display()))?;
    let mut buffer = [0; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|e| format!("cannot hash {}: {e}", path.display()))?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(())
}

fn tree_hash(root: &Path) -> Result<String, String> {
    fn visit(root: &Path, path: &Path, hash: &mut Sha256) -> Result<(), String> {
        let meta = fs::symlink_metadata(path)
            .map_err(|e| format!("cannot inspect {}: {e}", path.display()))?;
        let relative = path
            .strip_prefix(root)
            .unwrap()
            .components()
            .map(|component| {
                component
                    .as_os_str()
                    .to_str()
                    .ok_or_else(|| format!("non-UTF-8 Rust source path: {}", path.display()))
            })
            .collect::<Result<Vec<_>, _>>()?
            .join("/");
        hash.update((relative.len() as u64).to_le_bytes());
        hash.update(relative.as_bytes());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            hash.update(meta.permissions().mode().to_le_bytes());
        }
        #[cfg(not(unix))]
        hash.update([u8::from(meta.permissions().readonly())]);
        if meta.file_type().is_file() {
            hash.update(b"file");
            hash.update(meta.len().to_le_bytes());
            hash_file(path, hash)?;
        } else if meta.file_type().is_dir() {
            hash.update(b"directory");
            for entry in entries(path)? {
                visit(root, &entry.path(), hash)?;
            }
        } else {
            return Err(format!(
                "Rust source is not a regular file or directory: {}",
                path.display()
            ));
        }
        Ok(())
    }
    let mut hash = Sha256::new();
    hash.update(b"nagi-rust-source-tree-v1\0");
    visit(root, root, &mut hash)?;
    Ok(format!("{:x}", hash.finalize()))
}

#[cfg(test)]
mod tests;
