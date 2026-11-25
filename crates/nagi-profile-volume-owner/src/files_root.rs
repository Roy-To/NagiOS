//! Scoped, Linux-only filesystem operations for an A-issued User Files root.
//!
//! File descriptors, directory streams and iterators are created only for the
//! duration of a view method or its nested callback. Returned entries and
//! metadata are plain owned values.

use super::linux_data;
use rustix::fs::{fsync, Mode, OFlags};
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::fd::{AsFd, BorrowedFd, OwnedFd};
use std::path::Path;

/// The metadata category for a Files root entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FilesRootEntryKind {
    RegularFile,
    Directory,
    Symlink,
    Other,
}

/// Handle-free metadata for an entry on the owner-selected Data mount.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FilesRootMetadata {
    pub kind: FilesRootEntryKind,
    pub device: u64,
    pub inode: u64,
    pub size_bytes: u64,
    pub mode: u32,
    pub uid: u32,
    pub gid: u32,
    pub created_at_seconds: Option<i64>,
    pub modified_at_seconds: Option<i64>,
}

/// A plain directory entry value. No Rustix iterator or descriptor escapes.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilesRootEntry {
    pub name: String,
    pub metadata: FilesRootMetadata,
}

/// A Files root view borrowed only for one FilesRootScope callback.
pub struct FilesRootView<'root> {
    directory: &'root OwnedFd,
}

impl FilesRootView<'_> {
    pub(crate) fn new<'root>(directory: &'root OwnedFd) -> FilesRootView<'root> {
        FilesRootView { directory }
    }

    /// Sync the owner-selected root directory without exposing its descriptor.
    pub fn sync_root(&self) -> io::Result<()> {
        fsync(self.directory).map_err(io::Error::from)
    }

    /// Sync a directory beneath this root, or the root itself for empty or dot.
    pub fn sync_directory(&self, relative: &Path) -> io::Result<()> {
        linux_data::sync_files_directory(self.directory, relative)
    }

    /// Sync the parent directory of a relative entry.
    pub fn sync_parent(&self, relative: &Path) -> io::Result<()> {
        linux_data::sync_files_parent(self.directory, relative)
    }

    /// Return immediate child names and metadata as owned, handle-free values.
    pub fn read_directory(&self, relative: &Path) -> io::Result<Vec<FilesRootEntry>> {
        linux_data::read_files_directory(self.directory, relative)
    }

    /// Alias for read_directory, named for Files provider listing.
    pub fn list(&self, relative: &Path) -> io::Result<Vec<FilesRootEntry>> {
        self.read_directory(relative)
    }

    /// Read metadata without following the final symlink.
    pub fn symlink_metadata(&self, relative: &Path) -> io::Result<FilesRootMetadata> {
        linux_data::files_symlink_metadata(self.directory, relative)
    }

    /// Alias for symlink_metadata for callers that already reject links.
    pub fn metadata(&self, relative: &Path) -> io::Result<FilesRootMetadata> {
        self.symlink_metadata(relative)
    }

    /// Create a new private file exclusively, write all bytes, sync it and its
    /// parent, and return only metadata.
    pub fn create_file_new(
        &self,
        relative: &Path,
        contents: &[u8],
    ) -> io::Result<FilesRootMetadata> {
        linux_data::create_files_file(self.directory, relative, contents)
    }

    /// Truncate and durably update an existing regular file after checking its
    /// identity has not changed between metadata lookup and open.
    pub fn write_existing_file(
        &self,
        relative: &Path,
        contents: &[u8],
    ) -> io::Result<FilesRootMetadata> {
        linux_data::write_existing_files_file(self.directory, relative, contents)
    }

    /// Create one directory and sync its parent.
    pub fn create_directory(&self, relative: &Path) -> io::Result<()> {
        linux_data::create_files_directory(self.directory, relative)
    }

    /// Atomically rename an entry without replacing an existing destination.
    /// Directory trees are checked for links and mount crossings first.
    pub fn rename_without_replacement(
        &self,
        source: &Path,
        destination: &Path,
    ) -> io::Result<FilesRootMetadata> {
        linux_data::rename_files_entry(self.directory, source, destination)
    }

    /// Unlink one file or symlink entry without following it, then sync its
    /// parent directory.
    pub fn unlink(&self, relative: &Path) -> io::Result<()> {
        linux_data::unlink_files_entry(self.directory, relative)
    }

    /// Remove one empty directory and sync its parent.
    pub fn rmdir(&self, relative: &Path) -> io::Result<()> {
        linux_data::remove_empty_files_directory(self.directory, relative)
    }

    /// Recursively remove a regular-file/directory tree without following
    /// symlinks or traversing another mount, then sync its parent.
    pub fn remove_path(&self, relative: &Path) -> io::Result<()> {
        linux_data::remove_files_path(self.directory, relative)
    }

    /// Recursively copy a file or directory tree. Cancellation removes the
    /// partial destination; symlinks and cross-mount traversal fail closed.
    pub fn copy_path(
        &self,
        source: &Path,
        destination: &Path,
        is_cancelled: impl FnMut() -> bool,
    ) -> io::Result<FilesRootMetadata> {
        linux_data::copy_files_path(self.directory, source, destination, is_cancelled)
    }

    /// Check an entire subtree for symlinks and mount crossings.
    pub fn validate_tree_mounts(&self, relative: &Path) -> io::Result<()> {
        linux_data::validate_files_tree(self.directory, relative)
    }

    /// Write and sync a temporary file at one root-relative path, atomically
    /// replace another path in the same parent, and sync that parent. Existing
    /// temporary files or symlinks are removed first; other entry types fail
    /// closed. This supports both Files trash and tags indexes.
    pub fn write_temp_and_replace(
        &self,
        temporary: &Path,
        destination: &Path,
        contents: &[u8],
    ) -> io::Result<()> {
        if temporary == destination {
            return Err(io::ErrorKind::InvalidInput.into());
        }
        match self.symlink_metadata(temporary) {
            Ok(metadata)
                if matches!(
                    metadata.kind,
                    FilesRootEntryKind::RegularFile | FilesRootEntryKind::Symlink
                ) =>
            {
                self.unlink(temporary)?;
            }
            Ok(_) => return Err(io::ErrorKind::InvalidData.into()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        self.create_file_new(temporary, contents)?;
        linux_data::replace_files_temp(self.directory, temporary, destination)
    }

    /// Open a file under the root and expose only a scoped file-operation view.
    ///
    /// The file view cannot be returned from its callback:
    ///
    /// ```compile_fail
    /// use nagi_profile_volume_notes_owner::{FilesRootFileView, FilesRootScope};
    /// use rustix::fs::{Mode, OFlags};
    /// use std::path::Path;
    ///
    /// fn escape_file(
    ///     scope: &impl FilesRootScope,
    /// ) -> &'static mut FilesRootFileView<'static> {
    ///     scope.with_files_root(|view| {
    ///         view.with_open_file(Path::new("note"), OFlags::RDONLY, Mode::empty(), |file| {
    ///             Ok(file)
    ///         }).unwrap()
    ///     }).unwrap()
    /// }
    /// ```
    pub fn with_open_file<R>(
        &self,
        relative: &Path,
        flags: OFlags,
        mode: Mode,
        effect: impl for<'file> FnOnce(&mut FilesRootFileView<'file>) -> io::Result<R>,
    ) -> io::Result<R> {
        with_open_file(self.directory, relative, flags, mode, effect)
    }

    /// Open a directory under the root for one callback. The view offers list,
    /// metadata, nested opens and sync without returning a File, Dir or iterator.
    pub fn with_open_dir<R>(
        &self,
        relative: &Path,
        effect: impl for<'dir> FnOnce(FilesRootDirView<'dir>) -> io::Result<R>,
    ) -> io::Result<R> {
        if is_root_path(relative) {
            return effect(FilesRootDirView {
                directory: self.directory.as_fd(),
            });
        }
        let directory = linux_data::open_files_path(
            self.directory,
            relative,
            OFlags::RDONLY | OFlags::DIRECTORY,
            Mode::empty(),
        )?;
        effect(FilesRootDirView {
            directory: directory.as_fd(),
        })
    }
}

/// A directory view borrowed only for one open-directory callback.
pub struct FilesRootDirView<'dir> {
    directory: BorrowedFd<'dir>,
}

impl FilesRootDirView<'_> {
    /// List immediate children as plain values. No iterator or handle escapes.
    pub fn entries(&self) -> io::Result<Vec<FilesRootEntry>> {
        linux_data::read_files_directory(&self.directory, Path::new("."))
    }

    /// Metadata for this opened directory.
    pub fn metadata(&self) -> io::Result<FilesRootMetadata> {
        linux_data::files_symlink_metadata(&self.directory, Path::new("."))
    }

    /// Sync this opened directory.
    pub fn sync(&self) -> io::Result<()> {
        fsync(self.directory).map_err(io::Error::from)
    }

    /// Open a child directory for one nested traversal callback.
    pub fn with_open_dir<R>(
        &self,
        relative: &Path,
        effect: impl for<'nested> FnOnce(FilesRootDirView<'nested>) -> io::Result<R>,
    ) -> io::Result<R> {
        if is_root_path(relative) {
            return effect(FilesRootDirView {
                directory: self.directory,
            });
        }
        let directory = linux_data::open_files_path(
            &self.directory,
            relative,
            OFlags::RDONLY | OFlags::DIRECTORY,
            Mode::empty(),
        )?;
        effect(FilesRootDirView {
            directory: directory.as_fd(),
        })
    }

    /// Open a file relative to this directory for one scoped file callback.
    pub fn with_open_file<R>(
        &self,
        relative: &Path,
        flags: OFlags,
        mode: Mode,
        effect: impl for<'file> FnOnce(&mut FilesRootFileView<'file>) -> io::Result<R>,
    ) -> io::Result<R> {
        with_open_file(&self.directory, relative, flags, mode, effect)
    }
}

/// A borrowed file handle scoped to one open-file callback. It exposes normal
/// file operations, but not the underlying descriptor or File.
pub struct FilesRootFileView<'file> {
    file: &'file mut File,
}

impl FilesRootFileView<'_> {
    pub fn sync_all(&mut self) -> io::Result<()> {
        self.file.sync_all()
    }

    pub fn sync_data(&mut self) -> io::Result<()> {
        self.file.sync_data()
    }

    pub fn metadata(&self) -> io::Result<std::fs::Metadata> {
        self.file.metadata()
    }

    pub fn set_len(&self, size: u64) -> io::Result<()> {
        self.file.set_len(size)
    }
}

impl Read for FilesRootFileView<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        Read::read(self.file, buffer)
    }
}

impl Write for FilesRootFileView<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        Write::write(self.file, buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        Write::flush(self.file)
    }
}

impl Seek for FilesRootFileView<'_> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        Seek::seek(self.file, position)
    }
}

fn with_open_file<R>(
    directory: &impl AsFd,
    relative: &Path,
    flags: OFlags,
    mode: Mode,
    effect: impl for<'file> FnOnce(&mut FilesRootFileView<'file>) -> io::Result<R>,
) -> io::Result<R> {
    let mut file = linux_data::open_files_path(directory, relative, flags, mode)?;
    effect(&mut FilesRootFileView { file: &mut file })
}

fn is_root_path(relative: &Path) -> bool {
    relative.as_os_str().is_empty() || relative == Path::new(".")
}
