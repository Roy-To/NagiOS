//! Clipboard boundary for trusted browser chrome and explicit user gestures.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClipboardOperation {
    Read,
    Write,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClipboardContext {
    pub origin: Option<String>,
    user_gesture: TrustedClipboardGesture,
}

/// A gesture marker that can only be minted by trusted Albert input handling.
#[derive(Clone, Debug, Eq, PartialEq)]
struct TrustedClipboardGesture {
    _sealed: (),
}

impl ClipboardContext {
    #[allow(dead_code)] // M17 does not yet pass an input handle to Albert.
    pub(crate) fn from_trusted_browser_chrome(origin: Option<String>) -> Self {
        Self {
            origin,
            user_gesture: TrustedClipboardGesture { _sealed: () },
        }
    }

    pub fn has_user_gesture(&self) -> bool {
        let _ = &self.user_gesture;
        true
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClipboardStatus {
    Ready,
    Denied,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClipboardAccessError {
    Denied,
    Unavailable,
}

/// Implemented by a capability-checking clipboard service adapter.
pub trait ClipboardRuntime {
    fn read_text(&mut self, context: &ClipboardContext) -> Result<String, ClipboardAccessError>;

    fn write_text(
        &mut self,
        context: &ClipboardContext,
        value: &str,
    ) -> Result<(), ClipboardAccessError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BrowserClipboard {
    status: ClipboardStatus,
}

impl BrowserClipboard {
    pub fn new() -> Self {
        Self {
            status: ClipboardStatus::Unavailable,
        }
    }

    pub fn status(&self) -> ClipboardStatus {
        self.status
    }

    pub fn copy(
        &mut self,
        runtime: &mut impl ClipboardRuntime,
        context: &ClipboardContext,
        value: &str,
    ) -> Result<(), ClipboardAccessError> {
        self.status = ClipboardStatus::Unavailable;
        match runtime.write_text(context, value) {
            Ok(()) => {
                self.status = ClipboardStatus::Ready;
                Ok(())
            }
            Err(error) => {
                self.status = status_for(error);
                Err(error)
            }
        }
    }

    pub fn paste(
        &mut self,
        runtime: &mut impl ClipboardRuntime,
        context: &ClipboardContext,
    ) -> Result<String, ClipboardAccessError> {
        self.status = ClipboardStatus::Unavailable;
        match runtime.read_text(context) {
            Ok(value) => {
                self.status = ClipboardStatus::Ready;
                Ok(value)
            }
            Err(error) => {
                self.status = status_for(error);
                Err(error)
            }
        }
    }
}

impl Default for BrowserClipboard {
    fn default() -> Self {
        Self::new()
    }
}

fn status_for(error: ClipboardAccessError) -> ClipboardStatus {
    match error {
        ClipboardAccessError::Denied => ClipboardStatus::Denied,
        ClipboardAccessError::Unavailable => ClipboardStatus::Unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestClipboard {
        value: String,
        error: Option<ClipboardAccessError>,
    }

    impl ClipboardRuntime for TestClipboard {
        fn read_text(
            &mut self,
            _context: &ClipboardContext,
        ) -> Result<String, ClipboardAccessError> {
            self.error.map_or_else(|| Ok(self.value.clone()), Err)
        }

        fn write_text(
            &mut self,
            _context: &ClipboardContext,
            value: &str,
        ) -> Result<(), ClipboardAccessError> {
            if let Some(error) = self.error {
                return Err(error);
            }
            self.value = value.to_owned();
            Ok(())
        }
    }

    #[test]
    fn copy_and_paste_are_routed_through_the_runtime_adapter() {
        let context =
            ClipboardContext::from_trusted_browser_chrome(Some("https://example.test".to_owned()));
        let mut runtime = TestClipboard {
            value: String::new(),
            error: None,
        };
        let mut clipboard = BrowserClipboard::new();
        clipboard.copy(&mut runtime, &context, "copied").unwrap();
        assert_eq!(clipboard.status(), ClipboardStatus::Ready);
        assert_eq!(clipboard.paste(&mut runtime, &context).unwrap(), "copied");
    }

    #[test]
    fn denied_or_missing_clipboard_service_is_visible_to_the_ui() {
        let context = ClipboardContext::from_trusted_browser_chrome(None);
        let mut runtime = TestClipboard {
            value: String::new(),
            error: Some(ClipboardAccessError::Denied),
        };
        let mut clipboard = BrowserClipboard::new();
        assert_eq!(
            clipboard.paste(&mut runtime, &context),
            Err(ClipboardAccessError::Denied)
        );
        assert_eq!(clipboard.status(), ClipboardStatus::Denied);
    }
}
