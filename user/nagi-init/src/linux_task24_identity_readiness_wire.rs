use nagi_sdk::app_contract::{IpcEnvelope, MessageKind};
use nagi_sdk::AppId;

pub(super) const REQUEST_TYPE: &str = "nagi.identity.readiness.request@1";
pub(super) const RESPONSE_TYPE: &str = "nagi.identity.readiness.response@1";
pub(super) const CHALLENGE_BYTES: usize = 32;

pub(super) fn challenge(
    request: &IpcEnvelope<'_>,
    broker: AppId,
    identity: AppId,
) -> Option<[u8; CHALLENGE_BYTES]> {
    if request.kind != MessageKind::Request
        || request.source != broker
        || request.destination != Some(identity)
        || request.payload_type != REQUEST_TYPE
        || !request.payload.is_empty()
    {
        return None;
    }
    let mut challenge = [0; CHALLENGE_BYTES];
    challenge[..16].copy_from_slice(&request.request_id?.0);
    challenge[16..].copy_from_slice(&request.correlation_id.0);
    Some(challenge)
}

pub(super) fn response<'a>(
    request: IpcEnvelope<'a>,
    expected: &[u8; CHALLENGE_BYTES],
    confirmed: &'a [u8; CHALLENGE_BYTES],
) -> Option<IpcEnvelope<'a>> {
    if confirmed != expected {
        return None;
    }
    Some(IpcEnvelope {
        source: request.destination?,
        destination: Some(request.source),
        kind: MessageKind::Response,
        payload_type: RESPONSE_TYPE,
        payload: confirmed,
        ..request
    })
}
