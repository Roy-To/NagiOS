//! Init-side consumer for the existing service-contract call seam.
//!
//! This candidate reuses action@1's existing payload codec,
//! `nagi-service-contract`'s typed envelopes, and NIPC v1 SDK framing. Its
//! worker-backed Unix transport requires an explicit SVC-to-NIPC mapping and a
//! runtime-owned authenticated peer/registration provider. It does not
//! authenticate peers, map principals, or create lifecycle events. Without a
//! provider it fails closed; the production init backend must still connect
//! the runtime-owned `ServiceClient` and emit READY only after real handshakes.

mod action_adapter;
mod checked_call;
mod runtime;
mod unix_driver;
mod unix_service_transport;

pub use action_adapter::{call_action_blocking, ActionCallError};
pub use checked_call::CallError;
pub use runtime::call_registered_blocking;
pub use unix_driver::{call_unix_stream, UnixCallError};
pub use unix_service_transport::{
    AuthenticatedUnixStreamProvider, MappedNipcRequest, ServiceNipcMapping,
    UnixNipcServiceTransport,
};

#[cfg(test)]
mod tests;
