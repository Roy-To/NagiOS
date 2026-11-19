//! Task24's read-only Identity handshake. The service-side handler receives
//! the broker stream and its separate, inherited PID 1 control stream from
//! the launch owner. Neither route IDs nor the request confer authority.

use nagi_nipc_v1_stream_candidate as nipc;
use nagi_sdk::app_contract::{IpcEnvelope, MessageKind};
use nagi_sdk::AppId;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

pub(super) const REQUEST_TYPE: &str = "nagi.identity.readiness.request@1";
pub(super) const RESPONSE_TYPE: &str = "nagi.identity.readiness.response@1";
const CHALLENGE_BYTES: usize = 32;

struct Until<'a> {
    stream: &'a UnixStream,
    deadline: Instant,
}

impl Until<'_> {
    fn remaining(&self) -> io::Result<Duration> {
        self.deadline
            .checked_duration_since(Instant::now())
            .filter(|remaining| !remaining.is_zero())
            .ok_or_else(|| io::Error::from(io::ErrorKind::TimedOut))
    }
}

impl Read for Until<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.stream.set_read_timeout(Some(self.remaining()?))?;
        let read = self.stream.read(buffer)?;
        self.remaining()?;
        Ok(read)
    }
}

impl Write for Until<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.stream.set_write_timeout(Some(self.remaining()?))?;
        let written = self.stream.write(buffer)?;
        self.remaining()?;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.remaining().map(|_| ())
    }
}

fn invalid_request() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "invalid Identity readiness request",
    )
}

fn transport_error(error: nipc::FrameError) -> io::Error {
    match error {
        nipc::FrameError::Io(error) => error,
        nipc::FrameError::Contract(_) => invalid_request(),
    }
}

/// Answer one real NIPC status request in the Identity process. The child
/// returns a response only after PID 1 confirms this exact request on the
/// separate authenticated control connection. The caller retains both FDs.
pub(super) fn serve_identity_readiness(
    broker: &UnixStream,
    control: &UnixStream,
    broker_app: AppId,
    identity_app: AppId,
    deadline: Instant,
) -> io::Result<()> {
    let mut broker = Until {
        stream: broker,
        deadline,
    };
    let frame = nipc::read_frame(&mut broker)
        .map_err(transport_error)?
        .ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))?;
    let request = IpcEnvelope::decode(&frame).map_err(|_| invalid_request())?;
    if request.kind != MessageKind::Request
        || request.source != broker_app
        || request.destination != Some(identity_app)
        || request.payload_type != REQUEST_TYPE
        || !request.payload.is_empty()
    {
        return Err(invalid_request());
    }
    let request_id = request.request_id.ok_or_else(invalid_request)?;
    let mut challenge = [0; CHALLENGE_BYTES];
    challenge[..16].copy_from_slice(&request_id.0);
    challenge[16..].copy_from_slice(&request.correlation_id.0);

    let mut control = Until {
        stream: control,
        deadline,
    };
    control.write_all(&challenge)?;
    let mut confirmed = [0; CHALLENGE_BYTES];
    control.read_exact(&mut confirmed)?;
    if confirmed != challenge {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Identity status was not confirmed",
        ));
    }

    nipc::write_frame(
        &mut broker,
        &IpcEnvelope {
            source: identity_app,
            destination: Some(broker_app),
            kind: MessageKind::Response,
            payload_type: RESPONSE_TYPE,
            payload: &confirmed,
            ..request
        },
    )
    .map_err(transport_error)?;
    Ok(())
}

/// PID 1 handles exactly one challenge on the held C3 control endpoint.
/// The callback must enter the current C1/C3 owner gates and return before
/// the control reply is written. READY publication revalidates those gates.
pub(super) fn confirm_identity_readiness(
    control: &UnixStream,
    deadline: Instant,
    check_current_identity: impl FnOnce() -> io::Result<()>,
) -> io::Result<()> {
    let mut control = Until {
        stream: control,
        deadline,
    };
    let mut challenge = [0; CHALLENGE_BYTES];
    control.read_exact(&mut challenge)?;
    check_current_identity()?;
    control.write_all(&challenge)
}

#[cfg(test)]
mod tests {
    use super::*;
    use nagi_sdk::app_contract::{
        CorrelationId, OrderingMetadata, RequestId, IPC_PROTOCOL_VERSION,
    };

    #[test]
    fn service_replies_only_after_owner_confirms_current_state() {
        for owner_ready in [true, false] {
            let (mut broker, service) = UnixStream::pair().unwrap();
            let (owner, control) = UnixStream::pair().unwrap();
            let deadline = Instant::now() + Duration::from_secs(2);
            let request = IpcEnvelope {
                protocol_version: IPC_PROTOCOL_VERSION,
                source: AppId(7),
                destination: Some(AppId(8)),
                kind: MessageKind::Request,
                correlation_id: CorrelationId([11; 16]),
                request_id: Some(RequestId([12; 16])),
                ordering: OrderingMetadata {
                    timestamp_micros: None,
                    sequence: None,
                },
                payload_type: REQUEST_TYPE,
                payload: &[],
            };
            std::thread::scope(|scope| {
                let owner_task = scope.spawn(move || {
                    confirm_identity_readiness(&owner, deadline, || {
                        owner_ready
                            .then_some(())
                            .ok_or_else(|| io::Error::from(io::ErrorKind::PermissionDenied))
                    })
                });
                let service_task = scope.spawn(move || {
                    serve_identity_readiness(&service, &control, AppId(7), AppId(8), deadline)
                });
                broker
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                nipc::write_frame(&mut broker, &request).unwrap();
                let frame = nipc::read_frame(&mut broker).unwrap();
                if owner_ready {
                    owner_task.join().unwrap().unwrap();
                    service_task.join().unwrap().unwrap();
                    let frame = frame.unwrap();
                    let response = IpcEnvelope::decode(&frame).unwrap();
                    assert_eq!(response.kind, MessageKind::Response);
                    assert_eq!(response.source, AppId(8));
                    assert_eq!(response.destination, Some(AppId(7)));
                    assert_eq!(response.payload_type, RESPONSE_TYPE);
                    assert_eq!(response.request_id, request.request_id);
                    assert_eq!(response.correlation_id, request.correlation_id);
                    assert_eq!(&response.payload[..16], &[12; 16]);
                    assert_eq!(&response.payload[16..], &[11; 16]);
                } else {
                    assert_eq!(
                        owner_task.join().unwrap().unwrap_err().kind(),
                        io::ErrorKind::PermissionDenied
                    );
                    assert_eq!(
                        service_task.join().unwrap().unwrap_err().kind(),
                        io::ErrorKind::UnexpectedEof
                    );
                    assert!(frame.is_none());
                }
            });
        }
    }
}
