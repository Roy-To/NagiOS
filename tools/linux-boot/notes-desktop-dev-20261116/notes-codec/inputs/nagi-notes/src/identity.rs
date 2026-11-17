use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use nagi_model::ObjectId;

/// Allocates IDs through an injected boundary so a future Nagi Object service
/// can replace the host preview source without changing Notes domain code.
pub trait ObjectIdSource: Send + Sync {
    fn next_object_id(&self) -> ObjectId;
}

/// Host-preview-only ID source. It mixes process/time entropy with an
/// incrementing counter; it does not claim to be a Nagi Object ID service.
pub struct HostObjectIdSource {
    seed: u64,
    sequence: AtomicU64,
}

impl HostObjectIdSource {
    pub fn new() -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        let seed = nanos ^ (u64::from(std::process::id()) << 32);
        Self {
            seed,
            sequence: AtomicU64::new(1),
        }
    }
}

impl Default for HostObjectIdSource {
    fn default() -> Self {
        Self::new()
    }
}

impl ObjectIdSource for HostObjectIdSource {
    fn next_object_id(&self) -> ObjectId {
        let sequence = self.sequence.fetch_add(1, Ordering::Relaxed);
        let mut value = self.seed ^ sequence.wrapping_mul(0x9e37_79b9_7f4a_7c15);
        value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        value ^= value >> 31;
        ObjectId(value.max(1))
    }
}
