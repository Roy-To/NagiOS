//! Nagi C locale, character classification, and multibyte conversion.
//!
//! Nagi's C library has exactly one locale: the C/POSIX locale, whose
//! multibyte encoding is UTF-8 (Nagi's canonical text encoding,
//! `docs/architecture/language-architecture.md`). Character classes are the
//! POSIX C-locale ASCII classes for both narrow and wide characters; collation
//! is code-point order. `newlocale` accepts the names of that locale and
//! returns one shared handle, so every `*_l` function equals its
//! current-locale counterpart.
//!
//! These entry points let the target-built libc++ (`docs/decisions/0053-*`)
//! run its locale, iostream, and `<filesystem>` code on Nagi without a host
//! C runtime.

use core::{
    ffi::{VaList, c_char, c_double, c_float, c_int, c_long, c_void},
    ptr,
    sync::atomic::{AtomicUsize, Ordering},
};

use super::{EINVAL, EOF, NagiTm, nagi_parse_float, nagi_vsscanf, set_errno};

const ENOENT: c_int = 2;
const EILSEQ: c_int = 84;

type WChar = c_int;
type WInt = u32;

const WEOF: WInt = 0xffff_ffff;
const LC_ALL_MASK: c_int = 63;
/// `(locale_t)-1` in `<locale.h>`.
const LC_GLOBAL_LOCALE: usize = usize::MAX;
/// `(size_t)-1` and `(size_t)-2` multibyte conversion results.
const CONVERSION_ERROR: usize = usize::MAX;
const CONVERSION_INCOMPLETE: usize = usize::MAX - 1;

// --- Locale objects --------------------------------------------------------

/// The C/POSIX locale object. Only its address is meaningful.
static NAGI_C_LOCALE_OBJECT: u8 = 0;

/// Locale installed by `uselocale`. With a single locale the installed value
/// is observable only as the handle `uselocale` returns, so Nagi keeps it
/// process-wide rather than per thread.
static NAGI_CURRENT_LOCALE: AtomicUsize = AtomicUsize::new(LC_GLOBAL_LOCALE);

fn c_locale_handle() -> *mut c_void {
    ptr::addr_of!(NAGI_C_LOCALE_OBJECT).cast_mut().cast()
}

fn is_locale_handle(locale: *mut c_void) -> bool {
    locale as usize == LC_GLOBAL_LOCALE || locale == c_locale_handle()
}

/// Whether `name` selects the C/POSIX locale: `""` (the environment default),
/// `C`, `POSIX`, or the explicit UTF-8 spellings of the C locale.
unsafe fn is_c_locale_name(name: *const c_char) -> bool {
    let mut length = 0;
    while unsafe { name.add(length).read() } != 0 {
        length += 1;
        if length > 16 {
            return false;
        }
    }
    let name = unsafe { core::slice::from_raw_parts(name.cast::<u8>(), length) };
    matches!(name, b"" | b"C" | b"POSIX" | b"C.UTF-8" | b"C.utf8")
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn newlocale(
    category_mask: c_int,
    name: *const c_char,
    _base: *mut c_void,
) -> *mut c_void {
    if name.is_null() || category_mask & !LC_ALL_MASK != 0 {
        unsafe { set_errno(EINVAL) };
        return ptr::null_mut();
    }
    if !unsafe { is_c_locale_name(name) } {
        unsafe { set_errno(ENOENT) };
        return ptr::null_mut();
    }
    // Every category of `_base` already equals the C locale, so the shared
    // handle is the modified `_base` POSIX describes.
    c_locale_handle()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn duplocale(locale: *mut c_void) -> *mut c_void {
    if !is_locale_handle(locale) {
        unsafe { set_errno(EINVAL) };
        return ptr::null_mut();
    }
    c_locale_handle()
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn freelocale(_locale: *mut c_void) {}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn uselocale(locale: *mut c_void) -> *mut c_void {
    let previous = NAGI_CURRENT_LOCALE.load(Ordering::Acquire);
    if !locale.is_null() {
        if !is_locale_handle(locale) {
            unsafe { set_errno(EINVAL) };
            return ptr::null_mut();
        }
        NAGI_CURRENT_LOCALE.store(locale as usize, Ordering::Release);
    }
    previous as *mut c_void
}

// --- C-locale character classes --------------------------------------------

fn byte_class(value: c_int, class: fn(u8) -> bool) -> c_int {
    c_int::from(u8::try_from(value).is_ok_and(class))
}

fn wide_class(value: WInt, class: fn(u8) -> bool) -> c_int {
    c_int::from(u8::try_from(value).is_ok_and(|byte| byte.is_ascii() && class(byte)))
}

fn is_c_blank(byte: u8) -> bool {
    byte == b' ' || byte == b'\t'
}

fn is_c_print(byte: u8) -> bool {
    (0x20..=0x7e).contains(&byte)
}

#[unsafe(no_mangle)]
pub extern "C" fn isalpha(value: c_int) -> c_int {
    byte_class(value, |byte| byte.is_ascii_alphabetic())
}

#[unsafe(no_mangle)]
pub extern "C" fn isupper(value: c_int) -> c_int {
    byte_class(value, |byte| byte.is_ascii_uppercase())
}

#[unsafe(no_mangle)]
pub extern "C" fn iscntrl(value: c_int) -> c_int {
    byte_class(value, |byte| byte.is_ascii_control())
}

#[unsafe(no_mangle)]
pub extern "C" fn ispunct(value: c_int) -> c_int {
    byte_class(value, |byte| byte.is_ascii_punctuation())
}

#[unsafe(no_mangle)]
pub extern "C" fn isgraph(value: c_int) -> c_int {
    byte_class(value, |byte| byte.is_ascii_graphic())
}

#[unsafe(no_mangle)]
pub extern "C" fn isblank(value: c_int) -> c_int {
    byte_class(value, is_c_blank)
}

/// Defines `name_l(value, locale)` as `name(value)`.
macro_rules! locale_variant {
    ($($name:ident => $base:path : $value:ty => $result:ty;)*) => {$(
        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn $name(value: $value, _locale: *mut c_void) -> $result {
            #[allow(unused_unsafe)]
            unsafe { $base(value) }
        }
    )*};
}

locale_variant! {
    isalnum_l => super::isalnum: c_int => c_int;
    isalpha_l => isalpha: c_int => c_int;
    isblank_l => isblank: c_int => c_int;
    iscntrl_l => iscntrl: c_int => c_int;
    isdigit_l => super::isdigit: c_int => c_int;
    isgraph_l => isgraph: c_int => c_int;
    islower_l => super::islower: c_int => c_int;
    isprint_l => super::isprint: c_int => c_int;
    ispunct_l => ispunct: c_int => c_int;
    isspace_l => super::isspace: c_int => c_int;
    isupper_l => isupper: c_int => c_int;
    isxdigit_l => super::isxdigit: c_int => c_int;
    tolower_l => super::tolower: c_int => c_int;
    toupper_l => super::toupper: c_int => c_int;
    iswalnum_l => iswalnum: WInt => c_int;
    iswalpha_l => iswalpha: WInt => c_int;
    iswblank_l => iswblank: WInt => c_int;
    iswcntrl_l => iswcntrl: WInt => c_int;
    iswdigit_l => iswdigit: WInt => c_int;
    iswgraph_l => iswgraph: WInt => c_int;
    iswlower_l => iswlower: WInt => c_int;
    iswprint_l => iswprint: WInt => c_int;
    iswpunct_l => iswpunct: WInt => c_int;
    iswspace_l => iswspace: WInt => c_int;
    iswupper_l => iswupper: WInt => c_int;
    iswxdigit_l => iswxdigit: WInt => c_int;
    towlower_l => towlower: WInt => WInt;
    towupper_l => towupper: WInt => WInt;
}

#[unsafe(no_mangle)]
pub extern "C" fn iswalnum(value: WInt) -> c_int {
    wide_class(value, |byte| byte.is_ascii_alphanumeric())
}

#[unsafe(no_mangle)]
pub extern "C" fn iswalpha(value: WInt) -> c_int {
    wide_class(value, |byte| byte.is_ascii_alphabetic())
}

#[unsafe(no_mangle)]
pub extern "C" fn iswblank(value: WInt) -> c_int {
    wide_class(value, is_c_blank)
}

#[unsafe(no_mangle)]
pub extern "C" fn iswcntrl(value: WInt) -> c_int {
    wide_class(value, |byte| byte.is_ascii_control())
}

#[unsafe(no_mangle)]
pub extern "C" fn iswdigit(value: WInt) -> c_int {
    wide_class(value, |byte| byte.is_ascii_digit())
}

#[unsafe(no_mangle)]
pub extern "C" fn iswgraph(value: WInt) -> c_int {
    wide_class(value, |byte| byte.is_ascii_graphic())
}

#[unsafe(no_mangle)]
pub extern "C" fn iswlower(value: WInt) -> c_int {
    wide_class(value, |byte| byte.is_ascii_lowercase())
}

#[unsafe(no_mangle)]
pub extern "C" fn iswprint(value: WInt) -> c_int {
    wide_class(value, is_c_print)
}

#[unsafe(no_mangle)]
pub extern "C" fn iswpunct(value: WInt) -> c_int {
    wide_class(value, |byte| byte.is_ascii_punctuation())
}

#[unsafe(no_mangle)]
pub extern "C" fn iswspace(value: WInt) -> c_int {
    wide_class(value, |byte| matches!(byte, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r'))
}

#[unsafe(no_mangle)]
pub extern "C" fn iswupper(value: WInt) -> c_int {
    wide_class(value, |byte| byte.is_ascii_uppercase())
}

#[unsafe(no_mangle)]
pub extern "C" fn iswxdigit(value: WInt) -> c_int {
    wide_class(value, |byte| byte.is_ascii_hexdigit())
}

#[unsafe(no_mangle)]
pub extern "C" fn towlower(value: WInt) -> WInt {
    match u8::try_from(value) {
        Ok(byte) if byte.is_ascii_uppercase() => WInt::from(byte.to_ascii_lowercase()),
        _ => value,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn towupper(value: WInt) -> WInt {
    match u8::try_from(value) {
        Ok(byte) if byte.is_ascii_lowercase() => WInt::from(byte.to_ascii_uppercase()),
        _ => value,
    }
}

// --- Wide strings ----------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcslen(string: *const WChar) -> usize {
    let mut length = 0;
    while unsafe { string.add(length).read() } != 0 {
        length += 1;
    }
    length
}

/// C-locale collation is code-point order.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcscoll(first: *const WChar, second: *const WChar) -> c_int {
    let mut index = 0;
    loop {
        let left = unsafe { first.add(index).read() };
        let right = unsafe { second.add(index).read() };
        if left != right {
            return if left < right { -1 } else { 1 };
        }
        if left == 0 {
            return 0;
        }
        index += 1;
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcscoll_l(
    first: *const WChar,
    second: *const WChar,
    _locale: *mut c_void,
) -> c_int {
    unsafe { wcscoll(first, second) }
}

/// The C-locale collation transform is the identity, so comparing outputs
/// with `wcscmp` matches `wcscoll` on the inputs.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcsxfrm(destination: *mut WChar, source: *const WChar, count: usize) -> usize {
    let length = unsafe { wcslen(source) };
    if length < count {
        unsafe { ptr::copy_nonoverlapping(source, destination, length + 1) };
    }
    length
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcsxfrm_l(
    destination: *mut WChar,
    source: *const WChar,
    count: usize,
    _locale: *mut c_void,
) -> usize {
    unsafe { wcsxfrm(destination, source, count) }
}

/// Parses the ASCII prefix of a wide string with the narrow C-locale float
/// parser. Every character a C-locale floating constant can contain is ASCII.
unsafe fn parse_wide_float(input: *const WChar, end: *mut *mut WChar) -> c_double {
    let mut buffer = [0_u8; 512];
    let mut length = 0;
    while length + 1 < buffer.len() {
        let value = unsafe { input.add(length).read() };
        match u8::try_from(value) {
            Ok(byte) if byte != 0 && byte.is_ascii() => buffer[length] = byte,
            _ => break,
        }
        length += 1;
    }
    let mut narrow_end: *mut c_char = ptr::null_mut();
    let value = unsafe { nagi_parse_float(buffer.as_ptr().cast(), &mut narrow_end) };
    if !end.is_null() {
        let consumed = if narrow_end.is_null() {
            0
        } else {
            unsafe { narrow_end.cast_const().offset_from(buffer.as_ptr().cast()) as usize }
        };
        unsafe { end.write(input.add(consumed).cast_mut()) };
    }
    value
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcstod(input: *const WChar, end: *mut *mut WChar) -> c_double {
    unsafe { parse_wide_float(input, end) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcstof(input: *const WChar, end: *mut *mut WChar) -> c_float {
    unsafe { parse_wide_float(input, end) as c_float }
}

// x86-64 SysV returns `long double` in x87 ST(0), which Rust cannot express.
// Widen the `double` result exactly, keeping each caller's argument registers
// untouched (the `_l` locale argument is ignored).
#[cfg(target_arch = "x86_64")]
core::arch::global_asm!(
    r#"
        .text
        .globl strtold
        .globl strtold_l
strtold:
strtold_l:
        sub rsp, 8
        call strtod
        movsd [rsp], xmm0
        fld qword ptr [rsp]
        add rsp, 8
        ret

        .globl wcstold
wcstold:
        sub rsp, 8
        call wcstod
        movsd [rsp], xmm0
        fld qword ptr [rsp]
        add rsp, 8
        ret
"#
);

// --- UTF-8 multibyte conversion -------------------------------------------
//
// `mbstate_t` is `{ unsigned __count; unsigned __value; }`. `__count` holds
// the remaining continuation bytes in bits 0..8 and the sequence length in
// bits 8..16; `__value` holds the code-point bits decoded so far. A zeroed
// state is the initial state.

#[repr(C)]
pub struct MbState {
    count: u32,
    value: u32,
}

static mut MBRTOWC_STATE: MbState = MbState { count: 0, value: 0 };
static mut MBRLEN_STATE: MbState = MbState { count: 0, value: 0 };
static mut MBSRTOWCS_STATE: MbState = MbState { count: 0, value: 0 };
static mut WCRTOMB_STATE: MbState = MbState { count: 0, value: 0 };
static mut WCSRTOMBS_STATE: MbState = MbState { count: 0, value: 0 };

fn state_or(state: *mut MbState, internal: *mut MbState) -> *mut MbState {
    if state.is_null() { internal } else { state }
}

fn valid_scalar(code_point: u32, length: u32) -> bool {
    let minimum = match length {
        2 => 0x80,
        3 => 0x800,
        _ => 0x1_0000,
    };
    code_point >= minimum && code_point <= 0x10_ffff && !(0xd800..=0xdfff).contains(&code_point)
}

/// Decodes at most `limit` bytes of `input` into `state`. Returns the bytes
/// consumed and the completed character, `CONVERSION_INCOMPLETE` when the
/// input ends inside a character, or `CONVERSION_ERROR` for invalid input
/// (the state is then unspecified, as POSIX allows).
unsafe fn decode(input: *const u8, limit: usize, state: &mut MbState) -> (usize, Option<u32>) {
    let mut consumed = 0;
    while consumed < limit {
        let byte = u32::from(unsafe { input.add(consumed).read() });
        consumed += 1;
        let remaining = state.count & 0xff;
        if remaining == 0 {
            let (length, bits) = match byte {
                0x00..=0x7f => return (consumed, Some(byte)),
                0xc2..=0xdf => (2, byte & 0x1f),
                0xe0..=0xef => (3, byte & 0x0f),
                0xf0..=0xf4 => (4, byte & 0x07),
                _ => return (CONVERSION_ERROR, None),
            };
            state.count = (length << 8) | (length - 1);
            state.value = bits;
            continue;
        }
        if byte & 0xc0 != 0x80 {
            return (CONVERSION_ERROR, None);
        }
        state.value = (state.value << 6) | (byte & 0x3f);
        state.count -= 1;
        if state.count & 0xff == 0 {
            let length = state.count >> 8;
            let code_point = state.value;
            *state = MbState { count: 0, value: 0 };
            if !valid_scalar(code_point, length) {
                return (CONVERSION_ERROR, None);
            }
            return (consumed, Some(code_point));
        }
    }
    (CONVERSION_INCOMPLETE, None)
}

fn encode(code_point: u32, output: &mut [u8; 4]) -> Option<usize> {
    let character = char::from_u32(code_point)?;
    Some(character.encode_utf8(output).len())
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mbrtowc(
    output: *mut WChar,
    input: *const c_char,
    count: usize,
    state: *mut MbState,
) -> usize {
    let state = unsafe { &mut *state_or(state, ptr::addr_of_mut!(MBRTOWC_STATE)) };
    if input.is_null() {
        let incomplete = state.count != 0;
        *state = MbState { count: 0, value: 0 };
        if incomplete {
            unsafe { set_errno(EILSEQ) };
            return CONVERSION_ERROR;
        }
        return 0;
    }
    if count == 0 {
        return CONVERSION_INCOMPLETE;
    }
    match unsafe { decode(input.cast(), count, state) } {
        (_, Some(code_point)) if code_point == 0 => {
            if !output.is_null() {
                unsafe { output.write(0) };
            }
            0
        }
        (consumed, Some(code_point)) => {
            if !output.is_null() {
                unsafe { output.write(code_point as WChar) };
            }
            consumed
        }
        (CONVERSION_INCOMPLETE, None) => CONVERSION_INCOMPLETE,
        _ => {
            *state = MbState { count: 0, value: 0 };
            unsafe { set_errno(EILSEQ) };
            CONVERSION_ERROR
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mbrlen(input: *const c_char, count: usize, state: *mut MbState) -> usize {
    let state = state_or(state, ptr::addr_of_mut!(MBRLEN_STATE));
    unsafe { mbrtowc(ptr::null_mut(), input, count, state) }
}

/// Nagi's UTF-8 encoding has no shift states, so `mbsinit` is true for every
/// state that is not inside a character.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn mbsinit(state: *const MbState) -> c_int {
    c_int::from(state.is_null() || unsafe { (*state).count } == 0)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mbtowc(output: *mut WChar, input: *const c_char, count: usize) -> c_int {
    if input.is_null() {
        return 0;
    }
    let mut state = MbState { count: 0, value: 0 };
    match unsafe { mbrtowc(output, input, count, &mut state) } {
        CONVERSION_ERROR => -1,
        CONVERSION_INCOMPLETE => {
            unsafe { set_errno(EILSEQ) };
            -1
        }
        length => length as c_int,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mblen(input: *const c_char, count: usize) -> c_int {
    unsafe { mbtowc(ptr::null_mut(), input, count) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcrtomb(output: *mut c_char, character: WChar, state: *mut MbState) -> usize {
    let state = unsafe { &mut *state_or(state, ptr::addr_of_mut!(WCRTOMB_STATE)) };
    *state = MbState { count: 0, value: 0 };
    if output.is_null() {
        return 1;
    }
    let mut encoded = [0_u8; 4];
    match encode(character as u32, &mut encoded) {
        Some(length) => {
            unsafe { ptr::copy_nonoverlapping(encoded.as_ptr(), output.cast(), length) };
            length
        }
        None => {
            unsafe { set_errno(EILSEQ) };
            CONVERSION_ERROR
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn wctomb(output: *mut c_char, character: WChar) -> c_int {
    if output.is_null() {
        return 0;
    }
    let mut state = MbState { count: 0, value: 0 };
    match unsafe { wcrtomb(output, character, &mut state) } {
        CONVERSION_ERROR => -1,
        length => length as c_int,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn btowc(byte: c_int) -> WInt {
    match u8::try_from(byte) {
        Ok(byte) if byte.is_ascii() => WInt::from(byte),
        _ => WEOF,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn wctob(character: WInt) -> c_int {
    if character < 0x80 { character as c_int } else { EOF }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mbsnrtowcs(
    output: *mut WChar,
    source: *mut *const c_char,
    byte_limit: usize,
    capacity: usize,
    state: *mut MbState,
) -> usize {
    let state = unsafe { &mut *state_or(state, ptr::addr_of_mut!(MBSRTOWCS_STATE)) };
    let mut input = unsafe { source.read() }.cast::<u8>();
    let mut remaining = byte_limit;
    let mut written = 0;
    while output.is_null() || written < capacity {
        if remaining == 0 {
            break;
        }
        let character_start = input;
        match unsafe { decode(input, remaining, state) } {
            (CONVERSION_INCOMPLETE, None) => {
                // The partial character stays in `state`.
                input = unsafe { input.add(remaining) };
                break;
            }
            (consumed, Some(code_point)) => {
                if code_point == 0 {
                    if !output.is_null() {
                        unsafe {
                            output.add(written).write(0);
                            source.write(ptr::null());
                        }
                    }
                    return written;
                }
                if !output.is_null() {
                    unsafe { output.add(written).write(code_point as WChar) };
                }
                written += 1;
                input = unsafe { input.add(consumed) };
                remaining -= consumed;
            }
            _ => {
                *state = MbState { count: 0, value: 0 };
                if !output.is_null() {
                    unsafe { source.write(character_start.cast()) };
                }
                unsafe { set_errno(EILSEQ) };
                return CONVERSION_ERROR;
            }
        }
    }
    if !output.is_null() {
        unsafe { source.write(input.cast()) };
    }
    written
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mbsrtowcs(
    output: *mut WChar,
    source: *mut *const c_char,
    capacity: usize,
    state: *mut MbState,
) -> usize {
    unsafe { mbsnrtowcs(output, source, usize::MAX, capacity, state) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcsnrtombs(
    output: *mut c_char,
    source: *mut *const WChar,
    character_limit: usize,
    capacity: usize,
    state: *mut MbState,
) -> usize {
    let state = unsafe { &mut *state_or(state, ptr::addr_of_mut!(WCSRTOMBS_STATE)) };
    *state = MbState { count: 0, value: 0 };
    let mut input = unsafe { source.read() };
    let mut written = 0;
    for _ in 0..character_limit {
        let character = unsafe { input.read() };
        let mut encoded = [0_u8; 4];
        let Some(length) = encode(character as u32, &mut encoded) else {
            if !output.is_null() {
                unsafe { source.write(input) };
            }
            unsafe { set_errno(EILSEQ) };
            return CONVERSION_ERROR;
        };
        if !output.is_null() {
            if written + length > capacity {
                break;
            }
            unsafe { ptr::copy_nonoverlapping(encoded.as_ptr(), output.add(written).cast(), length) };
        }
        if character == 0 {
            if !output.is_null() {
                unsafe { source.write(ptr::null()) };
            }
            return written;
        }
        written += length;
        input = unsafe { input.add(1) };
    }
    if !output.is_null() {
        unsafe { source.write(input) };
    }
    written
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn wcsrtombs(
    output: *mut c_char,
    source: *mut *const WChar,
    capacity: usize,
    state: *mut MbState,
) -> usize {
    unsafe { wcsnrtombs(output, source, usize::MAX, capacity, state) }
}

// --- Formatting and scanning -----------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn vsscanf(input: *const c_char, format: *const c_char, args: VaList) -> c_int {
    unsafe { nagi_vsscanf(input, format, args) }
}

const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];
const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

/// Bounded `strftime` output buffer; overflow makes the call return 0.
struct TimeWriter {
    output: *mut u8,
    capacity: usize,
    length: usize,
    overflow: bool,
}

impl TimeWriter {
    fn byte(&mut self, byte: u8) {
        if self.length + 1 >= self.capacity {
            self.overflow = true;
            return;
        }
        unsafe { self.output.add(self.length).write(byte) };
        self.length += 1;
    }

    fn text(&mut self, text: &str) {
        for byte in text.bytes() {
            self.byte(byte);
        }
    }

    fn number(&mut self, value: c_long, width: usize, pad: u8) {
        let negative = value < 0;
        let mut magnitude = value.unsigned_abs();
        let mut digits = [0_u8; 20];
        let mut count = 0;
        loop {
            digits[count] = b'0' + (magnitude % 10) as u8;
            magnitude /= 10;
            count += 1;
            if magnitude == 0 {
                break;
            }
        }
        if negative {
            self.byte(b'-');
        }
        for _ in count + usize::from(negative)..width {
            self.byte(pad);
        }
        for index in (0..count).rev() {
            self.byte(digits[index]);
        }
    }
}

fn name(names: &[&'static str], index: c_int) -> &'static str {
    usize::try_from(index)
        .ok()
        .and_then(|index| names.get(index))
        .copied()
        .unwrap_or("?")
}

fn abbreviation(name: &'static str) -> &'static str {
    name.get(..3).unwrap_or(name)
}

/// ISO 8601 weeks in `year`: 53 when it starts on a Thursday, or is a leap
/// year starting on a Wednesday.
fn iso_weeks_in(year: c_long) -> c_long {
    let p = |year: c_long| (year + year.div_euclid(4) - year.div_euclid(100) + year.div_euclid(400)).rem_euclid(7);
    if p(year) == 4 || p(year - 1) == 3 { 53 } else { 52 }
}

/// ISO 8601 week-based year and week number (`%G` and `%V`).
fn iso_week(time: &NagiTm) -> (c_long, c_long) {
    let year = c_long::from(time.tm_year) + 1900;
    let monday_weekday = c_long::from((time.tm_wday + 6) % 7);
    let week = (c_long::from(time.tm_yday) - monday_weekday + 10) / 7;
    if week < 1 {
        (year - 1, iso_weeks_in(year - 1))
    } else if week > iso_weeks_in(year) {
        (year + 1, 1)
    } else {
        (year, week)
    }
}

unsafe fn format_time(writer: &mut TimeWriter, format: *const u8, time: &NagiTm) {
    let mut index = 0;
    loop {
        let byte = unsafe { format.add(index).read() };
        index += 1;
        if byte == 0 {
            return;
        }
        if byte != b'%' {
            writer.byte(byte);
            continue;
        }
        let mut conversion = unsafe { format.add(index).read() };
        index += 1;
        // POSIX `E` and `O` modifiers select alternative representations;
        // the C locale has none, so they are accepted and ignored.
        if conversion == b'E' || conversion == b'O' {
            conversion = unsafe { format.add(index).read() };
            index += 1;
        }
        let year = c_long::from(time.tm_year) + 1900;
        let hour12 = match time.tm_hour % 12 {
            0 => 12,
            hour => hour,
        };
        match conversion {
            b'a' => writer.text(abbreviation(name(&WEEKDAYS, time.tm_wday))),
            b'A' => writer.text(name(&WEEKDAYS, time.tm_wday)),
            b'b' | b'h' => writer.text(abbreviation(name(&MONTHS, time.tm_mon))),
            b'B' => writer.text(name(&MONTHS, time.tm_mon)),
            b'c' => unsafe { format_time(writer, b"%a %b %e %H:%M:%S %Y\0".as_ptr(), time) },
            b'C' => writer.number(year.div_euclid(100), 2, b'0'),
            b'd' => writer.number(time.tm_mday.into(), 2, b'0'),
            b'D' | b'x' => unsafe { format_time(writer, b"%m/%d/%y\0".as_ptr(), time) },
            b'e' => writer.number(time.tm_mday.into(), 2, b' '),
            b'F' => unsafe { format_time(writer, b"%Y-%m-%d\0".as_ptr(), time) },
            b'g' => writer.number(iso_week(time).0.rem_euclid(100), 2, b'0'),
            b'G' => writer.number(iso_week(time).0, 4, b'0'),
            b'H' => writer.number(time.tm_hour.into(), 2, b'0'),
            b'I' => writer.number(hour12.into(), 2, b'0'),
            b'j' => writer.number(c_long::from(time.tm_yday) + 1, 3, b'0'),
            b'm' => writer.number(c_long::from(time.tm_mon) + 1, 2, b'0'),
            b'M' => writer.number(time.tm_min.into(), 2, b'0'),
            b'n' => writer.byte(b'\n'),
            b'p' => writer.text(if time.tm_hour < 12 { "AM" } else { "PM" }),
            b'r' => unsafe { format_time(writer, b"%I:%M:%S %p\0".as_ptr(), time) },
            b'R' => unsafe { format_time(writer, b"%H:%M\0".as_ptr(), time) },
            b'S' => writer.number(time.tm_sec.into(), 2, b'0'),
            b't' => writer.byte(b'\t'),
            b'T' | b'X' => unsafe { format_time(writer, b"%H:%M:%S\0".as_ptr(), time) },
            b'u' => writer.number(if time.tm_wday == 0 { 7 } else { time.tm_wday.into() }, 1, b'0'),
            b'U' => writer.number(c_long::from((time.tm_yday + 7 - time.tm_wday) / 7), 2, b'0'),
            b'V' => writer.number(iso_week(time).1, 2, b'0'),
            b'w' => writer.number(time.tm_wday.into(), 1, b'0'),
            b'W' => writer.number(
                c_long::from((time.tm_yday + 7 - (time.tm_wday + 6) % 7) / 7),
                2,
                b'0',
            ),
            b'y' => writer.number(year.rem_euclid(100), 2, b'0'),
            b'Y' => writer.number(year, 1, b'0'),
            b'z' => {
                let offset = time.tm_gmtoff / 60;
                writer.byte(if offset < 0 { b'-' } else { b'+' });
                writer.number(offset.abs() / 60 * 100 + offset.abs() % 60, 4, b'0');
            }
            // Nagi 0.1 exposes UTC only.
            b'Z' => writer.text("UTC"),
            b'%' => writer.byte(b'%'),
            0 => return,
            other => {
                writer.byte(b'%');
                writer.byte(other);
            }
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strftime(
    output: *mut c_char,
    capacity: usize,
    format: *const c_char,
    time: *const NagiTm,
) -> usize {
    if output.is_null() || format.is_null() || time.is_null() || capacity == 0 {
        return 0;
    }
    let mut writer = TimeWriter {
        output: output.cast(),
        capacity,
        length: 0,
        overflow: false,
    };
    unsafe { format_time(&mut writer, format.cast(), &*time) };
    if writer.overflow {
        return 0;
    }
    unsafe { output.add(writer.length).write(0) };
    writer.length
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn strftime_l(
    output: *mut c_char,
    capacity: usize,
    format: *const c_char,
    time: *const NagiTm,
    _locale: *mut c_void,
) -> usize {
    unsafe { strftime(output, capacity, format, time) }
}

// --- Path configuration ----------------------------------------------------

const PC_LINK_MAX: c_int = 0;
const PC_NAME_MAX: c_int = 3;
const PC_PATH_MAX: c_int = 4;
const PC_NO_TRUNC: c_int = 7;

fn path_limit(name: c_int) -> c_long {
    match name {
        PC_LINK_MAX => 1,
        PC_NAME_MAX => 255,
        PC_PATH_MAX => 4096,
        PC_NO_TRUNC => 1,
        _ => {
            unsafe { set_errno(EINVAL) };
            -1
        }
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn pathconf(path: *const c_char, name: c_int) -> c_long {
    if path.is_null() {
        unsafe { set_errno(EINVAL) };
        return -1;
    }
    path_limit(name)
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn fpathconf(_fd: c_int, name: c_int) -> c_long {
    path_limit(name)
}
