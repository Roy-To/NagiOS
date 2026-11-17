//! Host-only read open without std's long-path CString allocation.
use std::{
    fs::File,
    io,
    os::{fd::FromRawFd, unix::ffi::OsStrExt},
    path::Path,
};

pub(super) fn open_snapshot(path: &Path) -> io::Result<File> {
    // Above the supported Linux/macOS pathname limit. Keep an explicit NUL,
    // including for paths longer than std's short stack-buffer optimization.
    let bytes = path.as_os_str().as_bytes();
    let mut terminated = [0; 32_768];
    if bytes.len() >= terminated.len() || bytes.contains(&0) {
        return Err(io::ErrorKind::InvalidInput.into());
    }
    terminated[..bytes.len()].copy_from_slice(bytes);
    let flags = libc::O_RDONLY | libc::O_CLOEXEC;
    #[cfg(target_os = "linux")]
    let flags = flags | libc::O_LARGEFILE;
    loop {
        // SAFETY: terminated contains bytes.len() path bytes and a final NUL;
        // no creation flag is set, so open has no variadic mode argument.
        let fd = unsafe { libc::open(terminated.as_ptr().cast(), flags) };
        if fd >= 0 {
            // SAFETY: open returned a fresh owned fd. File closes it on all
            // read success/error paths; CLOEXEC preserves std's inheritance rule.
            return Ok(unsafe { File::from_raw_fd(fd) });
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}
