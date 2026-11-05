//! `bar-summary@1`: bounded, caller-free transport for an OS-issued summary
//! intent. This is separate from action@1 execution and grants no authority.
use nagi_abi::{ChannelReceiveResult, ChannelSendRequest};

pub const PROTOCOL_ID: u16 = 0x4241;
pub const VERSION: u16 = 1;
pub const OFFER: u16 = 1;
pub const REQUEST: u16 = 2;
pub const RESULT: u16 = 3;
/// Trusted boot ticks, not milliseconds or a dispatch-time guarantee.
pub const MAX_PENDING_TICKS: u64 = 500;
const SUMMARY_TAG: u8 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Envelope,
    Payload,
    NoPending,
    Busy,
    Stale,
    Expired,
    Clock,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Terminal {
    Unavailable = 0,
    Denied = 1,
    Cancelled = 2,
}

fn envelope(message: &ChannelReceiveResult, opcode: u16) -> Result<(), Error> {
    if message.protocol_id != PROTOCOL_ID
        || message.version != VERSION
        || message.opcode != opcode
        || message.flags != 0
        || message.reserved != 0
        || message.transfer_count != 0
        || message.handles.iter().any(|h| *h != 0)
        || message.request_id == 0
        || message.payload_len != 1
    {
        return Err(Error::Envelope);
    }
    Ok(())
}
fn packet(id: u64, opcode: u16, value: u8) -> ChannelSendRequest {
    let mut message = ChannelSendRequest::new(PROTOCOL_ID, VERSION, id, opcode);
    message.payload_len = 1;
    message.payload[0] = value;
    message
}
pub fn offer(id: u64) -> Result<ChannelSendRequest, Error> {
    if id == 0 {
        return Err(Error::Stale);
    }
    Ok(packet(id, OFFER, SUMMARY_TAG))
}
/// The real child accepts offers ONLY from kernel-stamped init sender1. No
/// process/app/Session identity is decoded from the single payload byte.
pub fn request_for_offer(message: &ChannelReceiveResult) -> Result<ChannelSendRequest, Error> {
    envelope(message, OFFER)?;
    if message.sender_process_id != 1 || message.payload[0] != SUMMARY_TAG {
        return Err(Error::Payload);
    }
    Ok(packet(message.request_id, REQUEST, SUMMARY_TAG))
}
pub fn terminal(id: u64, status: Terminal) -> Result<ChannelSendRequest, Error> {
    if id == 0 {
        return Err(Error::Stale);
    }
    Ok(packet(id, RESULT, status as u8))
}
pub fn decode_terminal(message: &ChannelReceiveResult, expected: u64) -> Result<Terminal, Error> {
    envelope(message, RESULT)?;
    if message.sender_process_id != 1 || message.request_id != expected {
        return Err(Error::Stale);
    }
    match message.payload[0] {
        0 => Ok(Terminal::Unavailable),
        1 => Ok(Terminal::Denied),
        2 => Ok(Terminal::Cancelled),
        _ => Err(Error::Payload),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Pending {
    id: u64,
    deadline: u64,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ValidatedRequest {
    id: u64,
    valid_from: u64,
    deadline: u64,
}
impl ValidatedRequest {
    pub fn id(self) -> u64 {
        self.id
    }
    pub fn is_fresh(self, now: u64) -> bool {
        now != 0 && now >= self.valid_from && now < self.deadline
    }
}
/// One outstanding OS-local intent. IDs and time come from trusted UI/runtime,
/// never the client. This validates transport freshness, not app/user authority.
#[derive(Debug, Default)]
pub struct Exchange {
    last_id: u64,
    last_tick: u64,
    pending: Option<Pending>,
}
impl Exchange {
    pub const fn new() -> Self {
        Self {
            last_id: 0,
            last_tick: 0,
            pending: None,
        }
    }
    fn observe(&mut self, now: u64) -> Result<(), Error> {
        if now == 0 || now < self.last_tick {
            self.pending = None;
            return Err(Error::Clock);
        }
        self.last_tick = now;
        Ok(())
    }
    pub fn begin(&mut self, id: u64, now: u64) -> Result<ChannelSendRequest, Error> {
        self.observe(now)?;
        if self.pending.is_some() {
            return Err(Error::Busy);
        }
        if id == 0 || id <= self.last_id {
            return Err(Error::Stale);
        }
        let deadline = now.checked_add(MAX_PENDING_TICKS).ok_or(Error::Clock)?;
        self.last_id = id;
        self.pending = Some(Pending { id, deadline });
        offer(id)
    }
    /// Every received attempt consumes its pending transaction, including an
    /// invalid attempt. It must not silently retry the same local intent.
    pub fn accept(
        &mut self,
        message: &ChannelReceiveResult,
        now: u64,
    ) -> Result<ValidatedRequest, Error> {
        self.observe(now)?;
        let pending = self.pending.take().ok_or(Error::NoPending)?;
        if now >= pending.deadline {
            return Err(Error::Expired);
        }
        envelope(message, REQUEST)?;
        if message.payload[0] != SUMMARY_TAG {
            return Err(Error::Payload);
        }
        if message.request_id != pending.id {
            return Err(Error::Stale);
        }
        Ok(ValidatedRequest {
            id: pending.id,
            valid_from: now,
            deadline: pending.deadline,
        })
    }
    pub fn expire(&mut self, now: u64) -> Result<(), Error> {
        self.observe(now)?;
        if self.pending.is_some_and(|p| now >= p.deadline) {
            self.pending = None;
            return Err(Error::Expired);
        }
        Ok(())
    }
    pub fn cancel(&mut self) {
        self.pending = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn receive(packet: ChannelSendRequest, sender: u32) -> ChannelReceiveResult {
        ChannelReceiveResult {
            sender_process_id: sender,
            protocol_id: packet.protocol_id,
            version: packet.version,
            opcode: packet.opcode,
            flags: packet.flags,
            request_id: packet.request_id,
            payload_len: packet.payload_len,
            payload: packet.payload,
            ..ChannelReceiveResult::default()
        }
    }
    fn request(id: u64) -> ChannelReceiveResult {
        let offered = receive(offer(id).unwrap(), 1);
        receive(request_for_offer(&offered).unwrap(), 7)
    }
    #[test]
    fn exact_local_intent_is_accepted_once_without_caller_payload() {
        let mut e = Exchange::new();
        let p = e.begin(3, 10).unwrap();
        assert_eq!(p.payload_len, 1);
        assert_eq!(p.payload[0], SUMMARY_TAG);
        let validated = e.accept(&request(3), 11).unwrap();
        assert_eq!(validated.id(), 3);
        assert!(!validated.is_fresh(9));
        assert!(validated.is_fresh(509));
        assert!(!validated.is_fresh(510));
        assert_eq!(e.accept(&request(3), 12), Err(Error::NoPending));
        assert_eq!(e.begin(3, 13), Err(Error::Stale));
    }
    #[test]
    fn changed_wire_fields_and_payloads_never_accept_or_retry() {
        for field in 0..10 {
            let mut e = Exchange::new();
            e.begin(1, 10).unwrap();
            let mut r = request(1);
            match field {
                0 => r.protocol_id ^= 1,
                1 => r.version += 1,
                2 => r.opcode = OFFER,
                3 => r.flags = 1,
                4 => r.reserved = 1,
                5 => r.transfer_count = 1,
                6 => r.payload_len = u32::MAX,
                7 => r.payload[0] = 99,
                8 => r.request_id = 2,
                _ => r.handles[0] = 42,
            }
            assert!(e.accept(&r, 11).is_err());
            assert_eq!(e.accept(&request(1), 12), Err(Error::NoPending));
            assert_eq!(e.begin(1, 13), Err(Error::Stale));
        }
    }
    #[test]
    fn expiry_cancel_new_intent_and_late_reply_do_not_revive_request() {
        let mut e = Exchange::new();
        e.begin(1, 10).unwrap();
        assert_eq!(e.expire(510), Err(Error::Expired));
        e.begin(2, 511).unwrap();
        e.cancel();
        assert_eq!(e.accept(&request(2), 512), Err(Error::NoPending));
        e.begin(3, 513).unwrap();
        assert_eq!(e.accept(&request(2), 514), Err(Error::Stale));
        assert_eq!(e.accept(&request(3), 515), Err(Error::NoPending));
    }
    #[test]
    fn clock_failure_rollback_overflow_and_busy_are_explicit() {
        let mut e = Exchange::new();
        assert_eq!(e.begin(0, 10), Err(Error::Stale));
        assert_eq!(e.begin(1, 0), Err(Error::Clock));
        e.begin(1, 10).unwrap();
        assert_eq!(e.begin(2, 11), Err(Error::Busy));
        assert_eq!(e.accept(&request(1), 9), Err(Error::Clock));
        assert_eq!(e.begin(1, 12), Err(Error::Stale));
        assert_eq!(e.begin(2, u64::MAX), Err(Error::Clock));
    }
    #[test]
    fn client_checks_init_sender_version_request_and_terminal_values() {
        let p = receive(offer(5).unwrap(), 7);
        assert!(request_for_offer(&p).is_err());
        let mut p = receive(offer(5).unwrap(), 1);
        p.version += 1;
        assert!(request_for_offer(&p).is_err());
        for status in [Terminal::Unavailable, Terminal::Denied, Terminal::Cancelled] {
            let p = receive(terminal(5, status).unwrap(), 1);
            assert_eq!(decode_terminal(&p, 5), Ok(status));
            assert!(decode_terminal(&p, 6).is_err());
        }
        let mut p = receive(terminal(5, Terminal::Denied).unwrap(), 1);
        p.payload[0] = 3; // Positive/model-result terminal is deliberately undefined.
        assert!(decode_terminal(&p, 5).is_err());
    }
}
