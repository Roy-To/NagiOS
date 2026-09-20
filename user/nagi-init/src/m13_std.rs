use std::cell::Cell;
use std::fs;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const PASS: &[u8] = b"Nagi M13 Rust std PASS\r\n";
const FAIL_PROBE: &[u8] = b"Nagi M13 Rust std relibc FAIL\r\n";
const FAIL_ALLOC: &[u8] = b"Nagi M13 Rust std allocator FAIL\r\n";
const FAIL_FS_INIT: &[u8] = b"Nagi M13 Rust std VFS init FAIL\r\n";
const FAIL_CLOCK: &[u8] = b"Nagi M13 Rust std clock FAIL\r\n";
const FAIL_SYNC: &[u8] = b"Nagi M13 Rust std sync FAIL\r\n";
const FAIL_FS_IO: &[u8] = b"Nagi M13 Rust std VFS IO FAIL\r\n";
const FAIL_THREAD: &[u8] = b"Nagi M13 Rust std thread/TLS FAIL\r\n";
const FAIL_NETWORK: &[u8] = b"Nagi M13 Rust std network FAIL\r\n";
const PROBE_PASS: &[u8] = b"Nagi M13 Rust std relibc PASS\r\n";
const ALLOC_PASS: &[u8] = b"Nagi M13 Rust std allocator PASS\r\n";
const CLOCK_PASS: &[u8] = b"Nagi M13 Rust std clock PASS\r\n";
const SYNC_PASS: &[u8] = b"Nagi M13 Rust std sync PASS\r\n";
const FS_PASS: &[u8] = b"Nagi M13 Rust std VFS PASS\r\n";
const THREAD_PASS: &[u8] = b"Nagi M13 Rust std thread/TLS PASS\r\n";
const NETWORK_PASS: &[u8] = b"Nagi M13 Rust std network PASS\r\n";
const HTTP_RESPONSE_CAPACITY: usize = 1536;

static mut HTTP_RESPONSE: [u8; HTTP_RESPONSE_CAPACITY] = [0; HTTP_RESPONSE_CAPACITY];

thread_local! {
    static M13_THREAD_VALUE: Cell<u64> = const { Cell::new(0) };
}

pub fn run(block_capability: u64, net_capability: u64) -> ! {
    if relibc::nagi_backend_probe() != 0x4e41_4749 {
        libnagi::console_write(FAIL_PROBE);
        libnagi::exit(1);
    }
    libnagi::console_write(PROBE_PASS);
    // Keep the Nagi POSIX runtime in the final link; the Rust `std` PAL calls
    // the same exported user-space ABI symbols below.
    let probe = nagi_posix::nagi_posix_malloc(8);
    if probe.is_null() {
        libnagi::console_write(FAIL_ALLOC);
        libnagi::exit(1);
    }
    nagi_posix::nagi_posix_free(probe);
    libnagi::console_write(ALLOC_PASS);
    if unsafe { nagi_posix::nagi_posix_initialize_network(net_capability) } != 0 {
        libnagi::console_write(FAIL_NETWORK);
        libnagi::exit(1);
    }
    let Some(gateway) = nagi_posix::nagi_posix_network_default_gateway() else {
        libnagi::console_write(FAIL_NETWORK);
        libnagi::exit(1);
    };
    let response = unsafe { &mut *core::ptr::addr_of_mut!(HTTP_RESPONSE) };
    if nagi_posix::nagi_posix_network_http_get(
        gateway,
        18_080,
        b"/nagi-m12.txt",
        b"NAGI_M12_HTTP_FIXTURE_PASS",
        response,
    )
    .is_none()
    {
        libnagi::console_write(FAIL_NETWORK);
        libnagi::exit(1);
    }
    libnagi::console_write(NETWORK_PASS);

    if unsafe { nagi_posix::nagi_posix_initialize_filesystem(block_capability) } != 0 {
        libnagi::console_write(FAIL_FS_INIT);
        libnagi::exit(1);
    }

    let started = Instant::now();
    thread::sleep(Duration::from_millis(20));
    if started.elapsed() < Duration::from_millis(10) {
        libnagi::console_write(FAIL_CLOCK);
        libnagi::exit(1);
    }
    libnagi::console_write(CLOCK_PASS);

    M13_THREAD_VALUE.with(|value| value.set(7));
    let child = thread::spawn(|| {
        let initial = M13_THREAD_VALUE.with(Cell::get);
        M13_THREAD_VALUE.with(|value| value.set(42));
        (initial, M13_THREAD_VALUE.with(Cell::get))
    });
    let thread_passed = child
        .join()
        .map(|(initial, child_value)| initial == 0 && child_value == 42)
        .unwrap_or(false)
        && M13_THREAD_VALUE.with(Cell::get) == 7;
    if !thread_passed {
        libnagi::console_write(FAIL_THREAD);
        libnagi::exit(1);
    }
    libnagi::console_write(THREAD_PASS);

    let values = Arc::new(Mutex::new(Vec::from([1_u64, 2, 3])));
    let cloned = Arc::clone(&values);
    {
        let mut guard = cloned.lock().expect("std mutex");
        guard.push(Duration::from_millis(1).as_millis() as u64);
    }

    let sync_time_passed = values
        .lock()
        .map(|guard| guard.as_slice() == [1, 2, 3, 1])
        .unwrap_or(false);
    if !sync_time_passed {
        libnagi::console_write(FAIL_SYNC);
        libnagi::exit(1);
    }
    libnagi::console_write(SYNC_PASS);
    let fs_passed =
        match fs::write("/m13-std.txt", b"Nagi std VFS").and_then(|()| fs::read("/m13-std.txt")) {
            Ok(bytes) => bytes == b"Nagi std VFS",
            Err(_error) => false,
        };
    if !fs_passed {
        libnagi::console_write(FAIL_FS_IO);
        libnagi::exit(1);
    }
    libnagi::console_write(FS_PASS);
    libnagi::console_write(PASS);
    libnagi::exit(0)
}
