use core::{arch::asm, slice};

use libnagi::security::{
    AccountStore, AppTrust, DenyReason, PermissionBroker, PermissionDecision, PermissionRequest,
    Resource, Role,
};

#[no_mangle]
static NAGI_M11_OWNER_NAME: [u8; 5] = *b"owner";
#[no_mangle]
static NAGI_M11_OWNER_PASSWORD: [u8; 10] = *b"owner-pass";
#[no_mangle]
static NAGI_M11_STANDARD_NAME: [u8; 8] = *b"standard";
#[no_mangle]
static NAGI_M11_STANDARD_PASSWORD: [u8; 13] = *b"standard-pass";

#[no_mangle]
static NAGI_M11_LOGIN: [u8; 27] = *b"Nagi M11 local login PASS\r\n";
#[no_mangle]
static NAGI_M11_LOCK_SCREEN: [u8; 27] = *b"Nagi M11 lock screen PASS\r\n";
#[no_mangle]
static NAGI_M11_DEVELOPER_MODE: [u8; 30] = *b"Nagi M11 Developer Mode PASS\r\n";
#[no_mangle]
static NAGI_M11_DIALOG: [u8; 34] = *b"Nagi M11 trusted dialog ASK PASS\r\n";
#[no_mangle]
static NAGI_M11_FILE_DENIED: [u8; 32] = *b"Nagi M11 malicious file DENIED\r\n";
#[no_mangle]
static NAGI_M11_MICROPHONE_DENIED: [u8; 38] = *b"Nagi M11 malicious microphone DENIED\r\n";
#[no_mangle]
static NAGI_M11_ACCEPTANCE: [u8; 26] = *b"Nagi M11 acceptance PASS\r\n";
#[no_mangle]
static NAGI_M11_FAILURE: [u8; 26] = *b"Nagi M11 acceptance FAIL\r\n";

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

pub fn run() -> ! {
    let mut store = AccountStore::new();
    if store
        .add_account(
            static_bytes!(NAGI_M11_OWNER_NAME, 5),
            Role::Owner,
            static_bytes!(NAGI_M11_OWNER_PASSWORD, 10),
        )
        .is_err()
        || store
            .add_account(
                static_bytes!(NAGI_M11_STANDARD_NAME, 8),
                Role::Standard,
                static_bytes!(NAGI_M11_STANDARD_PASSWORD, 13),
            )
            .is_err()
    {
        fail();
    }
    let mut owner = match store.authenticate(
        static_bytes!(NAGI_M11_OWNER_NAME, 5),
        static_bytes!(NAGI_M11_OWNER_PASSWORD, 10),
    ) {
        Ok(session) => session,
        Err(_) => fail(),
    };
    let mut standard = match store.authenticate(
        static_bytes!(NAGI_M11_STANDARD_NAME, 8),
        static_bytes!(NAGI_M11_STANDARD_PASSWORD, 13),
    ) {
        Ok(session) => session,
        Err(_) => fail(),
    };
    if owner.role() != Role::Owner || standard.role() != Role::Standard {
        fail();
    }
    print(static_bytes!(NAGI_M11_LOGIN, 27));

    standard.lock();
    let broker = PermissionBroker::new();
    let locked = broker.decide(
        standard,
        request(Resource::FileRead, AppTrust::TrustedForeground, true),
    );
    if locked != PermissionDecision::Deny(DenyReason::SessionLocked)
        || !store.unlock(&mut standard, static_bytes!(NAGI_M11_STANDARD_PASSWORD, 13))
    {
        fail();
    }
    print(static_bytes!(NAGI_M11_LOCK_SCREEN, 27));

    if !store.enable_developer_mode(&mut owner) || !owner.developer_mode() {
        fail();
    }
    let mut broker = PermissionBroker::new();
    broker.sync_session(owner);
    print(static_bytes!(NAGI_M11_DEVELOPER_MODE, 30));

    let dialog = broker.decide(
        standard,
        request(Resource::FileRead, AppTrust::TrustedForeground, false),
    );
    if dialog != PermissionDecision::Ask {
        fail();
    }
    print(static_bytes!(NAGI_M11_DIALOG, 34));

    let file = broker.decide(
        standard,
        request(Resource::FileWrite, AppTrust::Untrusted, true),
    );
    if file != PermissionDecision::Deny(DenyReason::UntrustedApp) {
        fail();
    }
    print(static_bytes!(NAGI_M11_FILE_DENIED, 32));

    let microphone = broker.decide(
        standard,
        request(Resource::MicrophoneCapture, AppTrust::Untrusted, true),
    );
    if microphone != PermissionDecision::Deny(DenyReason::UntrustedApp) {
        fail();
    }
    print(static_bytes!(NAGI_M11_MICROPHONE_DENIED, 38));
    print(static_bytes!(NAGI_M11_ACCEPTANCE, 26));
    loop {
        unsafe { asm!("hlt", options(nomem, nostack, preserves_flags)) };
    }
}

fn request(resource: Resource, app_trust: AppTrust, explicit_consent: bool) -> PermissionRequest {
    PermissionRequest {
        app_trust,
        resource,
        foreground: true,
        explicit_consent,
    }
}

fn print(bytes: &[u8]) {
    libnagi::console_write(bytes);
}

fn fail() -> ! {
    print(static_bytes!(NAGI_M11_FAILURE, 26));
    libnagi::exit(1);
}
