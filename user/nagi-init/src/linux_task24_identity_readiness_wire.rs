use nagi_sdk::app_contract::{CorrelationId, IpcEnvelope, MessageKind, RequestId};
use nagi_sdk::AppId;

pub(crate) const REQUEST_TYPE: &str = "nagi.identity.readiness.request@2";
pub(crate) const RESPONSE_TYPE: &str = "nagi.identity.readiness.response@2";
pub(crate) const CHALLENGE_BYTES: usize = 32;
const CONTROL_CONFIRMATION_TAG: [u8; 8] = *b"NAGIRDY2";
pub(crate) const CONTROL_CONFIRMATION_BYTES: usize =
    CONTROL_CONFIRMATION_TAG.len() + CHALLENGE_BYTES;
pub(crate) const BROKER_APP: AppId = AppId::from_identifier(b"org.nagi.init");

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

pub(crate) fn control_confirmation(
    challenge: &[u8; CHALLENGE_BYTES],
) -> [u8; CONTROL_CONFIRMATION_BYTES] {
    let mut confirmation = [0; CONTROL_CONFIRMATION_BYTES];
    let (tag, payload) = confirmation.split_at_mut(CONTROL_CONFIRMATION_TAG.len());
    tag.copy_from_slice(&CONTROL_CONFIRMATION_TAG);
    payload.copy_from_slice(challenge);
    confirmation
}

pub(super) fn response<'a>(
    request: IpcEnvelope<'a>,
    expected: &[u8; CHALLENGE_BYTES],
    confirmed: &'a [u8; CONTROL_CONFIRMATION_BYTES],
) -> Option<IpcEnvelope<'a>> {
    if confirmed != &control_confirmation(expected) {
        return None;
    }
    Some(IpcEnvelope {
        source: request.destination?,
        destination: Some(request.source),
        kind: MessageKind::Response,
        payload_type: RESPONSE_TYPE,
        payload: &confirmed[CONTROL_CONFIRMATION_TAG.len()..],
        ..request
    })
}
