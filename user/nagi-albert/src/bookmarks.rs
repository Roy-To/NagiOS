//! Browser bookmarks with stable identities and duplicate URL updates.

use crate::address_bar::{normalize_address, AddressError};

pub const MAX_BOOKMARKS: usize = 500;
pub const MAX_BOOKMARK_TITLE_BYTES: usize = 512;

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct BookmarkId(pub u64);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Bookmark {
    pub id: BookmarkId,
    pub url: String,
    pub title: String,
    pub created_at: u64,
    pub updated_at: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BookmarkError {
    InvalidAddress(AddressError),
    NotFound,
    CapacityReached,
    IdsExhausted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BookmarkStore {
    entries: Vec<Bookmark>,
    next_id: u64,
}

impl BookmarkStore {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            next_id: 1,
        }
    }

    pub fn entries(&self) -> &[Bookmark] {
        &self.entries
    }

    pub fn add_or_update(
        &mut self,
        input_url: &str,
        title: &str,
        now: u64,
    ) -> Result<BookmarkId, BookmarkError> {
        let url = normalize_address(input_url).map_err(BookmarkError::InvalidAddress)?;
        if let Some(existing) = self.entries.iter_mut().find(|item| item.url == url) {
            existing.title = super::history::bounded_text(title, MAX_BOOKMARK_TITLE_BYTES);
            existing.updated_at = now;
            return Ok(existing.id);
        }
        if self.entries.len() >= MAX_BOOKMARKS {
            return Err(BookmarkError::CapacityReached);
        }
        let id = BookmarkId(self.next_id);
        self.next_id = self
            .next_id
            .checked_add(1)
            .ok_or(BookmarkError::IdsExhausted)?;
        self.entries.push(Bookmark {
            id,
            url,
            title: super::history::bounded_text(title, MAX_BOOKMARK_TITLE_BYTES),
            created_at: now,
            updated_at: now,
        });
        Ok(id)
    }

    pub fn remove(&mut self, id: BookmarkId) -> Result<Bookmark, BookmarkError> {
        let index = self
            .entries
            .iter()
            .position(|item| item.id == id)
            .ok_or(BookmarkError::NotFound)?;
        Ok(self.entries.remove(index))
    }

    pub fn get(&self, id: BookmarkId) -> Option<&Bookmark> {
        self.entries.iter().find(|item| item.id == id)
    }

    pub fn open_url(&self, id: BookmarkId) -> Result<&str, BookmarkError> {
        self.get(id)
            .map(|bookmark| bookmark.url.as_str())
            .ok_or(BookmarkError::NotFound)
    }

    pub(crate) fn from_entries(entries: Vec<Bookmark>) -> Result<Self, BookmarkError> {
        if entries.len() > MAX_BOOKMARKS {
            return Err(BookmarkError::CapacityReached);
        }
        let mut next_id = 1;
        for bookmark in &entries {
            next_id = next_id.max(bookmark.id.0.saturating_add(1));
        }
        Ok(Self { entries, next_id })
    }
}

impl Default for BookmarkStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_add_updates_metadata_without_creating_an_entry() {
        let mut bookmarks = BookmarkStore::new();
        let id = bookmarks.add_or_update("EXAMPLE.COM", "First", 10).unwrap();
        let duplicate = bookmarks
            .add_or_update("https://example.com/", "Updated", 20)
            .unwrap();
        assert_eq!(id, duplicate);
        assert_eq!(bookmarks.entries().len(), 1);
        assert_eq!(bookmarks.get(id).unwrap().title, "Updated");
        assert_eq!(bookmarks.get(id).unwrap().created_at, 10);
        assert_eq!(bookmarks.get(id).unwrap().updated_at, 20);
    }

    #[test]
    fn remove_and_open_use_stable_bookmark_identity() {
        let mut bookmarks = BookmarkStore::new();
        let id = bookmarks
            .add_or_update("example.net", "Example", 1)
            .unwrap();
        assert_eq!(bookmarks.open_url(id), Ok("https://example.net/"));
        assert_eq!(bookmarks.remove(id).unwrap().id, id);
        assert_eq!(bookmarks.open_url(id), Err(BookmarkError::NotFound));
    }
}
