//! PID 1 sends one explicitly submitted login over the existing C3 control
//! stream. This private frame is UI input, separate from C2 authentication.

use libnagi::credential::{valid_name, MAX_ACCOUNT_NAME_BYTES, MAX_PASSWORD_BYTES};
use nagi_sdk::AppId;

use zeroize::Zeroizing;

pub const CONNECTED: u8 = 1;

const MAGIC: &[u8; 4] = b"T24\x01";
const HEADER_BYTES: usize = 30;
const BODY_BYTES: usize = MAX_ACCOUNT_NAME_BYTES + MAX_PASSWORD_BYTES;

#[derive(Clone, Copy)]
pub struct RequestRoute {
    pub destination: AppId,
    pub request: [u8; 16],
}

pub struct SubmittedLogin {
    pub route: RequestRoute,
    body: Zeroizing<[u8; BODY_BYTES]>,
    account_len: usize,
    body_len: usize,
}

impl SubmittedLogin {
    pub fn account(&self) -> &[u8] {
        &self.body[..self.account_len]
    }

    pub fn password(&self) -> &[u8] {
        &self.body[self.account_len..self.body_len]
    }
}

pub fn send(
    mut write_all: impl FnMut(&[u8]) -> bool,
    route: RequestRoute,
    account: &[u8],
    password: &[u8],
) -> bool {
    if !valid_name(account) || password.is_empty() || password.len() > MAX_PASSWORD_BYTES {
        return false;
    }
    let mut header = [0; HEADER_BYTES];
    header[..4].copy_from_slice(MAGIC);
    header[4..12].copy_from_slice(&route.destination.0.to_le_bytes());
    header[12..28].copy_from_slice(&route.request);
    header[28] = account.len() as u8;
    header[29] = password.len() as u8;
    write_all(&header) && write_all(account) && write_all(password)
}

pub fn receive(mut read_exact: impl FnMut(&mut [u8]) -> bool) -> Option<SubmittedLogin> {
    let mut header = [0; HEADER_BYTES];
    if !read_exact(&mut header) {
        return None;
    }
    let account_len = usize::from(header[28]);
    let password_len = usize::from(header[29]);
    if &header[..4] != MAGIC
        || !(1..=MAX_ACCOUNT_NAME_BYTES).contains(&account_len)
        || !(1..=MAX_PASSWORD_BYTES).contains(&password_len)
    {
        return None;
    }
    let mut body = Zeroizing::new([0; BODY_BYTES]);
    let body_len = account_len + password_len;
    if !read_exact(&mut body[..body_len]) || !valid_name(&body[..account_len]) {
        return None;
    }
    Some(SubmittedLogin {
        route: RequestRoute {
            destination: AppId(u64::from_le_bytes(header[4..12].try_into().unwrap())),
            request: header[12..28].try_into().unwrap(),
        },
        body,
        account_len,
        body_len,
    })
}
