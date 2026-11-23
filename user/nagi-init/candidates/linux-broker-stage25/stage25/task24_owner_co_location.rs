//! Compile Task41 beside Task24's actual crate-private launch owner.
//!
//! The included owner file and its child files are read-only Task24 inputs.
//! This module exists only in the Stage24 candidate and does not widen their
//! visibility in the production crate.

#[allow(dead_code, unused_imports, unexpected_cfgs)]
mod task24_owner {
    include!(concat!(
        env!("NAGI_C3_SNAPSHOT_DIR"),
        "/user/nagi-init/src/linux_init_process_owner_candidate.rs"
    ));
}

pub(crate) use task24_owner::ExactChildLaunchLease;

use crate::{LaunchRecordView, Task24EndpointLaunchLease};
use nagi_init_spawn_contract::SpawnedProcessIdentity;

impl<R: LaunchRecordView> ExactChildLaunchLease<R> for Task24EndpointLaunchLease<R> {
    fn matches_spawn(&self, process_instance: &SpawnedProcessIdentity, record: &R) -> bool {
        Task24EndpointLaunchLease::matches_spawn(self, process_instance, record)
    }
}
