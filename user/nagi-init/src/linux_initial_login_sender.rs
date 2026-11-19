//! Freestanding Task24 adapter for the C2 initial-login wire contract.
//! The NIPC envelope is encoded by the SDK used by the C2 receiver.

use libnagi::credential::{valid_name, MAX_ACCOUNT_NAME_BYTES, MAX_PASSWORD_BYTES};
use nagi_sdk::app_contract::{
    CorrelationId, IpcEnvelope, MessageKind, OrderingMetadata, RequestId, IPC_PROTOCOL_VERSION,
};
use nagi_sdk::AppId;
use zeroize::Zeroizing;

const PAYLOAD_TYPE: &str = "nagi.auth.initial-login@1";
const MAX_BODY: usize = 8 + MAX_ACCOUNT_NAME_BYTES + MAX_PASSWORD_BYTES;
const MAX_FRAME: usize = 78 + PAYLOAD_TYPE.len() + MAX_BODY;
const LOCATOR: &[u8] = b"--nagi-service-endpoint-v1=";
const SOCKET_ROOT: &[u8] = b"/run/nagi/ipc/";
const LOGIN_PACKAGE: AppId = AppId::from_identifier(b"org.nagi.dev.task24-login");

pub(super) fn broker_path(argument: &[u8]) -> Option<&[u8]> {
    let path = argument.strip_prefix(LOCATOR)?;
    let leaf = path.strip_prefix(SOCKET_ROOT)?;
    if leaf.len() != 40
        || leaf[0] != b'r'
        || &leaf[17..19] != b"-e"
        || &leaf[35..] != b".sock"
        || !leaf[1..17]
            .iter()
            .chain(&leaf[19..35])
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
    {
        return None;
    }
    Some(path)
}

pub(super) fn send_initial_login(
    destination: AppId,
    request: [u8; 16],
    account: &[u8],
    password: &[u8],
    send: impl FnOnce(&[u8]) -> bool,
) -> bool {
    if !valid_name(account)
        || password.is_empty()
        || password.len() > MAX_PASSWORD_BYTES
        || core::str::from_utf8(password).is_err()
    {
        return false;
    }

    let mut payload = Zeroizing::new([0; MAX_BODY]);
    payload[..2].copy_from_slice(&1u16.to_le_bytes());
    payload[2..4].copy_from_slice(&(account.len() as u16).to_le_bytes());
    payload[4..8].copy_from_slice(&(password.len() as u32).to_le_bytes());
    let account_end = 8 + account.len();
    let payload_end = account_end + password.len();
    payload[8..account_end].copy_from_slice(account);
    payload[account_end..payload_end].copy_from_slice(password);

    let mut frame = Zeroizing::new([0; MAX_FRAME]);
    let envelope = IpcEnvelope {
        protocol_version: IPC_PROTOCOL_VERSION,
        source: LOGIN_PACKAGE,
        destination: Some(destination),
        kind: MessageKind::Request,
        correlation_id: CorrelationId(request),
        request_id: Some(RequestId(request)),
        ordering: OrderingMetadata {
            timestamp_micros: None,
            sequence: None,
        },
        payload_type: PAYLOAD_TYPE,
        payload: &payload[..payload_end],
    };
    match envelope.encode(frame.as_mut_slice()) {
        Ok(len) => {
            drop(payload);
            send(&frame[..len])
        }
        Err(_) => false,
    }
}
