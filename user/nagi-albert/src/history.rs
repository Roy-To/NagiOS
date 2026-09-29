//! Bounded browser visit history and navigation cursors.

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct HistoryEntryId(pub u64);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HistoryEntry {
    pub id: HistoryEntryId,
    pub url: String,
    pub title: String,
    pub visited_at: u64,
}

pub const MAX_HISTORY_ENTRIES: usize = 2000;
pub const MAX_TAB_HISTORY_ENTRIES: usize = 200;
pub const MAX_HISTORY_TITLE_BYTES: usize = 512;

pub(crate) fn bounded_text(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_limits_preserve_utf8_boundaries() {
        let value = format!("{}あ", "x".repeat(MAX_HISTORY_TITLE_BYTES - 1));
        let limited = bounded_text(&value, MAX_HISTORY_TITLE_BYTES);
        assert!(limited.len() <= MAX_HISTORY_TITLE_BYTES);
        assert!(limited.is_char_boundary(limited.len()));
    }
}
