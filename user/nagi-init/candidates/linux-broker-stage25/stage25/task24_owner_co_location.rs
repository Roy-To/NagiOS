//! Compile Task41 beside Task24's actual crate-private launch owner.
//!
//! The included owner file and its child files are read-only Task24 inputs.
//! This module exists only in the Stage24 candidate and does not widen their
//! visibility in the production crate.

#[path = "task24-c3-co-location/linux_init_process_owner_candidate.rs"]
mod task24_owner;

pub(crate) use task24_owner::ExactChildLaunchLease;
