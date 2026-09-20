#![no_std]

pub mod alloc;
pub mod fd;
pub mod io;
pub mod sync;
pub mod time;

pub use self::alloc::{AllocError, BumpAllocator};
pub use fd::{Fd, FdError, FdKind, FdTable};
pub use io::{FileSystem, Network};
pub use sync::{SpinMutex, TlsSlots};
pub use time::{Clock, TimeError};

/// Format an integer without requiring the Rust standard library.
pub fn format_u64(value: u64, output: &mut [u8; 20]) -> usize {
    let mut buffer = itoa::Buffer::new();
    let text = buffer.format(value).as_bytes();
    if text.len() > output.len() {
        return 0;
    }
    output[..text.len()].copy_from_slice(text);
    text.len()
}

#[cfg(test)]
mod tests {
    use super::format_u64;

    #[test]
    fn formats_numbers_without_std_runtime_support() {
        let mut output = [0; 20];
        let length = format_u64(4_294_967_295, &mut output);
        assert_eq!(&output[..length], b"4294967295");
    }
}
