//! Read-only Windows open without std's infallible UTF-16 Vec conversion.
use super::windows_path::{prepare_path, MAX_PATH_UNITS};
use core::{ffi::c_void, ptr};
use std::{
    fs::File,
    io,
    os::windows::{ffi::OsStrExt, io::FromRawHandle},
    path::Path,
};

#[link(name = "kernel32")]
extern "system" {
    fn GetFullPathNameW(
        name: *const u16,
        size: u32,
        buffer: *mut u16,
        file_part: *mut *mut u16,
    ) -> u32;
    fn CreateFileW(
        name: *const u16,
        access: u32,
        sharing: u32,
        security: *const c_void,
        creation: u32,
        attributes: u32,
        template: *mut c_void,
    ) -> *mut c_void;
}

pub(super) fn open_snapshot(path: &Path) -> io::Result<File> {
    // Host only: two bounded 64 KiB scratch arrays, no global allocator calls.
    // Keep the input alive and terminated throughout GetFullPathNameW.
    let mut wide = [0; MAX_PATH_UNITS];
    let mut absolute = [0; MAX_PATH_UNITS];
    let length = prepare_path(
        path.as_os_str().encode_wide(),
        &mut wide,
        &mut absolute,
        |input, output| {
            // SAFETY: input is NUL-terminated; output is writable for the supplied
            // count, and no interior pointers or buffers escape this call.
            let count = unsafe {
                GetFullPathNameW(
                    input.as_ptr(),
                    output.len() as u32,
                    output.as_mut_ptr(),
                    ptr::null_mut(),
                )
            };
            if count == 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(count as usize)
            }
        },
    )?;
    // Match std File::open's read access, sharing, OPEN_EXISTING and flags.
    // SAFETY: wide[..length] is initialized and NUL-terminated; null security
    // attributes make the handle non-inheritable. No creation/write access.
    let handle = unsafe {
        CreateFileW(
            wide[..length].as_ptr(),
            0x8000_0000,
            0x7,
            ptr::null(),
            3,
            0,
            ptr::null_mut(),
        )
    };
    if handle == (-1_isize) as *mut c_void {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: CreateFileW returned a valid, exclusively owned handle. File now
    // closes it on every subsequent success/error path.
    Ok(unsafe { File::from_raw_handle(handle) })
}
