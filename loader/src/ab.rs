//! Deterministic A/B boot control and a checksummed two-record journal.
//!
//! The journal deliberately depends on a small persistent-store interface.
//! A platform adapter can back it with UEFI variables or another durable
//! firmware store without coupling the state machine to a particular device.

pub const MAX_BOOT_ATTEMPTS: u8 = 3;
pub const RECORD_SIZE: usize = 32;

#[cfg(target_os = "uefi")]
pub mod uefi_store;

const RECORD_MAGIC: [u8; 4] = *b"NABS";
const RECORD_VERSION: u8 = 1;
const NO_PENDING_SLOT: u8 = 0xff;
const CHECKSUM_OFFSET: usize = 28;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum SystemSlot {
    A = 0,
    B = 1,
}

impl SystemSlot {
    pub const fn other(self) -> Self {
        match self {
            Self::A => Self::B,
            Self::B => Self::A,
        }
    }

    const fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::A),
            1 => Some(Self::B),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BootControlState {
    confirmed: SystemSlot,
    pending: Option<SystemSlot>,
    attempts: u8,
    generation: u64,
}

impl BootControlState {
    pub const fn initial() -> Self {
        Self {
            confirmed: SystemSlot::A,
            pending: None,
            attempts: 0,
            generation: 0,
        }
    }

    pub const fn confirmed_slot(self) -> SystemSlot {
        self.confirmed
    }

    pub const fn pending_slot(self) -> Option<SystemSlot> {
        self.pending
    }

    pub const fn attempts(self) -> u8 {
        self.attempts
    }

    pub const fn generation(self) -> u64 {
        self.generation
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BootDecision {
    pub slot: SystemSlot,
    /// One-based attempt count for an update trial; zero for the confirmed slot.
    pub trial_attempt: u8,
    pub rolled_back: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoreError {
    Io,
}

/// Durable storage for two fixed-size control records.
///
/// Implementations must make `flush` persist all preceding writes before it
/// returns success. A write may be interrupted; the inactive copy is written
/// first so a valid prior generation remains available for recovery.
pub trait BootControlStore {
    fn read_record(
        &mut self,
        copy: usize,
        output: &mut [u8; RECORD_SIZE],
    ) -> Result<Option<usize>, StoreError>;
    fn write_record(&mut self, copy: usize, record: &[u8; RECORD_SIZE]) -> Result<(), StoreError>;
    fn flush(&mut self) -> Result<(), StoreError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BootControlError {
    Storage(StoreError),
    CorruptRecords,
    UpdateAlreadyPending,
    CannotUpdateConfirmedSlot,
    UnexpectedBootSuccess,
    GenerationExhausted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RecordCopy {
    First,
    Second,
}

impl RecordCopy {
    const fn index(self) -> usize {
        match self {
            Self::First => 0,
            Self::Second => 1,
        }
    }

    const fn other(self) -> Self {
        match self {
            Self::First => Self::Second,
            Self::Second => Self::First,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct StoredState {
    state: BootControlState,
    copy: RecordCopy,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CurrentState {
    state: BootControlState,
    copy: Option<RecordCopy>,
}

pub struct BootControlJournal<S> {
    store: S,
}

impl<S: BootControlStore> BootControlJournal<S> {
    pub const fn new(store: S) -> Self {
        Self { store }
    }

    pub fn into_store(self) -> S {
        self.store
    }

    /// Return the durable state, defaulting to slot A when no record exists.
    /// Present but wholly invalid records fail closed instead of guessing.
    pub fn load(&mut self) -> Result<BootControlState, BootControlError> {
        Ok(self.current()?.state)
    }

    /// Stage the inactive slot. The slot is selected only by `begin_boot`,
    /// after this pending state and its checksum have been flushed.
    pub fn stage_update(&mut self, slot: SystemSlot) -> Result<(), BootControlError> {
        let mut current = self.current()?;
        if current.state.pending.is_some() {
            return Err(BootControlError::UpdateAlreadyPending);
        }
        if slot == current.state.confirmed {
            return Err(BootControlError::CannotUpdateConfirmedSlot);
        }

        // Establish a durable confirmed baseline before the first pending
        // record, so a torn first update can never erase the known-good slot.
        if current.copy.is_none() {
            self.persist(current, current.state)?;
            current = self.current()?;
        }

        let next = BootControlState {
            pending: Some(slot),
            attempts: 0,
            ..current.state
        };
        self.persist(current, next)
    }

    /// Persist an attempt before returning the pending slot to the caller.
    /// Repeated resets therefore consume the fixed retry budget. Once it is
    /// exhausted, the next boot clears the pending slot and selects the last
    /// confirmed slot.
    pub fn begin_boot(&mut self) -> Result<BootDecision, BootControlError> {
        let current = self.current()?;
        let Some(pending) = current.state.pending else {
            return Ok(BootDecision {
                slot: current.state.confirmed,
                trial_attempt: 0,
                rolled_back: false,
            });
        };

        if current.state.attempts >= MAX_BOOT_ATTEMPTS {
            let next = BootControlState {
                pending: None,
                attempts: 0,
                ..current.state
            };
            self.persist(current, next)?;
            return Ok(BootDecision {
                slot: next.confirmed,
                trial_attempt: 0,
                rolled_back: true,
            });
        }

        let attempt = current.state.attempts + 1;
        let next = BootControlState {
            attempts: attempt,
            ..current.state
        };
        self.persist(current, next)?;
        Ok(BootDecision {
            slot: pending,
            trial_attempt: attempt,
            rolled_back: false,
        })
    }

    /// Confirm a trial only after the caller's defined system-readiness gate.
    pub fn mark_boot_success(&mut self, slot: SystemSlot) -> Result<(), BootControlError> {
        let current = self.current()?;
        match current.state.pending {
            Some(pending) if pending == slot => {
                let next = BootControlState {
                    confirmed: pending,
                    pending: None,
                    attempts: 0,
                    ..current.state
                };
                self.persist(current, next)
            }
            None if current.state.confirmed == slot => Ok(()),
            _ => Err(BootControlError::UnexpectedBootSuccess),
        }
    }

    fn current(&mut self) -> Result<CurrentState, BootControlError> {
        let mut decoded = [None, None];
        let mut present = [false, false];
        for copy in 0..2 {
            let mut bytes = [0; RECORD_SIZE];
            let length = self
                .store
                .read_record(copy, &mut bytes)
                .map_err(BootControlError::Storage)?;
            if let Some(length) = length {
                present[copy] = true;
                if length == RECORD_SIZE {
                    decoded[copy] = decode_record(&bytes);
                }
            }
        }

        let first = decoded[0].map(|state| StoredState {
            state,
            copy: RecordCopy::First,
        });
        let second = decoded[1].map(|state| StoredState {
            state,
            copy: RecordCopy::Second,
        });

        match (first, second) {
            (None, None) if !present[0] && !present[1] => Ok(CurrentState {
                state: BootControlState::initial(),
                copy: None,
            }),
            (None, None) => Err(BootControlError::CorruptRecords),
            (Some(valid), None) | (None, Some(valid)) => Ok(CurrentState {
                state: valid.state,
                copy: Some(valid.copy),
            }),
            (Some(first), Some(second)) => {
                if first.state.generation == second.state.generation {
                    if first.state != second.state {
                        return Err(BootControlError::CorruptRecords);
                    }
                    return Ok(CurrentState {
                        state: first.state,
                        copy: Some(RecordCopy::First),
                    });
                }
                let newest = if first.state.generation > second.state.generation {
                    first
                } else {
                    second
                };
                Ok(CurrentState {
                    state: newest.state,
                    copy: Some(newest.copy),
                })
            }
        }
    }

    fn persist(
        &mut self,
        current: CurrentState,
        mut next: BootControlState,
    ) -> Result<(), BootControlError> {
        next.generation = current
            .state
            .generation
            .checked_add(1)
            .ok_or(BootControlError::GenerationExhausted)?;
        validate_state(next).ok_or(BootControlError::CorruptRecords)?;
        let destination = current
            .copy
            .map(RecordCopy::other)
            .unwrap_or(RecordCopy::First);
        let record = encode_record(next).ok_or(BootControlError::CorruptRecords)?;
        self.store
            .write_record(destination.index(), &record)
            .map_err(BootControlError::Storage)?;
        self.store.flush().map_err(BootControlError::Storage)
    }
}

fn validate_state(state: BootControlState) -> Option<()> {
    if state.attempts > MAX_BOOT_ATTEMPTS
        || state.pending == Some(state.confirmed)
        || (state.pending.is_none() && state.attempts != 0)
    {
        return None;
    }
    Some(())
}

fn encode_record(state: BootControlState) -> Option<[u8; RECORD_SIZE]> {
    validate_state(state)?;
    let mut bytes = [0; RECORD_SIZE];
    bytes[0..4].copy_from_slice(&RECORD_MAGIC);
    bytes[4] = RECORD_VERSION;
    bytes[5] = state.confirmed as u8;
    bytes[6] = state.pending.map_or(NO_PENDING_SLOT, |slot| slot as u8);
    bytes[7] = state.attempts;
    bytes[8] = MAX_BOOT_ATTEMPTS;
    bytes[10..18].copy_from_slice(&state.generation.to_le_bytes());
    let checksum = crc32(&bytes[..CHECKSUM_OFFSET]);
    bytes[CHECKSUM_OFFSET..RECORD_SIZE].copy_from_slice(&checksum.to_le_bytes());
    Some(bytes)
}

fn decode_record(bytes: &[u8; RECORD_SIZE]) -> Option<BootControlState> {
    if bytes[0..4] != RECORD_MAGIC
        || bytes[4] != RECORD_VERSION
        || bytes[8] != MAX_BOOT_ATTEMPTS
        || bytes[9] != 0
        || bytes[18..CHECKSUM_OFFSET].iter().any(|byte| *byte != 0)
    {
        return None;
    }
    let expected = u32::from_le_bytes(bytes[CHECKSUM_OFFSET..].try_into().ok()?);
    if crc32(&bytes[..CHECKSUM_OFFSET]) != expected {
        return None;
    }
    let confirmed = SystemSlot::from_code(bytes[5])?;
    let pending = if bytes[6] == NO_PENDING_SLOT {
        None
    } else {
        Some(SystemSlot::from_code(bytes[6])?)
    };
    let state = BootControlState {
        confirmed,
        pending,
        attempts: bytes[7],
        generation: u64::from_le_bytes(bytes[10..18].try_into().ok()?),
    };
    validate_state(state)?;
    Some(state)
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Clone, Default)]
    struct MemoryStore {
        records: [[u8; RECORD_SIZE]; 2],
        present: [bool; 2],
        fail_next_write_after: Option<usize>,
        flush_count: usize,
    }

    impl BootControlStore for MemoryStore {
        fn read_record(
            &mut self,
            copy: usize,
            output: &mut [u8; RECORD_SIZE],
        ) -> Result<Option<usize>, StoreError> {
            if self.present[copy] {
                *output = self.records[copy];
                Ok(Some(RECORD_SIZE))
            } else {
                Ok(None)
            }
        }

        fn write_record(
            &mut self,
            copy: usize,
            record: &[u8; RECORD_SIZE],
        ) -> Result<(), StoreError> {
            if let Some(prefix) = self.fail_next_write_after.take() {
                self.records[copy][..prefix].copy_from_slice(&record[..prefix]);
                self.present[copy] = true;
                return Err(StoreError::Io);
            }
            self.records[copy] = *record;
            self.present[copy] = true;
            Ok(())
        }

        fn flush(&mut self) -> Result<(), StoreError> {
            self.flush_count += 1;
            Ok(())
        }
    }

    fn journal() -> BootControlJournal<MemoryStore> {
        BootControlJournal::new(MemoryStore::default())
    }

    #[test]
    fn update_attempts_are_persisted_and_exhaustion_rolls_back() {
        let mut control = journal();
        assert_eq!(control.load(), Ok(BootControlState::initial()));
        control.stage_update(SystemSlot::B).expect("stage update");

        for attempt in 1..=MAX_BOOT_ATTEMPTS {
            let mut rebooted = BootControlJournal::new(control.into_store());
            let decision = rebooted.begin_boot().expect("record boot attempt");
            assert_eq!(decision.slot, SystemSlot::B);
            assert_eq!(decision.trial_attempt, attempt);
            assert!(!decision.rolled_back);
            assert_eq!(rebooted.load().expect("reload state").attempts(), attempt);
            control = rebooted;
        }

        let mut rebooted = BootControlJournal::new(control.into_store());
        let decision = rebooted.begin_boot().expect("rollback after retry limit");
        assert_eq!(
            decision,
            BootDecision {
                slot: SystemSlot::A,
                trial_attempt: 0,
                rolled_back: true,
            }
        );
        let state = rebooted.load().expect("rollback state");
        assert_eq!(state.confirmed_slot(), SystemSlot::A);
        assert_eq!(state.pending_slot(), None);
        assert_eq!(state.attempts(), 0);
    }

    #[test]
    fn readiness_success_confirms_the_trial_and_changes_the_fallback() {
        let mut control = journal();
        control.stage_update(SystemSlot::B).expect("stage update");
        let decision = control.begin_boot().expect("begin trial");
        assert_eq!(decision.slot, SystemSlot::B);
        assert_eq!(
            control.mark_boot_success(SystemSlot::A),
            Err(BootControlError::UnexpectedBootSuccess)
        );
        control
            .mark_boot_success(SystemSlot::B)
            .expect("confirm after readiness");

        let state = control.load().expect("confirmed state");
        assert_eq!(state.confirmed_slot(), SystemSlot::B);
        assert_eq!(state.pending_slot(), None);
        let decision = control.begin_boot().expect("normal boot");
        assert_eq!(decision.slot, SystemSlot::B);
        assert_eq!(decision.trial_attempt, 0);
        assert!(!decision.rolled_back);
    }

    #[test]
    fn torn_inactive_write_keeps_the_previous_generation_recoverable() {
        let mut control = journal();
        control.stage_update(SystemSlot::B).expect("stage update");
        let before = control.load().expect("state before boot");
        let mut store = control.into_store();
        store.fail_next_write_after = Some(11);
        let mut interrupted = BootControlJournal::new(store);
        assert_eq!(
            interrupted.begin_boot(),
            Err(BootControlError::Storage(StoreError::Io))
        );

        let mut recovered = BootControlJournal::new(interrupted.into_store());
        assert_eq!(recovered.load(), Ok(before));
        let decision = recovered.begin_boot().expect("retry from valid old record");
        assert_eq!(decision.slot, SystemSlot::B);
        assert_eq!(decision.trial_attempt, 1);
    }

    #[test]
    fn damaged_newest_record_falls_back_to_a_valid_older_generation() {
        let mut control = journal();
        control.stage_update(SystemSlot::B).expect("stage update");
        let first_trial = control.begin_boot().expect("begin trial");
        assert_eq!(first_trial.trial_attempt, 1);
        let mut store = control.into_store();
        let latest = if decode_record(&store.records[0]).is_some() {
            0
        } else {
            1
        };
        store.records[latest][5] ^= 0x40;
        let mut recovered = BootControlJournal::new(store);
        let fallback = recovered.load().expect("older record remains valid");
        assert_eq!(fallback.pending_slot(), Some(SystemSlot::B));
        assert_eq!(fallback.attempts(), 0);
    }

    #[test]
    fn invalid_or_ambiguous_persisted_state_fails_closed() {
        let mut store = MemoryStore::default();
        store.present[0] = true;
        store.records[0][..4].copy_from_slice(&RECORD_MAGIC);
        let mut control = BootControlJournal::new(store);
        assert_eq!(control.load(), Err(BootControlError::CorruptRecords));

        let mut store = MemoryStore::default();
        let first = encode_record(BootControlState::initial()).expect("encode initial");
        let conflicting = BootControlState {
            confirmed: SystemSlot::B,
            ..BootControlState::initial()
        };
        let second = encode_record(conflicting).expect("encode conflicting state");
        store.records = [first, second];
        store.present = [true, true];
        let mut control = BootControlJournal::new(store);
        assert_eq!(control.load(), Err(BootControlError::CorruptRecords));
    }

    #[test]
    fn record_codec_rejects_checksum_and_invariant_corruption() {
        let state = BootControlState {
            pending: Some(SystemSlot::B),
            attempts: 2,
            generation: 0x1020_3040_5060_7080,
            ..BootControlState::initial()
        };
        let mut record = encode_record(state).expect("valid encoding");
        assert_eq!(decode_record(&record), Some(state));
        record[7] = MAX_BOOT_ATTEMPTS + 1;
        let checksum = crc32(&record[..CHECKSUM_OFFSET]);
        record[CHECKSUM_OFFSET..].copy_from_slice(&checksum.to_le_bytes());
        assert_eq!(decode_record(&record), None);
    }
}
