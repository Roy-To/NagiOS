//! Minimal PID 1 for required CI-only Linux lifecycle tests. It is not a
//! product init, and must only run in a disposable, diskless guest.

use std::ffi::{c_char, c_int, c_ulong, c_void, CStr};
use std::fs;
use std::io::{self, Write};
use std::process::Command;

unsafe extern "C" {
    fn mount(
        source: *const c_char,
        target: *const c_char,
        filesystem: *const c_char,
        flags: c_ulong,
        data: *const c_void,
    ) -> c_int;
    fn reboot(command: c_int) -> c_int;
}

fn mount_guest_filesystem(name: &CStr, target: &CStr) -> io::Result<()> {
    // This disposable guest has no attached disks or network adapters.
    const MS_NOSUID_NODEV_NOEXEC: c_ulong = 2 | 4 | 8;
    if unsafe {
        mount(
            name.as_ptr(),
            target.as_ptr(),
            name.as_ptr(),
            if name == c"devtmpfs" {
                // Device nodes, including /dev/null, are test prerequisites.
                2 | 8
            } else {
                MS_NOSUID_NODEV_NOEXEC
            },
            std::ptr::null(),
        )
    } != 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn run() -> io::Result<()> {
    if std::process::id() != 1 {
        return Err(io::Error::other("test supervisor must be guest PID 1"));
    }
    mount_guest_filesystem(c"proc", c"/proc")?;
    mount_guest_filesystem(c"sysfs", c"/sys")?;
    mount_guest_filesystem(c"devtmpfs", c"/dev")?;
    let manifest = fs::read_to_string("/tests.txt")?;
    let tests: Vec<_> = manifest.lines().collect();
    if tests.is_empty() {
        return Err(io::Error::other("missing test names"));
    }
    for test in &tests {
        println!("LIVE_PID1_CI BEGIN {test}");
        io::stdout().flush()?;
        let status = Command::new("/testbin")
            .args(["--exact", "--test-threads=1", test])
            .status()?;
        if !status.success() {
            return Err(io::Error::other(format!("{test}: {status}")));
        }
        println!("LIVE_PID1_CI PASS {test}");
    }
    println!("LIVE_PID1_CI COMPLETE {}", tests.len());
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("LIVE_PID1_CI FAIL {error}");
    }
    let _ = io::stdout().flush();
    let _ = io::stderr().flush();
    // Linux RB_POWER_OFF. The host also enforces a bounded timeout.
    if unsafe { reboot(0x4321fedc) } != 0 {
        eprintln!("LIVE_PID1_CI FAIL poweroff: {}", io::Error::last_os_error());
    }
    loop {
        std::thread::park();
    }
}
