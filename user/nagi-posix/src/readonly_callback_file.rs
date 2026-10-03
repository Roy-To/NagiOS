use core::ffi::c_void;

pub type ReadAtCallback = unsafe extern "C" fn(
    context: *mut c_void,
    offset: u64,
    output: *mut u8,
    length: usize,
) -> isize;

#[derive(Clone, Copy)]
pub(crate) struct ReadOnlyCallbackFile {
    context: usize,
    length: usize,
    offset: usize,
    read_at: ReadAtCallback,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CallbackFileError {
    InvalidRange,
    ReadFailed,
}

impl ReadOnlyCallbackFile {
    /// Create a descriptor-backed file whose bytes are provided by a caller.
    ///
    /// # Safety
    /// `context` and `read_at` must remain valid until the descriptor is closed.
    /// The caller must serialize access to the context for the descriptor's
    /// lifetime. The callback must write no more than `length` bytes to `output`
    /// and return the number of bytes written or a negative value on failure.
    pub(crate) unsafe fn new(
        context: *mut c_void,
        length: u64,
        read_at: ReadAtCallback,
    ) -> Result<Self, CallbackFileError> {
        let length = usize::try_from(length).map_err(|_| CallbackFileError::InvalidRange)?;
        if length != 0 && context.is_null() {
            return Err(CallbackFileError::InvalidRange);
        }
        Ok(Self {
            context: context as usize,
            length,
            offset: 0,
            read_at,
        })
    }

    pub(crate) const fn len(self) -> usize {
        self.length
    }

    pub(crate) const fn offset(self) -> usize {
        self.offset
    }

    pub(crate) fn read_at(
        self,
        offset: usize,
        output: &mut [u8],
    ) -> Result<usize, CallbackFileError> {
        if offset >= self.length || output.is_empty() {
            return Ok(0);
        }
        let count = output.len().min(self.length - offset);
        let result = unsafe {
            (self.read_at)(
                self.context as *mut c_void,
                offset as u64,
                output.as_mut_ptr(),
                count,
            )
        };
        if result < 0 || result as usize > count {
            return Err(CallbackFileError::ReadFailed);
        }
        Ok(result as usize)
    }

    pub(crate) fn read(&mut self, output: &mut [u8]) -> Result<usize, CallbackFileError> {
        let count = self.read_at(self.offset, output)?;
        self.offset = self
            .offset
            .checked_add(count)
            .ok_or(CallbackFileError::InvalidRange)?;
        Ok(count)
    }

    pub(crate) fn seek(&mut self, offset: i64, whence: i32) -> Result<usize, CallbackFileError> {
        let base = match whence {
            0 => 0i128,
            1 => self.offset as i128,
            2 => self.length as i128,
            _ => return Err(CallbackFileError::InvalidRange),
        };
        let next = base + i128::from(offset);
        if next < 0 || next > usize::MAX as i128 {
            return Err(CallbackFileError::InvalidRange);
        }
        self.offset = next as usize;
        Ok(self.offset)
    }
}

#[cfg(test)]
mod tests {
    use super::{CallbackFileError, ReadAtCallback, ReadOnlyCallbackFile};
    use core::ffi::c_void;

    unsafe extern "C" fn read_memory(
        context: *mut c_void,
        offset: u64,
        output: *mut u8,
        length: usize,
    ) -> isize {
        if context.is_null() || (output.is_null() && length != 0) {
            return -1;
        }
        let bytes = unsafe { &*context.cast::<[u8; 6]>() };
        let start = usize::try_from(offset).unwrap_or(usize::MAX);
        if start >= bytes.len() {
            return 0;
        }
        let count = length.min(bytes.len() - start);
        unsafe { core::ptr::copy_nonoverlapping(bytes.as_ptr().add(start), output, count) };
        count as isize
    }

    fn file(bytes: &mut [u8; 6]) -> ReadOnlyCallbackFile {
        let callback: ReadAtCallback = read_memory;
        unsafe {
            ReadOnlyCallbackFile::new(bytes.as_mut_ptr().cast(), bytes.len() as u64, callback)
                .unwrap()
        }
    }

    #[test]
    fn callback_file_reads_are_bounded_and_eof_is_stable() {
        let mut source = *b"NAGIOS";
        let mut file = file(&mut source);
        assert_eq!(file.len(), 6);
        let mut output = [0; 8];
        assert_eq!(file.read(&mut output), Ok(6));
        assert_eq!(&output[..6], b"NAGIOS");
        assert_eq!(file.offset(), 6);
        assert_eq!(file.read(&mut output), Ok(0));
        assert_eq!(file.offset(), 6);

        assert_eq!(file.read_at(4, &mut output), Ok(2));
        assert_eq!(&output[..2], b"OS");
        assert_eq!(file.read_at(usize::MAX, &mut output), Ok(0));
    }

    #[test]
    fn callback_file_supports_checked_seek_from_each_origin() {
        let mut source = *b"NAGIOS";
        let mut file = file(&mut source);
        assert_eq!(file.seek(2, 0), Ok(2));
        assert_eq!(file.seek(1, 1), Ok(3));
        assert_eq!(file.seek(-2, 2), Ok(4));
        assert_eq!(file.seek(-5, 0), Err(CallbackFileError::InvalidRange));
        assert_eq!(file.seek(0, 3), Err(CallbackFileError::InvalidRange));
        assert_eq!(file.offset(), 4);
    }

    unsafe extern "C" fn invalid_read(
        _context: *mut c_void,
        _offset: u64,
        _output: *mut u8,
        length: usize,
    ) -> isize {
        length.saturating_add(1) as isize
    }

    #[test]
    fn callback_file_rejects_a_callback_that_overreports_bytes() {
        let callback: ReadAtCallback = invalid_read;
        let mut file =
            unsafe { ReadOnlyCallbackFile::new(core::ptr::dangling_mut::<c_void>(), 4, callback) }
                .unwrap();
        assert_eq!(file.read(&mut [0; 4]), Err(CallbackFileError::ReadFailed));
        assert_eq!(file.offset(), 0);
    }
}
