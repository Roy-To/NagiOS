//! Volume Manager storage for the boot-selected development Data mount.

use nagi_model::{ObjectId, UserId};
use rustix::fs::{
    flock, linkat, mkdirat, openat2, statx, AtFlags, Dir, FileType, FlockOperation, Mode, OFlags,
    ResolveFlags, StatxFlags,
};
use rustix::io::Errno;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, OwnedFd};
use std::os::unix::fs::MetadataExt;

const PROFILE_DIRECTORY: &str = "profile-notes-v1";
const OBJECT_DIRECTORY: &str = "objects";
const RESERVATION: &str = "reservation.v1";
const MAX_ID_BYTES: usize = 128;
const MAX_RECORD_BYTES: u64 = 512;

pub(super) fn reserve(
    data: &impl AsFd,
    selected_mount_id: u64,
    user: UserId,
    profile: &str,
    principal: &str,
) -> io::Result<(OwnedFd, ObjectId, bool)> {
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

    let profiles = private_directory(data, PROFILE_DIRECTORY)?;
    let profile_root = private_directory(&profiles, profile)?;
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
            let (record, allow_initial_index) =
                if let Some(rest) = record.strip_prefix("NAGI-NOTES-PROFILE-2\n") {
                    (rest, true)
                } else {
                    (
                        record
                            .strip_prefix("NAGI-NOTES-PROFILE-1\n")
                            .ok_or(io::ErrorKind::InvalidData)?,
                        false,
                    )
                };
            let hex = record
                .strip_prefix(&scope)
                .and_then(|suffix| suffix.strip_suffix('\n'))
                .ok_or(io::ErrorKind::InvalidData)?;
            let object = u64::from_str_radix(hex, 16).map_err(|_| io::ErrorKind::InvalidData)?;
            if object == 0 || hex != format!("{object:016x}") {
                return Err(io::ErrorKind::InvalidData.into());
            }
            file.sync_all()?;
            (ObjectId(object), allow_initial_index)
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
            writeln!(pending, "NAGI-NOTES-PROFILE-2\n{scope}{object:016x}")?;
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
    profile_root.sync_all()?;
    Ok((objects.into(), object, allow_initial_index))
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

fn private_directory(parent: &impl AsFd, name: &str) -> io::Result<File> {
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
    if metadata.uid() != 0 || metadata.mode() & 0o777 != 0o700 {
        return Err(io::ErrorKind::PermissionDenied.into());
    }
    rustix::fs::fsync(parent)?;
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
