use core::arch::asm;

use libnagi::{DisplayInfo, InputEvent};

const WINDOW_WIDTH: i32 = 112;
const WINDOW_HEIGHT: i32 = 72;
const INITIAL_X: i32 = 32;
const INITIAL_Y: i32 = 48;
const TITLE_HEIGHT: i32 = 16;

#[no_mangle]
static NAGI_M9_READY: [u8; b"Nagi M9 window READY\r\n".len()] = *b"Nagi M9 window READY\r\n";
#[no_mangle]
static NAGI_M9_MOUSE_PASS: [u8; b"Nagi M9 mouse move PASS\r\n".len()] =
    *b"Nagi M9 mouse move PASS\r\n";
#[no_mangle]
static NAGI_M9_FOCUS_PASS: [u8; b"Nagi M9 focus PASS\r\n".len()] = *b"Nagi M9 focus PASS\r\n";
#[no_mangle]
static NAGI_M9_KEYBOARD_PASS: [u8; b"Nagi M9 keyboard PASS\r\n".len()] =
    *b"Nagi M9 keyboard PASS\r\n";
#[no_mangle]
static NAGI_M9_ACCEPTANCE_PASS: [u8; b"Nagi M9 acceptance PASS\r\n".len()] =
    *b"Nagi M9 acceptance PASS\r\n";
#[no_mangle]
static NAGI_M9_ACCEPTANCE_FAIL: [u8; b"Nagi M9 acceptance FAIL\r\n".len()] =
    *b"Nagi M9 acceptance FAIL\r\n";
#[no_mangle]
static NAGI_M9_STATE_PREFIX: [u8; b"Nagi M9 state x=".len()] = *b"Nagi M9 state x=";
#[no_mangle]
static NAGI_M9_Y_PREFIX: [u8; b" y=".len()] = *b" y=";
#[no_mangle]
static NAGI_M9_CHECKSUM_PREFIX: [u8; b" checksum=".len()] = *b" checksum=";
#[no_mangle]
static NAGI_M9_LINE_END: [u8; b"\r\n".len()] = *b"\r\n";

macro_rules! message {
    ($symbol:ident) => {{
        let address: *const u8;
        unsafe {
            asm!(
                "lea {address}, [rip + {symbol}]",
                address = out(reg) address,
                symbol = sym $symbol,
                options(nostack, preserves_flags, readonly),
            );
            core::slice::from_raw_parts(address, $symbol.len())
        }
    }};
}

pub fn run(display_capability: u64, input_capability: u64) -> ! {
    let mut info = DisplayInfo::default();
    if !libnagi::display_info(&mut info)
        || info.surface_bytes as usize != libnagi::SURFACE_BYTES
        || info.width != libnagi::SURFACE_WIDTH
        || info.height != libnagi::SURFACE_HEIGHT
    {
        print(message!(NAGI_M9_ACCEPTANCE_FAIL));
        libnagi::exit(1);
    }
    let surface = unsafe {
        core::slice::from_raw_parts_mut(
            info.surface_address as *mut u32,
            libnagi::SURFACE_BYTES / core::mem::size_of::<u32>(),
        )
    };
    let mut x = INITIAL_X;
    let mut y = INITIAL_Y;
    let mut pointer_x = INITIAL_X + WINDOW_WIDTH / 2;
    let mut pointer_y = INITIAL_Y + TITLE_HEIGHT / 2;
    let mut focused = false;
    let mut moved = false;
    let mut keyboard = false;
    render(surface, x, y);
    if !libnagi::display_present(display_capability) {
        print(message!(NAGI_M9_ACCEPTANCE_FAIL));
        libnagi::exit(1);
    }
    let initial_checksum = checksum(surface);
    print(message!(NAGI_M9_READY));
    loop {
        let mut event = InputEvent::default();
        if !libnagi::input_read(input_capability, &mut event) {
            unsafe { asm!("pause", options(nomem, nostack, preserves_flags)) };
            continue;
        }
        let mut changed = false;
        if event.event_type == libnagi::INPUT_EVENT_REL {
            if event.code == libnagi::INPUT_REL_X && event.value != 0 {
                pointer_x = clamp_pointer(pointer_x.saturating_add(event.value), info.width as i32);
                x = clamp_position(x.saturating_add(event.value), info.width as i32);
                moved = true;
                changed = true;
            } else if event.code == libnagi::INPUT_REL_Y && event.value != 0 {
                pointer_y =
                    clamp_pointer(pointer_y.saturating_add(event.value), info.height as i32);
                y = clamp_position(y.saturating_add(event.value), info.height as i32);
                moved = true;
                changed = true;
            }
        } else if event.event_type == libnagi::INPUT_EVENT_KEY && event.value != 0 {
            if event.code == libnagi::INPUT_KEY_LEFT {
                if inside_window(pointer_x, pointer_y, x, y) {
                    focused = true;
                    print(message!(NAGI_M9_FOCUS_PASS));
                }
            } else if focused {
                keyboard = true;
                print(message!(NAGI_M9_KEYBOARD_PASS));
            }
        }
        if changed {
            render(surface, x, y);
            if !libnagi::display_present(display_capability) {
                print(message!(NAGI_M9_ACCEPTANCE_FAIL));
                libnagi::exit(1);
            }
            let next_checksum = checksum(surface);
            if next_checksum != initial_checksum {
                print(message!(NAGI_M9_MOUSE_PASS));
                print_state(x, y, next_checksum);
            }
        }
        if moved && focused && keyboard {
            print(message!(NAGI_M9_ACCEPTANCE_PASS));
            loop {
                unsafe { asm!("hlt", options(nomem, nostack, preserves_flags)) };
            }
        }
    }
}

fn render(surface: &mut [u32], window_x: i32, window_y: i32) {
    for y in 0..libnagi::SURFACE_HEIGHT as i32 {
        for x in 0..libnagi::SURFACE_WIDTH as i32 {
            let mut color = rgba(27, 38, 59, 255);
            if x >= window_x - 3
                && x < window_x + WINDOW_WIDTH + 3
                && y >= window_y - 3
                && y < window_y + WINDOW_HEIGHT + 3
            {
                color = rgba(10, 16, 28, 255);
            }
            if x >= window_x
                && x < window_x + WINDOW_WIDTH
                && y >= window_y
                && y < window_y + WINDOW_HEIGHT
            {
                color = if y < window_y + TITLE_HEIGHT {
                    rgba(38, 166, 154, 255)
                } else {
                    rgba(232, 240, 242, 255)
                };
            }
            let index = (y as usize) * libnagi::SURFACE_WIDTH as usize + x as usize;
            unsafe {
                core::ptr::write(surface.as_mut_ptr().add(index), color);
            }
        }
    }
}

fn checksum(surface: &[u32]) -> u64 {
    let mut value = 0xcbf2_9ce4_8422_2325_u64;
    for pixel in surface.iter().step_by(17) {
        value ^= u64::from(*pixel);
        value = value.wrapping_mul(0x1000_0000_01b3);
    }
    value
}

fn clamp_position(value: i32, limit: i32) -> i32 {
    value.max(4).min((limit - WINDOW_WIDTH - 4).max(4))
}

fn clamp_pointer(value: i32, limit: i32) -> i32 {
    value.max(0).min((limit - 1).max(0))
}

fn inside_window(pointer_x: i32, pointer_y: i32, window_x: i32, window_y: i32) -> bool {
    pointer_x >= window_x
        && pointer_x < window_x + WINDOW_WIDTH
        && pointer_y >= window_y
        && pointer_y < window_y + WINDOW_HEIGHT
}

fn rgba(red: u8, green: u8, blue: u8, alpha: u8) -> u32 {
    u32::from(red) | (u32::from(green) << 8) | (u32::from(blue) << 16) | (u32::from(alpha) << 24)
}

fn print(bytes: &[u8]) {
    libnagi::console_write(bytes);
}

fn print_state(x: i32, y: i32, checksum: u64) {
    let mut output = [0_u8; 96];
    let mut length = 0;
    length = append(&mut output, length, message!(NAGI_M9_STATE_PREFIX));
    length = append_decimal(&mut output, length, x as u64);
    length = append(&mut output, length, message!(NAGI_M9_Y_PREFIX));
    length = append_decimal(&mut output, length, y as u64);
    length = append(&mut output, length, message!(NAGI_M9_CHECKSUM_PREFIX));
    length = append_decimal(&mut output, length, checksum);
    length = append(&mut output, length, message!(NAGI_M9_LINE_END));
    let message = unsafe { core::slice::from_raw_parts(output.as_ptr(), length) };
    libnagi::console_write(message);
}

fn append(destination: &mut [u8], offset: usize, source: &[u8]) -> usize {
    let end = offset + source.len();
    unsafe {
        core::ptr::copy_nonoverlapping(
            source.as_ptr(),
            destination.as_mut_ptr().add(offset),
            source.len(),
        );
    }
    end
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
            let left_ptr = destination.as_mut_ptr().add(left);
            let right_ptr = destination.as_mut_ptr().add(right);
            core::ptr::swap(left_ptr, right_ptr);
        }
        left += 1;
        right -= 1;
    }
    offset
}
