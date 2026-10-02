use std::fmt;

/// Hard upper bounds. Configured [`ClipboardLimits`] may be lower, never higher.
pub mod hard_caps {
    /// Maximum ordered items in one clipboard content.
    pub const MAX_ITEMS: usize = 64;
    /// Maximum representations offered for one item.
    pub const MAX_REPRESENTATIONS_PER_ITEM: usize = 32;
    /// Maximum inline payload bytes for one representation (16 MiB).
    pub const MAX_REPRESENTATION_BYTES: usize = 16 * 1024 * 1024;
    /// Maximum inline payload bytes across the whole content (64 MiB).
    pub const MAX_TOTAL_BYTES: usize = 64 * 1024 * 1024;
    /// Maximum untrusted metadata entries.
    pub const MAX_METADATA_ENTRIES: usize = 32;
    /// Maximum metadata value bytes.
    pub const MAX_METADATA_VALUE_BYTES: usize = 1024;
}

/// Maximum metadata key bytes (fixed; keys are locale-neutral identifiers).
pub const MAX_METADATA_KEY_BYTES: usize = 64;
/// Maximum bytes of the untrusted origin label.
pub const MAX_ORIGIN_LABEL_BYTES: usize = 256;

/// Bounds enforced on content handled directly by this foundation.
///
/// Oversized content is rejected as a whole. Nothing is ever truncated.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClipboardLimits {
    /// Maximum ordered items.
    pub max_items: usize,
    /// Maximum representations per item.
    pub max_representations_per_item: usize,
    /// Maximum inline bytes for one representation.
    pub max_representation_bytes: usize,
    /// Maximum inline bytes across all representations.
    pub max_total_bytes: usize,
    /// Maximum untrusted metadata entries.
    pub max_metadata_entries: usize,
    /// Maximum bytes of one metadata value.
    pub max_metadata_value_bytes: usize,
}

impl ClipboardLimits {
    /// Conservative documented defaults for the host reference.
    pub const DEFAULT: Self = Self {
        max_items: 16,
        max_representations_per_item: 8,
        max_representation_bytes: 1024 * 1024,
        max_total_bytes: 4 * 1024 * 1024,
        max_metadata_entries: 8,
        max_metadata_value_bytes: 256,
    };

    /// Returns an error if any limit is zero or exceeds its hard cap.
    pub fn validate(&self) -> Result<(), LimitsError> {
        let checks = [
            (self.max_items, hard_caps::MAX_ITEMS),
            (
                self.max_representations_per_item,
                hard_caps::MAX_REPRESENTATIONS_PER_ITEM,
            ),
            (
                self.max_representation_bytes,
                hard_caps::MAX_REPRESENTATION_BYTES,
            ),
            (self.max_total_bytes, hard_caps::MAX_TOTAL_BYTES),
            (self.max_metadata_entries, hard_caps::MAX_METADATA_ENTRIES),
            (
                self.max_metadata_value_bytes,
                hard_caps::MAX_METADATA_VALUE_BYTES,
            ),
        ];
        if checks.iter().any(|&(value, cap)| value == 0 || value > cap) {
            return Err(LimitsError);
        }
        Ok(())
    }
}

impl Default for ClipboardLimits {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// A configured limit was zero or above its hard cap.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LimitsError;

impl fmt::Display for LimitsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("clipboard limits must be non-zero and within hard caps")
    }
}

impl std::error::Error for LimitsError {}
