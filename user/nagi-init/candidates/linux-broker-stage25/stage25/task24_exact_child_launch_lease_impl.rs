//! Co-location-only impl patch for the Task24 process-owner module.
//!
//! Include this file as a child module of the Task24
//! `linux_init_process_owner_candidate` module, where its existing
//! `pub(crate) use pid1_owned_launch_lease::ExactChildLaunchLease` is in scope.
//! Co-locate the Task41 `pid1_live_peer_supplier` module as its sibling.
//! This file is intentionally not part of the standalone Stage23 Cargo crate:
//! the Task24 trait is crate-private and must not be widened to make this
//! candidate compile across a crate boundary.

use super::pid1_live_peer_supplier::{LaunchRecordView, Task24EndpointLaunchLease};
use super::ExactChildLaunchLease;
use nagi_init_spawn_contract::SpawnedProcessIdentity;

impl<R: LaunchRecordView> ExactChildLaunchLease<R> for Task24EndpointLaunchLease<R> {
    fn matches_spawn(&self, process_instance: &SpawnedProcessIdentity, record: &R) -> bool {
        Task24EndpointLaunchLease::matches_spawn(self, process_instance, record)
    }
}
