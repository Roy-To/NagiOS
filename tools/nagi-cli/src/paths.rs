use std::fs;
use std::path::{Path, PathBuf};

/// Convert Windows extended-length paths to paths accepted by Git for
/// external command arguments. Filesystem APIs accept the `\\?\` prefix, but
/// Git for Windows treats it as part of a POSIX-looking path when it is passed
/// as a patch filename (for example `//?/D:/repo/file.patch`).
pub fn external_command_path(path: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        let value = path.as_os_str().to_string_lossy();
        if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
            return PathBuf::from(format!(r"\\{rest}"));
        }
        if let Some(rest) = value.strip_prefix(r"\\?\") {
            return PathBuf::from(rest);
        }
    }
    path.to_path_buf()
}

pub fn discover_repo_root(start: &Path) -> Result<PathBuf, String> {
    let mut current = if start.is_file() {
        start
            .parent()
            .ok_or_else(|| "start path has no parent".to_owned())?
            .to_path_buf()
    } else {
        start.to_path_buf()
    };
    if current.as_os_str().is_empty() {
        current = PathBuf::from(".");
    }
    let current = fs::canonicalize(&current)
        .map_err(|error| format!("cannot resolve repository path: {error}"))?;

    for candidate in current.ancestors() {
        if candidate.join("Cargo.toml").is_file() && candidate.join("nagi.toml").is_file() {
            return Ok(candidate.to_path_buf());
        }
    }
    Err("could not find Cargo.toml and nagi.toml in this path or its parents".to_owned())
}

pub fn clean_owned_outputs(root: &Path) -> Result<usize, String> {
    let root = fs::canonicalize(root).map_err(|error| format!("cannot resolve root: {error}"))?;
    let owned = [root.join("target"), root.join("out")];
    #[cfg(windows)]
    let current_executable = std::env::current_exe()
        .ok()
        .and_then(|path| fs::canonicalize(path).ok());
    let mut removed = 0;
    for path in owned {
        #[cfg(windows)]
        if path.file_name() == Some(std::ffi::OsStr::new("target"))
            && current_executable
                .as_ref()
                .is_some_and(|executable| executable.starts_with(&path))
        {
            continue;
        }
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(format!("cannot inspect {}: {error}", path.display()));
            }
        };
        if metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
            return Err(format!(
                "refusing to clean symbolic link or junction: {}",
                path.display()
            ));
        }
        if !metadata.is_dir() {
            return Err(format!(
                "refusing to clean non-directory path: {}",
                path.display()
            ));
        }
        if !path.exists() {
            continue;
        }
        let resolved = fs::canonicalize(&path)
            .map_err(|error| format!("cannot resolve {}: {error}", path.display()))?;
        if resolved != path || !resolved.starts_with(&root) || resolved == root {
            return Err(format!(
                "refusing to clean redirected path outside its owned directory: {}",
                resolved.display()
            ));
        }
        fs::remove_dir_all(&resolved)
            .map_err(|error| format!("cannot remove {}: {error}", resolved.display()))?;
        removed += 1;
    }
    Ok(removed)
}

pub fn ensure_owned_directory(root: &Path, relative: impl AsRef<Path>) -> Result<PathBuf, String> {
    let root = fs::canonicalize(root).map_err(|error| format!("cannot resolve root: {error}"))?;
    let relative = relative.as_ref();
    if relative.is_absolute() {
        return Err("owned output path must be relative to the repository".to_owned());
    }

    let mut current = root.clone();
    for component in relative.components() {
        let name = match component {
            std::path::Component::Normal(name) => name,
            _ => return Err("owned output path contains an invalid component".to_owned()),
        };
        current.push(name);
        let metadata = match fs::symlink_metadata(&current) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(|create_error| {
                    format!(
                        "cannot create owned output directory {}: {create_error}",
                        current.display()
                    )
                })?;
                fs::symlink_metadata(&current).map_err(|inspect_error| {
                    format!(
                        "cannot inspect owned output directory {}: {inspect_error}",
                        current.display()
                    )
                })?
            }
            Err(error) => {
                return Err(format!(
                    "cannot inspect owned output directory {}: {error}",
                    current.display()
                ))
            }
        };
        if metadata.file_type().is_symlink() || is_reparse_point(&metadata) {
            return Err(format!(
                "refusing to use symbolic link or junction as owned output directory: {}",
                current.display()
            ));
        }
        if !metadata.is_dir() {
            return Err(format!(
                "refusing to use non-directory as owned output path: {}",
                current.display()
            ));
        }
    }

    let resolved = fs::canonicalize(&current).map_err(|error| {
        format!(
            "cannot resolve owned output directory {}: {error}",
            current.display()
        )
    })?;
    if resolved != current || !resolved.starts_with(&root) || resolved == root {
        return Err(format!(
            "refusing to use redirected owned output directory: {}",
            resolved.display()
        ));
    }
    Ok(current)
}

fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;

        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        let _ = metadata;
        false
    }
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    #[test]
    fn clean_rejects_a_symlinked_owned_directory() {
        use super::clean_owned_outputs;
        use std::fs;
        use std::os::unix::fs::symlink;

        let root = std::env::temp_dir().join(format!("nagi-clean-{}", std::process::id()));
        fs::create_dir_all(root.join("docs")).expect("docs");
        symlink(root.join("docs"), root.join("out")).expect("out symlink");

        let result = clean_owned_outputs(&root);
        let _ = fs::remove_dir_all(&root);

        assert!(result.is_err());
    }

    #[cfg(unix)]
    #[test]
    fn ensure_owned_directory_rejects_a_symlinked_component() {
        use super::ensure_owned_directory;
        use std::fs;
        use std::os::unix::fs::symlink;
        use std::path::Path;

        let root = std::env::temp_dir().join(format!("nagi-owned-{}", std::process::id()));
        fs::create_dir_all(root.join("outside")).expect("outside");
        symlink(root.join("outside"), root.join("out")).expect("out symlink");

        let result = ensure_owned_directory(&root, Path::new("out").join("artifacts"));
        let _ = fs::remove_dir_all(&root);

        assert!(result.is_err());
    }
}
