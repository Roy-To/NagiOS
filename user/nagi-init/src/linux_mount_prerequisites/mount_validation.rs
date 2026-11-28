//! Pure fail-closed checks shared by release mount owners and host fixtures.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum CandidateCountFailure {
    DeviceNotFound,
    FilesystemUuidMismatch,
    DeviceAmbiguous,
}

pub(super) fn require_single_uuid_match(
    matches: usize,
    saw_ext4_candidate: bool,
) -> Result<(), CandidateCountFailure> {
    match matches {
        0 if saw_ext4_candidate => Err(CandidateCountFailure::FilesystemUuidMismatch),
        0 => Err(CandidateCountFailure::DeviceNotFound),
        1 => Ok(()),
        _ => Err(CandidateCountFailure::DeviceAmbiguous),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct MountIdentityObservation {
    pub(super) parent_mount_id: u64,
    pub(super) data_mount_id: u64,
    pub(super) selected_rdev: u64,
    pub(super) mounted_dev: u64,
    pub(super) filesystem_magic: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum MountIdentityFailure {
    MountIdUnavailable,
    MountIdNotDistinct,
    DeviceMismatch,
    WrongFilesystem,
}

pub(super) fn verify_mount_identity(
    observed: MountIdentityObservation,
) -> Result<(), MountIdentityFailure> {
    if observed.parent_mount_id == 0 || observed.data_mount_id == 0 {
        return Err(MountIdentityFailure::MountIdUnavailable);
    }
    if observed.parent_mount_id == observed.data_mount_id {
        return Err(MountIdentityFailure::MountIdNotDistinct);
    }
    if observed.selected_rdev == 0 || observed.mounted_dev != observed.selected_rdev {
        return Err(MountIdentityFailure::DeviceMismatch);
    }
    if observed.filesystem_magic != 0xef53 {
        return Err(MountIdentityFailure::WrongFilesystem);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        require_single_uuid_match, verify_mount_identity, CandidateCountFailure,
        MountIdentityFailure, MountIdentityObservation,
    };

    #[test]
    fn uuid_scan_requires_exactly_one_candidate_and_classifies_zero_matches() {
        assert_eq!(require_single_uuid_match(1, true), Ok(()));
        assert_eq!(
            require_single_uuid_match(0, true),
            Err(CandidateCountFailure::FilesystemUuidMismatch)
        );
        assert_eq!(
            require_single_uuid_match(0, false),
            Err(CandidateCountFailure::DeviceNotFound)
        );
        assert_eq!(
            require_single_uuid_match(2, true),
            Err(CandidateCountFailure::DeviceAmbiguous)
        );
    }

    #[test]
    fn mounted_root_must_match_uuid_candidate_device_and_be_a_distinct_ext4_mount() {
        let observed = MountIdentityObservation {
            parent_mount_id: 1,
            data_mount_id: 2,
            selected_rdev: 0x801,
            mounted_dev: 0x801,
            filesystem_magic: 0xef53,
        };
        assert_eq!(verify_mount_identity(observed), Ok(()));
        assert_eq!(
            verify_mount_identity(MountIdentityObservation {
                data_mount_id: 0,
                ..observed
            }),
            Err(MountIdentityFailure::MountIdUnavailable)
        );
        assert_eq!(
            verify_mount_identity(MountIdentityObservation {
                data_mount_id: observed.parent_mount_id,
                ..observed
            }),
            Err(MountIdentityFailure::MountIdNotDistinct)
        );
        assert_eq!(
            verify_mount_identity(MountIdentityObservation {
                mounted_dev: 0x802,
                ..observed
            }),
            Err(MountIdentityFailure::DeviceMismatch)
        );
        assert_eq!(
            verify_mount_identity(MountIdentityObservation {
                filesystem_magic: 0x6969,
                ..observed
            }),
            Err(MountIdentityFailure::WrongFilesystem)
        );
    }
}
