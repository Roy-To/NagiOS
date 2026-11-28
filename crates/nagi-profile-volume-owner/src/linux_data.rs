//! Volume Manager storage for the boot-selected development Data mount.

use super::{
    AppStorageKind, FilesRootEntry, FilesRootEntryKind, FilesRootMetadata, NOTES_APP_GID,
    NOTES_APP_UID, USER_FILES_APP_GID, USER_FILES_APP_UID,
};
use nagi_model::{ObjectId, UserId};
use rustix::fs::{
    fchown, flock, fsync, linkat, mkdirat, openat2, renameat, renameat_with, statx, unlinkat,
    AtFlags, Dir, FileType, FlockOperation, Gid, Mode, OFlags, RenameFlags, ResolveFlags,
    StatxFlags, Uid,
};
use rustix::io::Errno;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

const OBJECT_DIRECTORY: &str = "objects";
const FILES_ROOT: &str = "root";
const NOTES_APP_ROOT: &str = "app-root";
const NOTES_APP_ROOT_RESERVATION: &str = "app-root.v2";
const LEGACY_NOTES_APP_ROOT_RESERVATION: &str = "app-root.v1";
const RESERVATION: &str = "reservation.v1";
const MAX_ID_BYTES: usize = 128;
const MAX_RECORD_BYTES: u64 = 512;

fn storage_layout(kind: AppStorageKind) -> (&'static str, &'static str) {
    match kind {
        AppStorageKind::Notes => ("profile-notes-v1", "NAGI-NOTES-PROFILE-2"),
        AppStorageKind::Settings => ("profile-settings-v1", "NAGI-SETTINGS-PROFILE-1"),
        AppStorageKind::Calendar => ("profile-calendar-v1", "NAGI-CALENDAR-PROFILE-1"),
        AppStorageKind::UserFiles => ("profile-files-v1", "NAGI-FILES-PROFILE-1"),
    }
}

fn parse_reservation(
    kind: AppStorageKind,
    scope: &str,
    record: &str,
) -> io::Result<(ObjectId, bool)> {
    let (_, heading) = storage_layout(kind);
    let (observed_heading, record) = record.split_once('\n').ok_or(io::ErrorKind::InvalidData)?;
    if kind == AppStorageKind::Notes && observed_heading == "NAGI-NOTES-PROFILE-1" {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "legacy Notes reservation v1 is unsupported",
        ));
    }
    if observed_heading != heading {
        return Err(io::ErrorKind::InvalidData.into());
    }
    let hex = record
        .strip_prefix(scope)
        .and_then(|suffix| suffix.strip_suffix('\n'))
        .ok_or(io::ErrorKind::InvalidData)?;
    let object = u64::from_str_radix(hex, 16).map_err(|_| io::ErrorKind::InvalidData)?;
    if object == 0 || hex != format!("{object:016x}") {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok((ObjectId(object), true))
}

pub(super) fn reserve(
    data: &impl AsFd,
    selected_mount_id: u64,
    selected_data_identity_sha256: &[u8; 32],
    user: UserId,
    profile: &str,
    principal: &str,
    kind: AppStorageKind,
) -> io::Result<(OwnedFd, ObjectId, bool, Option<OwnedFd>)> {
    if kind == AppStorageKind::UserFiles {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let profile_root = open_profile_root(data, selected_mount_id, user, profile, principal, kind)?;
    let (_, heading) = storage_layout(kind);
    flock(&profile_root, FlockOperation::NonBlockingLockExclusive)?;
    let objects = private_directory(&profile_root, OBJECT_DIRECTORY)?;
    let scope = format!("{:016x}\n{profile}\n{principal}\n", user.0);
    let (object, allow_initial_index) = match open_beneath(
        &profile_root,
        RESERVATION,
        OFlags::RDONLY | OFlags::NONBLOCK,
    ) {
        Ok(fd) => {
            let file = File::from(fd);
            let metadata = file.metadata()?;
            if !metadata.is_file()
                || metadata.uid() != 0
                || metadata.mode() & 0o777 != 0o600
                || metadata.nlink() != 1
            {
                return Err(io::ErrorKind::PermissionDenied.into());
            }
            let mut record = String::new();
            (&file)
                .take(MAX_RECORD_BYTES + 1)
                .read_to_string(&mut record)?;
            let reservation = parse_reservation(kind, &scope, &record)?;
            file.sync_all()?;
            reservation
        }
        Err(Errno::NOENT) => {
            // A profile with existing objects needs its original reservation.
            for entry in Dir::read_from(&objects)? {
                let entry = entry?;
                if !matches!(entry.file_name().to_bytes(), b"." | b"..") {
                    return Err(io::ErrorKind::InvalidData.into());
                }
            }
            let mut random = [0; 8];
            File::open("/dev/urandom")?.read_exact(&mut random)?;
            let object = u64::from_le_bytes(random);
            if object == 0 {
                return Err(io::ErrorKind::WouldBlock.into());
            }
            let mut pending = File::from(rustix::fs::openat(
                &profile_root,
                ".",
                OFlags::TMPFILE | OFlags::WRONLY | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            )?);
            writeln!(pending, "{heading}\n{scope}{object:016x}")?;
            pending.sync_all()?;
            // Atomic publication without replacing an existing reservation.
            linkat(
                &pending,
                c"",
                &profile_root,
                RESERVATION,
                AtFlags::EMPTY_PATH,
            )?;
            (ObjectId(object), true)
        }
        Err(error) => return Err(error.into()),
    };
    let app_process_directory = if kind == AppStorageKind::Notes {
        Some(reserve_notes_app_process_root(
            &profile_root,
            selected_mount_id,
            selected_data_identity_sha256,
            user,
            profile,
            principal,
            NOTES_APP_UID,
            NOTES_APP_GID,
        )?)
    } else {
        None
    };
    profile_root.sync_all()?;
    Ok((
        objects.into(),
        object,
        allow_initial_index,
        app_process_directory.map(OwnedFd::from),
    ))
}

fn open_profile_root(
    data: &impl AsFd,
    selected_mount_id: u64,
    user: UserId,
    profile: &str,
    principal: &str,
    kind: AppStorageKind,
) -> io::Result<File> {
    if user.0 == 0
        || !valid_label(profile.strip_prefix("profile_").unwrap_or(""), false)
        || !valid_label(principal, true)
    {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    require_same_mount(data, selected_mount_id)?;
    let metadata = rustix::fs::fstat(data)?;
    if metadata.st_uid != 0 || metadata.st_mode & 0o022 != 0 {
        return Err(io::ErrorKind::PermissionDenied.into());
    }

    let (profile_directory, _) = storage_layout(kind);
    let profiles = private_directory(data, profile_directory)?;
    private_directory(&profiles, profile)
}

pub(super) fn reserve_files_root(
    data: &impl AsFd,
    selected_mount_id: u64,
    user: UserId,
    profile: &str,
    principal: &str,
) -> io::Result<OwnedFd> {
    let kind = AppStorageKind::UserFiles;
    let profile_root = open_profile_root(data, selected_mount_id, user, profile, principal, kind)?;
    flock(&profile_root, FlockOperation::NonBlockingLockExclusive)?;
    let (root, existing) = match open_beneath(
        &profile_root,
        RESERVATION,
        OFlags::RDONLY | OFlags::NONBLOCK,
    ) {
        Ok(record) => (
            open_files_private_directory(&profile_root, FILES_ROOT)?,
            Some(File::from(record)),
        ),
        Err(Errno::NOENT) => {
            let root = provision_empty_files_directory(&profile_root, FILES_ROOT)?;
            (root, None)
        }
        Err(error) => return Err(error.into()),
    };
    require_same_mount(&root, selected_mount_id)?;
    let metadata = root.metadata()?;
    let (_, heading) = storage_layout(kind);
    let expected = format!(
        "{heading}\n{:016x}\n{profile}\n{principal}\n{:016x}\n{:016x}\n",
        user.0,
        metadata.dev(),
        metadata.ino(),
    );

    if let Some(record) = existing {
        let attributes = record.metadata()?;
        if !attributes.is_file()
            || attributes.uid() != 0
            || attributes.mode() & 0o777 != 0o600
            || attributes.nlink() != 1
        {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        let mut observed = String::new();
        (&record)
            .take(MAX_RECORD_BYTES + 1)
            .read_to_string(&mut observed)?;
        if observed != expected {
            return Err(io::ErrorKind::InvalidData.into());
        }
        record.sync_all()?;
    } else {
        let mut pending = File::from(rustix::fs::openat(
            &profile_root,
            ".",
            OFlags::TMPFILE | OFlags::WRONLY | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )?);
        pending.write_all(expected.as_bytes())?;
        pending.sync_all()?;
        linkat(
            &pending,
            c"",
            &profile_root,
            RESERVATION,
            AtFlags::EMPTY_PATH,
        )?;
    }
    profile_root.sync_all()?;
    Ok(root.into())
}

pub(super) fn open_files_path(
    root: &impl AsFd,
    relative: &Path,
    flags: OFlags,
    mode: Mode,
) -> io::Result<File> {
    if relative.as_os_str().is_empty()
        || !relative
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let mount_id = files_mount_id(root)?;
    let file = openat2(
        root,
        relative,
        flags | OFlags::CLOEXEC,
        mode,
        files_resolve_flags(),
    )?;
    require_files_mount(&file, mount_id)?;
    Ok(File::from(file))
}

const fn files_resolve_flags() -> ResolveFlags {
    ResolveFlags::BENEATH
        .union(ResolveFlags::NO_MAGICLINKS)
        .union(ResolveFlags::NO_SYMLINKS)
        .union(ResolveFlags::NO_XDEV)
}

fn validate_files_path(path: &Path, allow_root: bool) -> io::Result<()> {
    let bytes = path.as_os_str().as_bytes();
    if bytes.is_empty() && allow_root {
        return Ok(());
    }
    if bytes == b"." && allow_root {
        return Ok(());
    }
    if path.is_absolute()
        || bytes.is_empty()
        || bytes
            .split(|byte| *byte == b'/')
            .any(|part| part.is_empty() || part == b"." || part == b"..")
    {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    Ok(())
}

fn files_mount_id(fd: &impl AsFd) -> io::Result<u64> {
    let stat = statx(fd, "", AtFlags::EMPTY_PATH, StatxFlags::MNT_ID)?;
    if stat.stx_mask & StatxFlags::MNT_ID.bits() == 0 {
        return Err(io::ErrorKind::Unsupported.into());
    }
    Ok(stat.stx_mnt_id)
}

fn require_files_mount(fd: &impl AsFd, expected: u64) -> io::Result<()> {
    if files_mount_id(fd)? != expected {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    Ok(())
}

fn open_files_directory(root: &impl AsFd, mount_id: u64, relative: &Path) -> io::Result<OwnedFd> {
    validate_files_path(relative, true)?;
    let path = if relative.as_os_str().is_empty() {
        Path::new(".")
    } else {
        relative
    };
    let directory = openat2(
        root,
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
        files_resolve_flags(),
    )?;
    require_files_mount(&directory, mount_id)?;
    Ok(directory)
}

fn files_parent_and_name(
    root: &impl AsFd,
    mount_id: u64,
    relative: &Path,
) -> io::Result<(OwnedFd, std::ffi::OsString)> {
    validate_files_path(relative, false)?;
    let name = relative
        .file_name()
        .filter(|name| name.as_bytes() != b"." && name.as_bytes() != b"..")
        .ok_or(io::ErrorKind::InvalidInput)?
        .to_os_string();
    let parent = relative.parent().unwrap_or_else(|| Path::new(""));
    Ok((open_files_directory(root, mount_id, parent)?, name))
}

fn statx_at(parent: &impl AsFd, name: &std::ffi::OsStr) -> io::Result<rustix::fs::Statx> {
    let requested = StatxFlags::BASIC_STATS | StatxFlags::MNT_ID;
    let stat = statx(parent, name, AtFlags::SYMLINK_NOFOLLOW, requested)?;
    if stat.stx_mask & (StatxFlags::TYPE | StatxFlags::INO | StatxFlags::MNT_ID).bits()
        != (StatxFlags::TYPE | StatxFlags::INO | StatxFlags::MNT_ID).bits()
    {
        return Err(io::ErrorKind::Unsupported.into());
    }
    Ok(stat)
}

fn statx_fd(fd: &impl AsFd) -> io::Result<rustix::fs::Statx> {
    let requested = StatxFlags::BASIC_STATS | StatxFlags::MNT_ID;
    let stat = statx(fd, "", AtFlags::EMPTY_PATH, requested)?;
    if stat.stx_mask & requested.bits() != requested.bits() {
        return Err(io::ErrorKind::Unsupported.into());
    }
    Ok(stat)
}

fn linux_device_number(major: u32, minor: u32) -> u64 {
    (u64::from(major & 0x0000_0fff) << 8)
        | u64::from(minor & 0x0000_00ff)
        | (u64::from(minor & 0xffff_ff00) << 12)
        | (u64::from(major & 0xffff_f000) << 32)
}

fn files_metadata(stat: &rustix::fs::Statx) -> FilesRootMetadata {
    let file_type = FileType::from_raw_mode(stat.stx_mode.into());
    let kind = if file_type.is_file() {
        FilesRootEntryKind::RegularFile
    } else if file_type.is_dir() {
        FilesRootEntryKind::Directory
    } else if file_type.is_symlink() {
        FilesRootEntryKind::Symlink
    } else {
        FilesRootEntryKind::Other
    };
    FilesRootMetadata {
        kind,
        device: linux_device_number(stat.stx_dev_major, stat.stx_dev_minor),
        inode: stat.stx_ino,
        size_bytes: stat.stx_size,
        mode: u32::from(stat.stx_mode),
        uid: stat.stx_uid,
        gid: stat.stx_gid,
        created_at_seconds: (stat.stx_mask & StatxFlags::BTIME.bits() != 0)
            .then_some(stat.stx_btime.tv_sec),
        modified_at_seconds: (stat.stx_mask & StatxFlags::MTIME.bits() != 0)
            .then_some(stat.stx_mtime.tv_sec),
    }
}

fn files_symlink_metadata_at(
    root: &impl AsFd,
    mount_id: u64,
    relative: &Path,
) -> io::Result<FilesRootMetadata> {
    let (parent, name) = files_parent_and_name(root, mount_id, relative)?;
    let stat = statx_at(&parent, &name)?;
    if stat.stx_mnt_id != mount_id {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    Ok(files_metadata(&stat))
}

pub(super) fn read_files_directory(
    root: &impl AsFd,
    relative: &Path,
) -> io::Result<Vec<FilesRootEntry>> {
    let mount_id = files_mount_id(root)?;
    let directory = open_files_directory(root, mount_id, relative)?;
    let entries = Dir::read_from(&directory)?;
    let mut values = Vec::new();
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name();
        if matches!(name.to_bytes(), b"." | b"..") {
            continue;
        }
        let name = name
            .to_str()
            .map_err(|_| io::ErrorKind::InvalidData)?
            .to_owned();
        let child = if relative.as_os_str().is_empty() || relative == Path::new(".") {
            PathBuf::from(&name)
        } else {
            relative.join(&name)
        };
        values.push(FilesRootEntry {
            name,
            metadata: files_symlink_metadata_at(root, mount_id, &child)?,
        });
    }
    Ok(values)
}

pub(super) fn files_symlink_metadata(
    root: &impl AsFd,
    relative: &Path,
) -> io::Result<FilesRootMetadata> {
    let mount_id = files_mount_id(root)?;
    if relative.as_os_str().is_empty() || relative == Path::new(".") {
        validate_files_path(relative, true)?;
        let stat = statx_fd(root)?;
        if stat.stx_mnt_id != mount_id {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        return Ok(files_metadata(&stat));
    }
    files_symlink_metadata_at(root, mount_id, relative)
}

fn sync_files_directory_at(root: &impl AsFd, mount_id: u64, relative: &Path) -> io::Result<()> {
    let directory = open_files_directory(root, mount_id, relative)?;
    rustix::fs::fsync(&directory)?;
    Ok(())
}

pub(super) fn sync_files_directory(root: &impl AsFd, relative: &Path) -> io::Result<()> {
    let mount_id = files_mount_id(root)?;
    sync_files_directory_at(root, mount_id, relative)
}

pub(super) fn sync_files_parent(root: &impl AsFd, relative: &Path) -> io::Result<()> {
    let mount_id = files_mount_id(root)?;
    validate_files_path(relative, false)?;
    let parent = relative.parent().unwrap_or_else(|| Path::new(""));
    sync_files_directory_at(root, mount_id, parent)
}

pub(super) fn create_files_file(
    root: &impl AsFd,
    relative: &Path,
    contents: &[u8],
) -> io::Result<FilesRootMetadata> {
    let mount_id = files_mount_id(root)?;
    let (parent, name) = files_parent_and_name(root, mount_id, relative)?;
    let mut created = false;
    let result = (|| {
        let fd = openat2(
            &parent,
            &name,
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
            files_resolve_flags(),
        )?;
        created = true;
        let mut file = File::from(fd);
        require_files_mount(&file, mount_id)?;
        file.write_all(contents)?;
        file.sync_all()?;
        rustix::fs::fsync(&parent)?;
        files_symlink_metadata_at(root, mount_id, relative)
    })();
    if result.is_err() && created {
        let _ = unlinkat(&parent, &name, AtFlags::empty());
        let _ = rustix::fs::fsync(&parent);
    }
    result
}

pub(super) fn write_existing_files_file(
    root: &impl AsFd,
    relative: &Path,
    contents: &[u8],
) -> io::Result<FilesRootMetadata> {
    let mount_id = files_mount_id(root)?;
    let before = files_symlink_metadata_at(root, mount_id, relative)?;
    if before.kind != FilesRootEntryKind::RegularFile {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let mut file = File::from(openat2(
        root,
        relative,
        OFlags::WRONLY | OFlags::CLOEXEC,
        Mode::empty(),
        files_resolve_flags(),
    )?);
    require_files_mount(&file, mount_id)?;
    let opened = statx_fd(&file)?;
    let opened_metadata = files_metadata(&opened);
    if opened_metadata.kind != FilesRootEntryKind::RegularFile
        || opened_metadata.device != before.device
        || opened_metadata.inode != before.inode
    {
        return Err(io::ErrorKind::Other.into());
    }
    file.set_len(0)?;
    file.write_all(contents)?;
    file.sync_all()?;
    sync_files_parent(root, relative)?;
    files_symlink_metadata_at(root, mount_id, relative)
}

pub(super) fn create_files_directory(root: &impl AsFd, relative: &Path) -> io::Result<()> {
    let mount_id = files_mount_id(root)?;
    let (parent, name) = files_parent_and_name(root, mount_id, relative)?;
    mkdirat(&parent, &name, Mode::RWXU)?;
    rustix::fs::fsync(&parent)?;
    Ok(())
}

pub(super) fn unlink_files_entry(root: &impl AsFd, relative: &Path) -> io::Result<()> {
    let mount_id = files_mount_id(root)?;
    let (parent, name) = files_parent_and_name(root, mount_id, relative)?;
    let metadata = files_symlink_metadata_at(root, mount_id, relative)?;
    match metadata.kind {
        FilesRootEntryKind::RegularFile | FilesRootEntryKind::Symlink => {
            unlinkat(&parent, &name, AtFlags::empty())?;
        }
        FilesRootEntryKind::Directory => return Err(io::ErrorKind::IsADirectory.into()),
        FilesRootEntryKind::Other => return Err(io::ErrorKind::Unsupported.into()),
    }
    rustix::fs::fsync(&parent)?;
    Ok(())
}

pub(super) fn remove_empty_files_directory(root: &impl AsFd, relative: &Path) -> io::Result<()> {
    let mount_id = files_mount_id(root)?;
    let (parent, name) = files_parent_and_name(root, mount_id, relative)?;
    let metadata = files_symlink_metadata_at(root, mount_id, relative)?;
    if metadata.kind != FilesRootEntryKind::Directory {
        return Err(io::ErrorKind::NotADirectory.into());
    }
    unlinkat(&parent, &name, AtFlags::REMOVEDIR)?;
    rustix::fs::fsync(&parent)?;
    Ok(())
}

pub(super) fn rename_files_entry(
    root: &impl AsFd,
    source: &Path,
    destination: &Path,
) -> io::Result<FilesRootMetadata> {
    let mount_id = files_mount_id(root)?;
    let (source_parent, source_name) = files_parent_and_name(root, mount_id, source)?;
    let (destination_parent, destination_name) =
        files_parent_and_name(root, mount_id, destination)?;
    let source_metadata = files_symlink_metadata_at(root, mount_id, source)?;
    if source_metadata.kind == FilesRootEntryKind::Symlink {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    if source_metadata.kind == FilesRootEntryKind::Directory {
        validate_files_tree_at(root, mount_id, source)?;
    } else if source_metadata.kind != FilesRootEntryKind::RegularFile {
        return Err(io::ErrorKind::Unsupported.into());
    }
    renameat_with(
        &source_parent,
        &source_name,
        &destination_parent,
        &destination_name,
        RenameFlags::NOREPLACE,
    )?;
    rustix::fs::fsync(&destination_parent)?;
    rustix::fs::fsync(&source_parent)?;
    files_symlink_metadata_at(root, mount_id, destination)
}

pub(super) fn replace_files_temp(
    root: &impl AsFd,
    temporary: &Path,
    destination: &Path,
) -> io::Result<()> {
    let mount_id = files_mount_id(root)?;
    validate_files_path(temporary, false)?;
    validate_files_path(destination, false)?;
    if temporary == destination {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    if temporary.parent().unwrap_or_else(|| Path::new(""))
        != destination.parent().unwrap_or_else(|| Path::new(""))
    {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    let (parent, temporary_name) = files_parent_and_name(root, mount_id, temporary)?;
    let (_, destination_name) = files_parent_and_name(root, mount_id, destination)?;
    let source_metadata = files_symlink_metadata_at(root, mount_id, temporary)?;
    if source_metadata.kind != FilesRootEntryKind::RegularFile {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    match files_symlink_metadata_at(root, mount_id, destination) {
        Ok(metadata) if metadata.kind == FilesRootEntryKind::RegularFile => {}
        Ok(_) => return Err(io::ErrorKind::InvalidInput.into()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    renameat(&parent, &temporary_name, &parent, &destination_name)?;
    rustix::fs::fsync(&parent)?;
    Ok(())
}

fn validate_files_tree_at(root: &impl AsFd, mount_id: u64, relative: &Path) -> io::Result<()> {
    let metadata = files_symlink_metadata_at(root, mount_id, relative)?;
    match metadata.kind {
        FilesRootEntryKind::RegularFile => return Ok(()),
        FilesRootEntryKind::Symlink => return Err(io::ErrorKind::PermissionDenied.into()),
        FilesRootEntryKind::Other => return Err(io::ErrorKind::Unsupported.into()),
        FilesRootEntryKind::Directory => {}
    }
    let directory = open_files_directory(root, mount_id, relative)?;
    for entry in Dir::read_from(&directory)? {
        let entry = entry?;
        let name = entry.file_name();
        if matches!(name.to_bytes(), b"." | b"..") {
            continue;
        }
        let child = relative.join(std::ffi::OsStr::from_bytes(name.to_bytes()));
        validate_files_tree_at(root, mount_id, &child)?;
    }
    Ok(())
}

pub(super) fn validate_files_tree(root: &impl AsFd, relative: &Path) -> io::Result<()> {
    let mount_id = files_mount_id(root)?;
    validate_files_path(relative, false)?;
    validate_files_tree_at(root, mount_id, relative)
}

fn remove_files_path_at(root: &impl AsFd, mount_id: u64, relative: &Path) -> io::Result<()> {
    let (parent, name) = files_parent_and_name(root, mount_id, relative)?;
    let metadata = files_symlink_metadata_at(root, mount_id, relative)?;
    if metadata.kind == FilesRootEntryKind::Directory {
        let directory = open_files_directory(root, mount_id, relative)?;
        for entry in Dir::read_from(&directory)? {
            let entry = entry?;
            let child_name = entry.file_name();
            if matches!(child_name.to_bytes(), b"." | b"..") {
                continue;
            }
            remove_files_path_at(
                root,
                mount_id,
                &relative.join(std::ffi::OsStr::from_bytes(child_name.to_bytes())),
            )?;
        }
        unlinkat(&parent, &name, AtFlags::REMOVEDIR)?;
    } else if matches!(
        metadata.kind,
        FilesRootEntryKind::RegularFile | FilesRootEntryKind::Symlink
    ) {
        unlinkat(&parent, &name, AtFlags::empty())?;
    } else {
        return Err(io::ErrorKind::Unsupported.into());
    }
    Ok(())
}

pub(super) fn remove_files_path(root: &impl AsFd, relative: &Path) -> io::Result<()> {
    let mount_id = files_mount_id(root)?;
    validate_files_path(relative, false)?;
    let parent = relative.parent().unwrap_or_else(|| Path::new(""));
    remove_files_path_at(root, mount_id, relative)?;
    sync_files_directory_at(root, mount_id, parent)
}

fn copy_files_path_at(
    root: &impl AsFd,
    mount_id: u64,
    source: &Path,
    destination: &Path,
    is_cancelled: &mut impl FnMut() -> bool,
    destination_created: &mut bool,
) -> io::Result<FilesRootMetadata> {
    if is_cancelled() {
        return Err(io::ErrorKind::Interrupted.into());
    }
    let source_metadata = files_symlink_metadata_at(root, mount_id, source)?;
    if source_metadata.kind == FilesRootEntryKind::Directory && destination.starts_with(source) {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    match source_metadata.kind {
        FilesRootEntryKind::RegularFile => {
            let (destination_parent, destination_name) =
                files_parent_and_name(root, mount_id, destination)?;
            let mut source_file = File::from(openat2(
                root,
                source,
                OFlags::RDONLY | OFlags::CLOEXEC,
                Mode::empty(),
                files_resolve_flags(),
            )?);
            require_files_mount(&source_file, mount_id)?;
            let opened = files_metadata(&statx_fd(&source_file)?);
            if opened.kind != FilesRootEntryKind::RegularFile
                || opened.device != source_metadata.device
                || opened.inode != source_metadata.inode
            {
                return Err(io::ErrorKind::Other.into());
            }
            let mut destination_file = File::from(openat2(
                &destination_parent,
                &destination_name,
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
                files_resolve_flags(),
            )?);
            *destination_created = true;
            require_files_mount(&destination_file, mount_id)?;
            let mut buffer = [0_u8; 64 * 1024];
            loop {
                if is_cancelled() {
                    return Err(io::ErrorKind::Interrupted.into());
                }
                let read = source_file.read(&mut buffer)?;
                if read == 0 {
                    break;
                }
                destination_file.write_all(&buffer[..read])?;
            }
            destination_file.sync_all()?;
            rustix::fs::fsync(&destination_parent)?;
            files_symlink_metadata_at(root, mount_id, destination)
        }
        FilesRootEntryKind::Directory => {
            create_files_directory(root, destination)?;
            *destination_created = true;
            let result = (|| {
                let directory = open_files_directory(root, mount_id, source)?;
                for entry in Dir::read_from(&directory)? {
                    let entry = entry?;
                    let name = entry.file_name();
                    if matches!(name.to_bytes(), b"." | b"..") {
                        continue;
                    }
                    if is_cancelled() {
                        return Err(io::ErrorKind::Interrupted.into());
                    }
                    copy_files_path_at(
                        root,
                        mount_id,
                        &source.join(std::ffi::OsStr::from_bytes(name.to_bytes())),
                        &destination.join(std::ffi::OsStr::from_bytes(name.to_bytes())),
                        is_cancelled,
                        destination_created,
                    )?;
                }
                sync_files_directory_at(root, mount_id, destination)?;
                files_symlink_metadata_at(root, mount_id, destination)
            })();
            result
        }
        FilesRootEntryKind::Symlink => Err(io::ErrorKind::PermissionDenied.into()),
        FilesRootEntryKind::Other => Err(io::ErrorKind::Unsupported.into()),
    }
}

pub(super) fn copy_files_path(
    root: &impl AsFd,
    source: &Path,
    destination: &Path,
    mut is_cancelled: impl FnMut() -> bool,
) -> io::Result<FilesRootMetadata> {
    validate_files_path(source, false)?;
    validate_files_path(destination, false)?;
    let mount_id = files_mount_id(root)?;
    let mut destination_created = false;
    let result = copy_files_path_at(
        root,
        mount_id,
        source,
        destination,
        &mut is_cancelled,
        &mut destination_created,
    );
    if result.is_err() && destination_created {
        remove_files_path_at(root, mount_id, destination)
            .map_err(|_| io::Error::other("copy failed and partial destination cleanup failed"))?;
        let parent = destination.parent().unwrap_or_else(|| Path::new(""));
        sync_files_directory_at(root, mount_id, parent)?;
    }
    result
}

pub(super) fn require_same_mount(fd: &impl AsFd, expected: u64) -> io::Result<()> {
    let requested = StatxFlags::TYPE | StatxFlags::MNT_ID;
    let stat = statx(fd, c"", AtFlags::EMPTY_PATH, requested)?;
    if stat.stx_mask & requested.bits() != requested.bits()
        || FileType::from_raw_mode(stat.stx_mode.into()) != FileType::Directory
        || stat.stx_mnt_id != expected
    {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(())
}

pub(super) fn mount_id(fd: &impl AsFd) -> io::Result<u64> {
    let requested = StatxFlags::MNT_ID;
    let stat = statx(fd, c"", AtFlags::EMPTY_PATH, requested)?;
    if stat.stx_mask & requested.bits() != requested.bits() {
        return Err(io::ErrorKind::Unsupported.into());
    }
    Ok(stat.stx_mnt_id)
}

fn private_directory(parent: &impl AsFd, name: &str) -> io::Result<File> {
    match mkdirat(parent, name, Mode::RWXU) {
        Ok(()) | Err(Errno::EXIST) => {}
        Err(error) => return Err(error.into()),
    }
    let dir = open_private_directory(parent, name)?;
    rustix::fs::fsync(parent)?;
    Ok(dir)
}

fn open_files_private_directory(parent: &impl AsFd, name: &str) -> io::Result<File> {
    let dir = File::from(open_beneath(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY,
    )?);
    let metadata = dir.metadata()?;
    if metadata.uid() != USER_FILES_APP_UID
        || metadata.gid() != USER_FILES_APP_GID
        || metadata.mode() & 0o777 != 0o700
    {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    Ok(dir)
}

/// Provision an empty User Files root for its fixed app identity. Existing
/// contents are never recursively reassigned; an old or unexpected owner with
/// data fails closed and requires an explicit migration outside this path.
fn provision_empty_files_directory(parent: &impl AsFd, name: &str) -> io::Result<File> {
    match mkdirat(parent, name, Mode::RWXU) {
        Ok(()) | Err(Errno::EXIST) => {}
        Err(error) => return Err(error.into()),
    }
    let dir = File::from(open_beneath(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY,
    )?);
    let metadata = dir.metadata()?;
    if metadata.mode() & 0o777 != 0o700 {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    for entry in Dir::read_from(&dir)? {
        if !matches!(entry?.file_name().to_bytes(), b"." | b"..") {
            return Err(io::ErrorKind::InvalidData.into());
        }
    }
    if (metadata.uid(), metadata.gid()) != (USER_FILES_APP_UID, USER_FILES_APP_GID) {
        if (metadata.uid(), metadata.gid()) != (0, 0) {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        fchown(
            &dir,
            Some(Uid::from_raw(USER_FILES_APP_UID)),
            Some(Gid::from_raw(USER_FILES_APP_GID)),
        )?;
        dir.sync_all()?;
        fsync(parent)?;
    }
    let metadata = dir.metadata()?;
    if metadata.uid() != USER_FILES_APP_UID
        || metadata.gid() != USER_FILES_APP_GID
        || metadata.mode() & 0o777 != 0o700
    {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    Ok(dir)
}

/// Provision an empty app-specific process root. Ownership is assigned only
/// while the directory is empty; an unexpected nonempty legacy root is never
/// recursively re-owned.
fn provision_empty_app_directory(
    parent: &impl AsFd,
    name: &str,
    uid: u32,
    gid: u32,
) -> io::Result<File> {
    match mkdirat(parent, name, Mode::RWXU) {
        Ok(()) | Err(Errno::EXIST) => {}
        Err(error) => return Err(error.into()),
    }
    let dir = File::from(open_beneath(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY,
    )?);
    let metadata = dir.metadata()?;
    if metadata.mode() & 0o777 != 0o700 {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    match (metadata.uid(), metadata.gid()) {
        (observed_uid, observed_gid) if (observed_uid, observed_gid) == (uid, gid) => {}
        (0, 0) => {
            for entry in Dir::read_from(&dir)? {
                if !matches!(entry?.file_name().to_bytes(), b"." | b"..") {
                    return Err(io::ErrorKind::InvalidData.into());
                }
            }
            fchown(&dir, Some(Uid::from_raw(uid)), Some(Gid::from_raw(gid)))?;
            dir.sync_all()?;
            fsync(parent)?;
        }
        _ => return Err(io::ErrorKind::PermissionDenied.into()),
    }
    let metadata = dir.metadata()?;
    if metadata.uid() != uid || metadata.gid() != gid || metadata.mode() & 0o777 != 0o700 {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    Ok(dir)
}

fn reserve_notes_app_process_root(
    profile_root: &impl AsFd,
    selected_mount_id: u64,
    selected_data_identity_sha256: &[u8; 32],
    user: UserId,
    profile: &str,
    principal: &str,
    uid: u32,
    gid: u32,
) -> io::Result<File> {
    let root = provision_empty_app_directory(profile_root, NOTES_APP_ROOT, uid, gid)?;
    require_same_mount(&root, selected_mount_id)?;
    let metadata = root.metadata()?;
    let selected_data_identity = lowercase_hex(selected_data_identity_sha256);
    let expected = format!(
        "NAGI-NOTES-APP-ROOT-2\n{:016x}\n{profile}\n{principal}\norg.nagi.notes\n{uid:08x}\n{gid:08x}\n{:016x}\n{:016x}\n{selected_data_identity}\n",
        user.0,
        metadata.dev(),
        metadata.ino(),
    );
    match open_beneath(
        profile_root,
        NOTES_APP_ROOT_RESERVATION,
        OFlags::RDONLY | OFlags::NONBLOCK,
    ) {
        Ok(fd) => {
            let record = File::from(fd);
            let attributes = record.metadata()?;
            if !attributes.is_file()
                || attributes.uid() != 0
                || attributes.mode() & 0o777 != 0o600
                || attributes.nlink() != 1
            {
                return Err(io::ErrorKind::PermissionDenied.into());
            }
            let mut observed = String::new();
            (&record)
                .take(MAX_RECORD_BYTES + 1)
                .read_to_string(&mut observed)?;
            if observed != expected {
                return Err(io::ErrorKind::InvalidData.into());
            }
            record.sync_all()?;
            retire_legacy_notes_app_root_record(
                profile_root,
                user,
                profile,
                principal,
                uid,
                gid,
                metadata.dev(),
                metadata.ino(),
            )?;
        }
        Err(Errno::NOENT) => match open_beneath(
            profile_root,
            LEGACY_NOTES_APP_ROOT_RESERVATION,
            OFlags::RDONLY | OFlags::NONBLOCK,
        ) {
            Ok(fd) => {
                let legacy = File::from(fd);
                validate_private_root_reservation(&legacy)?;
                let mut observed = String::new();
                (&legacy)
                    .take(MAX_RECORD_BYTES + 1)
                    .read_to_string(&mut observed)?;
                validate_legacy_notes_app_root_record(
                    &observed,
                    user,
                    profile,
                    principal,
                    uid,
                    gid,
                    metadata.dev(),
                    metadata.ino(),
                )?;
                legacy.sync_all()?;
                drop(legacy);
                publish_notes_app_root_record(profile_root, &expected)?;
                retire_legacy_notes_app_root_record(
                    profile_root,
                    user,
                    profile,
                    principal,
                    uid,
                    gid,
                    metadata.dev(),
                    metadata.ino(),
                )?;
            }
            Err(Errno::NOENT) => {
                for entry in Dir::read_from(&root)? {
                    if !matches!(entry?.file_name().to_bytes(), b"." | b"..") {
                        return Err(io::ErrorKind::InvalidData.into());
                    }
                }
                publish_notes_app_root_record(profile_root, &expected)?;
            }
            Err(error) => return Err(error.into()),
        },
        Err(error) => return Err(error.into()),
    }
    Ok(root)
}

fn validate_private_root_reservation(record: &File) -> io::Result<()> {
    let attributes = record.metadata()?;
    if !attributes.is_file()
        || attributes.uid() != 0
        || attributes.mode() & 0o777 != 0o600
        || attributes.nlink() != 1
    {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    Ok(())
}

fn retire_legacy_notes_app_root_record(
    profile_root: &impl AsFd,
    user: UserId,
    profile: &str,
    principal: &str,
    uid: u32,
    gid: u32,
    device: u64,
    inode: u64,
) -> io::Result<()> {
    let legacy = match open_beneath(
        profile_root,
        LEGACY_NOTES_APP_ROOT_RESERVATION,
        OFlags::RDONLY | OFlags::NONBLOCK,
    ) {
        Ok(fd) => File::from(fd),
        Err(Errno::NOENT) => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    validate_private_root_reservation(&legacy)?;
    let mut observed = String::new();
    (&legacy)
        .take(MAX_RECORD_BYTES + 1)
        .read_to_string(&mut observed)?;
    validate_legacy_notes_app_root_record(
        &observed, user, profile, principal, uid, gid, device, inode,
    )?;
    drop(legacy);
    match unlinkat(
        profile_root,
        LEGACY_NOTES_APP_ROOT_RESERVATION,
        AtFlags::empty(),
    ) {
        Ok(()) => fsync(profile_root)?,
        Err(Errno::NOENT) => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn validate_legacy_notes_app_root_record(
    observed: &str,
    user: UserId,
    profile: &str,
    principal: &str,
    uid: u32,
    gid: u32,
    device: u64,
    inode: u64,
) -> io::Result<()> {
    let prefix = format!(
        "NAGI-NOTES-APP-ROOT-1\n{:016x}\n{profile}\n{principal}\norg.nagi.notes\n{uid:08x}\n{gid:08x}\n{:016x}\n{:016x}\n",
        user.0, device, inode,
    );
    let mount_id = observed
        .strip_prefix(&prefix)
        .and_then(|suffix| suffix.strip_suffix('\n'))
        .ok_or(io::ErrorKind::InvalidData)?;
    if mount_id.len() != 16
        || !mount_id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || u64::from_str_radix(mount_id, 16).map_or(true, |value| {
            value == 0 || format!("{value:016x}") != mount_id
        })
    {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok(())
}

fn publish_notes_app_root_record(profile_root: &impl AsFd, expected: &str) -> io::Result<()> {
    let mut pending = File::from(rustix::fs::openat(
        profile_root,
        ".",
        OFlags::TMPFILE | OFlags::WRONLY | OFlags::CLOEXEC,
        Mode::RUSR | Mode::WUSR,
    )?);
    pending.write_all(expected.as_bytes())?;
    pending.sync_all()?;
    linkat(
        &pending,
        c"",
        profile_root,
        NOTES_APP_ROOT_RESERVATION,
        AtFlags::EMPTY_PATH,
    )?;
    fsync(profile_root)?;
    Ok(())
}

fn lowercase_hex(bytes: &[u8; 32]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = Vec::with_capacity(bytes.len() * 2);
    for byte in bytes {
        encoded.push(HEX[(byte >> 4) as usize]);
        encoded.push(HEX[(byte & 0x0f) as usize]);
    }
    String::from_utf8(encoded).expect("lowercase hexadecimal is valid UTF-8")
}

fn open_private_directory(parent: &impl AsFd, name: &str) -> io::Result<File> {
    let dir = File::from(open_beneath(
        parent,
        name,
        OFlags::RDONLY | OFlags::DIRECTORY,
    )?);
    let metadata = dir.metadata()?;
    if metadata.uid() != 0 || metadata.mode() & 0o777 != 0o700 {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    Ok(dir)
}

fn open_beneath(parent: &impl AsFd, name: &str, flags: OFlags) -> rustix::io::Result<OwnedFd> {
    openat2(
        parent,
        name,
        flags | OFlags::CLOEXEC,
        Mode::empty(),
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_XDEV,
    )
}

fn valid_label(label: &str, principal: bool) -> bool {
    (1..=MAX_ID_BYTES).contains(&label.len())
        && label.bytes().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(c, b'_' | b'-')
                || (principal && matches!(c, b':' | b'.'))
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct BindMountGuard {
        target: std::ffi::CString,
        mounted: bool,
    }

    impl BindMountGuard {
        fn unmount(mut self) -> io::Result<()> {
            let result = unsafe { umount2(self.target.as_ptr(), MNT_DETACH) };
            if result != 0 {
                return Err(io::Error::last_os_error());
            }
            self.mounted = false;
            Ok(())
        }
    }

    impl Drop for BindMountGuard {
        fn drop(&mut self) {
            if self.mounted {
                unsafe {
                    umount2(self.target.as_ptr(), MNT_DETACH);
                }
            }
        }
    }

    fn bind_mount_directory(source: &Path, target: &Path) -> io::Result<BindMountGuard> {
        use std::os::unix::ffi::OsStrExt;

        const MS_BIND: std::os::raw::c_ulong = 1 << 12;
        const MS_REC: std::os::raw::c_ulong = 1 << 14;
        const MS_PRIVATE: std::os::raw::c_ulong = 1 << 18;
        let source = std::ffi::CString::new(source.as_os_str().as_bytes())
            .map_err(|_| io::ErrorKind::InvalidInput)?;
        let target = std::ffi::CString::new(target.as_os_str().as_bytes())
            .map_err(|_| io::ErrorKind::InvalidInput)?;
        let result = unsafe {
            mount(
                source.as_ptr(),
                target.as_ptr(),
                std::ptr::null(),
                MS_BIND,
                std::ptr::null(),
            )
        };
        if result != 0 {
            return Err(io::Error::last_os_error());
        }
        let guard = BindMountGuard {
            target,
            mounted: true,
        };
        let result = unsafe {
            mount(
                std::ptr::null(),
                guard.target.as_ptr(),
                std::ptr::null(),
                MS_REC | MS_PRIVATE,
                std::ptr::null(),
            )
        };
        if result != 0 {
            let error = io::Error::last_os_error();
            let _ = guard.unmount();
            return Err(error);
        }
        Ok(guard)
    }

    const MNT_DETACH: std::os::raw::c_int = 2;

    extern "C" {
        fn mount(
            source: *const std::os::raw::c_char,
            target: *const std::os::raw::c_char,
            filesystemtype: *const std::os::raw::c_char,
            mountflags: std::os::raw::c_ulong,
            data: *const std::ffi::c_void,
        ) -> std::os::raw::c_int;
        fn umount2(
            target: *const std::os::raw::c_char,
            flags: std::os::raw::c_int,
        ) -> std::os::raw::c_int;
    }

    #[test]
    fn notes_reservation_returns_stable_private_app_process_root() {
        use std::fs::{self, DirBuilder};
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};

        let path = std::env::temp_dir().join(format!(
            "nagi-notes-app-process-root-{}",
            std::process::id()
        ));
        DirBuilder::new().mode(0o700).create(&path).unwrap();
        let data = File::open(&path).unwrap();
        // This regression must run with the same root authority as PID 1 so
        // it exercises root-owned Data provisioning and the app-root chown.
        assert_eq!(
            data.metadata().unwrap().uid(),
            0,
            "run this focused fixture as root; an unprivileged skip would hide coverage"
        );
        let mount_id = statx(&data, c"", AtFlags::EMPTY_PATH, StatxFlags::MNT_ID)
            .unwrap()
            .stx_mnt_id;
        let selected_data_identity = [0x57; 32];
        let (user, profile, principal) = (UserId(6), "profile_notes", "principal_notes");
        let attach = || {
            reserve(
                &data,
                mount_id,
                &selected_data_identity,
                user,
                profile,
                principal,
                AppStorageKind::Notes,
            )
        };
        let (objects, object_id, allow_initial_index, root) = attach().unwrap();
        assert_ne!(object_id.0, 0);
        assert!(allow_initial_index);
        let root = root.unwrap();
        let first = File::from(root);
        let first_metadata = first.metadata().unwrap();
        assert_eq!(first_metadata.uid(), NOTES_APP_UID);
        assert_eq!(first_metadata.gid(), NOTES_APP_GID);
        assert_eq!(first_metadata.mode() & 0o777, 0o700);
        let reservation_path = path.join("profile-notes-v1/profile_notes/app-root.v2");
        let reservation = fs::read_to_string(&reservation_path).unwrap();
        assert_eq!(reservation.lines().count(), 10);
        assert_eq!(
            reservation.lines().last(),
            Some(lowercase_hex(&selected_data_identity).as_str())
        );
        fs::set_permissions(&reservation_path, fs::Permissions::from_mode(0o644)).unwrap();
        assert_eq!(
            attach().err().unwrap().kind(),
            io::ErrorKind::PermissionDenied,
            "the v2 marker must retain its private-file mode"
        );
        fs::set_permissions(&reservation_path, fs::Permissions::from_mode(0o600)).unwrap();
        fs::write(
            path.join("profile-notes-v1/profile_notes/app-root/persisted"),
            b"note",
        )
        .unwrap();
        fs::remove_file(&reservation_path).unwrap();
        let legacy_reservation_path = path.join("profile-notes-v1/profile_notes/app-root.v1");
        let legacy_record = format!(
            "NAGI-NOTES-APP-ROOT-1\n{:016x}\n{profile}\n{principal}\norg.nagi.notes\n{:08x}\n{:08x}\n{:016x}\n{:016x}\n{mount_id:016x}\n",
            user.0,
            NOTES_APP_UID,
            NOTES_APP_GID,
            first_metadata.dev(),
            first_metadata.ino(),
        );
        fs::write(&legacy_reservation_path, &legacy_record).unwrap();
        fs::set_permissions(&legacy_reservation_path, fs::Permissions::from_mode(0o600)).unwrap();
        drop(objects);
        let remounted = path.with_file_name(format!(
            "nagi-notes-app-process-root-remount-{}",
            std::process::id()
        ));
        DirBuilder::new().mode(0o700).create(&remounted).unwrap();
        let bind_mount = bind_mount_directory(&path, &remounted).unwrap();
        let remounted_data = File::open(&remounted).unwrap();
        let remounted_mount_id = statx(
            &remounted_data,
            c"",
            AtFlags::EMPTY_PATH,
            StatxFlags::MNT_ID,
        )
        .unwrap()
        .stx_mnt_id;
        assert_ne!(remounted_mount_id, mount_id);
        assert!(
            reserve(
                &remounted_data,
                mount_id,
                &selected_data_identity,
                user,
                profile,
                principal,
                AppStorageKind::Notes,
            )
            .is_err(),
            "the previous live mount ID must not be accepted after remount"
        );
        let (reopened_objects, reopened_id, reopened_allow_initial_index, reopened_root) = reserve(
            &remounted_data,
            remounted_mount_id,
            &selected_data_identity,
            user,
            profile,
            principal,
            AppStorageKind::Notes,
        )
        .unwrap();
        let reopened = File::from(reopened_root.unwrap());
        assert_eq!(reopened_id, object_id);
        assert!(reopened_allow_initial_index);
        assert_eq!(reopened.metadata().unwrap().ino(), first_metadata.ino());
        assert_eq!(
            fs::read(path.join("profile-notes-v1/profile_notes/app-root/persisted")).unwrap(),
            b"note"
        );
        let migrated_reservation = fs::read_to_string(&reservation_path).unwrap();
        assert_eq!(migrated_reservation.lines().count(), 10);
        assert_eq!(
            migrated_reservation.lines().last(),
            Some(lowercase_hex(&selected_data_identity).as_str())
        );
        assert!(
            !legacy_reservation_path.exists(),
            "successful v1 migration must durably retire the legacy marker"
        );

        // Simulate interruption after the v2 file and parent are synced but
        // before the old v1 marker is unlinked. Reopen must retire v1 before
        // accepting the durable v2 root.
        fs::write(&legacy_reservation_path, &legacy_record).unwrap();
        fs::set_permissions(&legacy_reservation_path, fs::Permissions::from_mode(0o600)).unwrap();
        let (interrupted_objects, _, _, interrupted_root) = reserve(
            &remounted_data,
            remounted_mount_id,
            &selected_data_identity,
            user,
            profile,
            principal,
            AppStorageKind::Notes,
        )
        .unwrap();
        drop((interrupted_objects, interrupted_root));
        assert!(
            !legacy_reservation_path.exists(),
            "v2 readback must finish interrupted v1 retirement"
        );
        assert_eq!(
            reserve(
                &remounted_data,
                remounted_mount_id,
                &[0x58; 32],
                user,
                profile,
                principal,
                AppStorageKind::Notes,
            )
            .unwrap_err()
            .kind(),
            io::ErrorKind::InvalidData,
            "the durable Notes root must reject a different selected Data identity"
        );
        let original = path.join("profile-notes-v1/profile_notes/app-root");
        let displaced = original.with_file_name("replaced-app-root");
        fs::rename(&original, &displaced).unwrap();
        DirBuilder::new().mode(0o700).create(&original).unwrap();
        let replacement = File::open(&original).unwrap();
        rustix::fs::fchown(
            &replacement,
            Some(Uid::from_raw(NOTES_APP_UID)),
            Some(Gid::from_raw(NOTES_APP_GID)),
        )
        .unwrap();
        drop(replacement);
        assert_eq!(
            attach().unwrap_err().kind(),
            io::ErrorKind::InvalidData,
            "a replacement app root must not inherit the old reservation"
        );
        assert_eq!(fs::read(displaced.join("persisted")).unwrap(), b"note");
        drop((first, reopened, reopened_objects, remounted_data, data));
        bind_mount.unmount().unwrap();
        fs::remove_dir(remounted).unwrap();
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn files_reservation_pins_the_original_root_identity() {
        use std::fs::{self, DirBuilder};
        use std::os::unix::fs::DirBuilderExt;

        let path =
            std::env::temp_dir().join(format!("nagi-files-reservation-{}", std::process::id()));
        DirBuilder::new().mode(0o700).create(&path).unwrap();
        let data = File::open(&path).unwrap();
        // Root-owned profile provisioning is exercised in the Linux guest.
        if data.metadata().unwrap().uid() != 0 {
            fs::remove_dir(&path).unwrap();
            return;
        }
        let mount_id = statx(&data, c"", AtFlags::EMPTY_PATH, StatxFlags::MNT_ID)
            .unwrap()
            .stx_mnt_id;
        let (user, profile, principal) = (UserId(3), "profile_fixture", "principal_fixture");
        let attach = || reserve_files_root(&data, mount_id, user, profile, principal);
        let root = File::from(attach().unwrap());
        assert_eq!(root.metadata().unwrap().uid(), USER_FILES_APP_UID);
        assert_eq!(root.metadata().unwrap().gid(), USER_FILES_APP_GID);
        fs::write(
            path.join("profile-files-v1/profile_fixture/root/file"),
            b"preserved",
        )
        .unwrap();
        let reopened = File::from(attach().unwrap());
        assert_eq!(
            root.metadata().unwrap().ino(),
            reopened.metadata().unwrap().ino()
        );
        assert!(reserve_files_root(&data, mount_id, user, profile, "another_principal").is_err());
        let original = path.join("profile-files-v1/profile_fixture/root");
        let displaced = original.with_file_name("old-root");
        fs::rename(&original, &displaced).unwrap();
        DirBuilder::new().mode(0o700).create(&original).unwrap();
        let replacement = File::open(&original).unwrap();
        rustix::fs::fchown(
            &replacement,
            Some(Uid::from_raw(USER_FILES_APP_UID)),
            Some(Gid::from_raw(USER_FILES_APP_GID)),
        )
        .unwrap();
        drop(replacement);
        assert_eq!(attach().unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert_eq!(fs::read(displaced.join("file")).unwrap(), b"preserved");
        drop((root, reopened, data));
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn files_root_does_not_reassign_nonempty_legacy_storage() {
        use std::fs::{self, DirBuilder};
        use std::os::unix::fs::DirBuilderExt;

        let path =
            std::env::temp_dir().join(format!("nagi-files-legacy-owner-{}", std::process::id()));
        DirBuilder::new().mode(0o700).create(&path).unwrap();
        let data = File::open(&path).unwrap();
        if data.metadata().unwrap().uid() != 0 {
            fs::remove_dir(&path).unwrap();
            return;
        }
        let mount_id = statx(&data, c"", AtFlags::EMPTY_PATH, StatxFlags::MNT_ID)
            .unwrap()
            .stx_mnt_id;
        let (user, profile, principal) = (UserId(4), "profile_legacy", "principal_legacy");
        let profile_root = open_profile_root(
            &data,
            mount_id,
            user,
            profile,
            principal,
            AppStorageKind::UserFiles,
        )
        .unwrap();
        let legacy_root = private_directory(&profile_root, FILES_ROOT).unwrap();
        fs::write(
            path.join("profile-files-v1/profile_legacy/root/preserved"),
            b"keep",
        )
        .unwrap();
        drop(legacy_root);

        let error = reserve_files_root(&data, mount_id, user, profile, principal).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        let reopened = File::open(path.join("profile-files-v1/profile_legacy/root")).unwrap();
        assert_eq!(reopened.metadata().unwrap().uid(), 0);
        assert_eq!(reopened.metadata().unwrap().gid(), 0);
        assert_eq!(
            fs::read(path.join("profile-files-v1/profile_legacy/root/preserved")).unwrap(),
            b"keep"
        );
        assert!(!path
            .join(format!("profile-files-v1/profile_legacy/{RESERVATION}"))
            .exists());
        drop((reopened, profile_root, data));
        fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn kind_namespaces_accept_only_their_own_reservations() {
        use AppStorageKind::*;
        let scope = "0000000000000001\nprofile_fixture\nprincipal\n";
        let layouts = [
            (Notes, "profile-notes-v1", "NAGI-NOTES-PROFILE-2"),
            (Settings, "profile-settings-v1", "NAGI-SETTINGS-PROFILE-1"),
            (Calendar, "profile-calendar-v1", "NAGI-CALENDAR-PROFILE-1"),
        ];
        for (kind, directory, heading) in layouts {
            assert_eq!(storage_layout(kind), (directory, heading));
            let record = format!("{heading}\n{scope}0000000000000002\n");
            for (candidate, _, _) in layouts {
                let result = parse_reservation(candidate, scope, &record);
                if candidate == kind {
                    assert_eq!(result.unwrap(), (ObjectId(2), true));
                } else {
                    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidData);
                }
            }
        }
        let legacy = format!("NAGI-NOTES-PROFILE-1\n{scope}0000000000000002\n");
        assert_eq!(
            parse_reservation(Notes, scope, &legacy).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
