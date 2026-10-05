//! Read-only files whose bytes are part of the loaded guest image.
//!
//! System assets such as bundled fonts are compiled into the init image and
//! published here under fixed absolute paths (for example
//! `/system/fonts/NotoSans-Regular.ttf`). They are served through the
//! existing read-only callback descriptor, so `open`, `read`, `fstat`, `stat`
//! and file-backed `mmap` work without placing them on the writable User Data
//! VFS. Nothing here reads a host file.

use core::ffi::c_void;

/// Upper bound on published static files.
pub const MAX_STATIC_FILES: usize = 8;
/// Only paths below this prefix may be published.
pub const STATIC_FILE_PREFIX: &[u8] = b"/system/";

#[derive(Clone, Copy, Debug)]
pub struct StaticFile {
    pub path: &'static [u8],
    pub data: &'static [u8],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StaticFileError {
    InvalidPath,
    Duplicate,
    TableFull,
    TooLarge,
}

/// Bounded registry of static files.
#[derive(Debug)]
pub struct StaticFileTable {
    files: [Option<StaticFile>; MAX_STATIC_FILES],
}

impl StaticFileTable {
    pub const fn new() -> Self {
        Self {
            files: [None; MAX_STATIC_FILES],
        }
    }

    pub fn register(
        &mut self,
        path: &'static [u8],
        data: &'static [u8],
    ) -> Result<(), StaticFileError> {
        let valid_path = path.len() > STATIC_FILE_PREFIX.len()
            && path.starts_with(STATIC_FILE_PREFIX)
            && !path.ends_with(b"/")
            && !path.windows(2).any(|pair| pair == b"//")
            && !path.split(|byte| *byte == b'/').any(|part| part == b"..")
            && !path.contains(&0);
        if !valid_path {
            return Err(StaticFileError::InvalidPath);
        }
        // File sizes are reported through 32-bit stat metadata.
        if u32::try_from(data.len()).is_err() {
            return Err(StaticFileError::TooLarge);
        }
        if self.lookup(path).is_some() {
            return Err(StaticFileError::Duplicate);
        }
        let slot = self
            .files
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(StaticFileError::TableFull)?;
        *slot = Some(StaticFile { path, data });
        Ok(())
    }

    pub fn lookup(&self, path: &[u8]) -> Option<StaticFile> {
        self.files
            .iter()
            .flatten()
            .find(|file| file.path == path)
            .copied()
    }
}

impl Default for StaticFileTable {
    fn default() -> Self {
        Self::new()
    }
}

/// Read-at callback for a static file. `context` is the start of the file's
/// `'static` bytes; the callback-file wrapper bounds `offset + length` by the
/// file length before calling it.
///
/// # Safety
/// `context` must point to at least `offset + length` readable bytes and
/// `output` must be valid for `length` writes.
pub unsafe extern "C" fn read_static(
    context: *mut c_void,
    offset: u64,
    output: *mut u8,
    length: usize,
) -> isize {
    let Ok(offset) = usize::try_from(offset) else {
        return -1;
    };
    unsafe {
        core::ptr::copy_nonoverlapping((context as *const u8).add(offset), output, length);
    }
    length as isize
}

#[cfg(test)]
mod tests {
    use super::*;

    static FONT: [u8; 6] = *b"OTTO\0\x01";

    #[test]
    fn registers_and_finds_system_files() {
        let mut table = StaticFileTable::new();
        table
            .register(b"/system/fonts/NotoSans-Regular.ttf", &FONT)
            .unwrap();
        let file = table.lookup(b"/system/fonts/NotoSans-Regular.ttf").unwrap();
        assert_eq!(file.data, &FONT);
        assert!(table.lookup(b"/system/fonts/missing.ttf").is_none());
        assert_eq!(
            table.register(b"/system/fonts/NotoSans-Regular.ttf", &FONT),
            Err(StaticFileError::Duplicate)
        );
    }

    #[test]
    fn rejects_paths_outside_the_system_tree() {
        let mut table = StaticFileTable::new();
        for path in [
            b"/tmp/font.ttf".as_slice(),
            b"/system/",
            b"/system/../tmp/x",
            b"/system//fonts/x",
            b"system/fonts/x",
            b"/system/fonts/",
        ] {
            assert_eq!(
                table.register(path, &FONT),
                Err(StaticFileError::InvalidPath),
                "{path:?}"
            );
        }
    }

    #[test]
    fn table_is_bounded() {
        static PATHS: [&[u8]; MAX_STATIC_FILES + 1] = [
            b"/system/a",
            b"/system/b",
            b"/system/c",
            b"/system/d",
            b"/system/e",
            b"/system/f",
            b"/system/g",
            b"/system/h",
            b"/system/i",
        ];
        let mut table = StaticFileTable::new();
        for path in &PATHS[..MAX_STATIC_FILES] {
            table.register(path, &FONT).unwrap();
        }
        assert_eq!(
            table.register(PATHS[MAX_STATIC_FILES], &FONT),
            Err(StaticFileError::TableFull)
        );
    }

    #[test]
    fn read_callback_copies_the_requested_range() {
        let mut output = [0_u8; 3];
        let count = unsafe { read_static(FONT.as_ptr() as *mut c_void, 2, output.as_mut_ptr(), 3) };
        assert_eq!(count, 3);
        assert_eq!(&output, b"TO\0");
    }
}
