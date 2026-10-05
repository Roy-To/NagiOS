//! Kernel-owned M27 candidate-readiness persistence and the ADR 0062
//! update staging request.

use core::sync::atomic::{AtomicU64, AtomicU8, Ordering};

use nagi_bootinfo::{BootControlInfo, BootReadyRecord, BootStageRecord};

const UNCONFIGURED: u8 = 0;
const NOT_REPORTED: u8 = 1;
const IN_PROGRESS: u8 = 2;
const PERSISTED: u8 = 3;
const FAILED: u8 = 4;
const NO_TRIAL: u8 = 5;

const UEFI_VARIABLE_ATTRIBUTES: u32 = 0x1 | 0x2 | 0x4;
const NAGI_BOOT_CONTROL_VENDOR: EfiGuid = EfiGuid {
    data1: 0xc00c_1ca4,
    data2: 0x6d7c,
    data3: 0x4a36,
    data4: [0xb9, 0x13, 0xa2, 0x2f, 0x69, 0x1c, 0x48, 0x95],
};
const NAGI_BOOT_READY_VARIABLE: [u16; 14] = [
    78, 97, 103, 105, 66, 111, 111, 116, 82, 101, 97, 100, 121, 0,
];
/// `NagiBootStage`, consumed by the loader on the next boot.
const NAGI_BOOT_STAGE_VARIABLE: [u16; 14] = [
    78, 97, 103, 105, 66, 111, 111, 116, 83, 116, 97, 103, 101, 0,
];

#[repr(C)]
#[derive(Clone, Copy)]
struct EfiGuid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

type EfiSetVariable =
    unsafe extern "efiapi" fn(*const u16, *const EfiGuid, u32, usize, *const u8) -> usize;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BootReadinessOutcome {
    NoTrial,
    Persisted(BootReadyRecord),
    AlreadyPersisted(BootReadyRecord),
    PersistenceFailed(BootReadyRecord),
    InProgress,
}

/// One-shot gate configured only from the loader-owned BootInfo.
pub struct BootReadinessGate {
    set_variable_address: AtomicU64,
    journal_generation: AtomicU64,
    slot: AtomicU8,
    attempt: AtomicU8,
    state: AtomicU8,
}

impl BootReadinessGate {
    pub const fn new() -> Self {
        Self {
            set_variable_address: AtomicU64::new(0),
            journal_generation: AtomicU64::new(0),
            slot: AtomicU8::new(0),
            attempt: AtomicU8::new(0),
            state: AtomicU8::new(UNCONFIGURED),
        }
    }

    pub fn configure(&self, context: BootControlInfo) {
        self.set_variable_address
            .store(context.set_variable_address, Ordering::Relaxed);
        self.journal_generation
            .store(context.journal_generation, Ordering::Relaxed);
        self.slot.store(context.slot, Ordering::Relaxed);
        self.attempt.store(context.attempt, Ordering::Relaxed);
        self.state.store(
            if context.is_trial() {
                NOT_REPORTED
            } else {
                NO_TRIAL
            },
            Ordering::Release,
        );
    }

    /// Request readiness without accepting any slot or journal coordinates
    /// from user space. `persist` exists so the one-shot policy can be tested
    /// independently of UEFI firmware.
    pub fn report_with(
        &self,
        persist: impl FnOnce(BootReadyRecord) -> bool,
    ) -> BootReadinessOutcome {
        let state = self.state.load(Ordering::Acquire);
        if state == UNCONFIGURED || state == NO_TRIAL {
            return BootReadinessOutcome::NoTrial;
        }
        let record = self.record();
        match self.state.compare_exchange(
            NOT_REPORTED,
            IN_PROGRESS,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                let persisted = persist(record);
                self.state.store(
                    if persisted { PERSISTED } else { FAILED },
                    Ordering::Release,
                );
                if persisted {
                    BootReadinessOutcome::Persisted(record)
                } else {
                    BootReadinessOutcome::PersistenceFailed(record)
                }
            }
            Err(PERSISTED) => BootReadinessOutcome::AlreadyPersisted(record),
            Err(FAILED) => BootReadinessOutcome::PersistenceFailed(record),
            Err(IN_PROGRESS) => BootReadinessOutcome::InProgress,
            Err(_) => BootReadinessOutcome::NoTrial,
        }
    }

    pub fn report(&self) -> BootReadinessOutcome {
        let context = self.context();
        self.report_with(|record| unsafe { persist_readiness(context, record) })
    }

    fn context(&self) -> BootControlInfo {
        BootControlInfo {
            set_variable_address: self.set_variable_address.load(Ordering::Relaxed),
            journal_generation: self.journal_generation.load(Ordering::Relaxed),
            slot: self.slot.load(Ordering::Relaxed),
            attempt: self.attempt.load(Ordering::Relaxed),
            flags: 0,
            reserved: [0; 5],
        }
    }

    fn record(&self) -> BootReadyRecord {
        let context = self.context();
        BootReadyRecord {
            slot: context.slot,
            attempt: context.attempt,
            journal_generation: context.journal_generation,
        }
    }
}

impl Default for BootReadinessGate {
    fn default() -> Self {
        Self::new()
    }
}

static BOOT_READINESS: BootReadinessGate = BootReadinessGate::new();

pub fn initialize(context: BootControlInfo) {
    BOOT_READINESS.configure(context);
    initialize_stage(context);
}

pub fn report() -> BootReadinessOutcome {
    BOOT_READINESS.report()
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BootStageOutcome {
    /// This boot is not a confirmed-slot boot that may stage an update.
    Unavailable,
    Staged(BootStageRecord),
    AlreadyStaged(BootStageRecord),
    Failed,
}

const STAGE_UNAVAILABLE: u8 = 0;
const STAGE_READY: u8 = 1;
const STAGE_IN_PROGRESS: u8 = 2;
const STAGE_DONE: u8 = 3;
const STAGE_FAILED: u8 = 4;

/// One-shot gate for the update staging request (ADR 0062). Like readiness,
/// every coordinate comes from the loader's BootInfo, never from user space:
/// the target is always the slot that is not confirmed.
pub struct BootStageGate {
    set_variable_address: AtomicU64,
    journal_generation: AtomicU64,
    confirmed_slot: AtomicU8,
    state: AtomicU8,
}

impl BootStageGate {
    pub const fn new() -> Self {
        Self {
            set_variable_address: AtomicU64::new(0),
            journal_generation: AtomicU64::new(0),
            confirmed_slot: AtomicU8::new(0),
            state: AtomicU8::new(STAGE_UNAVAILABLE),
        }
    }

    pub fn configure(&self, context: BootControlInfo) {
        self.set_variable_address
            .store(context.set_variable_address, Ordering::Relaxed);
        self.journal_generation
            .store(context.journal_generation, Ordering::Relaxed);
        self.confirmed_slot.store(context.slot, Ordering::Relaxed);
        self.state.store(
            if context.is_update_stageable() {
                STAGE_READY
            } else {
                STAGE_UNAVAILABLE
            },
            Ordering::Release,
        );
    }

    /// The slot an update may be written to, if this boot allows one.
    pub fn target_slot(&self) -> Option<u8> {
        (self.state.load(Ordering::Acquire) != STAGE_UNAVAILABLE)
            .then(|| 1 - self.confirmed_slot.load(Ordering::Relaxed).min(1))
    }

    fn record(&self) -> Option<BootStageRecord> {
        Some(BootStageRecord {
            slot: self.target_slot()?,
            journal_generation: self.journal_generation.load(Ordering::Relaxed),
        })
    }

    pub fn stage_with(&self, persist: impl FnOnce(BootStageRecord) -> bool) -> BootStageOutcome {
        let Some(record) = self.record() else {
            return BootStageOutcome::Unavailable;
        };
        match self.state.compare_exchange(
            STAGE_READY,
            STAGE_IN_PROGRESS,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {
                let persisted = persist(record);
                self.state.store(
                    if persisted { STAGE_DONE } else { STAGE_FAILED },
                    Ordering::Release,
                );
                if persisted {
                    BootStageOutcome::Staged(record)
                } else {
                    BootStageOutcome::Failed
                }
            }
            Err(STAGE_DONE) => BootStageOutcome::AlreadyStaged(record),
            Err(_) => BootStageOutcome::Failed,
        }
    }

    pub fn stage(&self) -> BootStageOutcome {
        let address = self.set_variable_address.load(Ordering::Relaxed);
        self.stage_with(|record| match record.encode() {
            // SAFETY: the address was validated as UEFI runtime code by the
            // kernel entry before this gate was configured stageable.
            Some(bytes) => unsafe { set_variable(address, &NAGI_BOOT_STAGE_VARIABLE, &bytes) },
            None => false,
        })
    }
}

impl Default for BootStageGate {
    fn default() -> Self {
        Self::new()
    }
}

static BOOT_STAGE: BootStageGate = BootStageGate::new();

pub fn initialize_stage(context: BootControlInfo) {
    BOOT_STAGE.configure(context);
}

/// The inactive system slot (0 = A, 1 = B) this boot may update.
pub fn update_target_slot() -> Option<u8> {
    BOOT_STAGE.target_slot()
}

pub fn stage_update() -> BootStageOutcome {
    BOOT_STAGE.stage()
}

/// Call firmware `SetVariable` for one of Nagi's private variables.
///
/// # Safety
///
/// `address` must be the validated UEFI runtime `SetVariable` entry point.
unsafe fn set_variable(address: u64, name: &[u16; 14], bytes: &[u8]) -> bool {
    let Ok(address) = usize::try_from(address) else {
        return false;
    };
    if address == 0 {
        return false;
    }
    // SAFETY: required by this function's contract.
    let set_variable: EfiSetVariable = unsafe { core::mem::transmute(address) };
    // SAFETY: the name is a fixed private variable and `bytes` is bounded.
    unsafe {
        set_variable(
            name.as_ptr(),
            &NAGI_BOOT_CONTROL_VENDOR,
            UEFI_VARIABLE_ATTRIBUTES,
            bytes.len(),
            bytes.as_ptr(),
        ) == 0
    }
}

unsafe fn persist_readiness(context: BootControlInfo, record: BootReadyRecord) -> bool {
    let Some(bytes) = record.encode() else {
        return false;
    };
    if !context.is_trial() {
        return false;
    }
    let Ok(address) = usize::try_from(context.set_variable_address) else {
        return false;
    };
    // SAFETY: the loader put this firmware entry point in trusted BootInfo;
    // the kernel validated it against a UEFI Runtime Services Code descriptor
    // before configuring this gate.
    let set_variable: EfiSetVariable = unsafe { core::mem::transmute(address) };
    // SAFETY: the arguments are fixed to Nagi's private variable, the bounded
    // encoded record, and UEFI's required nonvolatile/runtime attributes.
    unsafe {
        set_variable(
            NAGI_BOOT_READY_VARIABLE.as_ptr(),
            &NAGI_BOOT_CONTROL_VENDOR,
            UEFI_VARIABLE_ATTRIBUTES,
            bytes.len(),
            bytes.as_ptr(),
        ) == 0
    }
}

#[cfg(test)]
mod tests {
    use core::sync::atomic::{AtomicUsize, Ordering};

    use super::{BootReadinessGate, BootReadinessOutcome, BootStageGate, BootStageOutcome};
    use nagi_bootinfo::{
        BootControlInfo, BootReadyRecord, BootStageRecord, BOOT_CONTROL_UPDATE_STAGEABLE,
    };

    fn confirmed_context(slot: u8) -> BootControlInfo {
        BootControlInfo {
            set_variable_address: 0x1234,
            journal_generation: 7,
            slot,
            attempt: 0,
            flags: BOOT_CONTROL_UPDATE_STAGEABLE,
            reserved: [0; 5],
        }
    }

    #[test]
    fn staging_targets_the_inactive_slot_once() {
        let gate = BootStageGate::new();
        gate.configure(confirmed_context(0));
        assert_eq!(gate.target_slot(), Some(1));
        let expected = BootStageRecord {
            slot: 1,
            journal_generation: 7,
        };
        let calls = AtomicUsize::new(0);
        assert_eq!(
            gate.stage_with(|record| {
                calls.fetch_add(1, Ordering::Relaxed);
                assert_eq!(record, expected);
                true
            }),
            BootStageOutcome::Staged(expected)
        );
        assert_eq!(
            gate.stage_with(|_| panic!("must not stage twice")),
            BootStageOutcome::AlreadyStaged(expected)
        );
        assert_eq!(calls.load(Ordering::Relaxed), 1);

        gate.configure(confirmed_context(1));
        assert_eq!(gate.target_slot(), Some(0));
    }

    #[test]
    fn trials_and_ordinary_boots_cannot_stage() {
        let gate = BootStageGate::new();
        assert_eq!(gate.target_slot(), None);
        gate.configure(trial_context());
        assert_eq!(gate.target_slot(), None);
        assert_eq!(
            gate.stage_with(|_| panic!("trial must not stage")),
            BootStageOutcome::Unavailable
        );
        gate.configure(BootControlInfo::default());
        assert_eq!(gate.stage_with(|_| true), BootStageOutcome::Unavailable);
        // A failed write is final for this boot.
        gate.configure(confirmed_context(0));
        assert_eq!(gate.stage_with(|_| false), BootStageOutcome::Failed);
        assert_eq!(gate.stage_with(|_| true), BootStageOutcome::Failed);
    }

    fn trial_context() -> BootControlInfo {
        BootControlInfo {
            set_variable_address: 0x1234,
            journal_generation: 42,
            slot: 1,
            attempt: 2,
            flags: 0,
            reserved: [0; 5],
        }
    }

    #[test]
    fn ready_request_derives_slot_and_generation_from_boot_context_once() {
        let gate = BootReadinessGate::new();
        gate.configure(trial_context());
        let calls = AtomicUsize::new(0);
        let expected = BootReadyRecord {
            slot: 1,
            attempt: 2,
            journal_generation: 42,
        };
        assert_eq!(
            gate.report_with(|record| {
                calls.fetch_add(1, Ordering::Relaxed);
                assert_eq!(record, expected);
                true
            }),
            BootReadinessOutcome::Persisted(expected)
        );
        assert_eq!(
            gate.report_with(|_| panic!("must not persist twice")),
            BootReadinessOutcome::AlreadyPersisted(expected)
        );
        assert_eq!(calls.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn ordinary_boot_does_not_write_a_success_record() {
        let gate = BootReadinessGate::new();
        gate.configure(BootControlInfo::default());
        assert_eq!(
            gate.report_with(|_| panic!("ordinary boot has no trial")),
            BootReadinessOutcome::NoTrial
        );
    }

    #[test]
    fn failed_persistence_is_fail_closed_and_one_shot() {
        let gate = BootReadinessGate::new();
        gate.configure(trial_context());
        let record = BootReadyRecord {
            slot: 1,
            attempt: 2,
            journal_generation: 42,
        };
        assert_eq!(
            gate.report_with(|_| false),
            BootReadinessOutcome::PersistenceFailed(record)
        );
        assert_eq!(
            gate.report_with(|_| panic!("failed one-shot must not retry")),
            BootReadinessOutcome::PersistenceFailed(record)
        );
    }
}
