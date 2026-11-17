//! Same-crate implementation of Task24's actual sealed lease trait.
//! This compiles the Stage25 endpoint lease against the exact C3 owner copy.

use super::ExactChildLaunchLease;
use crate::{LaunchRecordView, Task24EndpointLaunchLease};
use nagi_init_spawn_contract::SpawnedProcessIdentity;

impl<R: LaunchRecordView> ExactChildLaunchLease<R> for Task24EndpointLaunchLease<R> {
    fn matches_spawn(&self, process_instance: &SpawnedProcessIdentity, record: &R) -> bool {
        Task24EndpointLaunchLease::matches_spawn(self, process_instance, record)
    }
}
