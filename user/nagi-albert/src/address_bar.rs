//! Browser address-bar editing and deterministic URL normalization.

use url::Url;

use crate::ime::{TextEditError, TextEntry};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AddressError {
    Empty,
    InvalidUrl,
    UnsupportedScheme,
    CredentialsNotAllowed,
    TooLong,
    CompositionActive,
    Text(TextEditError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AddressBar {
    entry: TextEntry,
    focused: bool,
    invalid: bool,
}

impl AddressBar {
    pub fn new() -> Self {
        Self {
            entry: TextEntry::default(),
            focused: false,
            invalid: false,
        }
    }

    pub fn text(&self) -> String {
        self.entry.visible_text()
    }

    pub fn committed_text(&self) -> &str {
        self.entry.text()
    }

    pub fn is_focused(&self) -> bool {
        self.focused
    }

    pub fn is_invalid(&self) -> bool {
        self.invalid
    }

    pub fn is_composing(&self) -> bool {
        self.entry.is_composing()
    }

    pub fn focus(&mut self) {
        self.focused = true;
        self.entry.select_all();
        self.invalid = false;
    }

    pub fn blur(&mut self) {
        self.focused = false;
        self.invalid = false;
    }

    pub fn set_text(&mut self, text: impl Into<String>) -> Result<(), AddressError> {
        self.entry.set_text(text).map_err(AddressError::Text)?;
        self.invalid = false;
        Ok(())
    }

    pub fn insert_text(&mut self, text: &str) -> Result<(), AddressError> {
        self.entry.insert(text).map_err(AddressError::Text)?;
        self.invalid = false;
        Ok(())
    }

    pub fn delete_backward(&mut self) -> Result<bool, AddressError> {
        let changed = self.entry.delete_backward().map_err(AddressError::Text)?;
        self.invalid = false;
        Ok(changed)
    }

    pub fn update_composition(
        &mut self,
        text: &str,
        selection: core::ops::Range<usize>,
    ) -> Result<(), AddressError> {
        self.entry
            .update_composition(text, selection)
            .map_err(AddressError::Text)
    }

    pub fn commit_composition(&mut self, text: &str) -> Result<(), AddressError> {
        self.entry
            .commit_composition(text)
            .map_err(AddressError::Text)
    }

    pub fn cancel_composition(&mut self) -> Result<(), AddressError> {
        self.entry.cancel_composition().map_err(AddressError::Text)
    }

    pub fn submit(&mut self) -> Result<String, AddressError> {
        if self.entry.is_composing() {
            return Err(AddressError::CompositionActive);
        }
        match normalize_address(self.entry.text()) {
            Ok(url) => {
                self.invalid = false;
                Ok(url)
            }
            Err(error) => {
                self.invalid = true;
                Err(error)
            }
        }
    }
}

impl Default for AddressBar {
    fn default() -> Self {
        Self::new()
    }
}

/// Normalize a typed address without accepting script or privileged schemes.
pub fn normalize_address(input: &str) -> Result<String, AddressError> {
    let input = input.trim();
    if input.is_empty() {
        return Err(AddressError::Empty);
    }
    if input.len() > crate::ime::MAX_TEXT_BYTES {
        return Err(AddressError::TooLong);
    }
    let lower = input.to_ascii_lowercase();
    let unsupported_scheme = ["javascript:", "file:", "data:", "blob:"]
        .iter()
        .any(|scheme| lower.starts_with(scheme));
    if unsupported_scheme {
        return Err(AddressError::UnsupportedScheme);
    }
    let candidate = if input.starts_with("about:") || input.contains("://") {
        input.to_owned()
    } else {
        format!("https://{input}")
    };
    let parsed = Url::parse(&candidate).map_err(|_| AddressError::InvalidUrl)?;
    match parsed.scheme() {
        "about" if parsed.as_str() == "about:blank" => Ok("about:blank".to_owned()),
        "http" | "https" => {
            if parsed.host_str().is_none() {
                return Err(AddressError::InvalidUrl);
            }
            if !parsed.username().is_empty() || parsed.password().is_some() {
                return Err(AddressError::CredentialsNotAllowed);
            }
            let normalized = parsed.to_string();
            if normalized.len() > crate::ime::MAX_TEXT_BYTES {
                return Err(AddressError::TooLong);
            }
            Ok(normalized)
        }
        _ => Err(AddressError::UnsupportedScheme),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_hosts_and_explicit_https_are_normalized() {
        assert_eq!(
            normalize_address(" example.com/path ").unwrap(),
            "https://example.com/path"
        );
        assert_eq!(
            normalize_address("HTTP://Example.COM").unwrap(),
            "http://example.com/"
        );
        assert_eq!(normalize_address("about:blank").unwrap(), "about:blank");
    }

    #[test]
    fn invalid_and_privileged_inputs_are_rejected() {
        assert_eq!(normalize_address(""), Err(AddressError::Empty));
        assert_eq!(
            normalize_address("javascript:alert(1)"),
            Err(AddressError::UnsupportedScheme)
        );
        assert_eq!(
            normalize_address("https://user:secret@example.com"),
            Err(AddressError::CredentialsNotAllowed)
        );
    }

    #[test]
    fn normalized_url_cannot_exceed_the_address_and_storage_bound() {
        let long_path = format!("https://example.test/{}", "日".repeat(1100));
        assert!(long_path.len() < crate::ime::MAX_TEXT_BYTES);
        assert_eq!(normalize_address(&long_path), Err(AddressError::TooLong));
    }

    #[test]
    fn submit_tracks_invalid_input_and_waits_for_ime_commit() {
        let mut bar = AddressBar::new();
        bar.set_text("javascript:alert(1)").unwrap();
        assert_eq!(bar.submit(), Err(AddressError::UnsupportedScheme));
        assert!(bar.is_invalid());
        bar.set_text("example.org/").unwrap();
        bar.update_composition("に", 0..3).unwrap();
        assert_eq!(bar.submit(), Err(AddressError::CompositionActive));
        bar.commit_composition("日本語").unwrap();
        assert_eq!(
            bar.submit(),
            Ok("https://example.org/%E6%97%A5%E6%9C%AC%E8%AA%9E".to_owned())
        );
    }

    #[test]
    fn focusing_selects_the_full_value_for_keyboard_replacement() {
        let mut bar = AddressBar::new();
        bar.set_text("https://example.org/path").unwrap();
        bar.focus();
        assert!(bar.is_focused());
        bar.insert_text("https://nagi.example").unwrap();
        assert_eq!(bar.committed_text(), "https://nagi.example");
        bar.blur();
        assert!(!bar.is_focused());
    }
}
