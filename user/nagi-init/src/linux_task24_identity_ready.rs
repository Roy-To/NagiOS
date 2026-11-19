//! Task24's read-only Identity handshake. The service-side handler receives
//! the broker stream and its separate, inherited PID 1 control stream from
//! the launch owner. Neither route IDs nor the request confer authority.

use nagi_nipc_v1_stream_candidate as nipc;
use nagi_sdk::app_contract::IpcEnvelope;
use nagi_sdk::AppId;
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

#[path = "linux_task24_identity_readiness_wire.rs"]
mod wire;
pub(super) use wire::{
    challenge_for as readiness_challenge, BROKER_APP, CHALLENGE_BYTES, REQUEST_TYPE, RESPONSE_TYPE,
};

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
    let challenge =
        wire::challenge(&request, broker_app, identity_app).ok_or_else(invalid_request)?;

    let mut control = Until {
        stream: control,
        deadline,
    };
    control.write_all(&challenge)?;
    let mut confirmed = [0; wire::CONTROL_CONFIRMATION_BYTES];
    control.read_exact(&mut confirmed)?;
    let response = wire::response(request, &challenge, &confirmed)
        .ok_or_else(|| io::Error::from(io::ErrorKind::PermissionDenied))?;
    nipc::write_frame(&mut broker, &response).map_err(transport_error)?;
    Ok(())
}

/// PID 1 handles exactly one challenge on the held C3 control endpoint.
/// The callback must enter the current C1/C3 owner gates and return before
/// the control reply is written. READY publication revalidates those gates.
pub(super) fn confirm_identity_readiness(
    control: &UnixStream,
    deadline: Instant,
    check_current_identity: impl FnOnce(&[u8; CHALLENGE_BYTES]) -> io::Result<()>,
) -> io::Result<()> {
    let mut control = Until {
        stream: control,
        deadline,
    };
    let mut challenge = [0; CHALLENGE_BYTES];
    control.read_exact(&mut challenge)?;
    check_current_identity(&challenge)?;
    control.write_all(&wire::control_confirmation(&challenge))
}

#[cfg(test)]
mod tests {
    use super::*;
    use nagi_sdk::app_contract::{
        CorrelationId, MessageKind, OrderingMetadata, RequestId, IPC_PROTOCOL_VERSION,
    };

    #[test]
    fn service_replies_only_after_owner_confirms_current_state() {
        for (owner_ready, matching_attempt) in [(true, true), (false, true), (true, false)] {
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
            let mut expected =
                readiness_challenge(request.request_id.unwrap(), request.correlation_id);
            if !matching_attempt {
                expected[0] ^= 1;
            }
            std::thread::scope(|scope| {
                let owner_task = scope.spawn(move || {
                    confirm_identity_readiness(&owner, deadline, |challenge| {
                        (owner_ready && challenge == &expected)
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
                if owner_ready && matching_attempt {
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

    #[test]
    fn service_rejects_legacy_control_echo() {
        let (mut broker, service) = UnixStream::pair().unwrap();
        let (mut owner, control) = UnixStream::pair().unwrap();
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
                let mut challenge = [0; CHALLENGE_BYTES];
                owner
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                owner.read_exact(&mut challenge).unwrap();
                owner.write_all(&challenge).unwrap();
            });
            let service_task = scope.spawn(move || {
                serve_identity_readiness(&service, &control, AppId(7), AppId(8), deadline)
            });
            broker
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            nipc::write_frame(&mut broker, &request).unwrap();
            assert!(nipc::read_frame(&mut broker).unwrap().is_none());
            owner_task.join().unwrap();
            assert_eq!(
                service_task.join().unwrap().unwrap_err().kind(),
                io::ErrorKind::UnexpectedEof
            );
        });
    }
}
