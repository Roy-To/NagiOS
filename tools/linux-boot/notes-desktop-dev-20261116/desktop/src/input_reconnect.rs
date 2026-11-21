pub(crate) use crate::linux_input_reopen::DeviceIdentity;
#[cfg(test)]
pub(crate) use crate::linux_input_reopen::DeviceIdentityError;
use linux_evdev_stream::{EvdevStream, LinuxEvdevTarget};
use std::io::Read;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReconnectPhase {
    Connected,
    WaitingForDevice,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ReconnectError {
    MissingAuthorizedIdentity,
    AlreadyWaiting,
    NotWaiting,
    NoCandidates,
    EmptyNodePath,
    CandidateIdentityUnavailable,
    NoMatchingDevice,
    AmbiguousMatchingDevices,
    ParserEpochExhausted,
}

/// One already-opened and capability-validated input source from a hotplug
/// scan. `identity` must have been queried from this same source handle, not
/// derived from `node_path`. The caller owns authorization and access control.
pub(crate) struct ReconnectCandidate<T> {
    node_path: String,
    identity: Option<DeviceIdentity>,
    source: T,
}

impl<T> ReconnectCandidate<T> {
    pub(crate) fn new(
        node_path: impl Into<String>,
        identity: Option<DeviceIdentity>,
        source: T,
    ) -> Self {
        Self {
            node_path: node_path.into(),
            identity,
            source,
        }
    }
}

/// Identity gate for one initially authorized input source.
///
/// It does not open `/dev/input` or implement a device identity supplier. The
/// current boot path has no stable unique identity contract, so callers must
/// fail closed when they cannot provide the original trusted identity and a
/// trusted identity for every eligible reopened candidate.
pub(crate) struct ReconnectGate {
    authorized_identity: DeviceIdentity,
    phase: ReconnectPhase,
    parser_epoch: u64,
    partial_record_bytes: usize,
}

pub(crate) struct AcceptedReconnect<T> {
    source: T,
    pub node_path: String,
    pub parser_epoch: u64,
    pub discarded_partial_record_bytes: usize,
}

pub(crate) struct ReconnectedInput<T> {
    pub stream: EvdevStream<T>,
    pub node_path: String,
    pub parser_epoch: u64,
    pub discarded_partial_record_bytes: usize,
}

impl<T> AcceptedReconnect<T> {
    /// Start a new parser over the accepted opened handle. The prior parser
    /// and any partial record are never transferred to this stream.
    pub(crate) fn start_fresh_stream(self, target: LinuxEvdevTarget) -> ReconnectedInput<T>
    where
        T: Read,
    {
        ReconnectedInput {
            stream: EvdevStream::new(self.source, target),
            node_path: self.node_path,
            parser_epoch: self.parser_epoch,
            discarded_partial_record_bytes: self.discarded_partial_record_bytes,
        }
    }
}

impl ReconnectGate {
    pub(crate) fn authorize(identity: Option<DeviceIdentity>) -> Result<Self, ReconnectError> {
        Ok(Self {
            authorized_identity: identity.ok_or(ReconnectError::MissingAuthorizedIdentity)?,
            phase: ReconnectPhase::Connected,
            parser_epoch: 0,
            partial_record_bytes: 0,
        })
    }

    pub(crate) fn phase(&self) -> ReconnectPhase {
        self.phase
    }

    pub(crate) fn disconnect(&mut self, partial_record_bytes: usize) -> Result<(), ReconnectError> {
        if self.phase != ReconnectPhase::Connected {
            return Err(ReconnectError::AlreadyWaiting);
        }
        self.phase = ReconnectPhase::WaitingForDevice;
        self.partial_record_bytes = partial_record_bytes;
        Ok(())
    }

    /// Accept exactly one identity match among the complete set of eligible
    /// opened candidates. Any missing identity makes the set ambiguous and is
    /// rejected. Node paths are returned for diagnostics only and never
    /// participate in identity matching.
    pub(crate) fn reconnect<T>(
        &mut self,
        mut candidates: Vec<ReconnectCandidate<T>>,
    ) -> Result<AcceptedReconnect<T>, ReconnectError> {
        if self.phase != ReconnectPhase::WaitingForDevice {
            return Err(ReconnectError::NotWaiting);
        }
        if candidates.is_empty() {
            return Err(ReconnectError::NoCandidates);
        }
        if candidates
            .iter()
            .any(|candidate| candidate.node_path.trim().is_empty())
        {
            return Err(ReconnectError::EmptyNodePath);
        }
        if candidates
            .iter()
            .any(|candidate| candidate.identity.is_none())
        {
            return Err(ReconnectError::CandidateIdentityUnavailable);
        }

        let matches = candidates
            .iter()
            .enumerate()
            .filter_map(|(index, candidate)| {
                (candidate.identity.as_ref() == Some(&self.authorized_identity)).then_some(index)
            })
            .collect::<Vec<_>>();
        let index = match matches.as_slice() {
            [] => return Err(ReconnectError::NoMatchingDevice),
            [index] => *index,
            _ => return Err(ReconnectError::AmbiguousMatchingDevices),
        };
        let parser_epoch = self
            .parser_epoch
            .checked_add(1)
            .ok_or(ReconnectError::ParserEpochExhausted)?;
        let candidate = candidates.swap_remove(index);

        self.parser_epoch = parser_epoch;
        self.phase = ReconnectPhase::Connected;
        let discarded_partial_record_bytes = self.partial_record_bytes;
        self.partial_record_bytes = 0;

        Ok(AcceptedReconnect {
            source: candidate.source,
            node_path: candidate.node_path,
            parser_epoch,
            discarded_partial_record_bytes,
        })
    }
}
