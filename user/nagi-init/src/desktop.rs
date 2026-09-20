use core::arch::asm;

use libnagi::{DisplayInfo, InputEvent};

use crate::ui::{rgba, Painter, Rect};

const APP_COUNT: usize = 4;
const WINDOW_WIDTH: i32 = 145;
const WINDOW_HEIGHT: i32 = 70;
const TITLE_HEIGHT: i32 = 14;
const POINTER_START_X: i32 = 80;
const POINTER_START_Y: i32 = 58;
const BACKGROUND: u32 = rgba(16, 24, 40);
const PANEL: u32 = rgba(228, 235, 240);
const TITLE: u32 = rgba(38, 166, 154);
const BORDER: u32 = rgba(8, 12, 20);
const TEXT: u32 = rgba(15, 23, 42);

#[no_mangle]
static NAGI_M10_READY: [u8; b"Nagi M10 desktop READY\r\n".len()] = *b"Nagi M10 desktop READY\r\n";
#[no_mangle]
static NAGI_M10_CHECKSUM_PREFIX: [u8; b"Nagi M10 surface checksum=".len()] =
    *b"Nagi M10 surface checksum=";
#[no_mangle]
static NAGI_M10_LINE_END: [u8; 2] = *b"\r\n";
#[no_mangle]
static NAGI_M10_CALCULATOR: [u8; b"Nagi M10 Calculator focus PASS\r\n".len()] =
    *b"Nagi M10 Calculator focus PASS\r\n";
#[no_mangle]
static NAGI_M10_NOTES: [u8; b"Nagi M10 Notes focus PASS\r\n".len()] =
    *b"Nagi M10 Notes focus PASS\r\n";
#[no_mangle]
static NAGI_M10_FILES: [u8; b"Nagi M10 Files focus PASS\r\n".len()] =
    *b"Nagi M10 Files focus PASS\r\n";
#[no_mangle]
static NAGI_M10_TERMINAL: [u8; b"Nagi M10 GUI Terminal focus PASS\r\n".len()] =
    *b"Nagi M10 GUI Terminal focus PASS\r\n";
#[no_mangle]
static NAGI_M10_JAPANESE: [u8; b"Nagi M10 Japanese input PASS\r\n".len()] =
    *b"Nagi M10 Japanese input PASS\r\n";
#[no_mangle]
static NAGI_M10_ACCEPTANCE: [u8; b"Nagi M10 acceptance PASS\r\n".len()] =
    *b"Nagi M10 acceptance PASS\r\n";
#[no_mangle]
static NAGI_M10_FAIL: [u8; b"Nagi M10 acceptance FAIL\r\n".len()] =
    *b"Nagi M10 acceptance FAIL\r\n";

#[no_mangle]
static CALCULATOR_TITLE: [u8; 10] = *b"Calculator";
#[no_mangle]
static NOTES_TITLE: [u8; 5] = *b"Notes";
#[no_mangle]
static FILES_TITLE: [u8; 5] = *b"Files";
#[no_mangle]
static TERMINAL_TITLE: [u8; 8] = *b"Terminal";
#[no_mangle]
static CALCULATOR_TEXT: [u8; 9] = *b"1 + 2 = 3";
#[no_mangle]
static NOTES_TEXT: [u8; 8] = *b"\xe3\x83\xa1\xe3\x83\xa2: ";
#[no_mangle]
static NOTES_KANA: [u8; 3] = *b"\xe3\x81\x82";
#[no_mangle]
static FILES_TEXT: [u8; 19] = *b"nagi-persistent.txt";
#[no_mangle]
static TERMINAL_TEXT: [u8; 9] = *b"$ nagi ps";

macro_rules! message {
    ($symbol:ident, $length:expr) => {{
        let address: *const u8;
        unsafe {
            asm!(
                "lea {address}, [rip + {symbol}]",
                address = out(reg) address,
                symbol = sym $symbol,
                options(nostack, preserves_flags, readonly),
            );
            core::slice::from_raw_parts(address, $length)
        }
    }};
}

pub struct Desktop {
    windows: [Rect; APP_COUNT],
    focused: [bool; APP_COUNT],
    pointer_x: i32,
    pointer_y: i32,
    notes_has_input: bool,
}

impl Desktop {
    pub const fn new() -> Self {
        Self {
            windows: [
                Rect::new(8, 24, WINDOW_WIDTH, WINDOW_HEIGHT),
                Rect::new(167, 24, WINDOW_WIDTH, WINDOW_HEIGHT),
                Rect::new(8, 104, WINDOW_WIDTH, WINDOW_HEIGHT),
                Rect::new(167, 104, WINDOW_WIDTH, WINDOW_HEIGHT),
            ],
            focused: [false; APP_COUNT],
            pointer_x: POINTER_START_X,
            pointer_y: POINTER_START_Y,
            notes_has_input: false,
        }
    }

    pub fn render(&self, surface: &mut [u32]) {
        let mut painter = Painter::new(surface);
        painter.fill(Rect::new(0, 0, 320, 200), BACKGROUND);
        self.render_app(
            &mut painter,
            0,
            message!(CALCULATOR_TITLE, 10),
            message!(CALCULATOR_TEXT, 9),
        );
        self.render_app(
            &mut painter,
            1,
            message!(NOTES_TITLE, 5),
            message!(NOTES_TEXT, 8),
        );
        if self.notes_has_input {
            painter.text(176, 57, message!(NOTES_KANA, 3), TEXT);
        }
        self.render_app(
            &mut painter,
            2,
            message!(FILES_TITLE, 5),
            message!(FILES_TEXT, 19),
        );
        self.render_app(
            &mut painter,
            3,
            message!(TERMINAL_TITLE, 8),
            message!(TERMINAL_TEXT, 9),
        );
        painter.fill(
            Rect::new(self.pointer_x - 1, self.pointer_y - 1, 3, 3),
            rgba(245, 158, 11),
        );
    }

    pub fn handle_event(&mut self, event: InputEvent) -> bool {
        if event.event_type == libnagi::INPUT_EVENT_REL {
            if event.code == libnagi::INPUT_REL_X {
                self.pointer_x = clamp(self.pointer_x.saturating_add(event.value), 0, 319);
                return event.value != 0;
            }
            if event.code == libnagi::INPUT_REL_Y {
                self.pointer_y = clamp(self.pointer_y.saturating_add(event.value), 0, 199);
                return event.value != 0;
            }
        }
        if event.event_type == libnagi::INPUT_EVENT_KEY && event.value != 0 {
            if event.code == libnagi::INPUT_KEY_LEFT {
                if self.windows[0].contains(self.pointer_x, self.pointer_y) {
                    if !self.focused[0] {
                        self.focused[0] = true;
                        print(message!(NAGI_M10_CALCULATOR, 32));
                    }
                    return true;
                }
                if self.windows[1].contains(self.pointer_x, self.pointer_y) {
                    if !self.focused[1] {
                        self.focused[1] = true;
                        print(message!(NAGI_M10_NOTES, 27));
                    }
                    return true;
                }
                if self.windows[2].contains(self.pointer_x, self.pointer_y) {
                    if !self.focused[2] {
                        self.focused[2] = true;
                        print(message!(NAGI_M10_FILES, 27));
                    }
                    return true;
                }
                if self.windows[3].contains(self.pointer_x, self.pointer_y) {
                    if !self.focused[3] {
                        self.focused[3] = true;
                        print(message!(NAGI_M10_TERMINAL, 34));
                    }
                    return true;
                }
            } else if self.focused[1] && !self.notes_has_input {
                self.notes_has_input = true;
                print(message!(NAGI_M10_JAPANESE, 30));
                return true;
            }
        }
        false
    }

    pub fn acceptance_ready(&self) -> bool {
        self.focused.iter().all(|focused| *focused) && self.notes_has_input
    }

    fn render_app(&self, painter: &mut Painter<'_>, index: usize, title: &[u8], content: &[u8]) {
        let window = self.windows[index];
        painter.fill(window, PANEL);
        painter.frame(window, if self.focused[index] { TITLE } else { BORDER });
        painter.fill(
            Rect::new(window.x + 1, window.y + 1, window.width - 2, TITLE_HEIGHT),
            TITLE,
        );
        painter.text(window.x + 6, window.y + 4, title, PANEL);
        painter.text(window.x + 8, window.y + TITLE_HEIGHT + 12, content, TEXT);
    }
}

pub fn run(display_capability: u64, input_capability: u64) -> ! {
    let mut info = DisplayInfo::default();
    if !libnagi::display_info(&mut info)
        || info.surface_bytes as usize != libnagi::SURFACE_BYTES
        || info.width != libnagi::SURFACE_WIDTH
        || info.height != libnagi::SURFACE_HEIGHT
    {
        print(message!(NAGI_M10_FAIL, 26));
        libnagi::exit(1);
    }
    let surface = unsafe {
        core::slice::from_raw_parts_mut(
            info.surface_address as *mut u32,
            libnagi::SURFACE_BYTES / core::mem::size_of::<u32>(),
        )
    };
    let mut desktop = Desktop::new();
    desktop.render(surface);
    if !libnagi::display_present(display_capability) {
        print(message!(NAGI_M10_FAIL, 26));
        libnagi::exit(1);
    }
    let initial_checksum = checksum(surface);
    print(message!(NAGI_M10_READY, 24));
    print_checksum(initial_checksum);
    loop {
        let mut event = InputEvent::default();
        if !libnagi::input_read(input_capability, &mut event) {
            unsafe { asm!("pause", options(nomem, nostack, preserves_flags)) };
            continue;
        }
        if desktop.handle_event(event) {
            desktop.render(surface);
            if !libnagi::display_present(display_capability) {
                print(message!(NAGI_M10_FAIL, 26));
                libnagi::exit(1);
            }
            let next_checksum = checksum(surface);
            if next_checksum == initial_checksum {
                print(message!(NAGI_M10_FAIL, 26));
                libnagi::exit(1);
            }
        }
        if desktop.acceptance_ready() {
            print(message!(NAGI_M10_ACCEPTANCE, 26));
            loop {
                unsafe { asm!("hlt", options(nomem, nostack, preserves_flags)) };
            }
        }
    }
}

fn clamp(value: i32, minimum: i32, maximum: i32) -> i32 {
    value.max(minimum).min(maximum)
}

fn checksum(surface: &[u32]) -> u64 {
    let mut value = 0xcbf2_9ce4_8422_2325_u64;
    for pixel in surface.iter().step_by(17) {
        value ^= u64::from(*pixel);
        value = value.wrapping_mul(0x1000_0000_01b3);
    }
    value
}

fn print(bytes: &[u8]) {
    libnagi::console_write(bytes);
}

fn print_checksum(value: u64) {
    let mut output = [0_u8; 80];
    let mut length = 0;
    length = append(&mut output, length, message!(NAGI_M10_CHECKSUM_PREFIX, 26));
    length = append_decimal(&mut output, length, value);
    length = append(&mut output, length, message!(NAGI_M10_LINE_END, 2));
    let message = unsafe { core::slice::from_raw_parts(output.as_ptr(), length) };
    print(message);
}

fn append(destination: &mut [u8], offset: usize, source: &[u8]) -> usize {
    unsafe {
        core::ptr::copy_nonoverlapping(
            source.as_ptr(),
            destination.as_mut_ptr().add(offset),
            source.len(),
        );
    }
    offset + source.len()
}

fn append_decimal(destination: &mut [u8], mut offset: usize, mut value: u64) -> usize {
    let start = offset;
    loop {
        unsafe {
            core::ptr::write(
                destination.as_mut_ptr().add(offset),
                b'0' + (value % 10) as u8,
            );
        }
        offset += 1;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    let mut left = start;
    let mut right = offset - 1;
    while left < right {
        unsafe {
            core::ptr::swap(
                destination.as_mut_ptr().add(left),
                destination.as_mut_ptr().add(right),
            );
        }
        left += 1;
        right -= 1;
    }
    offset
}
