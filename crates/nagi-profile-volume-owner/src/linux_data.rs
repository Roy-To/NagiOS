//! Volume Manager storage for the boot-selected development Data mount.

use super::AppStorageKind;
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
use std::path::{Component, Path};

const OBJECT_DIRECTORY: &str = "objects";
const FILES_ROOT: &str = "root";
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
    let allow_initial_index = match observed_heading {
        found if found == heading => true,
        "NAGI-NOTES-PROFILE-1" if kind == AppStorageKind::Notes => false,
        _ => return Err(io::ErrorKind::InvalidData.into()),
    };
    let hex = record
        .strip_prefix(scope)
        .and_then(|suffix| suffix.strip_suffix('\n'))
        .ok_or(io::ErrorKind::InvalidData)?;
    let object = u64::from_str_radix(hex, 16).map_err(|_| io::ErrorKind::InvalidData)?;
    if object == 0 || hex != format!("{object:016x}") {
        return Err(io::ErrorKind::InvalidData.into());
    }
    Ok((ObjectId(object), allow_initial_index))
}

pub(super) fn reserve(
    data: &impl AsFd,
    selected_mount_id: u64,
    user: UserId,
    profile: &str,
    principal: &str,
    kind: AppStorageKind,
) -> io::Result<(OwnedFd, ObjectId, bool)> {
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
    profile_root.sync_all()?;
    Ok((objects.into(), object, allow_initial_index))
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
            open_private_directory(&profile_root, FILES_ROOT)?,
            Some(File::from(record)),
        ),
        Err(Errno::NOENT) => {
            let root = private_directory(&profile_root, FILES_ROOT)?;
            for entry in Dir::read_from(&root)? {
                if !matches!(entry?.file_name().to_bytes(), b"." | b"..") {
                    return Err(io::ErrorKind::InvalidData.into());
                }
            }
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
    Ok(File::from(openat2(
        root,
        relative,
        flags | OFlags::CLOEXEC,
        mode,
        ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_XDEV,
    )?))
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
    let dir = open_private_directory(parent, name)?;
    rustix::fs::fsync(parent)?;
    Ok(dir)
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
        assert_eq!(attach().unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert_eq!(fs::read(displaced.join("file")).unwrap(), b"preserved");
        drop((root, reopened, data));
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
            parse_reservation(Notes, scope, &legacy).unwrap(),
            (ObjectId(2), false)
        );
        for kind in [Settings, Calendar] {
            assert_eq!(
                parse_reservation(kind, scope, &legacy).unwrap_err().kind(),
                io::ErrorKind::InvalidData
            );
        }
    }
}
