//! Composition-aware text editing boundary for browser chrome.

use core::ops::Range;

pub const MAX_TEXT_BYTES: usize = 4096;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TextEditError {
    TooLong,
    InvalidSelection,
    CompositionActive,
    NoComposition,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Composition {
    pub text: String,
    pub selection: Range<usize>,
}

/// UTF-8 text input state that keeps IME preedit separate from committed text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TextEntry {
    text: String,
    selection: Range<usize>,
    composition: Option<Composition>,
}

impl TextEntry {
    pub fn new(text: impl Into<String>) -> Result<Self, TextEditError> {
        let text = text.into();
        if text.len() > MAX_TEXT_BYTES {
            return Err(TextEditError::TooLong);
        }
        let end = text.len();
        Ok(Self {
            text,
            selection: end..end,
            composition: None,
        })
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn selection(&self) -> Range<usize> {
        self.selection.clone()
    }

    pub fn composition(&self) -> Option<&Composition> {
        self.composition.as_ref()
    }

    pub fn is_composing(&self) -> bool {
        self.composition.is_some()
    }

    pub fn visible_text(&self) -> String {
        let Some(composition) = &self.composition else {
            return self.text.clone();
        };
        let mut visible = String::with_capacity(self.text.len() + composition.text.len());
        visible.push_str(&self.text[..self.selection.start]);
        visible.push_str(&composition.text);
        visible.push_str(&self.text[self.selection.end..]);
        visible
    }

    pub fn set_text(&mut self, text: impl Into<String>) -> Result<(), TextEditError> {
        let text = text.into();
        if text.len() > MAX_TEXT_BYTES {
            return Err(TextEditError::TooLong);
        }
        let end = text.len();
        self.text = text;
        self.selection = end..end;
        self.composition = None;
        Ok(())
    }

    pub fn select(&mut self, range: Range<usize>) -> Result<(), TextEditError> {
        if range.start > range.end
            || range.end > self.text.len()
            || !self.text.is_char_boundary(range.start)
            || !self.text.is_char_boundary(range.end)
        {
            return Err(TextEditError::InvalidSelection);
        }
        self.selection = range;
        self.composition = None;
        Ok(())
    }

    pub fn select_all(&mut self) {
        self.selection = 0..self.text.len();
        self.composition = None;
    }

    pub fn insert(&mut self, inserted: &str) -> Result<(), TextEditError> {
        if self.is_composing() {
            return Err(TextEditError::CompositionActive);
        }
        let new_len = self
            .text
            .len()
            .saturating_sub(self.selection.end - self.selection.start)
            .saturating_add(inserted.len());
        if new_len > MAX_TEXT_BYTES {
            return Err(TextEditError::TooLong);
        }
        self.text.replace_range(self.selection.clone(), inserted);
        let cursor = self.selection.start + inserted.len();
        self.selection = cursor..cursor;
        Ok(())
    }

    pub fn delete_backward(&mut self) -> Result<bool, TextEditError> {
        if self.is_composing() {
            return Err(TextEditError::CompositionActive);
        }
        if !self.selection.is_empty() {
            self.text.replace_range(self.selection.clone(), "");
            self.selection.end = self.selection.start;
            return Ok(true);
        }
        if self.selection.start == 0 {
            return Ok(false);
        }
        let previous = self.text[..self.selection.start]
            .char_indices()
            .next_back()
            .map(|(index, _)| index)
            .unwrap_or(0);
        self.text.replace_range(previous..self.selection.start, "");
        self.selection = previous..previous;
        Ok(true)
    }

    pub fn update_composition(
        &mut self,
        preedit: &str,
        selection: Range<usize>,
    ) -> Result<(), TextEditError> {
        if preedit.len() > MAX_TEXT_BYTES
            || selection.start > selection.end
            || selection.end > preedit.len()
            || !preedit.is_char_boundary(selection.start)
            || !preedit.is_char_boundary(selection.end)
        {
            return Err(if preedit.len() > MAX_TEXT_BYTES {
                TextEditError::TooLong
            } else {
                TextEditError::InvalidSelection
            });
        }
        self.composition = Some(Composition {
            text: preedit.to_owned(),
            selection,
        });
        Ok(())
    }

    pub fn commit_composition(&mut self, committed: &str) -> Result<(), TextEditError> {
        if self.composition.is_none() {
            return Err(TextEditError::NoComposition);
        }
        let new_len = self
            .text
            .len()
            .saturating_sub(self.selection.end - self.selection.start)
            .saturating_add(committed.len());
        if new_len > MAX_TEXT_BYTES {
            return Err(TextEditError::TooLong);
        }
        self.text.replace_range(self.selection.clone(), committed);
        let cursor = self.selection.start + committed.len();
        self.selection = cursor..cursor;
        self.composition = None;
        Ok(())
    }

    pub fn cancel_composition(&mut self) -> Result<(), TextEditError> {
        if self.composition.take().is_some() {
            Ok(())
        } else {
            Err(TextEditError::NoComposition)
        }
    }
}

impl Default for TextEntry {
    fn default() -> Self {
        Self::new("").expect("empty text is within the text limit")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn composition_is_visible_but_committed_only_after_commit() {
        let mut entry = TextEntry::new("https://").unwrap();
        entry.update_composition("に", 0..3).unwrap();
        assert_eq!(entry.text(), "https://");
        assert_eq!(entry.visible_text(), "https://に");
        entry.commit_composition("日本語").unwrap();
        assert_eq!(entry.text(), "https://日本語");
        assert!(!entry.is_composing());
    }

    #[test]
    fn cancel_discards_preedit_and_selection_editing_is_utf8_safe() {
        let mut entry = TextEntry::new("nagi日本語").unwrap();
        entry.select(4..13).unwrap();
        assert!(matches!(
            entry.select(5..6),
            Err(TextEditError::InvalidSelection)
        ));
        entry.update_composition("あ", 0..3).unwrap();
        entry.cancel_composition().unwrap();
        entry.insert("OS").unwrap();
        assert_eq!(entry.text(), "nagiOS");
    }

    #[test]
    fn editing_respects_the_bounded_text_capacity() {
        let mut entry = TextEntry::new("x".repeat(MAX_TEXT_BYTES)).unwrap();
        assert_eq!(entry.insert("y"), Err(TextEditError::TooLong));
        assert_eq!(entry.text().len(), MAX_TEXT_BYTES);
    }

    #[test]
    fn constructor_rejects_text_over_the_bound() {
        assert!(matches!(
            TextEntry::new("x".repeat(MAX_TEXT_BYTES + 1)),
            Err(TextEditError::TooLong)
        ));
    }

    #[test]
    fn rejected_ime_commit_preserves_the_preedit_for_recovery() {
        let mut entry = TextEntry::new("x".repeat(MAX_TEXT_BYTES)).unwrap();
        entry.select_all();
        entry.update_composition("日本語", 0..9).unwrap();
        assert_eq!(
            entry.commit_composition(&"y".repeat(MAX_TEXT_BYTES + 1)),
            Err(TextEditError::TooLong)
        );
        assert!(entry.is_composing());
        assert_eq!(entry.text().len(), MAX_TEXT_BYTES);
        entry.commit_composition("日本語").unwrap();
        assert_eq!(entry.text(), "日本語");
        assert!(!entry.is_composing());
    }
}
