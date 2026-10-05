//! Guest-side integration gate for Albert's HTTPS navigation and rendering path.

#[cfg(target_os = "nagi")]
use std::sync::atomic::{AtomicU8, Ordering};

#[cfg(any(target_os = "nagi", test))]
const HTTPS_PAGES: [(&str, &str); 3] = [
    ("https://example.com/", "example.com"),
    ("https://example.org/", "example.org"),
    ("https://example.net/", "example.net"),
];

#[cfg(target_os = "nagi")]
const SERVO_CONFIG_DIR: &str = "/tmp/nagi-servo-profile";

#[cfg(target_os = "nagi")]
static VERIFIED_HOST_MASK: AtomicU8 = AtomicU8::new(0);

#[cfg(any(target_os = "nagi", test))]
fn verified_host_index(host: &[u8]) -> Option<(usize, &'static str)> {
    HTTPS_PAGES
        .iter()
        .enumerate()
        .find_map(|(index, (_, expected_host))| {
            (expected_host.as_bytes() == host).then_some((index, *expected_host))
        })
}

/// Called only by the pinned Servo TLS verifier after its normal WebPKI
/// verifier accepts both the certificate chain and requested server name.
#[cfg(target_os = "nagi")]
pub(crate) fn record_tls_verification(host: &[u8]) {
    let Some((index, expected_host)) = verified_host_index(host) else {
        return;
    };

    if VERIFIED_HOST_MASK.fetch_or(1 << index, Ordering::AcqRel) & (1 << index) != 0 {
        return;
    }

    let prefix = b"Nagi M18 HTTPS TLS PASS host=";
    let suffix = b" chain=verified hostname=verified\r\n";
    let mut line = [0_u8; 128];
    let length = prefix.len() + expected_host.len() + suffix.len();
    line[..prefix.len()].copy_from_slice(prefix);
    line[prefix.len()..prefix.len() + expected_host.len()]
        .copy_from_slice(expected_host.as_bytes());
    line[prefix.len() + expected_host.len()..length].copy_from_slice(suffix);
    let _ = libnagi::console_write(&line[..length]);
}

#[cfg(test)]
mod tests {
    use super::verified_host_index;

    #[test]
    fn tls_evidence_uses_only_the_allowlisted_hostname() {
        assert_eq!(
            verified_host_index(b"example.com"),
            Some((0, "example.com"))
        );
        assert_eq!(
            verified_host_index(b"example.org"),
            Some((1, "example.org"))
        );
        assert_eq!(
            verified_host_index(b"example.net"),
            Some((2, "example.net"))
        );
        assert_eq!(verified_host_index(b"https://example.com/"), None);
        assert_eq!(verified_host_index(b"attacker.example"), None);
    }
}

#[cfg(target_os = "nagi")]
fn tls_verified(index: usize) -> bool {
    VERIFIED_HOST_MASK.load(Ordering::Acquire) & (1 << index) != 0
}

#[cfg(target_os = "nagi")]
mod guest {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::sync::Arc;

    use dpi::PhysicalSize;
    use nagi_servo_adapter::{BrowserInput, EventLoopSignal, InputBridge, NagiSurface};
    use servo::{
        ClipboardDelegate, Code, DeviceIntPoint, DeviceIntRect, DeviceIntSize, DevicePoint,
        EmbedderControl, EventLoopWaker, InputEvent as ServoInputEvent, Key, KeyState,
        KeyboardEvent, LoadStatus, Location, Modifiers, MouseButton, MouseButtonAction,
        MouseButtonEvent, MouseMoveEvent, NamedKey, PermissionFeature, PermissionRequest,
        RenderingContext, Servo, ServoBuilder, SoftwareRenderingContext, StringRequest, WebView,
        WebViewBuilder, WebViewDelegate,
    };
    use url::Url;

    use super::{tls_verified, Ordering, HTTPS_PAGES, SERVO_CONFIG_DIR, VERIFIED_HOST_MASK};
    use crate::browser_state::{BrowserState, NavigationEventResult};
    use crate::chrome_surface::{FilePickerView, PAGE_TOP};
    use crate::clipboard::{
        single_line_paste, tab_gesture_scope, BrowserClipboard, ClipboardContext,
        NagiClipboardRuntime,
    };
    use crate::downloads::{download_file_name, DOWNLOAD_DIRECTORY};
    use crate::file_picker::{
        picker_title, FilePickerState, PickerEntry, PickerKey, PickerOutcome, PICKER_DIRECTORY,
    };
    use crate::input::{
        chrome_action_at, clipboard_shortcut, evdev_character, ime_key, is_backspace_key,
        is_control_key, is_enter_key, is_escape_key, is_shift_key, ClipboardShortcut,
    };
    use crate::nagi_storage::GuestBrowserStorage;
    use crate::permission_prompt::{
        self, PermissionPromptAction, PermissionPromptLabels, PermissionPromptView,
    };
    use crate::permissions::{
        PermissionBrokerState, PermissionKind, PermissionRequestId, UserDecision,
    };
    use crate::tabs::TabId;
    use crate::ui::{self, BrowserChromeAction, BrowserChromeOutcome};
    use nagi_clipboard::{ClipboardEndpoint, ClipboardError, GestureScope, GestureSource};
    use nagi_ime::{ImeKey, ImeResponse, InputMethod, InputMode, KanaCandidates};
    use nagi_localization::Locale;
    use servo::{CompositionEvent, CompositionState, EmbedderControlId, ImeEvent};

    const WIDTH: u32 = 320;
    const HEIGHT: u32 = 200;
    /// Servo's viewport sits below Albert's chrome and status strip.
    const PAGE_HEIGHT: u32 = HEIGHT - PAGE_TOP;
    const PAGE_TIMEOUT_TICKS: u64 = 3_000;
    /// A download must follow a trusted page input within ~5 s.
    const DOWNLOAD_GESTURE_TICKS: u64 = 500;
    const MAX_PENDING_DOWNLOADS: usize = 4;
    /// Nagi VFS directory-entry name limit.
    const VFS_NAME_BYTES: usize = 32;
    /// Up to ~3 s for the frame that shows a just-observed DOM change.
    const SETTLE_FRAME_TICKS: u64 = 300;
    /// A frame pipeline idle for ~0.5 s is treated as settled.
    const SETTLE_QUIET_TICKS: u64 = 50;
    /// Summed RGB distance from the page background that counts as ink.
    const INK_THRESHOLD: u32 = 96;
    const PERMISSION_PROMPT_TIMEOUT_TICKS: u64 = 3_000;
    const MAX_PERMISSION_INPUT_DRAIN: usize = 64;

    #[derive(Clone)]
    struct NagiWaker(Arc<EventLoopSignal>);

    impl EventLoopWaker for NagiWaker {
        fn clone_box(&self) -> Box<dyn EventLoopWaker> {
            Box::new(self.clone())
        }

        fn wake(&self) {
            self.0.wake();
        }
    }

    struct PendingServoPermission {
        id: PermissionRequestId,
        request: PermissionRequest,
        origin: String,
        kind: PermissionKind,
        requested_at: u64,
    }

    struct AcceptanceDelegate {
        signal: Arc<EventLoopSignal>,
        navigation_started: Cell<bool>,
        frame_ready: Cell<bool>,
        tab_id: TabId,
        permission_broker: RefCell<PermissionBrokerState>,
        pending_permission: RefCell<Option<PendingServoPermission>>,
        permission_locale: Locale,
        /// Servo's input-method request for the focused text field, if any.
        ime_target: Cell<Option<EmbedderControlId>>,
        /// Latest URL Servo reported for this tab, not yet applied to the
        /// browser state.
        reported_url: RefCell<Option<Url>>,
        /// A page file input waiting for Albert's trusted picker.
        pending_file_picker: RefCell<Option<servo::FilePicker>>,
        /// Downloads Servo produced for this tab, not yet saved.
        pending_downloads: RefCell<Vec<servo::DownloadRequest>>,
        /// Tick of the last trusted page click or key press in this tab.
        last_page_input: Cell<Option<u64>>,
    }

    impl AcceptanceDelegate {
        fn new(signal: Arc<EventLoopSignal>, tab_id: TabId, permission_locale: Locale) -> Self {
            Self {
                signal,
                navigation_started: Cell::new(false),
                frame_ready: Cell::new(false),
                tab_id,
                permission_broker: RefCell::new(PermissionBrokerState::new()),
                pending_permission: RefCell::new(None),
                permission_locale,
                ime_target: Cell::new(None),
                reported_url: RefCell::new(None),
                pending_file_picker: RefCell::new(None),
                pending_downloads: RefCell::new(Vec::new()),
                last_page_input: Cell::new(None),
            }
        }

        fn reset(&self) {
            self.navigation_started.set(false);
            self.frame_ready.set(false);
        }

        fn clear_frame(&self) {
            self.frame_ready.set(false);
        }

        fn navigation_started(&self) -> bool {
            self.navigation_started.get()
        }

        fn has_frame(&self) -> bool {
            self.frame_ready.get()
        }

        fn pending_permission_info(
            &self,
        ) -> Option<(PermissionRequestId, String, PermissionKind, u64)> {
            self.pending_permission.borrow().as_ref().map(|pending| {
                (
                    pending.id,
                    pending.origin.clone(),
                    pending.kind,
                    pending.requested_at,
                )
            })
        }

        fn resolve_permission(&self, decision: UserDecision) -> bool {
            let Some(pending) = self.pending_permission.borrow_mut().take() else {
                return false;
            };
            let recorded = self
                .permission_broker
                .borrow_mut()
                .respond(pending.id, decision)
                .is_ok();
            if recorded && decision == UserDecision::Allow {
                pending.request.allow();
                report_permission_decision(UserDecision::Allow);
            } else {
                pending.request.deny();
                report_permission_decision(if recorded {
                    decision
                } else {
                    UserDecision::Deny
                });
            }
            self.signal.wake();
            true
        }
    }

    fn report_permission_decision(decision: UserDecision) {
        let marker: &[u8] = match decision {
            UserDecision::Allow => b"Nagi M18 site permission ALLOWED_BY_USER\r\n".as_slice(),
            UserDecision::Deny => b"Nagi M18 site permission DENIED_BY_USER\r\n",
            UserDecision::Dismiss => b"Nagi M18 site permission CANCELED_BY_USER\r\n",
        };
        let _ = libnagi::console_write(marker);
    }

    /// Albert's connection to the Nagi clipboard service. The endpoint is
    /// shared with per-tab Servo delegates; the gesture source stays with
    /// Albert's trusted input routing and is never reachable from content.
    struct InputServices {
        /// Trace routed device input on the serial console (acceptance only).
        trace_input: Cell<bool>,
        endpoint: ClipboardEndpoint,
        gestures: GestureSource,
        chrome: RefCell<BrowserClipboard>,
        counters: Rc<ClipboardCounters>,
        /// User-space input method; it sees keys only from Albert's
        /// trusted device-input routing.
        ime: RefCell<InputMethod<KanaCandidates>>,
        /// A composition is open in the focused page field.
        ime_composing: Cell<bool>,
        /// Evdev codes whose press the IME consumed; their release is
        /// swallowed too.
        ime_consumed_keys: RefCell<[bool; 256]>,
        ime_commits: Cell<u32>,
    }

    #[derive(Default)]
    struct ClipboardCounters {
        reads: Cell<u32>,
        writes: Cell<u32>,
        denied_reads: Cell<u32>,
        denied_writes: Cell<u32>,
        last_read: RefCell<String>,
    }

    #[derive(Clone, Copy, Default)]
    struct KeyModifiers {
        shift: bool,
        control: bool,
    }

    fn clipboard_error_reason(error: ClipboardError) -> &'static [u8] {
        match error {
            ClipboardError::MissingRight => b"missing-right",
            ClipboardError::NoUserGesture => b"no-user-gesture",
            ClipboardError::UnknownClient => b"unknown-client",
            ClipboardError::TooLarge => b"too-large",
            ClipboardError::ClientTableFull => b"client-table-full",
            ClipboardError::GestureTableFull => b"gesture-table-full",
        }
    }

    fn report_clipboard_denial(operation: &[u8], error: ClipboardError) {
        let _ = libnagi::console_write(b"Nagi M18 clipboard ");
        let _ = libnagi::console_write(operation);
        let _ = libnagi::console_write(b" DENIED reason=");
        let _ = libnagi::console_write(clipboard_error_reason(error));
        let _ = libnagi::console_write(b"\r\n");
    }

    /// Servo clipboard delegate for one tab. Every operation goes through
    /// the Nagi clipboard service, which requires a user gesture recorded by
    /// Albert's input routing for this tab's scope.
    struct NagiClipboardDelegate {
        endpoint: ClipboardEndpoint,
        scope: GestureScope,
        counters: Rc<ClipboardCounters>,
    }

    impl NagiClipboardDelegate {
        fn record_write(&self, result: Result<u64, ClipboardError>, operation: &[u8]) {
            match result {
                Ok(_) => self.counters.writes.set(self.counters.writes.get() + 1),
                Err(error) => {
                    self.counters
                        .denied_writes
                        .set(self.counters.denied_writes.get() + 1);
                    report_clipboard_denial(operation, error);
                }
            }
        }
    }

    impl ClipboardDelegate for NagiClipboardDelegate {
        fn clear(&self, _webview: WebView) {
            let result = self.endpoint.clear(self.scope, libnagi::time_ticks());
            self.record_write(result, b"clear");
        }

        fn get_text(&self, _webview: WebView, request: StringRequest) {
            match self.endpoint.read_text(self.scope, libnagi::time_ticks()) {
                Ok(text) => {
                    self.counters.reads.set(self.counters.reads.get() + 1);
                    self.counters.last_read.replace(text.clone());
                    request.success(text);
                }
                Err(error) => {
                    self.counters
                        .denied_reads
                        .set(self.counters.denied_reads.get() + 1);
                    report_clipboard_denial(b"read", error);
                    request.failure("Nagi clipboard access denied".to_owned());
                }
            }
        }

        fn set_text(&self, _webview: WebView, new_contents: String) {
            let result = self
                .endpoint
                .write_text(self.scope, &new_contents, libnagi::time_ticks());
            self.record_write(result, b"write");
        }
    }

    fn permission_kind(feature: PermissionFeature) -> PermissionKind {
        match feature {
            PermissionFeature::Geolocation => PermissionKind::Location,
            PermissionFeature::Notifications => PermissionKind::Notifications,
            PermissionFeature::Push => PermissionKind::Push,
            PermissionFeature::Midi => PermissionKind::Midi,
            PermissionFeature::Camera => PermissionKind::Camera,
            PermissionFeature::Microphone => PermissionKind::Microphone,
            PermissionFeature::Speaker => PermissionKind::Speaker,
            PermissionFeature::DeviceInfo => PermissionKind::DeviceInfo,
            PermissionFeature::BackgroundSync => PermissionKind::BackgroundSync,
            PermissionFeature::Bluetooth => PermissionKind::Bluetooth,
            PermissionFeature::PersistentStorage => PermissionKind::PersistentStorage,
            PermissionFeature::ScreenWakeLock(_) => PermissionKind::ScreenWakeLock,
            PermissionFeature::Gamepad => PermissionKind::Gamepad,
        }
    }

    struct TabRuntime {
        id: TabId,
        webview: WebView,
        delegate: Rc<AcceptanceDelegate>,
    }

    fn runtime_for_tab(runtimes: &[TabRuntime], tab_id: TabId) -> Option<&TabRuntime> {
        runtimes.iter().find(|runtime| runtime.id == tab_id)
    }

    fn active_runtime<'a>(
        browser_state: &BrowserState,
        runtimes: &'a [TabRuntime],
    ) -> Option<&'a TabRuntime> {
        browser_state
            .active_tab_id()
            .and_then(|tab_id| runtime_for_tab(runtimes, tab_id))
    }

    fn yield_guest_workers(signal: &EventLoopSignal) {
        // Nagi's bootstrap scheduler is cooperative. A pending embedder wake
        // does not mean the Constellation, script, or network worker yielded,
        // so hand off after every event-loop turn even when the signal is set.
        let _ = signal.take();
        let _ = libnagi::thread_yield();
    }

    fn sync_tab_runtimes(
        servo: &Servo,
        context: &Rc<SoftwareRenderingContext>,
        signal: &Arc<EventLoopSignal>,
        browser_state: &BrowserState,
        runtimes: &mut Vec<TabRuntime>,
        defer_initial_navigation: bool,
        permission_locale: Locale,
        services: &InputServices,
    ) {
        runtimes.retain(|runtime| browser_state.tab(runtime.id).is_some());
        for tab in browser_state.tabs() {
            if runtime_for_tab(runtimes, tab.id()).is_some() {
                continue;
            }
            let initial_url = if defer_initial_navigation {
                Url::parse("about:blank").expect("static blank URL is valid")
            } else {
                tab.navigation()
                    .display_url()
                    .and_then(|url| Url::parse(url).ok())
                    .unwrap_or_else(|| {
                        Url::parse("about:blank").expect("static blank URL is valid")
                    })
            };
            let delegate = Rc::new(AcceptanceDelegate::new(
                signal.clone(),
                tab.id(),
                permission_locale,
            ));
            let webview = WebViewBuilder::new(servo, context.clone())
                .url(initial_url)
                .delegate(delegate.clone())
                .clipboard_delegate(Rc::new(NagiClipboardDelegate {
                    endpoint: services.endpoint.clone(),
                    scope: tab_gesture_scope(tab.id()),
                    counters: services.counters.clone(),
                }))
                .build();
            runtimes.push(TabRuntime {
                id: tab.id(),
                webview,
                delegate,
            });
        }

        let active_tab = browser_state.active_tab_id();
        for runtime in runtimes {
            if Some(runtime.id) == active_tab {
                runtime.webview.show();
                // Page content receives keyboard input only while its
                // WebView holds Servo's keyboard focus. Albert chrome keys
                // (address bar) are handled before they reach Servo.
                if !runtime.webview.focused() {
                    runtime.webview.focus();
                }
            } else {
                runtime.webview.hide();
            }
        }
    }

    impl WebViewDelegate for AcceptanceDelegate {
        fn notify_new_frame_ready(&self, _webview: WebView) {
            self.frame_ready.set(true);
            self.signal.wake();
        }

        fn notify_page_title_changed(&self, _webview: WebView, title: Option<String>) {
            if let Some(title) = title.filter(|title| title.starts_with("nagi-")) {
                // Truncate on a character boundary so the serial log stays
                // valid UTF-8 for Japanese titles.
                let mut end = title.len().min(96);
                while !title.is_char_boundary(end) {
                    end -= 1;
                }
                let _ = libnagi::console_write(b"Nagi M18 fixture page title=");
                let _ = libnagi::console_write(&title.as_bytes()[..end]);
                let _ = libnagi::console_write(b"\r\n");
            }
        }

        fn notify_download_requested(&self, _webview: WebView, request: servo::DownloadRequest) {
            let mut pending = self.pending_downloads.borrow_mut();
            if pending.len() < MAX_PENDING_DOWNLOADS {
                pending.push(request);
                let _ = libnagi::console_write(b"Nagi M18 download requested\r\n");
            } else {
                let _ = libnagi::console_write(b"Nagi M18 download DROPPED reason=queue-full\r\n");
            }
            self.signal.wake();
        }

        fn notify_focus_changed(&self, _webview: WebView, focused: bool) {
            let _ = libnagi::console_write(if focused {
                b"Nagi M18 browser WebView focus=true\r\n".as_slice()
            } else {
                b"Nagi M18 browser WebView focus=false\r\n"
            });
        }

        fn notify_url_changed(&self, _webview: WebView, url: Url) {
            report_url(b"Nagi M18 browser URL changed to=", &url);
            self.reported_url.replace(Some(url.clone()));
            self.signal.wake();
        }

        fn notify_load_status_changed(&self, _webview: WebView, status: LoadStatus) {
            let marker = match status {
                LoadStatus::Started => b"Nagi M18 browser load status=Started\r\n".as_slice(),
                LoadStatus::HeadParsed => b"Nagi M18 browser load status=HeadParsed\r\n",
                LoadStatus::Complete => b"Nagi M18 browser load status=Complete\r\n",
            };
            let _ = libnagi::console_write(marker);
            if status == LoadStatus::Started {
                self.navigation_started.set(true);
                self.frame_ready.set(false);
            }
            self.signal.wake();
        }

        fn request_permission(&self, _webview: WebView, request: PermissionRequest) {
            let origin = request.origin().to_owned();
            let kind = permission_kind(request.feature());
            let requested_at = libnagi::time_ticks();
            let id = match self.permission_broker.borrow_mut().request(
                self.tab_id,
                &origin,
                kind,
                requested_at,
            ) {
                Ok(id) => id,
                Err(_) => {
                    request.deny();
                    report_permission_decision(UserDecision::Deny);
                    return;
                }
            };
            if self.pending_permission.borrow().is_some() {
                let _ = self
                    .permission_broker
                    .borrow_mut()
                    .respond(id, UserDecision::Deny);
                request.deny();
                report_permission_decision(UserDecision::Deny);
                return;
            }
            let display_origin = self
                .permission_broker
                .borrow()
                .requests()
                .iter()
                .find(|entry| entry.id == id)
                .map(|entry| entry.origin.clone())
                .unwrap_or_else(|| "null".to_owned());
            *self.pending_permission.borrow_mut() = Some(PendingServoPermission {
                id,
                request,
                origin: display_origin,
                kind,
                requested_at,
            });
            self.signal.wake();
        }

        fn show_embedder_control(&self, _webview: WebView, control: EmbedderControl) {
            match control {
                EmbedderControl::FilePicker(file_picker) => {
                    // One picker at a time; a second request is dismissed.
                    if self.pending_file_picker.borrow().is_some() {
                        file_picker.dismiss();
                    } else {
                        self.pending_file_picker.replace(Some(file_picker));
                        let _ = libnagi::console_write(b"Nagi M18 upload picker requested\r\n");
                    }
                    self.signal.wake();
                }
                EmbedderControl::InputMethod(input_method) => {
                    self.ime_target.set(Some(input_method.id()));
                    let _ = libnagi::console_write(b"Nagi M18 IME target ACTIVE\r\n");
                }
                _ => {}
            }
        }

        fn hide_embedder_control(&self, _webview: WebView, control_id: EmbedderControlId) {
            let stale_picker = self
                .pending_file_picker
                .borrow()
                .as_ref()
                .is_some_and(|picker| picker.id() == control_id);
            if stale_picker {
                if let Some(picker) = self.pending_file_picker.take() {
                    picker.dismiss();
                }
            }
            if self.ime_target.get() == Some(control_id) {
                self.ime_target.set(None);
                let _ = libnagi::console_write(b"Nagi M18 IME target INACTIVE\r\n");
            }
        }
    }

    /// Paint a frame Servo reports ready, as an embedder must before Servo
    /// produces the next one. Returns whether a frame was consumed.
    fn pump_frame(runtime: &TabRuntime) -> bool {
        if !runtime.delegate.has_frame() {
            return false;
        }
        runtime.delegate.clear_frame();
        runtime.webview.paint();
        true
    }

    /// After a DOM change observed through script, keep consuming frames
    /// until Servo stops producing new ones (bounded), then present. A quiet
    /// pipeline does not fail acceptance; the current frame is presented.
    fn present_settled_frame(
        servo: &Servo,
        context: &SoftwareRenderingContext,
        signal: &Arc<EventLoopSignal>,
        surface: &mut NagiSurface,
        browser_state: &BrowserState,
        runtime: &TabRuntime,
    ) -> bool {
        settle_frames(servo, signal, runtime);
        present_browser_surface(
            context,
            &runtime.webview,
            surface,
            browser_state,
            None,
            None,
        )
    }

    /// Consume frames until Servo stops producing new ones (bounded).
    fn settle_frames(servo: &Servo, signal: &Arc<EventLoopSignal>, runtime: &TabRuntime) {
        let deadline = libnagi::time_ticks().saturating_add(SETTLE_FRAME_TICKS);
        let mut quiet_since = libnagi::time_ticks();
        while libnagi::time_ticks() < deadline
            && libnagi::time_ticks().saturating_sub(quiet_since) < SETTLE_QUIET_TICKS
        {
            servo.spin_event_loop();
            if pump_frame(runtime) {
                quiet_since = libnagi::time_ticks();
            }
            yield_guest_workers(signal);
        }
    }

    fn frame_rectangle() -> DeviceIntRect {
        DeviceIntRect::from_origin_and_size(
            DeviceIntPoint::zero(),
            DeviceIntSize::new(WIDTH as i32, PAGE_HEIGHT as i32),
        )
    }

    fn checksum(frame: &[u8]) -> u32 {
        frame.iter().fold(0x811c9dc5_u32, |state, byte| {
            state.wrapping_mul(0x01000193) ^ u32::from(*byte)
        })
    }

    fn fail(reason: &[u8]) -> ! {
        let _ = libnagi::console_write(b"Nagi M18 browser FAIL ");
        let _ = libnagi::console_write(reason);
        let _ = libnagi::console_write(b"\r\n");
        libnagi::exit(1)
    }

    fn report_chrome(host: &str) -> bool {
        let prefix = b"Nagi M18 browser chrome PRESENTED host=";
        let suffix = b"\r\n";
        let mut line = [0_u8; 96];
        let length = prefix.len() + host.len() + suffix.len();
        line[..prefix.len()].copy_from_slice(prefix);
        line[prefix.len()..prefix.len() + host.len()].copy_from_slice(host.as_bytes());
        line[prefix.len() + host.len()..length].copy_from_slice(suffix);
        libnagi::console_write(&line[..length]) == length
    }

    fn report_page(host: &str, frame_checksum: u32, ink_pixels: u32) -> bool {
        let prefix = b"Nagi M18 HTTPS page RENDERED host=";
        let middle = b" frame_checksum=0x";
        let ink = format!(" ink_pixels={ink_pixels}\r\n");
        let suffix = ink.as_bytes();
        let mut line = [0_u8; 160];
        let mut cursor = 0;
        for part in [prefix.as_slice(), host.as_bytes(), middle.as_slice()] {
            line[cursor..cursor + part.len()].copy_from_slice(part);
            cursor += part.len();
        }
        for shift in (0..8).rev() {
            line[cursor] = b"0123456789abcdef"[((frame_checksum >> (shift * 4)) & 0xf) as usize];
            cursor += 1;
        }
        line[cursor..cursor + suffix.len()].copy_from_slice(suffix);
        cursor += suffix.len();
        libnagi::console_write(&line[..cursor]) == cursor
    }

    fn accepted_host(url: &Url, expected_host: &str) -> bool {
        url.scheme() == "https" && url.host_str() == Some(expected_host)
    }

    fn report_url(prefix: &[u8], url: &Url) {
        let url_bytes = url.as_str().as_bytes();
        let displayed_url = &url_bytes[..url_bytes.len().min(180)];
        let _ = libnagi::console_write(prefix);
        let _ = libnagi::console_write(displayed_url);
        let _ = libnagi::console_write(b"\r\n");
    }

    fn servo_point(x: u32, y: u32) -> servo::WebViewPoint {
        DevicePoint::new(x as f32, y as f32).into()
    }

    fn dispatch_chrome_action(
        servo: &Servo,
        context: &Rc<SoftwareRenderingContext>,
        signal: &Arc<EventLoopSignal>,
        browser_state: &mut BrowserState,
        runtimes: &mut Vec<TabRuntime>,
        permission_locale: Locale,
        services: &InputServices,
        action: BrowserChromeAction,
    ) -> Option<crate::navigation::NavigationRequest> {
        let outcome = ui::dispatch(browser_state, action, libnagi::time_ticks());
        sync_tab_runtimes(
            servo,
            context,
            signal,
            browser_state,
            runtimes,
            false,
            permission_locale,
            services,
        );
        match outcome {
            Ok(BrowserChromeOutcome::Navigate(request)) => {
                let url = Url::parse(&request.url).expect("browser state emits normalized URLs");
                let Some(runtime) = runtime_for_tab(runtimes, request.tab_id) else {
                    return None;
                };
                runtime.delegate.reset();
                // A pending paste gesture belongs to the document the user
                // was looking at, not to the one being loaded.
                services
                    .gestures
                    .forget_scope(tab_gesture_scope(request.tab_id));
                report_url(b"Nagi M18 browser requesting URL=", &url);
                runtime.webview.load(url);
                let _ = libnagi::console_write(
                    b"Nagi M18 browser navigation request queued for Servo\r\n",
                );
                Some(request)
            }
            Ok(BrowserChromeOutcome::Changed | BrowserChromeOutcome::Stop(_)) | Err(_) => None,
        }
    }

    fn page_keyboard_event(
        code: u16,
        pressed: bool,
        modifiers: KeyModifiers,
    ) -> Option<ServoInputEvent> {
        let (key, physical_code) = if let Some(character) = evdev_character(code, modifiers.shift) {
            let physical_code = match code {
                16..=25 => [
                    Code::KeyQ,
                    Code::KeyW,
                    Code::KeyE,
                    Code::KeyR,
                    Code::KeyT,
                    Code::KeyY,
                    Code::KeyU,
                    Code::KeyI,
                    Code::KeyO,
                    Code::KeyP,
                ][usize::from(code - 16)],
                30..=38 => [
                    Code::KeyA,
                    Code::KeyS,
                    Code::KeyD,
                    Code::KeyF,
                    Code::KeyG,
                    Code::KeyH,
                    Code::KeyJ,
                    Code::KeyK,
                    Code::KeyL,
                ][usize::from(code - 30)],
                44..=50 => [
                    Code::KeyZ,
                    Code::KeyX,
                    Code::KeyC,
                    Code::KeyV,
                    Code::KeyB,
                    Code::KeyN,
                    Code::KeyM,
                ][usize::from(code - 44)],
                2..=11 => [
                    Code::Digit1,
                    Code::Digit2,
                    Code::Digit3,
                    Code::Digit4,
                    Code::Digit5,
                    Code::Digit6,
                    Code::Digit7,
                    Code::Digit8,
                    Code::Digit9,
                    Code::Digit0,
                ][usize::from(code - 2)],
                52 => Code::Period,
                53 => Code::Slash,
                57 => Code::Space,
                _ => Code::Unidentified,
            };
            (Key::Character(character.to_string()), physical_code)
        } else {
            match code {
                1 => (Key::Named(NamedKey::Escape), Code::Escape),
                14 => (Key::Named(NamedKey::Backspace), Code::Backspace),
                15 => (Key::Named(NamedKey::Tab), Code::Tab),
                28 => (Key::Named(NamedKey::Enter), Code::Enter),
                _ => return None,
            }
        };
        let mut servo_modifiers = Modifiers::default();
        if modifiers.shift {
            servo_modifiers.insert(Modifiers::SHIFT);
        }
        if modifiers.control {
            servo_modifiers.insert(Modifiers::CONTROL);
        }
        Some(ServoInputEvent::Keyboard(KeyboardEvent::new_without_event(
            if pressed {
                KeyState::Down
            } else {
                KeyState::Up
            },
            key,
            physical_code,
            Location::Standard,
            servo_modifiers,
            false,
            false,
        )))
    }

    /// Save downloads that follow a trusted page input in the same tab to
    /// `/Downloads`; anything else is rejected.
    fn save_pending_downloads(runtimes: &[TabRuntime]) {
        for runtime in runtimes {
            let requests: Vec<_> = runtime
                .delegate
                .pending_downloads
                .borrow_mut()
                .drain(..)
                .collect();
            for request in requests {
                let now = libnagi::time_ticks();
                let gestured = runtime
                    .delegate
                    .last_page_input
                    .get()
                    .is_some_and(|tick| now.saturating_sub(tick) <= DOWNLOAD_GESTURE_TICKS);
                if !gestured {
                    let _ = libnagi::console_write(
                        b"Nagi M18 download REJECTED reason=no-user-gesture\r\n",
                    );
                    continue;
                }
                match save_download(&request) {
                    Ok(path) => {
                        let line = format!(
                            "Nagi M18 download saved path={path} bytes={}\r\n",
                            request.bytes.len()
                        );
                        let _ = libnagi::console_write(line.as_bytes());
                    }
                    Err(reason) => {
                        let _ = libnagi::console_write(b"Nagi M18 download FAILED-SAVE reason=");
                        let _ = libnagi::console_write(reason);
                        let _ = libnagi::console_write(b"\r\n");
                    }
                }
            }
        }
    }

    fn save_download(request: &servo::DownloadRequest) -> Result<String, &'static [u8]> {
        std::fs::create_dir_all(DOWNLOAD_DIRECTORY).map_err(|_| b"directory".as_slice())?;
        let name = download_file_name(&request.suggested_filename, VFS_NAME_BYTES, |name| {
            std::fs::metadata(format!("{DOWNLOAD_DIRECTORY}/{name}")).is_ok()
        })
        .ok_or(b"no-free-name".as_slice())?;
        let path = format!("{DOWNLOAD_DIRECTORY}/{name}");
        std::fs::write(&path, &request.bytes).map_err(|_| b"write".as_slice())?;
        Ok(path)
    }

    /// Apply URL changes Servo reported for navigations Albert did not
    /// request (links, scripts, embedder-loaded content) to the chrome and
    /// history. Albert-requested navigations are owned by their own events.
    fn apply_content_navigation(browser_state: &mut BrowserState, runtimes: &[TabRuntime]) {
        for runtime in runtimes {
            let Some(url) = runtime.delegate.reported_url.take() else {
                continue;
            };
            let title = runtime.webview.page_title().unwrap_or_default();
            if let Ok(NavigationEventResult::Applied) = browser_state.content_navigated(
                runtime.id,
                url.as_str(),
                &title,
                libnagi::time_ticks(),
            ) {
                report_url(b"Nagi M18 browser content navigation recorded url=", &url);
            }
        }
    }

    fn route_guest_input(
        input_capability: u64,
        bridge: &mut InputBridge,
        browser_state: &mut BrowserState,
        servo: &Servo,
        context: &Rc<SoftwareRenderingContext>,
        signal: &Arc<EventLoopSignal>,
        runtimes: &mut Vec<TabRuntime>,
        modifiers: &mut KeyModifiers,
        permission_locale: Locale,
        services: &InputServices,
    ) -> Option<crate::navigation::NavigationRequest> {
        apply_content_navigation(browser_state, runtimes);
        save_pending_downloads(runtimes);
        let mut event = libnagi::InputEvent::default();
        if !libnagi::input_read(input_capability, &mut event) {
            return None;
        }
        match bridge.translate(event)? {
            BrowserInput::MouseMove { x, y } => {
                if y >= PAGE_TOP {
                    if let Some(runtime) = active_runtime(browser_state, runtimes) {
                        runtime
                            .webview
                            .notify_input_event(ServoInputEvent::MouseMove(MouseMoveEvent::new(
                                servo_point(x, y - PAGE_TOP),
                            )));
                    }
                }
                None
            }
            BrowserInput::MouseButton { button: 0, pressed } => {
                let (x, y) = bridge.position();
                // Toolbar clicks dispatch chrome actions; the status strip
                // between the toolbar and the page is inert.
                if y < PAGE_TOP {
                    if pressed {
                        if let Some(action) =
                            chrome_action_at(&ui::view(browser_state), WIDTH, x, y)
                        {
                            return dispatch_chrome_action(
                                servo,
                                context,
                                signal,
                                browser_state,
                                runtimes,
                                permission_locale,
                                services,
                                action,
                            );
                        }
                    }
                    return None;
                }
                trace_routed_input(services, b"page-button", 0, pressed);
                if pressed && browser_state.address_bar().is_focused() {
                    // Clicking page content moves keyboard focus to the page.
                    let _ = ui::dispatch(
                        browser_state,
                        BrowserChromeAction::BlurAddressBar,
                        libnagi::time_ticks(),
                    );
                }
                if let Some(runtime) = active_runtime(browser_state, runtimes) {
                    if pressed {
                        record_page_gesture(services, runtime.id, None);
                        runtime
                            .delegate
                            .last_page_input
                            .set(Some(libnagi::time_ticks()));
                    }
                    runtime
                        .webview
                        .notify_input_event(ServoInputEvent::MouseButton(MouseButtonEvent::new(
                            if pressed {
                                MouseButtonAction::Down
                            } else {
                                MouseButtonAction::Up
                            },
                            MouseButton::Primary,
                            servo_point(x, y - PAGE_TOP),
                        )));
                }
                None
            }
            BrowserInput::MouseButton { .. } => None,
            BrowserInput::Key { code, pressed } if is_shift_key(code) => {
                modifiers.shift = pressed;
                None
            }
            BrowserInput::Key { code, pressed } if is_control_key(code) => {
                trace_routed_input(services, b"control", code, pressed);
                modifiers.control = pressed;
                None
            }
            BrowserInput::Key { code, pressed } if browser_state.address_bar().is_focused() => {
                trace_routed_input(services, b"address-key", code, pressed);
                if !pressed {
                    return None;
                }
                if let Some(shortcut) = clipboard_shortcut(code, modifiers.control) {
                    address_bar_clipboard(browser_state, services, shortcut);
                    return None;
                }
                if modifiers.control {
                    return None;
                }
                if is_enter_key(code) {
                    return dispatch_chrome_action(
                        servo,
                        context,
                        signal,
                        browser_state,
                        runtimes,
                        permission_locale,
                        services,
                        BrowserChromeAction::SubmitAddress,
                    );
                }
                if is_backspace_key(code) {
                    let _ = ui::dispatch(
                        browser_state,
                        BrowserChromeAction::DeleteAddressBackward,
                        libnagi::time_ticks(),
                    );
                } else if is_escape_key(code) {
                    let _ = ui::dispatch(
                        browser_state,
                        BrowserChromeAction::BlurAddressBar,
                        libnagi::time_ticks(),
                    );
                } else if let Some(character) = evdev_character(code, modifiers.shift) {
                    let _ = ui::dispatch(
                        browser_state,
                        BrowserChromeAction::InsertAddressText(character.to_string()),
                        libnagi::time_ticks(),
                    );
                }
                None
            }
            BrowserInput::Key { code, pressed } => {
                trace_routed_input(services, b"page-key", code, pressed);
                let Some(runtime) = active_runtime(browser_state, runtimes) else {
                    return None;
                };
                if !route_ime_key(services, runtime, code, pressed, *modifiers) {
                    return None;
                }
                if let Some(event) = page_keyboard_event(code, pressed, *modifiers) {
                    if pressed {
                        record_page_gesture(
                            services,
                            runtime.id,
                            clipboard_shortcut(code, modifiers.control),
                        );
                        runtime
                            .delegate
                            .last_page_input
                            .set(Some(libnagi::time_ticks()));
                    }
                    runtime.webview.notify_input_event(event);
                }
                None
            }
        }
    }

    fn send_composition(webview: &WebView, state: CompositionState, data: String) {
        webview.notify_input_event(ServoInputEvent::Ime(ImeEvent::Composition(
            CompositionEvent { state, data },
        )));
    }

    fn report_ime_mode(mode: InputMode) {
        let _ = libnagi::console_write(match mode {
            InputMode::Direct => b"Nagi M18 IME mode=direct\r\n".as_slice(),
            InputMode::Hiragana => b"Nagi M18 IME mode=hiragana\r\n",
        });
    }

    /// Offer a page key to the input method. Returns `true` when the
    /// original key must still be delivered to the page.
    fn route_ime_key(
        services: &InputServices,
        runtime: &TabRuntime,
        code: u16,
        pressed: bool,
        modifiers: KeyModifiers,
    ) -> bool {
        let slot = usize::from(code).min(255);
        if !pressed {
            // Swallow the release of a press the IME consumed.
            return !std::mem::replace(&mut services.ime_consumed_keys.borrow_mut()[slot], false);
        }
        let Some(key) = ime_key(code, modifiers.control, modifiers.shift) else {
            return true;
        };
        let mode_key = matches!(key, ImeKey::ToggleMode | ImeKey::ModeOn | ImeKey::ModeOff);
        if runtime.delegate.ime_target.get().is_none() {
            if services.ime_composing.replace(false) {
                services.ime.borrow_mut().reset();
            }
            // Without a focused text field only mode keys reach the IME,
            // so the input language can still be switched.
            if !mode_key {
                return true;
            }
        }
        let response: ImeResponse = services.ime.borrow_mut().handle(key);
        if let Some(mode) = response.mode_changed {
            report_ime_mode(mode);
        }
        if let Some(text) = response.commit {
            if services.ime_composing.replace(false) {
                if !text.is_empty() {
                    services.ime_commits.set(services.ime_commits.get() + 1);
                }
                send_composition(&runtime.webview, CompositionState::End, text);
            }
        }
        if let Some(text) = response.preedit {
            if response.preedit_started || !services.ime_composing.get() {
                send_composition(&runtime.webview, CompositionState::Start, String::new());
                services.ime_composing.set(true);
            }
            send_composition(&runtime.webview, CompositionState::Update, text);
        }
        if !response.pass_through {
            services.ime_consumed_keys.borrow_mut()[slot] = true;
        }
        response.pass_through
    }

    fn trace_routed_input(services: &InputServices, kind: &[u8], code: u16, pressed: bool) {
        if !services.trace_input.get() {
            return;
        }
        let mut line = Vec::with_capacity(80);
        line.extend_from_slice(b"Nagi M18 clipboard trace input=");
        line.extend_from_slice(kind);
        line.extend_from_slice(
            format!(" code={code} pressed={}\r\n", u8::from(pressed)).as_bytes(),
        );
        let _ = libnagi::console_write(&line);
    }

    /// Record a trusted gesture for the tab receiving fresh device input.
    /// Only an explicit paste shortcut authorizes a clipboard read.
    fn record_page_gesture(
        services: &InputServices,
        tab_id: TabId,
        shortcut: Option<ClipboardShortcut>,
    ) {
        let scope = tab_gesture_scope(tab_id);
        let now = libnagi::time_ticks();
        let result = if shortcut == Some(ClipboardShortcut::Paste) {
            services.gestures.record_paste(scope, now)
        } else {
            services.gestures.record_activation(scope, now)
        };
        if let Err(error) = result {
            report_clipboard_denial(b"gesture", error);
        }
    }

    fn address_bar_clipboard(
        browser_state: &mut BrowserState,
        services: &InputServices,
        shortcut: ClipboardShortcut,
    ) {
        let context = ClipboardContext::from_trusted_browser_chrome(None);
        let mut runtime = NagiClipboardRuntime::new(&services.endpoint, &services.gestures, || {
            libnagi::time_ticks()
        });
        let mut chrome = services.chrome.borrow_mut();
        match shortcut {
            ClipboardShortcut::Copy | ClipboardShortcut::Cut => {
                let text = browser_state.address_bar().text();
                if chrome.copy(&mut runtime, &context, &text).is_ok()
                    && shortcut == ClipboardShortcut::Cut
                {
                    let _ = ui::dispatch(
                        browser_state,
                        BrowserChromeAction::SetAddressText(String::new()),
                        libnagi::time_ticks(),
                    );
                }
            }
            ClipboardShortcut::Paste => {
                if let Ok(text) = chrome.paste(&mut runtime, &context) {
                    let line = single_line_paste(&text);
                    if !line.is_empty() {
                        let _ = ui::dispatch(
                            browser_state,
                            BrowserChromeAction::InsertAddressText(line),
                            libnagi::time_ticks(),
                        );
                    }
                }
            }
        }
    }

    fn wait_for_address_navigation(
        servo: &Servo,
        context: &Rc<SoftwareRenderingContext>,
        signal: &Arc<EventLoopSignal>,
        input_capability: u64,
        input_bridge: &mut InputBridge,
        browser_state: &mut BrowserState,
        runtimes: &mut Vec<TabRuntime>,
        modifiers: &mut KeyModifiers,
        permission_locale: Locale,
        services: &InputServices,
    ) -> crate::navigation::NavigationRequest {
        let deadline = libnagi::time_ticks().saturating_add(PAGE_TIMEOUT_TICKS);
        loop {
            servo.spin_event_loop();
            if let Some(request) = route_guest_input(
                input_capability,
                input_bridge,
                browser_state,
                servo,
                context,
                signal,
                runtimes,
                modifiers,
                permission_locale,
                services,
            ) {
                let url = Url::parse(&request.url).expect("normalized address is a valid URL");
                if !accepted_host(&url, HTTPS_PAGES[0].1) {
                    fail(b"address-bar acceptance entered an unexpected host");
                }
                return request;
            }
            if libnagi::time_ticks() >= deadline {
                fail(b"address-bar navigation input timed out");
            }
            yield_guest_workers(signal);
        }
    }

    fn wait_for_initial_blank(servo: &Servo, runtime: &TabRuntime) {
        let deadline = libnagi::time_ticks().saturating_add(PAGE_TIMEOUT_TICKS);
        loop {
            servo.spin_event_loop();
            if runtime.webview.load_status() == LoadStatus::Complete
                && runtime.delegate.has_frame()
                && runtime
                    .webview
                    .url()
                    .is_some_and(|url| url.as_str() == "about:blank")
            {
                crate::M18_NAVIGATION_TRACE_ACTIVE.store(true, Ordering::Release);
                let _ = libnagi::console_write(
                    b"Nagi M18 Constellation diagnostics armed after initial blank\r\n",
                );
                let _ = libnagi::console_write(b"Nagi M18 browser initial WebView READY\r\n");
                return;
            }
            if libnagi::time_ticks() >= deadline {
                fail(b"initial blank page timed out");
            }
            yield_guest_workers(&runtime.delegate.signal);
        }
    }

    fn present_browser_surface(
        context: &SoftwareRenderingContext,
        webview: &WebView,
        surface: &mut NagiSurface,
        browser_state: &BrowserState,
        prompt: Option<(&str, PermissionKind, Locale)>,
        picker: Option<&crate::chrome_surface::FilePickerView<'_>>,
    ) -> bool {
        webview.paint();
        let Some(image) = context.read_to_image(frame_rectangle()) else {
            return false;
        };
        let mut frame = vec![0_u8; WIDTH as usize * HEIGHT as usize * 4];
        if crate::chrome_surface::place_page(
            &mut frame,
            WIDTH,
            HEIGHT,
            WIDTH as usize * 4,
            image.as_raw(),
            PAGE_HEIGHT,
        )
        .is_err()
        {
            return false;
        }
        let chrome = ui::view(browser_state);
        if crate::chrome_surface::render_chrome(
            &mut frame,
            WIDTH,
            HEIGHT,
            WIDTH as usize * 4,
            &chrome,
        )
        .is_err()
        {
            return false;
        }
        if let Some((origin, kind, locale)) = prompt {
            let displayed_origin = permission_prompt::bounded_origin(origin);
            let feature = PermissionPromptLabels::feature_name(locale, kind);
            let view = PermissionPromptView {
                origin: &displayed_origin,
                feature,
                labels: PermissionPromptLabels::for_locale(locale),
            };
            if crate::chrome_surface::render_permission_prompt(
                &mut frame,
                WIDTH,
                HEIGHT,
                WIDTH as usize * 4,
                &view,
            )
            .is_err()
            {
                return false;
            }
        }
        if let Some(view) = picker {
            if crate::chrome_surface::render_file_picker(
                &mut frame,
                WIDTH,
                HEIGHT,
                WIDTH as usize * 4,
                view,
            )
            .is_err()
            {
                return false;
            }
        }
        surface
            .copy_rgba_frame(&frame, WIDTH, HEIGHT, WIDTH as usize * 4)
            .is_ok()
            && surface.present()
    }

    /// Consume queued input around first presentation so events from before
    /// the prompt appeared cannot resolve a site request.
    fn drain_permission_input(input_capability: u64, bridge: &mut InputBridge) -> bool {
        if input_capability == 0 {
            return false;
        }
        for _ in 0..MAX_PERMISSION_INPUT_DRAIN {
            let mut event = libnagi::InputEvent::default();
            if !libnagi::input_read(input_capability, &mut event) {
                return true;
            }
            let _ = bridge.translate(event);
        }
        false
    }

    fn route_permission_prompt_input(
        input_capability: u64,
        bridge: &mut InputBridge,
        delegate: &AcceptanceDelegate,
    ) {
        let mut event = libnagi::InputEvent::default();
        if !libnagi::input_read(input_capability, &mut event) {
            return;
        }
        match bridge.translate(event) {
            Some(BrowserInput::MouseButton {
                button: 0,
                pressed: true,
            }) => {
                let (x, y) = bridge.position();
                if let Some(action) = permission_prompt::action_at(WIDTH, HEIGHT, x, y) {
                    delegate.resolve_permission(action.decision());
                }
            }
            Some(BrowserInput::Key {
                code,
                pressed: true,
            }) if is_escape_key(code) => {
                delegate.resolve_permission(PermissionPromptAction::Cancel.decision());
            }
            _ => {}
        }
    }

    fn wait_for_verified_page(
        servo: &Servo,
        context: &SoftwareRenderingContext,
        surface: &mut NagiSurface,
        browser_state: &BrowserState,
        input_capability: u64,
        input_bridge: &mut InputBridge,
        runtimes: &[TabRuntime],
        runtime: &TabRuntime,
        index: usize,
    ) -> Url {
        let (_, expected_host) = HTTPS_PAGES[index];
        let deadline = libnagi::time_ticks().saturating_add(PAGE_TIMEOUT_TICKS);
        let mut diagnostic_turns_remaining = if index > 0 { 64 } else { 0 };
        let mut visible_permission: Option<(TabId, PermissionRequestId)> = None;
        loop {
            let trace_turn = diagnostic_turns_remaining > 0;
            if trace_turn {
                let _ =
                    libnagi::console_write(b"Nagi M18 acceptance next-page poll turn entered\r\n");
            }
            servo.spin_event_loop();
            if trace_turn {
                let _ = libnagi::console_write(
                    b"Nagi M18 acceptance next-page event loop returned\r\n",
                );
            }

            let pending_permission = runtimes.iter().find_map(|candidate| {
                candidate.delegate.pending_permission_info().map(
                    |(id, origin, kind, requested_at)| {
                        (
                            candidate.id,
                            candidate.delegate.as_ref(),
                            id,
                            origin,
                            kind,
                            requested_at,
                        )
                    },
                )
            });
            if let Some((tab_id, delegate, id, origin, kind, requested_at)) = pending_permission {
                let permission_key = (tab_id, id);
                if visible_permission != Some(permission_key) {
                    if input_capability == 0 {
                        delegate.resolve_permission(UserDecision::Deny);
                        let _ = libnagi::console_write(
                            b"Nagi M18 site permission input unavailable; denied\r\n",
                        );
                    } else if !present_browser_surface(
                        context,
                        &runtime.webview,
                        surface,
                        browser_state,
                        Some((&origin, kind, delegate.permission_locale)),
                        None,
                    ) {
                        delegate.resolve_permission(UserDecision::Deny);
                        let _ = libnagi::console_write(
                            b"Nagi M18 site permission prompt unavailable; denied\r\n",
                        );
                    } else {
                        visible_permission = Some(permission_key);
                        if !drain_permission_input(input_capability, input_bridge) {
                            delegate.resolve_permission(UserDecision::Deny);
                            let _ = libnagi::console_write(
                                b"Nagi M18 site permission input unavailable; denied\r\n",
                            );
                        }
                    }
                } else if input_capability == 0 {
                    delegate.resolve_permission(UserDecision::Deny);
                    let _ = libnagi::console_write(
                        b"Nagi M18 site permission input unavailable; denied\r\n",
                    );
                } else if libnagi::time_ticks().saturating_sub(requested_at)
                    >= PERMISSION_PROMPT_TIMEOUT_TICKS
                {
                    delegate.resolve_permission(UserDecision::Deny);
                    let _ = libnagi::console_write(
                        b"Nagi M18 site permission prompt timed out; denied\r\n",
                    );
                } else {
                    route_permission_prompt_input(input_capability, input_bridge, delegate);
                }
            } else if visible_permission.take().is_some()
                && !present_browser_surface(
                    context,
                    &runtime.webview,
                    surface,
                    browser_state,
                    None,
                    None,
                )
            {
                fail(b"Albert surface restore after permission prompt failed");
            }

            if !runtimes
                .iter()
                .any(|candidate| candidate.delegate.pending_permission_info().is_some())
                && runtime.delegate.navigation_started()
                && runtime.webview.load_status() == LoadStatus::Complete
                && runtime.delegate.has_frame()
            {
                let Some(url) = runtime.webview.url() else {
                    fail(b"completed page has no URL");
                };
                if !accepted_host(&url, expected_host) {
                    let url_bytes = url.as_str().as_bytes();
                    let displayed_url = &url_bytes[..url_bytes.len().min(180)];
                    let _ = libnagi::console_write(b"Nagi M18 browser observed URL=");
                    let _ = libnagi::console_write(displayed_url);
                    let _ = libnagi::console_write(b"\r\n");
                    fail(b"HTTPS navigation ended at an unexpected host");
                }
                if tls_verified(index) {
                    return url;
                }
            }

            if libnagi::time_ticks() >= deadline {
                for candidate in runtimes {
                    candidate.delegate.resolve_permission(UserDecision::Deny);
                }
                let _ = libnagi::console_write(if runtime.delegate.navigation_started() {
                    b"Nagi M18 browser timeout navigation_started=true\r\n"
                } else {
                    b"Nagi M18 browser timeout navigation_started=false\r\n"
                });
                let _ = libnagi::console_write(if runtime.delegate.has_frame() {
                    b"Nagi M18 browser timeout frame_ready=true\r\n"
                } else {
                    b"Nagi M18 browser timeout frame_ready=false\r\n"
                });
                let _ = libnagi::console_write(if tls_verified(index) {
                    b"Nagi M18 browser timeout tls_verified=true\r\n"
                } else {
                    b"Nagi M18 browser timeout tls_verified=false\r\n"
                });
                if let Some(url) = runtime.webview.url() {
                    report_url(b"Nagi M18 browser timeout current_url=", &url);
                }
                fail(b"HTTPS page or certificate validation timed out");
            }
            if trace_turn {
                let _ = libnagi::console_write(
                    b"Nagi M18 acceptance next-page worker yield entered\r\n",
                );
            }
            yield_guest_workers(&runtime.delegate.signal);
            if trace_turn {
                let _ = libnagi::console_write(
                    b"Nagi M18 acceptance next-page worker yield returned\r\n",
                );
                diagnostic_turns_remaining -= 1;
            }
        }
    }

    fn render_page(
        context: &SoftwareRenderingContext,
        webview: &WebView,
        surface: &mut NagiSurface,
        host: &str,
        browser_state: &BrowserState,
    ) {
        webview.paint();
        let Some(image) = context.read_to_image(frame_rectangle()) else {
            fail(b"Servo frame readback failed");
        };
        let frame = image.as_raw();
        let frame_checksum = checksum(frame);
        if frame_checksum == 0 {
            fail(b"Servo frame checksum was zero");
        }
        let mut composed_frame = vec![0_u8; WIDTH as usize * HEIGHT as usize * 4];
        if crate::chrome_surface::place_page(
            &mut composed_frame,
            WIDTH,
            HEIGHT,
            WIDTH as usize * 4,
            frame,
            PAGE_HEIGHT,
        )
        .is_err()
        {
            fail(b"Albert page composition failed");
        }
        let chrome = crate::ui::view(browser_state);
        if crate::chrome_surface::render_chrome(
            &mut composed_frame,
            WIDTH,
            HEIGHT,
            WIDTH as usize * 4,
            &chrome,
        )
        .is_err()
        {
            fail(b"Albert chrome composition failed");
        }
        if surface
            .copy_rgba_frame(&composed_frame, WIDTH, HEIGHT, WIDTH as usize * 4)
            .is_err()
        {
            fail(b"Nagi Surface rejected Servo frame");
        }
        if !surface.present() {
            fail(b"Nagi Surface presentation failed");
        }
        if !report_chrome(host) {
            fail(b"serial chrome evidence write failed");
        }
        // Count non-background pixels in Servo's own frame (before chrome is
        // composed) so a page that painted only its background is visible
        // in the evidence and rejected by the host validator.
        let ink = crate::frame_analysis::ink_pixels(frame, WIDTH, PAGE_HEIGHT, INK_THRESHOLD);
        if !report_page(host, frame_checksum, ink) {
            fail(b"serial page evidence write failed");
        }
    }

    fn persist_browser_state(browser_state: &BrowserState, storage: &mut GuestBrowserStorage) {
        match crate::session::save(browser_state, storage) {
            Ok(()) => {
                let _ = libnagi::console_write(b"Nagi M18 browser storage SAVE PASS\r\n");
            }
            Err(crate::persistence::StorageError::Capacity) => {
                let _ = libnagi::console_write(
                    b"Nagi M18 browser storage SAVE CAPACITY limit=16384\r\n",
                );
            }
            Err(_) => {
                let _ = libnagi::console_write(b"Nagi M18 browser storage SAVE UNAVAILABLE\r\n");
            }
        }
    }

    /// Bundled clipboard fixture. Its source field is focused and selected on
    /// load; the page reports the destination field's value in its title so
    /// the guest can observe the paste without evaluating page script.
    const CLIPBOARD_TOKEN: &str = "nagi-clip-7f3a";
    const CLIPBOARD_READY_TITLE: &str = "nagi-clip:ready";
    const CLIPBOARD_PAGE: &str = "data:text/html,%3C%21doctype%20html%3E%3Cmeta%20charset%3Dutf-8%3E%3Cbody%20style%3D%22margin%3A0%3Bfont%3A16px%20sans-serif%22%3E%3Cinput%20id%3Ds%20value%3Dnagi-clip-7f3a%20style%3D%22position%3Aabsolute%3Bleft%3A8px%3Btop%3A8px%3Bwidth%3A280px%3Bheight%3A24px%22%3E%3Cinput%20id%3Dd%20style%3D%22position%3Aabsolute%3Bleft%3A8px%3Btop%3A58px%3Bwidth%3A280px%3Bheight%3A24px%22%3E%3Cscript%3Evar%20s%3Ddocument.getElementById%28%27s%27%29%2Cd%3Ddocument.getElementById%28%27d%27%29%3Bd.addEventListener%28%27focus%27%2Cfunction%28%29%7Bdocument.title%3D%27nagi-clip%3Afocus%3Ad%27%3B%7D%29%3Bd.addEventListener%28%27input%27%2Cfunction%28%29%7Bdocument.title%3D%27nagi-clip%3A%27%2Bd.value%3B%7D%29%3Bd.addEventListener%28%27compositionend%27%2Cfunction%28e%29%7BsetTimeout%28function%28%29%7Bdocument.title%3D%27nagi-ime%3A%27%2Be.data%2B%27%7C%27%2Bd.value%3B%7D%2C0%29%3B%7D%29%3Bdocument.addEventListener%28%27keydown%27%2Cfunction%28e%29%7Bvar%20a%3Ddocument.activeElement%3Bdocument.title%3D%27nagi-clip%3Akey%3A%27%2Be.key%2B%27%3A%27%2B%28e.ctrlKey%3F1%3A0%29%2B%27%3A%27%2B%28a%26%26a.id%3Fa.id%3A%27none%27%29%2B%27%3A%27%2B%28a%26%26a.selectionStart%21%3Dnull%3Fa.selectionStart%2B%27-%27%2Ba.selectionEnd%3A%27na%27%29%3B%7D%29%3Bs.focus%28%29%3Bs.select%28%29%3Bdocument.title%3D%27nagi-clip%3Aready%27%3B%3C%2Fscript%3E";
    /// Page with one `.txt` file input; it reports the chosen file's name,
    /// size, and contents through its title.
    const UPLOAD_PAGE: &str = "data:text/html,%3C%21doctype%20html%3E%3Cmeta%20charset%3Dutf-8%3E%3Cbody%20style%3D%22margin%3A0%3Bfont%3A16px%20sans-serif%22%3E%3Cinput%20type%3Dfile%20id%3Df%20accept%3D.txt%20style%3D%22position%3Aabsolute%3Bleft%3A8px%3Btop%3A8px%3Bwidth%3A280px%3Bheight%3A40px%22%3E%3Cscript%3Evar%20f%3Ddocument.getElementById%28%27f%27%29%3Bf.addEventListener%28%27change%27%2Cfunction%28%29%7Bvar%20file%3Df.files%5B0%5D%3Bif%28%21file%29%7Breturn%3B%7Dvar%20reader%3Dnew%20FileReader%28%29%3Breader.onload%3Dfunction%28%29%7Bdocument.title%3D%27nagi-upload%3A%27%2Bfile.name%2B%27%3A%27%2Bfile.size%2B%27%3A%27%2Breader.result%3B%7D%3Breader.readAsText%28file%29%3B%7D%29%3Bdocument.title%3D%27nagi-upload%3Aready%27%3B%3C%2Fscript%3E";
    /// Page with one `download` link. Its script calls `a.click()` on load,
    /// which has no user activation and must not produce a file.
    const DOWNLOAD_PAGE: &str = "data:text/html,%3C%21doctype%20html%3E%3Cmeta%20charset%3Dutf-8%3E%3Cbody%20style%3D%22margin%3A0%3Bfont%3A16px%20sans-serif%22%3E%3Ca%20id%3Da%20download%3Dnagi-download.txt%20href%3D%22data%3Atext%2Fplain%2Cnagi-download-ok%22%20style%3D%22position%3Aabsolute%3Bleft%3A8px%3Btop%3A8px%3Bwidth%3A280px%3Bheight%3A40px%3Bdisplay%3Ablock%3Bbackground%3A%23dde%22%3EDownload%3C%2Fa%3E%3Cscript%3Evar%20a%3Ddocument.getElementById%28%27a%27%29%3Ba.click%28%29%3Bdocument.title%3D%27nagi-download%3Aready%27%3B%3C%2Fscript%3E";
    const DOWNLOAD_FILE_NAME: &str = "nagi-download.txt";
    const DOWNLOAD_FILE_CONTENT: &str = "nagi-download-ok";
    /// Hiragana for the romaji `nihongo` that the M18 harness types.
    const IME_EXPECTED_COMMIT: &str = "にほんご";

    /// Copy from one page field and paste into another using only
    /// QMP-delivered keyboard and pointer input routed through the Nagi
    /// clipboard service.
    fn run_clipboard_acceptance(
        servo: &Servo,
        context: &Rc<SoftwareRenderingContext>,
        signal: &Arc<EventLoopSignal>,
        surface: &mut NagiSurface,
        browser_state: &mut BrowserState,
        runtimes: &mut Vec<TabRuntime>,
        input_capability: u64,
        input_bridge: &mut InputBridge,
        modifiers: &mut KeyModifiers,
        permission_locale: Locale,
        services: &InputServices,
    ) {
        let Some(tab_id) = browser_state.active_tab_id() else {
            fail(b"clipboard acceptance has no active tab");
        };
        {
            let Some(runtime) = runtime_for_tab(runtimes, tab_id) else {
                fail(b"clipboard acceptance tab has no Servo WebView");
            };
            runtime.delegate.reset();
            services.gestures.forget_scope(tab_gesture_scope(tab_id));
            runtime
                .webview
                .load(Url::parse(CLIPBOARD_PAGE).expect("the bundled clipboard page is valid"));
            let deadline = libnagi::time_ticks().saturating_add(PAGE_TIMEOUT_TICKS);
            loop {
                servo.spin_event_loop();
                if runtime.webview.load_status() == LoadStatus::Complete
                    && runtime.delegate.has_frame()
                    && runtime.webview.page_title().as_deref() == Some(CLIPBOARD_READY_TITLE)
                {
                    break;
                }
                if libnagi::time_ticks() >= deadline {
                    fail(b"clipboard page load timed out");
                }
                yield_guest_workers(signal);
            }
            let counters = &services.counters;
            if counters.reads.get() != 0 || counters.denied_reads.get() != 0 {
                fail(b"clipboard was read before any user paste gesture");
            }
        }
        // The fixture is embedder-loaded content; record it so the chrome
        // shows its URL instead of the previous HTTPS page.
        apply_content_navigation(browser_state, runtimes);
        if browser_state
            .active_tab()
            .and_then(|tab| tab.navigation().current_url())
            .is_none_or(|url| !url.starts_with("data:text/html"))
        {
            fail(b"chrome did not record the content-loaded fixture URL");
        }
        if libnagi::console_write(b"Nagi M18 browser content navigation PASS\r\n")
            != b"Nagi M18 browser content navigation PASS\r\n".len()
        {
            fail(b"serial content-navigation evidence write failed");
        }
        {
            let Some(runtime) = runtime_for_tab(runtimes, tab_id) else {
                fail(b"clipboard acceptance tab has no Servo WebView");
            };
            if !present_browser_surface(
                context,
                &runtime.webview,
                surface,
                browser_state,
                None,
                None,
            ) {
                fail(b"clipboard page presentation failed");
            }
        }
        services.trace_input.set(true);
        if libnagi::console_write(b"Nagi M18 clipboard page READY\r\n")
            != b"Nagi M18 clipboard page READY\r\n".len()
        {
            fail(b"serial clipboard-ready marker write failed");
        }

        let expected_title = format!("nagi-clip:{CLIPBOARD_TOKEN}");
        let deadline = libnagi::time_ticks().saturating_add(PAGE_TIMEOUT_TICKS);
        // QEMU keyboard and pointer devices are separate queues, so the host
        // sends each step only after the guest reports the previous one.
        let mut copy_reported = false;
        let mut focus_reported = false;
        loop {
            servo.spin_event_loop();
            if route_guest_input(
                input_capability,
                input_bridge,
                browser_state,
                servo,
                context,
                signal,
                runtimes,
                modifiers,
                permission_locale,
                services,
            )
            .is_some()
            {
                fail(b"clipboard input unexpectedly started a navigation");
            }
            let Some(runtime) = runtime_for_tab(runtimes, tab_id) else {
                fail(b"clipboard acceptance tab closed");
            };
            pump_frame(runtime);
            if !copy_reported && services.counters.writes.get() > 0 {
                copy_reported = true;
                let _ = libnagi::console_write(b"Nagi M18 clipboard copy observed\r\n");
            }
            if copy_reported
                && !focus_reported
                && runtime.webview.page_title().as_deref() == Some("nagi-clip:focus:d")
            {
                focus_reported = true;
                let _ = libnagi::console_write(b"Nagi M18 clipboard destination focused\r\n");
            }
            if runtime.webview.page_title().as_deref() == Some(expected_title.as_str()) {
                break;
            }
            if libnagi::time_ticks() >= deadline {
                let counters = &services.counters;
                let _ = libnagi::console_write(if counters.writes.get() == 0 {
                    b"Nagi M18 clipboard timeout writes=0\r\n".as_slice()
                } else {
                    b"Nagi M18 clipboard timeout writes>0\r\n"
                });
                let _ = libnagi::console_write(if counters.reads.get() == 0 {
                    b"Nagi M18 clipboard timeout reads=0\r\n".as_slice()
                } else {
                    b"Nagi M18 clipboard timeout reads>0\r\n"
                });
                let _ = libnagi::console_write(if runtime.webview.focused() {
                    b"Nagi M18 clipboard timeout webview_focused=true\r\n".as_slice()
                } else {
                    b"Nagi M18 clipboard timeout webview_focused=false\r\n"
                });
                fail(b"clipboard copy/paste input timed out");
            }
            yield_guest_workers(signal);
        }

        let counters = &services.counters;
        if counters.writes.get() == 0
            || counters.reads.get() != 1
            || counters.denied_reads.get() != 0
            || counters.last_read.borrow().as_str() != CLIPBOARD_TOKEN
        {
            fail(b"clipboard service did not carry the copied text to the paste");
        }
        // The single paste gesture was consumed by that read. A further read
        // with no new user gesture must be refused by the service.
        match services
            .endpoint
            .read_text(tab_gesture_scope(tab_id), libnagi::time_ticks())
        {
            Err(ClipboardError::NoUserGesture) => {
                let _ = libnagi::console_write(
                    b"Nagi M18 clipboard ungestured read DENIED reason=no-user-gesture\r\n",
                );
            }
            _ => fail(b"clipboard service allowed a read without a user gesture"),
        }
        let Some(runtime) = runtime_for_tab(runtimes, tab_id) else {
            fail(b"clipboard acceptance tab closed");
        };
        if !present_settled_frame(servo, context, signal, surface, browser_state, runtime) {
            fail(b"clipboard result presentation failed");
        }
        if libnagi::console_write(b"Nagi M18 clipboard copy/paste PASS\r\n")
            != b"Nagi M18 clipboard copy/paste PASS\r\n".len()
        {
            fail(b"serial clipboard evidence write failed");
        }
    }

    /// Compose Japanese into the focused fixture field with QMP-typed romaji,
    /// committed through Servo composition events.
    fn run_ime_acceptance(
        servo: &Servo,
        context: &Rc<SoftwareRenderingContext>,
        signal: &Arc<EventLoopSignal>,
        surface: &mut NagiSurface,
        browser_state: &mut BrowserState,
        runtimes: &mut Vec<TabRuntime>,
        input_capability: u64,
        input_bridge: &mut InputBridge,
        modifiers: &mut KeyModifiers,
        permission_locale: Locale,
        services: &InputServices,
    ) {
        let Some(tab_id) = browser_state.active_tab_id() else {
            fail(b"IME acceptance has no active tab");
        };
        {
            let Some(runtime) = runtime_for_tab(runtimes, tab_id) else {
                fail(b"IME acceptance tab has no Servo WebView");
            };
            let deadline = libnagi::time_ticks().saturating_add(PAGE_TIMEOUT_TICKS);
            while runtime.delegate.ime_target.get().is_none() {
                servo.spin_event_loop();
                if libnagi::time_ticks() >= deadline {
                    fail(b"focused text field never requested an input method");
                }
                yield_guest_workers(signal);
            }
        }
        if services.ime.borrow().mode() != InputMode::Direct || services.ime_composing.get() {
            fail(b"IME was not idle in direct mode before the scenario");
        }
        if libnagi::console_write(b"Nagi M18 IME page READY\r\n")
            != b"Nagi M18 IME page READY\r\n".len()
        {
            fail(b"serial IME-ready marker write failed");
        }

        let expected_title =
            format!("nagi-ime:{IME_EXPECTED_COMMIT}|{CLIPBOARD_TOKEN}{IME_EXPECTED_COMMIT}");
        let deadline = libnagi::time_ticks().saturating_add(PAGE_TIMEOUT_TICKS);
        loop {
            servo.spin_event_loop();
            if route_guest_input(
                input_capability,
                input_bridge,
                browser_state,
                servo,
                context,
                signal,
                runtimes,
                modifiers,
                permission_locale,
                services,
            )
            .is_some()
            {
                fail(b"IME input unexpectedly started a navigation");
            }
            let Some(runtime) = runtime_for_tab(runtimes, tab_id) else {
                fail(b"IME acceptance tab closed");
            };
            pump_frame(runtime);
            if runtime.webview.page_title().as_deref() == Some(expected_title.as_str()) {
                break;
            }
            if libnagi::time_ticks() >= deadline {
                let _ = libnagi::console_write(if services.ime_commits.get() == 0 {
                    b"Nagi M18 IME timeout commits=0\r\n".as_slice()
                } else {
                    b"Nagi M18 IME timeout commits>0\r\n"
                });
                fail(b"IME composition input timed out");
            }
            yield_guest_workers(signal);
        }
        if services.ime_commits.get() != 1 || services.ime_composing.get() {
            fail(b"IME did not finish exactly one composition");
        }
        let Some(runtime) = runtime_for_tab(runtimes, tab_id) else {
            fail(b"IME acceptance tab closed");
        };
        if !present_settled_frame(servo, context, signal, surface, browser_state, runtime) {
            fail(b"IME result presentation failed");
        }
        if libnagi::console_write(b"Nagi M18 IME commit PASS\r\n")
            != b"Nagi M18 IME commit PASS\r\n".len()
        {
            fail(b"serial IME evidence write failed");
        }
    }

    const UPLOAD_FILE_NAME: &str = "nagi-upload.txt";
    const UPLOAD_FILE_CONTENT: &str = "nagi-upload-ok";
    const UPLOAD_READY_TITLE: &str = "nagi-upload:ready";

    /// List regular files in the picker's Documents folder.
    fn picker_candidates() -> Vec<PickerEntry> {
        let Ok(directory) = std::fs::read_dir(PICKER_DIRECTORY) else {
            return Vec::new();
        };
        directory
            .flatten()
            .filter_map(|entry| {
                let metadata = entry.metadata().ok()?;
                metadata.is_file().then(|| PickerEntry {
                    name: entry.file_name().to_string_lossy().into_owned(),
                    size: metadata.len(),
                })
            })
            .collect()
    }

    fn picker_key(code: u16) -> Option<PickerKey> {
        match code {
            103 => Some(PickerKey::Up),
            108 => Some(PickerKey::Down),
            28 => Some(PickerKey::Enter),
            1 => Some(PickerKey::Escape),
            _ => None,
        }
    }

    /// Choose a file for a page file input with Albert's trusted picker and
    /// confirm that the page received exactly that file.
    fn run_upload_acceptance(
        servo: &Servo,
        context: &Rc<SoftwareRenderingContext>,
        signal: &Arc<EventLoopSignal>,
        surface: &mut NagiSurface,
        browser_state: &mut BrowserState,
        runtimes: &mut Vec<TabRuntime>,
        input_capability: u64,
        input_bridge: &mut InputBridge,
        modifiers: &mut KeyModifiers,
        permission_locale: Locale,
        services: &InputServices,
    ) {
        if std::fs::create_dir_all(PICKER_DIRECTORY).is_err()
            || std::fs::write(
                format!("{PICKER_DIRECTORY}/{UPLOAD_FILE_NAME}"),
                UPLOAD_FILE_CONTENT,
            )
            .is_err()
        {
            fail(b"upload fixture file could not be written");
        }
        let Some(tab_id) = browser_state.active_tab_id() else {
            fail(b"upload acceptance has no active tab");
        };
        {
            let Some(runtime) = runtime_for_tab(runtimes, tab_id) else {
                fail(b"upload acceptance tab has no Servo WebView");
            };
            runtime.delegate.reset();
            runtime
                .webview
                .load(Url::parse(UPLOAD_PAGE).expect("the bundled upload page is valid"));
            let deadline = libnagi::time_ticks().saturating_add(PAGE_TIMEOUT_TICKS);
            loop {
                servo.spin_event_loop();
                if runtime.webview.load_status() == LoadStatus::Complete
                    && runtime.delegate.has_frame()
                    && runtime.webview.page_title().as_deref() == Some(UPLOAD_READY_TITLE)
                {
                    break;
                }
                if libnagi::time_ticks() >= deadline {
                    fail(b"upload page load timed out");
                }
                yield_guest_workers(signal);
            }
        }
        apply_content_navigation(browser_state, runtimes);
        {
            let Some(runtime) = runtime_for_tab(runtimes, tab_id) else {
                fail(b"upload acceptance tab closed");
            };
            if !present_settled_frame(servo, context, signal, surface, browser_state, runtime) {
                fail(b"upload page presentation failed");
            }
        }
        if libnagi::console_write(b"Nagi M18 upload page READY\r\n")
            != b"Nagi M18 upload page READY\r\n".len()
        {
            fail(b"serial upload-ready marker write failed");
        }

        let expected_title = format!(
            "nagi-upload:{UPLOAD_FILE_NAME}:{}:{UPLOAD_FILE_CONTENT}",
            UPLOAD_FILE_CONTENT.len()
        );
        let mut picker: Option<FilePickerState> = None;
        let mut chosen = false;
        let deadline = libnagi::time_ticks().saturating_add(PAGE_TIMEOUT_TICKS);
        loop {
            servo.spin_event_loop();
            let Some(runtime) = runtime_for_tab(runtimes, tab_id) else {
                fail(b"upload acceptance tab closed");
            };
            pump_frame(runtime);
            let request_pending = runtime.delegate.pending_file_picker.borrow().is_some();
            if request_pending && picker.is_none() {
                let filters: Vec<String> = runtime
                    .delegate
                    .pending_file_picker
                    .borrow()
                    .as_ref()
                    .map(|request| {
                        request
                            .filter_patterns()
                            .iter()
                            .map(|pattern| pattern.0.clone())
                            .collect()
                    })
                    .unwrap_or_default();
                let state = FilePickerState::new(picker_candidates(), &filters);
                let view = FilePickerView {
                    title: picker_title(permission_locale),
                    entries: state.entries(),
                    selected: state.selected(),
                };
                if !present_browser_surface(
                    context,
                    &runtime.webview,
                    surface,
                    browser_state,
                    None,
                    Some(&view),
                ) {
                    fail(b"file picker presentation failed");
                }
                picker = Some(state);
                let _ = libnagi::console_write(b"Nagi M18 upload picker READY\r\n");
            }
            if let Some(state) = picker.as_mut() {
                // While the picker is open, device input belongs to it.
                let mut event = libnagi::InputEvent::default();
                let outcome = if libnagi::input_read(input_capability, &mut event) {
                    match input_bridge.translate(event) {
                        Some(BrowserInput::Key {
                            code,
                            pressed: true,
                        }) => picker_key(code).map(|key| state.handle_key(key)),
                        Some(BrowserInput::MouseButton {
                            button: 0,
                            pressed: true,
                        }) => {
                            let (x, y) = input_bridge.position();
                            crate::file_picker::PickerLayout::new(WIDTH, HEIGHT)
                                .and_then(|layout| layout.row_at(x, y))
                                .map(|row| state.choose(row))
                        }
                        _ => None,
                    }
                } else {
                    None
                };
                match outcome {
                    Some(PickerOutcome::Chosen(path)) => {
                        if let Some(mut request) = runtime.delegate.pending_file_picker.take() {
                            request.select(&[std::path::PathBuf::from(&path)]);
                            request.submit();
                            chosen = true;
                            report_url(
                                b"Nagi M18 upload file chosen path=",
                                &Url::parse(&format!("file://{path}"))
                                    .expect("picker paths are absolute"),
                            );
                        }
                        picker = None;
                    }
                    Some(PickerOutcome::Canceled) => {
                        if let Some(request) = runtime.delegate.pending_file_picker.take() {
                            request.dismiss();
                        }
                        picker = None;
                        fail(b"upload picker was canceled");
                    }
                    Some(PickerOutcome::Pending) => {
                        let view = FilePickerView {
                            title: picker_title(permission_locale),
                            entries: state.entries(),
                            selected: state.selected(),
                        };
                        let _ = present_browser_surface(
                            context,
                            &runtime.webview,
                            surface,
                            browser_state,
                            None,
                            Some(&view),
                        );
                    }
                    None => {}
                }
            } else if route_guest_input(
                input_capability,
                input_bridge,
                browser_state,
                servo,
                context,
                signal,
                runtimes,
                modifiers,
                permission_locale,
                services,
            )
            .is_some()
            {
                fail(b"upload input unexpectedly started a navigation");
            }
            let Some(runtime) = runtime_for_tab(runtimes, tab_id) else {
                fail(b"upload acceptance tab closed");
            };
            if chosen && runtime.webview.page_title().as_deref() == Some(expected_title.as_str()) {
                break;
            }
            if libnagi::time_ticks() >= deadline {
                fail(b"upload input timed out");
            }
            yield_guest_workers(signal);
        }
        let Some(runtime) = runtime_for_tab(runtimes, tab_id) else {
            fail(b"upload acceptance tab closed");
        };
        if !present_settled_frame(servo, context, signal, surface, browser_state, runtime) {
            fail(b"upload result presentation failed");
        }
        if libnagi::console_write(b"Nagi M18 upload PASS\r\n") != b"Nagi M18 upload PASS\r\n".len()
        {
            fail(b"serial upload evidence write failed");
        }
    }

    /// Download a `data:` link through a real click and confirm that a
    /// script-dispatched click without user activation produced nothing.
    fn run_download_acceptance(
        servo: &Servo,
        context: &Rc<SoftwareRenderingContext>,
        signal: &Arc<EventLoopSignal>,
        surface: &mut NagiSurface,
        browser_state: &mut BrowserState,
        runtimes: &mut Vec<TabRuntime>,
        input_capability: u64,
        input_bridge: &mut InputBridge,
        modifiers: &mut KeyModifiers,
        permission_locale: Locale,
        services: &InputServices,
    ) {
        let saved_path = format!("{DOWNLOAD_DIRECTORY}/{DOWNLOAD_FILE_NAME}");
        // The User Data disk persists across runs; start from a clean name.
        let _ = std::fs::remove_file(&saved_path);
        let Some(tab_id) = browser_state.active_tab_id() else {
            fail(b"download acceptance has no active tab");
        };
        {
            let Some(runtime) = runtime_for_tab(runtimes, tab_id) else {
                fail(b"download acceptance tab has no Servo WebView");
            };
            runtime.delegate.reset();
            runtime
                .webview
                .load(Url::parse(DOWNLOAD_PAGE).expect("the bundled download page is valid"));
            let deadline = libnagi::time_ticks().saturating_add(PAGE_TIMEOUT_TICKS);
            loop {
                servo.spin_event_loop();
                if runtime.webview.load_status() == LoadStatus::Complete
                    && runtime.delegate.has_frame()
                    && runtime.webview.page_title().as_deref() == Some("nagi-download:ready")
                {
                    break;
                }
                if libnagi::time_ticks() >= deadline {
                    fail(b"download page load timed out");
                }
                yield_guest_workers(signal);
            }
            if !runtime.delegate.pending_downloads.borrow().is_empty()
                || std::fs::metadata(&saved_path).is_ok()
            {
                fail(b"script click without user activation produced a download");
            }
        }
        let _ = libnagi::console_write(b"Nagi M18 download unactivated click IGNORED\r\n");
        apply_content_navigation(browser_state, runtimes);
        {
            let Some(runtime) = runtime_for_tab(runtimes, tab_id) else {
                fail(b"download acceptance tab closed");
            };
            if !present_settled_frame(servo, context, signal, surface, browser_state, runtime) {
                fail(b"download page presentation failed");
            }
        }
        if libnagi::console_write(b"Nagi M18 download page READY\r\n")
            != b"Nagi M18 download page READY\r\n".len()
        {
            fail(b"serial download-ready marker write failed");
        }

        let deadline = libnagi::time_ticks().saturating_add(PAGE_TIMEOUT_TICKS);
        loop {
            servo.spin_event_loop();
            if route_guest_input(
                input_capability,
                input_bridge,
                browser_state,
                servo,
                context,
                signal,
                runtimes,
                modifiers,
                permission_locale,
                services,
            )
            .is_some()
            {
                fail(b"download click unexpectedly started a navigation");
            }
            let Some(runtime) = runtime_for_tab(runtimes, tab_id) else {
                fail(b"download acceptance tab closed");
            };
            pump_frame(runtime);
            if let Ok(contents) = std::fs::read(&saved_path) {
                if contents != DOWNLOAD_FILE_CONTENT.as_bytes() {
                    fail(b"saved download contents differ from the link");
                }
                break;
            }
            if libnagi::time_ticks() >= deadline {
                fail(b"download click timed out");
            }
            yield_guest_workers(signal);
        }
        if libnagi::console_write(b"Nagi M18 download PASS\r\n")
            != b"Nagi M18 download PASS\r\n".len()
        {
            fail(b"serial download evidence write failed");
        }
    }

    pub fn run(
        display_capability: u64,
        input_capability: u64,
        permission_locale: Locale,
        clipboard_endpoint: ClipboardEndpoint,
        clipboard_gestures: GestureSource,
    ) -> ! {
        let services = InputServices {
            trace_input: Cell::new(false),
            endpoint: clipboard_endpoint,
            gestures: clipboard_gestures,
            chrome: RefCell::new(BrowserClipboard::new()),
            counters: Rc::new(ClipboardCounters::default()),
            ime: RefCell::new(InputMethod::new(KanaCandidates)),
            ime_composing: Cell::new(false),
            ime_consumed_keys: RefCell::new([false; 256]),
            ime_commits: Cell::new(0),
        };
        VERIFIED_HOST_MASK.store(0, std::sync::atomic::Ordering::Release);
        crate::M18_NAVIGATION_TRACE_ACTIVE.store(false, Ordering::Release);

        let domain_list = servo::resources::read_bytes(servo::resources::Resource::DomainList);
        if domain_list.is_empty() {
            fail(b"Servo resources unavailable");
        }
        drop(domain_list);

        let Some(mut surface) = NagiSurface::acquire(display_capability) else {
            fail(b"display Surface unavailable");
        };
        let context = match SoftwareRenderingContext::new(PhysicalSize::new(WIDTH, PAGE_HEIGHT)) {
            Ok(context) => Rc::new(context),
            Err(_) => fail(b"Servo rendering context creation failed"),
        };
        let signal = Arc::new(EventLoopSignal::new());
        let mut servo_options = servo::Opts::default();
        servo_options.config_dir = Some(std::path::PathBuf::from(SERVO_CONFIG_DIR));
        servo_options.ignore_certificate_errors = false;
        let servo = ServoBuilder::default()
            .opts(servo_options)
            .event_loop_waker(Box::new(NagiWaker(signal.clone())))
            .build();
        servo.setup_logging();
        let mut browser_storage = GuestBrowserStorage;
        let restored = crate::session::restore(&mut browser_storage);
        if restored.warnings.is_empty() {
            let _ = libnagi::console_write(b"Nagi M18 browser storage RESTORE PASS\r\n");
        } else {
            let _ =
                libnagi::console_write(b"Nagi M18 browser storage RESTORE EMPTY_OR_INVALID\r\n");
        }
        let mut browser_state = restored.browser;
        if browser_state.active_tab_id().is_none() {
            fail(b"browser state has no initial tab");
        }
        let mut runtimes = Vec::new();
        // The acceptance must obtain its first HTTPS request from QMP-typed
        // address-bar input. Restored tabs therefore wait for user navigation
        // instead of issuing a network request before the READY marker.
        sync_tab_runtimes(
            &servo,
            &context,
            &signal,
            &browser_state,
            &mut runtimes,
            true,
            permission_locale,
            &services,
        );
        let Some(initial_runtime) = active_runtime(&browser_state, &runtimes) else {
            fail(b"browser has no initial Servo WebView");
        };
        wait_for_initial_blank(&servo, initial_runtime);
        let mut input_bridge = InputBridge::new();
        let mut modifiers = KeyModifiers::default();
        if libnagi::console_write(b"Nagi M18 browser READY\r\n")
            != b"Nagi M18 browser READY\r\n".len()
        {
            fail(b"serial browser-ready marker write failed");
        }

        for (index, (_, host)) in HTTPS_PAGES.iter().enumerate() {
            let request = if index == 0 {
                wait_for_address_navigation(
                    &servo,
                    &context,
                    &signal,
                    input_capability,
                    &mut input_bridge,
                    &mut browser_state,
                    &mut runtimes,
                    &mut modifiers,
                    permission_locale,
                    &services,
                )
            } else {
                let Some(tab_id) = browser_state.active_tab_id() else {
                    fail(b"browser state has no active tab");
                };
                let request = match browser_state.navigate(tab_id, HTTPS_PAGES[index].0) {
                    Ok(request) => request,
                    Err(_) => fail(b"browser rejected an HTTPS address"),
                };
                let url = Url::parse(&request.url).expect("normalized HTTPS URL is valid");
                let Some(runtime) = runtime_for_tab(&runtimes, request.tab_id) else {
                    fail(b"browser tab has no Servo WebView");
                };
                runtime.delegate.reset();
                runtime.webview.load(url);
                request
            };
            let Some(runtime) = runtime_for_tab(&runtimes, request.tab_id) else {
                fail(b"completed browser tab has no Servo WebView");
            };
            let final_url = wait_for_verified_page(
                &servo,
                &context,
                &mut surface,
                &browser_state,
                input_capability,
                &mut input_bridge,
                &runtimes,
                runtime,
                index,
            );
            if index == 0
                && libnagi::console_write(
                    b"Nagi M18 browser input navigation PASS host=example.com\r\n",
                ) != b"Nagi M18 browser input navigation PASS host=example.com\r\n".len()
            {
                fail(b"serial address-bar input evidence write failed");
            }
            if final_url.as_str() != request.url
                && !matches!(
                    browser_state.redirect(request.tab_id, request.id, final_url.as_str()),
                    Ok(NavigationEventResult::Applied)
                )
            {
                fail(b"browser state rejected the final redirect URL");
            }
            let title = runtime
                .webview
                .page_title()
                .unwrap_or_else(|| (*host).to_owned());
            if !matches!(
                browser_state.complete_navigation(
                    request.tab_id,
                    request.id,
                    &title,
                    libnagi::time_ticks(),
                ),
                Ok(NavigationEventResult::Applied)
            ) {
                fail(b"browser state rejected completed navigation");
            }
            // The first ready frame can predate the page's content; let Servo
            // finish painting before the evidence frame is read.
            settle_frames(&servo, &signal, runtime);
            render_page(
                &context,
                &runtime.webview,
                &mut surface,
                host,
                &browser_state,
            );
            persist_browser_state(&browser_state, &mut browser_storage);
        }

        run_clipboard_acceptance(
            &servo,
            &context,
            &signal,
            &mut surface,
            &mut browser_state,
            &mut runtimes,
            input_capability,
            &mut input_bridge,
            &mut modifiers,
            permission_locale,
            &services,
        );
        run_ime_acceptance(
            &servo,
            &context,
            &signal,
            &mut surface,
            &mut browser_state,
            &mut runtimes,
            input_capability,
            &mut input_bridge,
            &mut modifiers,
            permission_locale,
            &services,
        );
        run_upload_acceptance(
            &servo,
            &context,
            &signal,
            &mut surface,
            &mut browser_state,
            &mut runtimes,
            input_capability,
            &mut input_bridge,
            &mut modifiers,
            permission_locale,
            &services,
        );
        run_download_acceptance(
            &servo,
            &context,
            &signal,
            &mut surface,
            &mut browser_state,
            &mut runtimes,
            input_capability,
            &mut input_bridge,
            &mut modifiers,
            permission_locale,
            &services,
        );

        if libnagi::console_write(b"Nagi M18 browser scenario complete pages=3\r\n")
            != b"Nagi M18 browser scenario complete pages=3\r\n".len()
        {
            fail(b"serial scenario summary write failed");
        }
        libnagi::exit(0)
    }
}

#[cfg(target_os = "nagi")]
pub fn run_m18_https_acceptance(
    display_capability: u64,
    input_capability: u64,
    services: (
        nagi_clipboard::ClipboardEndpoint,
        nagi_clipboard::GestureSource,
    ),
) -> ! {
    guest::run(
        display_capability,
        input_capability,
        nagi_localization::Locale::EnUs,
        services.0,
        services.1,
    )
}

#[cfg(target_os = "nagi")]
pub fn run_m18_https_acceptance_with_locale(
    display_capability: u64,
    input_capability: u64,
    locale: nagi_localization::Locale,
    services: (
        nagi_clipboard::ClipboardEndpoint,
        nagi_clipboard::GestureSource,
    ),
) -> ! {
    guest::run(
        display_capability,
        input_capability,
        locale,
        services.0,
        services.1,
    )
}
