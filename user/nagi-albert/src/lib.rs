//! Nagi's minimal Servo embedder for the M17 first-web-pixel gate.
//!
//! The browser engine remains Servo. The rendering context is Servo's real
//! software Surfman context, whose Nagi target backend is pinned and patched
//! to guest Mesa/Softpipe. The only output handoff is the existing
//! capability-checked Nagi Surface.

pub mod address_bar;
pub mod bookmarks;
pub mod browser_state;
pub mod chrome_surface;
pub mod clipboard;
pub mod downloads;
pub mod history;
pub mod ime;
pub mod input;
pub mod navigation;
pub mod permissions;
pub mod persistence;
pub mod session;
#[cfg(any(test, all(feature = "m18-acceptance", target_os = "nagi")))]
mod storage_bundle;
pub mod tabs;
pub mod ui;
pub mod uploads;

#[cfg(target_os = "nagi")]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(target_os = "nagi")]
pub(crate) static M18_NAVIGATION_TRACE_ACTIVE: AtomicBool = AtomicBool::new(false);

#[cfg(feature = "m18-acceptance")]
mod m18_acceptance;

#[cfg(any(test, all(feature = "m18-acceptance", target_os = "nagi")))]
mod nagi_storage;

#[cfg(target_os = "nagi")]
mod guest {
    use std::cell::{Cell, RefCell};
    use std::rc::Rc;
    use std::sync::Arc;

    use dpi::PhysicalSize;
    use nagi_servo_adapter::{EventLoopSignal, NagiSurface};
    use servo::{
        DeviceIntPoint, DeviceIntRect, DeviceIntSize, EventLoopWaker, LoadStatus, RenderingContext,
        Servo, ServoBuilder, SoftwareRenderingContext, WebView, WebViewBuilder, WebViewDelegate,
    };
    use url::Url;

    use super::{Ordering, M18_NAVIGATION_TRACE_ACTIVE};
    use crate::browser_state::BrowserState;
    use crate::chrome_surface::render_chrome;

    /// Write bounded Servo initialization diagnostics through Nagi's guest console syscall.
    ///
    /// # Safety
    ///
    /// `stage` must point to `length` readable bytes for the duration of this call.
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn nagi_m17_console_trace(stage: *const u8, length: usize) {
        if stage.is_null() || length == 0 || length > 128 {
            return;
        }
        let stage = unsafe { core::slice::from_raw_parts(stage, length) };
        if stage == b"M18 ConstellationProxy LoadUrl send started" {
            M18_NAVIGATION_TRACE_ACTIVE.store(true, Ordering::Release);
        }
        let prefix = b"Nagi M17 trace: ";
        let written = libnagi::console_write(prefix) == prefix.len()
            && libnagi::console_write(stage) == stage.len()
            && libnagi::console_write(b"\r\n") == 2;
        if !written {
            libnagi::console_write(b"Nagi M17 trace FAIL console write\r\n");
        }
    }

    #[unsafe(no_mangle)]
    pub extern "C" fn nagi_m18_navigation_trace_enabled() -> u8 {
        u8::from(M18_NAVIGATION_TRACE_ACTIVE.load(Ordering::Acquire))
    }

    fn trace_stage(stage: &'static [u8]) {
        if !stage.is_empty() && stage.len() <= 128 {
            // SAFETY: `stage` is a valid static slice for the duration of the call.
            unsafe { nagi_m17_console_trace(stage.as_ptr(), stage.len()) };
        }
    }

    const WIDTH: u32 = 320;
    const HEIGHT: u32 = 200;
    const FIRST_WEB_PAGE: &str = "data:text/html,%3C!doctype%20html%3E%3Cmeta%20charset%3Dutf-8%3E%3Cbody%20style%3D%22margin%3A0%3Bbackground%3A%2320384d%3Bcolor%3A%23f7f3e8%3Bfont%3A24px%20sans-serif%3Bdisplay%3Agrid%3Bplace-items%3Acenter%22%3ENagi%20M17%3C%2Fbody%3E";

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

    struct FirstPixelDelegate {
        context: Rc<SoftwareRenderingContext>,
        surface: RefCell<NagiSurface>,
        browser_state: RefCell<BrowserState>,
        composed_frame: RefCell<Vec<u8>>,
        current_url: RefCell<String>,
        frame_diagnostics_emitted: Cell<bool>,
        chrome_rendered_emitted: Cell<bool>,
    }

    impl FirstPixelDelegate {
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

        fn report(checksum: u32) {
            let mut line = [0_u8; 54];
            let prefix = b"Nagi M17 first web pixel checksum=0x";
            line[..prefix.len()].copy_from_slice(prefix);
            let mut value = checksum;
            let mut index = prefix.len() + 8;
            while index > prefix.len() {
                index -= 1;
                line[index] = b"0123456789abcdef"[(value & 0xf) as usize];
                value >>= 4;
            }
            line[prefix.len() + 8..prefix.len() + 10].copy_from_slice(b"\r\n");
            libnagi::console_write(&line[..prefix.len() + 10]);
            libnagi::console_write(b"Nagi M17 first web pixel PASS\r\n");
        }
    }

    impl WebViewDelegate for FirstPixelDelegate {
        fn notify_new_frame_ready(&self, webview: WebView) {
            let trace_first = !self.frame_diagnostics_emitted.replace(true);
            if trace_first {
                trace_stage(b"first web frame callback entered");
            }
            webview.paint();
            if trace_first {
                trace_stage(b"first web frame paint returned");
            }
            let Some(image) = self.context.read_to_image(Self::frame_rectangle()) else {
                if trace_first {
                    trace_stage(b"first web frame readback unavailable");
                }
                return;
            };
            if trace_first {
                trace_stage(b"first web frame readback returned");
            }
            let frame = image.as_raw();
            let checksum = Self::checksum(frame);
            if checksum == 0 {
                if trace_first {
                    trace_stage(b"first web frame checksum was zero");
                }
                return;
            }
            let mut composed_frame = self.composed_frame.borrow_mut();
            if composed_frame.len() != frame.len() {
                if trace_first {
                    trace_stage(b"browser chrome frame size mismatch");
                }
                return;
            }
            composed_frame.copy_from_slice(frame);
            let mut chrome = crate::ui::view(&self.browser_state.borrow());
            let current_url = self.current_url.borrow();
            if !current_url.is_empty() {
                chrome.address_text = current_url.clone();
            }
            if render_chrome(
                &mut composed_frame,
                WIDTH,
                HEIGHT,
                WIDTH as usize * 4,
                &chrome,
            )
            .is_err()
            {
                if trace_first {
                    trace_stage(b"browser chrome render rejected");
                }
                return;
            }
            let mut surface = self.surface.borrow_mut();
            if surface
                .copy_rgba_frame(&composed_frame, WIDTH, HEIGHT, WIDTH as usize * 4)
                .is_err()
            {
                if trace_first {
                    trace_stage(b"first web frame surface copy rejected");
                }
                return;
            }
            if trace_first {
                trace_stage(b"first web frame copied to surface");
            }
            if !surface.present() {
                if trace_first {
                    trace_stage(b"first web frame surface present failed");
                }
                return;
            }
            if !self.chrome_rendered_emitted.replace(true) {
                libnagi::console_write(b"Nagi M18B Albert chrome presented\r\n");
            }
            if trace_first {
                trace_stage(b"first web frame surface present completed");
            }
            Self::report(checksum);
        }

        fn notify_url_changed(&self, _webview: WebView, url: Url) {
            *self.current_url.borrow_mut() = url.to_string();
            trace_stage(b"WebView URL changed");
        }

        fn notify_load_status_changed(&self, _webview: WebView, status: LoadStatus) {
            let stage = match status {
                LoadStatus::Started => b"WebView load status started" as &'static [u8],
                LoadStatus::HeadParsed => b"WebView load status head parsed",
                LoadStatus::Complete => b"WebView load status complete",
            };
            trace_stage(stage);
        }
    }

    pub fn run_first_web_pixel(display_capability: u64) -> ! {
        libnagi::console_write(b"Nagi M17 trace: Servo resource reader preflight started\r\n");
        let domain_list = servo::resources::read_bytes(servo::resources::Resource::DomainList);
        if domain_list.is_empty() {
            libnagi::console_write(b"Nagi M17 first web pixel FAIL Servo resources\r\n");
            libnagi::exit(1);
        }
        libnagi::console_write(b"Nagi M17 trace: Servo resource reader registered\r\n");
        drop(domain_list);

        libnagi::console_write(b"Nagi M17 trace: Surface acquisition started\r\n");
        let Some(surface) = NagiSurface::acquire(display_capability) else {
            libnagi::console_write(b"Nagi M17 first web pixel FAIL surface\r\n");
            libnagi::exit(1);
        };
        libnagi::console_write(b"Nagi M17 trace: Surface acquired\r\n");
        libnagi::console_write(b"Nagi M17 trace: GL context creation started\r\n");
        let callback_probe = b"Albert console callback self-test";
        // SAFETY: The byte slice remains readable for the synchronous callback.
        unsafe {
            nagi_m17_console_trace(callback_probe.as_ptr(), callback_probe.len());
        }
        let context = match SoftwareRenderingContext::new(PhysicalSize::new(WIDTH, HEIGHT)) {
            Ok(context) => Rc::new(context),
            Err(_) => {
                libnagi::console_write(b"Nagi M17 first web pixel FAIL GL context\r\n");
                libnagi::exit(1);
            }
        };
        libnagi::console_write(b"Nagi M17 trace: GL context created\r\n");
        let signal = Arc::new(EventLoopSignal::new());
        libnagi::console_write(b"Nagi M17 trace: Servo construction started\r\n");
        let servo = ServoBuilder::default()
            .event_loop_waker(Box::new(NagiWaker(signal.clone())))
            .build();
        servo.setup_logging();
        libnagi::console_write(b"Nagi M17 trace: Servo constructed\r\n");
        let delegate = Rc::new(FirstPixelDelegate {
            context: context.clone(),
            surface: RefCell::new(surface),
            browser_state: RefCell::new(BrowserState::new()),
            composed_frame: RefCell::new(vec![0; WIDTH as usize * HEIGHT as usize * 4]),
            current_url: RefCell::new(String::new()),
            frame_diagnostics_emitted: Cell::new(false),
            chrome_rendered_emitted: Cell::new(false),
        });
        let url = Url::parse(FIRST_WEB_PAGE).expect("the bundled M17 data URL is valid");
        libnagi::console_write(b"Nagi M17 trace: WebView construction started\r\n");
        let _webview = WebViewBuilder::new(&servo, context)
            .url(url)
            .delegate(delegate)
            .build();
        libnagi::console_write(b"Nagi M17 trace: WebView constructed\r\n");
        libnagi::console_write(b"Nagi M17 trace: Servo event loop started\r\n");
        let mut first_spin = true;
        loop {
            // The pinned Servo embedder owns shutdown handling and exposes
            // `spin_event_loop` as a unit-returning heartbeat.
            servo.spin_event_loop();
            if first_spin {
                trace_stage(b"Servo first event-loop dispatch returned");
                first_spin = false;
            }
            if !signal.take() {
                std::thread::yield_now();
            }
        }
    }
}

#[cfg(target_os = "nagi")]
pub use guest::run_first_web_pixel;

#[cfg(all(target_os = "nagi", feature = "m18-acceptance"))]
pub use m18_acceptance::run_m18_https_acceptance;

/// FFI callback used by the pinned Servo verifier after chain and hostname
/// validation succeeds. Builds without the M18 acceptance feature keep the
/// symbol available for the same pinned Servo target graph, but report no M18
/// evidence.
#[cfg(target_os = "nagi")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn nagi_m18_tls_verified(host: *const u8, length: usize) {
    if host.is_null() || length == 0 || length > 253 {
        return;
    }
    let host = unsafe { core::slice::from_raw_parts(host, length) };
    #[cfg(feature = "m18-acceptance")]
    m18_acceptance::record_tls_verification(host);
    #[cfg(not(feature = "m18-acceptance"))]
    let _ = host;
}

#[cfg(not(target_os = "nagi"))]
pub fn run_first_web_pixel(_display_capability: u64) -> ! {
    panic!("nagi-albert requires the Nagi guest target")
}
