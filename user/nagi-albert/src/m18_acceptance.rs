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
    use crate::input::{
        chrome_action_at, evdev_character, is_backspace_key, is_enter_key, is_escape_key,
        is_shift_key, CHROME_HEIGHT,
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
    use nagi_localization::Locale;

    const WIDTH: u32 = 320;
    const HEIGHT: u32 = 200;
    const PAGE_TIMEOUT_TICKS: u64 = 3_000;
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
            }
        }

        fn reset(&self) {
            self.navigation_started.set(false);
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

    struct UnavailableClipboardDelegate;

    impl ClipboardDelegate for UnavailableClipboardDelegate {
        fn clear(&self, _webview: WebView) {
            let _ = libnagi::console_write(b"Nagi M18 clipboard service unavailable\r\n");
        }

        fn get_text(&self, _webview: WebView, request: StringRequest) {
            request.failure("Nagi clipboard service unavailable".to_owned());
        }

        fn set_text(&self, _webview: WebView, _new_contents: String) {
            let _ = libnagi::console_write(b"Nagi M18 clipboard service unavailable\r\n");
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
                .clipboard_delegate(Rc::new(UnavailableClipboardDelegate))
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

        fn notify_url_changed(&self, _webview: WebView, url: Url) {
            report_url(b"Nagi M18 browser URL changed to=", &url);
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
                    file_picker.dismiss();
                    let _ = libnagi::console_write(b"Nagi M18 upload service unavailable\r\n");
                }
                EmbedderControl::InputMethod(input_method) => {
                    let _ = input_method.id();
                    let _ = input_method.text();
                    let _ = input_method.insertion_point();
                    let _ = libnagi::console_write(b"Nagi M18 IME service unavailable\r\n");
                }
                _ => {}
            }
        }
    }

    fn frame_rectangle() -> DeviceIntRect {
        DeviceIntRect::from_origin_and_size(
            DeviceIntPoint::zero(),
            DeviceIntSize::new(WIDTH as i32, HEIGHT as i32),
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

    fn report_page(host: &str, frame_checksum: u32) -> bool {
        let prefix = b"Nagi M18 HTTPS page RENDERED host=";
        let middle = b" frame_checksum=0x";
        let suffix = b"\r\n";
        let mut line = [0_u8; 128];
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
        );
        match outcome {
            Ok(BrowserChromeOutcome::Navigate(request)) => {
                let url = Url::parse(&request.url).expect("browser state emits normalized URLs");
                let Some(runtime) = runtime_for_tab(runtimes, request.tab_id) else {
                    return None;
                };
                runtime.delegate.reset();
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

    fn page_keyboard_event(code: u16, pressed: bool, shifted: bool) -> Option<ServoInputEvent> {
        let (key, physical_code) = if let Some(character) = evdev_character(code, shifted) {
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
        let mut modifiers = Modifiers::default();
        if shifted {
            modifiers.insert(Modifiers::SHIFT);
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
            modifiers,
            false,
            false,
        )))
    }

    fn route_guest_input(
        input_capability: u64,
        bridge: &mut InputBridge,
        browser_state: &mut BrowserState,
        servo: &Servo,
        context: &Rc<SoftwareRenderingContext>,
        signal: &Arc<EventLoopSignal>,
        runtimes: &mut Vec<TabRuntime>,
        shifted: &mut bool,
        permission_locale: Locale,
    ) -> Option<crate::navigation::NavigationRequest> {
        let mut event = libnagi::InputEvent::default();
        if !libnagi::input_read(input_capability, &mut event) {
            return None;
        }
        match bridge.translate(event)? {
            BrowserInput::MouseMove { x, y } => {
                if y >= CHROME_HEIGHT {
                    if let Some(runtime) = active_runtime(browser_state, runtimes) {
                        runtime
                            .webview
                            .notify_input_event(ServoInputEvent::MouseMove(MouseMoveEvent::new(
                                servo_point(x, y),
                            )));
                    }
                }
                None
            }
            BrowserInput::MouseButton { button: 0, pressed } => {
                let (x, y) = bridge.position();
                if y < CHROME_HEIGHT {
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
                                action,
                            );
                        }
                    }
                    return None;
                }
                if let Some(runtime) = active_runtime(browser_state, runtimes) {
                    runtime
                        .webview
                        .notify_input_event(ServoInputEvent::MouseButton(MouseButtonEvent::new(
                            if pressed {
                                MouseButtonAction::Down
                            } else {
                                MouseButtonAction::Up
                            },
                            MouseButton::Primary,
                            servo_point(x, y),
                        )));
                }
                None
            }
            BrowserInput::MouseButton { .. } => None,
            BrowserInput::Key { code, pressed } if is_shift_key(code) => {
                *shifted = pressed;
                None
            }
            BrowserInput::Key { code, pressed } if browser_state.address_bar().is_focused() => {
                if !pressed {
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
                } else if let Some(character) = evdev_character(code, *shifted) {
                    let _ = ui::dispatch(
                        browser_state,
                        BrowserChromeAction::InsertAddressText(character.to_string()),
                        libnagi::time_ticks(),
                    );
                }
                None
            }
            BrowserInput::Key { code, pressed } => {
                if let (Some(runtime), Some(event)) = (
                    active_runtime(browser_state, runtimes),
                    page_keyboard_event(code, pressed, *shifted),
                ) {
                    runtime.webview.notify_input_event(event);
                }
                None
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
        shifted: &mut bool,
        permission_locale: Locale,
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
                shifted,
                permission_locale,
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
    ) -> bool {
        webview.paint();
        let Some(image) = context.read_to_image(frame_rectangle()) else {
            return false;
        };
        let mut frame = image.as_raw().to_vec();
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
                && !present_browser_surface(context, &runtime.webview, surface, browser_state, None)
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
        let mut composed_frame = frame.to_vec();
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
        if !report_page(host, frame_checksum) {
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
                    b"Nagi M18 browser storage SAVE CAPACITY limit=1024\r\n",
                );
            }
            Err(_) => {
                let _ = libnagi::console_write(b"Nagi M18 browser storage SAVE UNAVAILABLE\r\n");
            }
        }
    }

    pub fn run(display_capability: u64, input_capability: u64, permission_locale: Locale) -> ! {
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
        let context = match SoftwareRenderingContext::new(PhysicalSize::new(WIDTH, HEIGHT)) {
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
        );
        let Some(initial_runtime) = active_runtime(&browser_state, &runtimes) else {
            fail(b"browser has no initial Servo WebView");
        };
        wait_for_initial_blank(&servo, initial_runtime);
        let mut input_bridge = InputBridge::new();
        let mut shifted = false;
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
                    &mut shifted,
                    permission_locale,
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
            render_page(
                &context,
                &runtime.webview,
                &mut surface,
                host,
                &browser_state,
            );
            persist_browser_state(&browser_state, &mut browser_storage);
        }

        if libnagi::console_write(b"Nagi M18 browser scenario complete pages=3\r\n")
            != b"Nagi M18 browser scenario complete pages=3\r\n".len()
        {
            fail(b"serial scenario summary write failed");
        }
        libnagi::exit(0)
    }
}

#[cfg(target_os = "nagi")]
pub fn run_m18_https_acceptance(display_capability: u64, input_capability: u64) -> ! {
    guest::run(
        display_capability,
        input_capability,
        nagi_localization::Locale::EnUs,
    )
}

#[cfg(target_os = "nagi")]
pub fn run_m18_https_acceptance_with_locale(
    display_capability: u64,
    input_capability: u64,
    locale: nagi_localization::Locale,
) -> ! {
    guest::run(display_capability, input_capability, locale)
}
