use nagi_abi::MAX_CHANNEL_INLINE_PAYLOAD;
use nagi_action_ipc::{decode_result, encode_intent, ActionResult, WireError};
use nagi_service_contract::{
    CancellationToken, ContractVersion, OperationId, RequestBuildError, RequestEnvelope, RequestId,
    ServiceClient, ServiceId,
};
use std::time::Duration;

use crate::{call_registered_blocking, CallError};

/// Failures while adapting the existing action@1 payload to a registered
/// service-contract request and back.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ActionCallError {
    InvalidRequestId,
    EncodeIntent(WireError),
    BuildRequest(RequestBuildError),
    Call(CallError),
    DecodeResult(WireError),
}

/// Performs one synchronous, typed action@1 call through the already supplied
/// service-contract client. Service/version/operation come from the caller's
/// registered service handle; this function does not invent their mapping.
/// The init u64 request id is widened losslessly into the contract's nonzero
/// u128 RequestId and is checked against the full response metadata by the
/// shared consumer.
pub fn call_action_blocking(
    client: &ServiceClient,
    request_id: u64,
    service_id: ServiceId,
    contract_version: ContractVersion,
    operation_id: OperationId,
    intent: &str,
    cancellation: CancellationToken,
    timeout: Duration,
) -> Result<ActionResult, ActionCallError> {
    let request_id =
        RequestId::new(u128::from(request_id)).ok_or(ActionCallError::InvalidRequestId)?;

    let mut payload = [0_u8; MAX_CHANNEL_INLINE_PAYLOAD];
    let payload_len = encode_intent(intent, &mut payload).map_err(ActionCallError::EncodeIntent)?;
    let request = RequestEnvelope::new(
        request_id,
        None,
        None,
        service_id,
        contract_version,
        operation_id,
        payload[..payload_len].to_vec(),
    )
    .map_err(ActionCallError::BuildRequest)?;

    let response = call_registered_blocking(client, request, cancellation, timeout)
        .map_err(ActionCallError::Call)?;
    let payload = response
        .result()
        .map_err(|error| ActionCallError::Call(CallError::Remote(error.clone())))?;
    decode_result(payload).map_err(ActionCallError::DecodeResult)
}
