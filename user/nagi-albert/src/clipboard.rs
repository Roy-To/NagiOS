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
    #[allow(dead_code)] // Only the M18 guest input path mints chrome contexts.
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

/// Gesture scope reserved for Albert's own chrome (address bar).
pub const CHROME_GESTURE_SCOPE: nagi_clipboard::GestureScope = nagi_clipboard::GestureScope(0);

const TAB_GESTURE_SCOPE_TAG: u64 = 1 << 63;

/// Gesture scope for one browser tab. Tab scopes carry a tag bit so that no
/// tab ID, including one restored from a damaged session, can alias the
/// chrome scope.
pub fn tab_gesture_scope(tab: crate::tabs::TabId) -> nagi_clipboard::GestureScope {
    nagi_clipboard::GestureScope(TAB_GESTURE_SCOPE_TAG | tab.0)
}

/// [`ClipboardRuntime`] backed by the Nagi user-space clipboard service.
///
/// A [`ClipboardContext`] can only be minted by trusted chrome input
/// handling, so each chrome operation records its gesture in
/// [`CHROME_GESTURE_SCOPE`] immediately before using it.
pub struct NagiClipboardRuntime<'a, Clock: FnMut() -> u64> {
    endpoint: &'a nagi_clipboard::ClipboardEndpoint,
    gestures: &'a nagi_clipboard::GestureSource,
    now: Clock,
}

impl<'a, Clock: FnMut() -> u64> NagiClipboardRuntime<'a, Clock> {
    pub fn new(
        endpoint: &'a nagi_clipboard::ClipboardEndpoint,
        gestures: &'a nagi_clipboard::GestureSource,
        now: Clock,
    ) -> Self {
        Self {
            endpoint,
            gestures,
            now,
        }
    }
}

impl<Clock: FnMut() -> u64> ClipboardRuntime for NagiClipboardRuntime<'_, Clock> {
    fn read_text(&mut self, context: &ClipboardContext) -> Result<String, ClipboardAccessError> {
        if !context.has_user_gesture() {
            return Err(ClipboardAccessError::Denied);
        }
        let now = (self.now)();
        self.gestures
            .record_paste(CHROME_GESTURE_SCOPE, now)
            .map_err(access_error)?;
        self.endpoint
            .read_text(CHROME_GESTURE_SCOPE, now)
            .map_err(access_error)
    }

    fn write_text(
        &mut self,
        context: &ClipboardContext,
        value: &str,
    ) -> Result<(), ClipboardAccessError> {
        if !context.has_user_gesture() {
            return Err(ClipboardAccessError::Denied);
        }
        let now = (self.now)();
        self.gestures
            .record_activation(CHROME_GESTURE_SCOPE, now)
            .map_err(access_error)?;
        self.endpoint
            .write_text(CHROME_GESTURE_SCOPE, value, now)
            .map(|_| ())
            .map_err(access_error)
    }
}

pub fn access_error(error: nagi_clipboard::ClipboardError) -> ClipboardAccessError {
    use nagi_clipboard::ClipboardError;
    match error {
        ClipboardError::MissingRight
        | ClipboardError::NoUserGesture
        | ClipboardError::UnknownClient => ClipboardAccessError::Denied,
        ClipboardError::TooLarge
        | ClipboardError::ClientTableFull
        | ClipboardError::GestureTableFull => ClipboardAccessError::Unavailable,
    }
}

/// Reduce pasted text to one address-bar line: control characters
/// (including newlines and tabs) become spaces and the ends are trimmed.
pub fn single_line_paste(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect::<String>()
        .trim()
        .to_owned()
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

    fn nagi_service() -> (
        nagi_clipboard::ClipboardServiceOwner,
        nagi_clipboard::ClipboardEndpoint,
        nagi_clipboard::GestureSource,
    ) {
        let owner = nagi_clipboard::ClipboardServiceOwner::new(nagi_clipboard::ClipboardPolicy {
            paste_grant_ticks: 5,
            activation_ticks: 5,
        });
        let (endpoint, gestures) = owner
            .register_client(nagi_clipboard::ClipboardRights::READ_WRITE)
            .unwrap();
        (owner, endpoint, gestures)
    }

    #[test]
    fn chrome_copy_and_paste_use_the_nagi_clipboard_service() {
        let (owner, endpoint, gestures) = nagi_service();
        let mut ticks = 0;
        let mut runtime = NagiClipboardRuntime::new(&endpoint, &gestures, || {
            ticks += 1;
            ticks
        });
        let context = ClipboardContext::from_trusted_browser_chrome(None);
        let mut clipboard = BrowserClipboard::new();
        clipboard
            .copy(&mut runtime, &context, "https://example.com/")
            .unwrap();
        assert_eq!(
            clipboard.paste(&mut runtime, &context).unwrap(),
            "https://example.com/"
        );
        assert_eq!(owner.stats().reads, 1);
        assert_eq!(owner.stats().writes, 1);
    }

    #[test]
    fn chrome_gestures_do_not_authorize_tab_reads() {
        let (_owner, endpoint, gestures) = nagi_service();
        let mut runtime = NagiClipboardRuntime::new(&endpoint, &gestures, || 1);
        let context = ClipboardContext::from_trusted_browser_chrome(None);
        runtime.write_text(&context, "secret").unwrap();
        let tab = tab_gesture_scope(crate::tabs::TabId(1));
        assert_ne!(tab, CHROME_GESTURE_SCOPE);
        assert_ne!(
            tab_gesture_scope(crate::tabs::TabId(0)),
            CHROME_GESTURE_SCOPE
        );
        assert_eq!(
            endpoint.read_text(tab, 1),
            Err(nagi_clipboard::ClipboardError::NoUserGesture)
        );
    }

    #[test]
    fn service_denials_are_visible_to_the_chrome() {
        let owner = nagi_clipboard::ClipboardServiceOwner::new(nagi_clipboard::ClipboardPolicy {
            paste_grant_ticks: 5,
            activation_ticks: 5,
        });
        let (write_only, gestures) = owner
            .register_client(nagi_clipboard::ClipboardRights::WRITE)
            .unwrap();
        let mut runtime = NagiClipboardRuntime::new(&write_only, &gestures, || 1);
        let context = ClipboardContext::from_trusted_browser_chrome(None);
        let mut clipboard = BrowserClipboard::new();
        assert_eq!(
            clipboard.paste(&mut runtime, &context),
            Err(ClipboardAccessError::Denied)
        );
        assert_eq!(clipboard.status(), ClipboardStatus::Denied);
    }

    #[test]
    fn pasted_addresses_are_reduced_to_one_line() {
        assert_eq!(
            single_line_paste("  https://example.com/\r\n"),
            "https://example.com/"
        );
        assert_eq!(single_line_paste("a\tb\nc"), "a b c");
        assert_eq!(single_line_paste("なぎ"), "なぎ");
    }
}
