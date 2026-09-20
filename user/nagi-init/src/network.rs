use core::{arch::asm, slice};

use nagi_net::{SocketApi, SyscallDevice};

#[no_mangle]
static NAGI_M12_READY: [u8; 24] = *b"Nagi M12 network READY\r\n";
#[no_mangle]
static NAGI_M12_ARP: [u8; 19] = *b"Nagi M12 ARP PASS\r\n";
#[no_mangle]
static NAGI_M12_TCP: [u8; 29] = *b"Nagi M12 TCP handshake PASS\r\n";
#[no_mangle]
static NAGI_M12_HTTP: [u8; 29] = *b"Nagi M12 HTTP response PASS\r\n";
#[no_mangle]
static NAGI_M12_DHCP: [u8; 20] = *b"Nagi M12 DHCP PASS\r\n";
#[no_mangle]
static NAGI_M12_ICMP: [u8; 20] = *b"Nagi M12 ICMP PASS\r\n";
#[no_mangle]
static NAGI_M12_UDP_DNS: [u8; 23] = *b"Nagi M12 UDP/DNS PASS\r\n";
#[no_mangle]
static NAGI_M12_ACCEPTANCE: [u8; 26] = *b"Nagi M12 acceptance PASS\r\n";
#[no_mangle]
static NAGI_M12_FAILURE: [u8; 26] = *b"Nagi M12 acceptance FAIL\r\n";
#[no_mangle]
static NAGI_M12_PATH: [u8; 13] = *b"/nagi-m12.txt";
#[no_mangle]
static NAGI_M12_EXPECTED: [u8; 26] = *b"NAGI_M12_HTTP_FIXTURE_PASS";

macro_rules! static_bytes {
    ($symbol:ident, $length:expr) => {{
        let address: *const u8;
        unsafe {
            asm!(
                "lea {address}, [rip + {symbol}]",
                address = out(reg) address,
                symbol = sym $symbol,
                options(nostack, preserves_flags, readonly),
            );
            slice::from_raw_parts(address, $length)
        }
    }};
}

pub fn run(capability: u64) -> ! {
    print(static_bytes!(NAGI_M12_READY, 24));
    if capability == 0 {
        fail();
    }
    let device = SyscallDevice::new(capability);
    let mut stack = SocketApi::new(device);
    let gateway_ip = match stack.dhcp_gateway() {
        Ok(address) => address,
        Err(_) => fail(),
    };
    print(static_bytes!(NAGI_M12_DHCP, 20));
    if stack.icmp_echo(gateway_ip).is_err() {
        fail();
    }
    print(static_bytes!(NAGI_M12_ICMP, 20));
    if stack.resolve_ipv4("example.com").is_err() {
        fail();
    }
    print(static_bytes!(NAGI_M12_UDP_DNS, 23));
    let mut response = [0_u8; 1536];
    let length = match stack.http_get(
        gateway_ip,
        18_080,
        static_bytes!(NAGI_M12_PATH, 13),
        static_bytes!(NAGI_M12_EXPECTED, 26),
        &mut response,
    ) {
        Ok(length) => length,
        Err(_) => fail(),
    };
    print(static_bytes!(NAGI_M12_ARP, 19));
    print(static_bytes!(NAGI_M12_TCP, 29));
    if !contains(&response, length, static_bytes!(NAGI_M12_EXPECTED, 26)) {
        fail();
    }
    print(static_bytes!(NAGI_M12_HTTP, 29));
    print(static_bytes!(NAGI_M12_ACCEPTANCE, 26));
    loop {
        unsafe { asm!("hlt", options(nomem, nostack, preserves_flags)) };
    }
}

fn contains(bytes: &[u8], length: usize, needle: &[u8]) -> bool {
    if needle.is_empty() || length < needle.len() {
        return false;
    }
    for start in 0..=length - needle.len() {
        let mut index = 0;
        while index < needle.len()
            && unsafe { core::ptr::read(bytes.as_ptr().add(start + index)) }
                == unsafe { core::ptr::read(needle.as_ptr().add(index)) }
        {
            index += 1;
        }
        if index == needle.len() {
            return true;
        }
    }
    false
}

fn print(bytes: &[u8]) {
    libnagi::console_write(bytes);
}

fn fail() -> ! {
    print(static_bytes!(NAGI_M12_FAILURE, 26));
    libnagi::exit(1);
}
