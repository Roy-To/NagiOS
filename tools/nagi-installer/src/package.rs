use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::path::validate_destination_paths;
use crate::{InstallerError, PackageMetadata, PackageRelativePath};

const MAX_PACKAGE_FILES: usize = 100_000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VirtualFile {
    pub path: String,
    pub contents: Vec<u8>,
    pub executable: bool,
}

#[derive(Clone, Debug)]
pub struct PackageSource {
    pub(crate) metadata: PackageMetadata,
    pub(crate) files: Vec<PackageFile>,
    payload: PackagePayload,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PackageFile {
    pub(crate) path: PackageRelativePath,
    pub(crate) size: u64,
    pub(crate) executable: bool,
    pub(crate) sha256: [u8; 32],
}

#[derive(Clone, Debug)]
enum PackagePayload {
    Directory(PathBuf),
    Memory(BTreeMap<String, Vec<u8>>),
}

impl PackageSource {
    /// Enumerate a directory package without following symlinks or special
    /// files. The format is a directory contract, not an archive commitment.
    pub fn from_directory(
        root: impl AsRef<Path>,
        metadata: PackageMetadata,
    ) -> Result<Self, InstallerError> {
        metadata.validate()?;
        let canonical_root = fs::canonicalize(root.as_ref())
            .map_err(|error| crate::error::io_error("canonicalize package root", error))?;
        if !fs::metadata(&canonical_root)
            .map_err(|error| crate::error::io_error("inspect package root", error))?
            .is_dir()
        {
            return Err(InstallerError::InvalidPackage(
                "package root is not a directory".into(),
            ));
        }

        let mut files = Vec::new();
        enumerate_directory(&canonical_root, &canonical_root, &mut files)?;
        validate_files(&metadata, &files)?;
        Ok(Self {
            metadata,
            files,
            payload: PackagePayload::Directory(canonical_root),
        })
    }

    /// Construct a virtual package source. This is useful for deterministic
    /// fixtures and leaves room for future non-archive sources.
    pub fn from_virtual_files(
        metadata: PackageMetadata,
        files: Vec<VirtualFile>,
    ) -> Result<Self, InstallerError> {
        metadata.validate()?;
        if files.len() > MAX_PACKAGE_FILES {
            return Err(InstallerError::InvalidPackage(format!(
                "package contains more than {MAX_PACKAGE_FILES} files"
            )));
        }
        let mut payload = BTreeMap::new();
        let mut entries = Vec::with_capacity(files.len());
        for file in files {
            let path = PackageRelativePath::parse(&file.path)?;
            if payload
                .insert(path.as_str().to_owned(), file.contents)
                .is_some()
            {
                return Err(InstallerError::DuplicateDestination(path.as_str().into()));
            }
            let contents = payload
                .get(path.as_str())
                .expect("inserted virtual package file");
            entries.push(PackageFile {
                path,
                size: contents.len() as u64,
                executable: file.executable,
                sha256: Sha256::digest(contents).into(),
            });
        }
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        validate_files(&metadata, &entries)?;
        Ok(Self {
            metadata,
            files: entries,
            payload: PackagePayload::Memory(payload),
        })
    }

    pub fn metadata(&self) -> &PackageMetadata {
        &self.metadata
    }

    pub(crate) fn directory_root(&self) -> Option<&Path> {
        match &self.payload {
            PackagePayload::Directory(root) => Some(root),
            PackagePayload::Memory(_) => None,
        }
    }

    pub fn files(&self) -> impl Iterator<Item = (&str, u64, bool, [u8; 32])> {
        self.files
            .iter()
            .map(|file| (file.path.as_str(), file.size, file.executable, file.sha256))
    }

    pub(crate) fn read_file(&self, entry: &PackageFile) -> Result<Vec<u8>, InstallerError> {
        let bytes = match &self.payload {
            PackagePayload::Memory(files) => {
                files.get(entry.path.as_str()).cloned().ok_or_else(|| {
                    InstallerError::InvalidPackage(format!(
                        "virtual package file disappeared: {}",
                        entry.path.as_str()
                    ))
                })?
            }
            PackagePayload::Directory(root) => {
                let path = entry
                    .path
                    .as_str()
                    .split('/')
                    .fold(root.clone(), |joined, segment| joined.join(segment));
                let metadata = fs::symlink_metadata(&path)
                    .map_err(|error| crate::error::io_error("inspect staged source file", error))?;
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(InstallerError::UnsafePath(format!(
                        "package source changed to a link or non-file: {}",
                        entry.path.as_str()
                    )));
                }
                let canonical = fs::canonicalize(&path).map_err(|error| {
                    crate::error::io_error("resolve package source file", error)
                })?;
                if !canonical.starts_with(root) {
                    return Err(InstallerError::UnsafePath(format!(
                        "package source escaped its root: {}",
                        entry.path.as_str()
                    )));
                }
                fs::read(path)
                    .map_err(|error| crate::error::io_error("read package source file", error))?
            }
        };
        if bytes.len() as u64 != entry.size {
            return Err(InstallerError::InvalidPackage(format!(
                "package source file changed size: {}",
                entry.path.as_str()
            )));
        }
        let digest: [u8; 32] = Sha256::digest(&bytes).into();
        if digest != entry.sha256 {
            return Err(InstallerError::InvalidPackage(format!(
                "package source content changed after validation: {}",
                entry.path.as_str()
            )));
        }
        Ok(bytes)
    }
}

fn validate_files(metadata: &PackageMetadata, files: &[PackageFile]) -> Result<(), InstallerError> {
    if files.is_empty() {
        return Err(InstallerError::InvalidPackage(
            "package contains no files".into(),
        ));
    }
    if files.len() > MAX_PACKAGE_FILES {
        return Err(InstallerError::InvalidPackage(format!(
            "package contains more than {MAX_PACKAGE_FILES} files"
        )));
    }
    validate_destination_paths(files.iter().map(|file| &file.path))?;
    if !files
        .iter()
        .any(|file| file.path.as_str() == metadata.entrypoint)
    {
        return Err(InstallerError::InvalidPackage(format!(
            "entrypoint {:?} is not present in package contents",
            metadata.entrypoint
        )));
    }
    Ok(())
}

fn enumerate_directory(
    root: &Path,
    directory: &Path,
    output: &mut Vec<PackageFile>,
) -> Result<(), InstallerError> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| crate::error::io_error("enumerate package directory", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| crate::error::io_error("read package directory entry", error))?;
    entries.sort_by_key(|entry| entry.file_name());

    for entry in entries {
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| crate::error::io_error("inspect package entry", error))?;
        if file_type.is_symlink() {
            return Err(InstallerError::UnsafePath(format!(
                "package source contains a symlink: {}",
                path.display()
            )));
        }
        if file_type.is_dir() {
            enumerate_directory(root, &path, output)?;
            continue;
        }
        if !file_type.is_file() {
            return Err(InstallerError::InvalidPackage(format!(
                "package contains an unsupported special file: {}",
                path.display()
            )));
        }
        let relative = path.strip_prefix(root).map_err(|_| {
            InstallerError::UnsafePath("package entry escaped its source root".into())
        })?;
        let relative = relative
            .to_str()
            .ok_or_else(|| InstallerError::UnsafePath("package path is not valid UTF-8".into()))?;
        let normalized = relative.replace(std::path::MAIN_SEPARATOR, "/");
        let relative_path = PackageRelativePath::parse(&normalized)?;
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| crate::error::io_error("inspect package file", error))?;
        #[cfg(unix)]
        let executable = {
            use std::os::unix::fs::PermissionsExt;
            metadata.permissions().mode() & 0o111 != 0
        };
        #[cfg(not(unix))]
        let executable = false;
        let sha256 = hash_file(&path)?;
        output.push(PackageFile {
            path: relative_path,
            size: metadata.len(),
            executable,
            sha256,
        });
        if output.len() > MAX_PACKAGE_FILES {
            return Err(InstallerError::InvalidPackage(format!(
                "package contains more than {MAX_PACKAGE_FILES} files"
            )));
        }
    }
    Ok(())
}

fn hash_file(path: &Path) -> Result<[u8; 32], InstallerError> {
    let mut file = fs::File::open(path)
        .map_err(|error| crate::error::io_error("open package file for validation", error))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| crate::error::io_error("read package file for validation", error))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hasher.finalize().into())
}
