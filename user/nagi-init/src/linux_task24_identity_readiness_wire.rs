use nagi_sdk::app_contract::{CorrelationId, IpcEnvelope, MessageKind, RequestId};
use nagi_sdk::AppId;

pub(crate) const REQUEST_TYPE: &str = "nagi.identity.readiness.request@1";
pub(crate) const RESPONSE_TYPE: &str = "nagi.identity.readiness.response@1";
pub(crate) const CHALLENGE_BYTES: usize = 32;

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
    Some(challenge_for(request.request_id?, request.correlation_id))
}

pub(crate) fn challenge_for(
    request: RequestId,
    correlation: CorrelationId,
) -> [u8; CHALLENGE_BYTES] {
    let mut challenge = [0; CHALLENGE_BYTES];
    challenge[..16].copy_from_slice(&request.0);
    challenge[16..].copy_from_slice(&correlation.0);
    challenge
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
