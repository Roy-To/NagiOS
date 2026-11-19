extern crate self as nagi_pal;

#[path = "../../../user/nagi-pal/src/time.rs"]
pub mod time;
pub use time::{Clock, TimeError};

// The production helper imports these same ABI constants from nagi-posix.
mod errno {
    pub const EINVAL: i32 = 22;
    pub const ENOSYS: i32 = 38;
}

#[path = "../../../user/nagi-posix/src/posix_time.rs"]
mod posix_time;
