//! Nagi's minimal Servo embedder for the M17 first-web-pixel gate.
//!
//! The browser engine remains Servo. The rendering context is Servo's real
//! software Surfman context, whose Nagi target backend is pinned and patched
//! to guest Mesa/Softpipe. The only output handoff is the existing
//! capability-checked Nagi Surface.

#[cfg(target_os = "nagi")]
mod guest {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::Arc;

    use dpi::PhysicalSize;
    use nagi_servo_adapter::{EventLoopSignal, NagiSurface};
    use servo::{
        DeviceIntPoint, DeviceIntRect, DeviceIntSize, EventLoopWaker, RenderingContext, Servo,
        ServoBuilder, SoftwareRenderingContext, WebView, WebViewBuilder, WebViewDelegate,
    };
    use url::Url;

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
        let prefix = b"Nagi M17 trace: ";
        let written = libnagi::console_write(prefix) == prefix.len()
            && libnagi::console_write(stage) == stage.len()
            && libnagi::console_write(b"\r\n") == 2;
        if !written {
            libnagi::console_write(b"Nagi M17 trace FAIL console write\r\n");
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
            webview.paint();
            let Some(image) = self.context.read_to_image(Self::frame_rectangle()) else {
                return;
            };
            let frame = image.as_raw();
            let checksum = Self::checksum(frame);
            if checksum == 0 {
                return;
            }
            let mut surface = self.surface.borrow_mut();
            if surface
                .copy_rgba_frame(frame, WIDTH, HEIGHT, WIDTH as usize * 4)
                .is_ok()
                && surface.present()
            {
                Self::report(checksum);
            }
        }
    }

    pub fn run_first_web_pixel(display_capability: u64) -> ! {
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
        });
        let url = Url::parse(FIRST_WEB_PAGE).expect("the bundled M17 data URL is valid");
        libnagi::console_write(b"Nagi M17 trace: WebView construction started\r\n");
        let _webview = WebViewBuilder::new(&servo, context)
            .url(url)
            .delegate(delegate)
            .build();
        libnagi::console_write(b"Nagi M17 trace: WebView constructed\r\n");
        libnagi::console_write(b"Nagi M17 trace: Servo event loop started\r\n");
        loop {
            // The pinned Servo embedder owns shutdown handling and exposes
            // `spin_event_loop` as a unit-returning heartbeat.
            servo.spin_event_loop();
            if !signal.take() {
                std::thread::yield_now();
            }
        }
    }
}

#[cfg(target_os = "nagi")]
pub use guest::run_first_web_pixel;

#[cfg(not(target_os = "nagi"))]
pub fn run_first_web_pixel(_display_capability: u64) -> ! {
    panic!("nagi-albert requires the Nagi guest target")
}
