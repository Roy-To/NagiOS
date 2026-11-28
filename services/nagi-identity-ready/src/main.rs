//! Product Identity READY process.
#![no_std]
#![no_main]

#[path = "../../../user/nagi-init/src/linux_task24_identity_readiness_wire.rs"]
mod wire;

use nagi_sdk::AppId;

mod process;

const IDENTITY_APP: AppId = AppId::from_identifier(b"org.nagi.identity");
