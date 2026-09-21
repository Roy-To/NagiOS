use core::{arch::asm, cell::UnsafeCell};

pub const EINVAL: i32 = 22;
pub const EBADF: i32 = 9;
pub const ENOSYS: i32 = 38;
pub const ENOMEM: i32 = 12;
pub const EAGAIN: i32 = 11;
pub const EACCES: i32 = 13;
pub const ENOPROTOOPT: i32 = 92;

#[allow(dead_code)]
struct ErrnoCell(UnsafeCell<i32>);

unsafe impl Sync for ErrnoCell {}

#[link_section = ".data"]
#[no_mangle]
static ERRNO: ErrnoCell = ErrnoCell(UnsafeCell::new(0));

fn errno_pointer() -> *mut i32 {
    let pointer: *mut i32;
    unsafe {
        asm!(
            "lea {pointer}, [rip + {symbol}]",
            pointer = out(reg) pointer,
            symbol = sym ERRNO,
            options(nostack, preserves_flags, readonly),
        );
    }
    pointer
}

pub fn set_errno(value: i32) {
    unsafe { *errno_pointer() = value };
}

pub fn errno() -> i32 {
    unsafe { *errno_pointer() }
}

#[no_mangle]
pub extern "C" fn nagi_posix_errno_location() -> *mut i32 {
    errno_pointer()
}

#[cfg(test)]
mod tests {
    use super::{errno, set_errno, EINVAL};

    #[test]
    fn errno_is_deterministic() {
        set_errno(EINVAL);
        assert_eq!(errno(), EINVAL);
    }
}
