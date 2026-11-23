//! Private Linux development runtime. This module is not a Nagi service, has no
//! default feature, and must not be treated as guest or production acceptance.

use super::{
    DedupeEntry, DedupeScope, InMemoryJobStore, InsertOutcome, JobStore, JobStoreError, StoreInner,
};
use crate::authorization::{CallerRef, JobAuthorization};
use crate::diagnostics::JobDiagnosticSink;
use crate::model::{
    decode_job_record, encode_job_record, JobId, JobRecord, JobRequest, JobState, TimestampMillis,
};
use crate::scheduler::{
    ClockUnavailable, DispatchResult, EnqueueResult, FallibleClock, JobProviderRegistry, Scheduler,
    SchedulerConfig, SchedulerConfigError, SchedulerError,
};
use rustix::fd::OwnedFd;
use rustix::fs::{
    fchmod, flock, openat, renameat, unlinkat, AtFlags, FlockOperation, Mode, OFlags,
};
use rustix::io::Errno;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};

const SNAPSHOT_SCHEMA_VERSION: u16 = 1;
const DEFAULT_MAX_SNAPSHOT_BYTES: usize = 8 * 1024 * 1024;
const HARD_MAX_SNAPSHOT_BYTES: usize = 64 * 1024 * 1024;
const MAX_SNAPSHOT_RECORDS: usize = 262_144;
const CLOCK_DISCONTINUITY_TOLERANCE_NS: u128 = 2_000_000_000;
const SNAPSHOT_NAME: &str = "jobs.snapshot.json";
const SNAPSHOT_TEMP_NAME: &str = "jobs.snapshot.tmp";
const INTENT_NAME: &str = "jobs.pending";
const INTENT_TEMP_NAME: &str = "jobs.pending.tmp";
const LOCK_NAME: &str = "jobs.lock";

/// Fail-closed errors for the non-public development runtime.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DevelopmentOpenError {
    UnsupportedPlatform,
    InvalidDirectory,
    AlreadyOpen,
    ClockUnavailable,
    ClockRollback,
    Corrupt,
    UnsupportedSnapshotVersion,
    CommitUnknown,
    RecoveryRequired,
    Store(JobStoreError),
    Scheduler(SchedulerConfigError),
    StartupRecovery,
    Capacity,
}

impl From<JobStoreError> for DevelopmentOpenError {
    fn from(error: JobStoreError) -> Self {
        Self::Store(error)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum DevelopmentEnqueueError {
    IdempotencyKeyRequired,
    Scheduler(SchedulerError),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ClockReading {
    unix_millis: u128,
    monotonic_nanos: u128,
    boot_id: String,
}

trait ClockSource: Send + Sync {
    fn sample(&self) -> Result<ClockReading, ClockUnavailable>;
}

/// Wall-clock epoch timestamps for persisted deadlines, checked against Linux
/// BOOTTIME so clock changes and time spent suspended are detected. BOOTTIME
/// resets on reboot; snapshot boot identity and startup recovery handle that.
pub(crate) struct RuntimeClock {
    source: Arc<dyn ClockSource>,
    boot_id: String,
    previous: Mutex<ClockReading>,
    failed: AtomicBool,
}

impl RuntimeClock {
    fn new(source: Arc<dyn ClockSource>) -> Result<Self, DevelopmentOpenError> {
        let first = source
            .sample()
            .map_err(|_| DevelopmentOpenError::ClockUnavailable)?;
        validate_boot_id(&first.boot_id)?;
        u64::try_from(first.unix_millis).map_err(|_| DevelopmentOpenError::ClockUnavailable)?;
        Ok(Self {
            source,
            boot_id: first.boot_id.clone(),
            previous: Mutex::new(first),
            failed: AtomicBool::new(false),
        })
    }

    fn system_linux() -> Result<Self, DevelopmentOpenError> {
        if !cfg!(target_os = "linux") {
            return Err(DevelopmentOpenError::UnsupportedPlatform);
        }
        Self::new(Arc::new(LinuxClockSource::new()?))
    }

    fn boot_id(&self) -> &str {
        &self.boot_id
    }

    fn sample_now(&self) -> Result<TimestampMillis, ClockUnavailable> {
        if self.failed.load(Ordering::SeqCst) {
            return Err(ClockUnavailable);
        }
        let mut previous = match self.previous.lock() {
            Ok(value) => value,
            Err(_) => return self.fail(),
        };
        let next = match self.source.sample() {
            Ok(value) => value,
            Err(_) => return self.fail(),
        };
        if next.boot_id != self.boot_id {
            return self.fail();
        }
        let next_wall = match u64::try_from(next.unix_millis) {
            Ok(value) => value,
            Err(_) => return self.fail(),
        };
        let wall_delta_ms = match next.unix_millis.checked_sub(previous.unix_millis) {
            Some(value) => value,
            None => return self.fail(),
        };
        let monotonic_delta = match next.monotonic_nanos.checked_sub(previous.monotonic_nanos) {
            Some(value) => value,
            None => return self.fail(),
        };
        let wall_delta_ns = match wall_delta_ms.checked_mul(1_000_000) {
            Some(value) => value,
            None => return self.fail(),
        };
        if wall_delta_ns.abs_diff(monotonic_delta) > CLOCK_DISCONTINUITY_TOLERANCE_NS {
            return self.fail();
        }
        *previous = next;
        Ok(TimestampMillis(next_wall))
    }

    fn fail<T>(&self) -> Result<T, ClockUnavailable> {
        self.failed.store(true, Ordering::SeqCst);
        Err(ClockUnavailable)
    }
}

impl FallibleClock for RuntimeClock {
    fn try_now(&self) -> Result<TimestampMillis, ClockUnavailable> {
        self.sample_now()
    }
}

struct LinuxClockSource;

impl LinuxClockSource {
    fn new() -> Result<Self, DevelopmentOpenError> {
        #[cfg(target_os = "linux")]
        {
            let _ = read_boot_id()?;
            Ok(Self)
        }
        #[cfg(not(target_os = "linux"))]
        {
            Err(DevelopmentOpenError::UnsupportedPlatform)
        }
    }
}

impl ClockSource for LinuxClockSource {
    fn sample(&self) -> Result<ClockReading, ClockUnavailable> {
        #[cfg(target_os = "linux")]
        {
            use rustix::time::{clock_gettime_dynamic, ClockId, DynamicClockId};

            let realtime = clock_gettime_dynamic(DynamicClockId::Known(ClockId::Realtime))
                .map_err(|_| ClockUnavailable)?;
            let boottime =
                clock_gettime_dynamic(DynamicClockId::Boottime).map_err(|_| ClockUnavailable)?;
            let unix_nanos = timespec_nanos(realtime.tv_sec, realtime.tv_nsec)?;
            let monotonic_nanos = timespec_nanos(boottime.tv_sec, boottime.tv_nsec)?;
            Ok(ClockReading {
                unix_millis: unix_nanos / 1_000_000,
                monotonic_nanos,
                boot_id: read_boot_id().map_err(|_| ClockUnavailable)?,
            })
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = self;
            Err(ClockUnavailable)
        }
    }
}

#[cfg(target_os = "linux")]
fn timespec_nanos(seconds: i64, nanoseconds: i64) -> Result<u128, ClockUnavailable> {
    let seconds = u128::try_from(seconds).map_err(|_| ClockUnavailable)?;
    let nanoseconds = u128::try_from(nanoseconds).map_err(|_| ClockUnavailable)?;
    if nanoseconds >= 1_000_000_000 {
        return Err(ClockUnavailable);
    }
    seconds
        .checked_mul(1_000_000_000)
        .and_then(|value| value.checked_add(nanoseconds))
        .ok_or(ClockUnavailable)
}

fn read_boot_id() -> Result<String, DevelopmentOpenError> {
    let value = fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .map_err(|_| DevelopmentOpenError::ClockUnavailable)?;
    let value = value.trim().to_ascii_lowercase();
    validate_boot_id(&value)?;
    Ok(value)
}

fn validate_boot_id(value: &str) -> Result<(), DevelopmentOpenError> {
    let bytes = value.as_bytes();
    if bytes.len() != 36
        || [8, 13, 18, 23]
            .iter()
            .any(|index| bytes.get(*index) != Some(&b'-'))
        || bytes
            .iter()
            .enumerate()
            .any(|(index, byte)| ![8, 13, 18, 23].contains(&index) && !byte.is_ascii_hexdigit())
    {
        return Err(DevelopmentOpenError::ClockUnavailable);
    }
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedDedupeEntry {
    scope: DedupeScope,
    job_id: JobId,
    retained_until: TimestampMillis,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotPayload {
    schema_version: u16,
    generation: u64,
    boot_id: String,
    last_commit_wall_ms: u64,
    next_sequence: u64,
    records: Vec<String>,
    dedupe: Vec<PersistedDedupeEntry>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SnapshotEnvelope {
    payload: SnapshotPayload,
    sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PendingIntent {
    schema_version: u16,
    generation: u64,
}

struct RuntimeStoreState {
    memory: InMemoryJobStore,
    generation: u64,
    boot_id: String,
    last_commit_wall_ms: u64,
    fenced: bool,
}

/// Versioned, bounded, single-writer development snapshot implementing the
/// existing JobStore contract without changing its public trait or JobRecord.
pub(crate) struct SnapshotJobStore {
    directory: File,
    _lock: File,
    clock: Arc<RuntimeClock>,
    max_bytes: usize,
    state: Mutex<RuntimeStoreState>,
    faults: FaultInjector,
}

impl SnapshotJobStore {
    pub(crate) fn open_linux(
        path: &Path,
        clock: Arc<RuntimeClock>,
    ) -> Result<Self, DevelopmentOpenError> {
        if !cfg!(target_os = "linux") {
            return Err(DevelopmentOpenError::UnsupportedPlatform);
        }
        Self::open_with_clock(
            path,
            clock,
            DEFAULT_MAX_SNAPSHOT_BYTES,
            FaultInjector::default(),
        )
    }

    fn open_with_clock(
        path: &Path,
        clock: Arc<RuntimeClock>,
        max_bytes: usize,
        faults: FaultInjector,
    ) -> Result<Self, DevelopmentOpenError> {
        if !(1024..=HARD_MAX_SNAPSHOT_BYTES).contains(&max_bytes) {
            return Err(DevelopmentOpenError::Capacity);
        }
        let metadata =
            fs::symlink_metadata(path).map_err(|_| DevelopmentOpenError::InvalidDirectory)?;
        if !metadata.file_type().is_dir() || metadata.permissions().mode() & 0o077 != 0 {
            return Err(DevelopmentOpenError::InvalidDirectory);
        }
        let directory = File::open(path).map_err(|_| DevelopmentOpenError::InvalidDirectory)?;
        let _lock = open_lock_file(&directory)?;
        flock(&_lock, FlockOperation::NonBlockingLockExclusive)
            .map_err(|_| DevelopmentOpenError::AlreadyOpen)?;

        if named_file_exists(&directory, INTENT_NAME)?
            || named_file_exists(&directory, INTENT_TEMP_NAME)?
        {
            return Err(DevelopmentOpenError::CommitUnknown);
        }
        if named_file_exists(&directory, SNAPSHOT_TEMP_NAME)? {
            return Err(DevelopmentOpenError::Corrupt);
        }

        let now = clock
            .sample_now()
            .map_err(|_| DevelopmentOpenError::ClockUnavailable)?
            .0;
        let loaded = read_named(&directory, SNAPSHOT_NAME, max_bytes)?;
        let needs_initial_commit = loaded.is_none();
        let (mut payload, memory) = if let Some(bytes) = loaded {
            let payload = decode_snapshot(&bytes)?;
            if now < payload.last_commit_wall_ms {
                return Err(DevelopmentOpenError::ClockRollback);
            }
            let memory = memory_from_payload(&payload)?;
            if has_unresolved_execution(&memory) {
                return Err(DevelopmentOpenError::RecoveryRequired);
            }
            (payload, memory)
        } else {
            (
                SnapshotPayload {
                    schema_version: SNAPSHOT_SCHEMA_VERSION,
                    generation: 0,
                    boot_id: clock.boot_id().to_owned(),
                    last_commit_wall_ms: now,
                    next_sequence: 0,
                    records: Vec::new(),
                    dedupe: Vec::new(),
                },
                StoreInner::default(),
            )
        };

        if needs_initial_commit || payload.boot_id != clock.boot_id() {
            if !needs_initial_commit {
                payload.generation = payload
                    .generation
                    .checked_add(1)
                    .ok_or(DevelopmentOpenError::Capacity)?;
                payload.boot_id = clock.boot_id().to_owned();
                payload.last_commit_wall_ms = now;
                payload =
                    payload_from_inner(&memory, payload.generation, payload.boot_id.clone(), now)?;
            }
            write_snapshot_transaction(&directory, &payload, max_bytes, &faults)?;
        }

        let generation = payload.generation;
        let boot_id = payload.boot_id.clone();
        let last_commit_wall_ms = payload.last_commit_wall_ms;
        Ok(Self {
            directory,
            _lock,
            clock,
            max_bytes,
            state: Mutex::new(RuntimeStoreState {
                memory: InMemoryJobStore {
                    inner: Arc::new(Mutex::new(memory)),
                },
                generation,
                boot_id,
                last_commit_wall_ms,
                fenced: false,
            }),
            faults,
        })
    }

    fn has_unresolved_execution(&self) -> Result<bool, JobStoreError> {
        let state = self.state.lock().map_err(|_| JobStoreError::Unavailable)?;
        if state.fenced {
            return Err(JobStoreError::Unavailable);
        }
        let inner = state
            .memory
            .inner
            .lock()
            .map_err(|_| JobStoreError::Unavailable)?;
        Ok(has_unresolved_execution(&inner))
    }

    fn commit_candidate(
        &self,
        state: &mut RuntimeStoreState,
        candidate: InMemoryJobStore,
    ) -> Result<(), JobStoreError> {
        let now = self
            .clock
            .sample_now()
            .map_err(|_| JobStoreError::Unavailable)?
            .0;
        if now < state.last_commit_wall_ms {
            state.fenced = true;
            return Err(JobStoreError::Unavailable);
        }
        let generation = state
            .generation
            .checked_add(1)
            .ok_or(JobStoreError::Capacity)?;
        let inner = candidate
            .inner
            .lock()
            .map_err(|_| JobStoreError::Unavailable)?
            .clone();
        let payload = payload_from_inner(&inner, generation, state.boot_id.clone(), now)
            .map_err(|_| JobStoreError::Corrupt)?;
        let bytes = encode_snapshot(&payload).map_err(|_| JobStoreError::Corrupt)?;
        if bytes.len() > self.max_bytes {
            return Err(JobStoreError::Capacity);
        }
        match write_snapshot_transaction(&self.directory, &payload, self.max_bytes, &self.faults) {
            Ok(()) => {
                state.memory = candidate;
                state.generation = generation;
                state.last_commit_wall_ms = now;
                Ok(())
            }
            Err(DevelopmentOpenError::Capacity) => Err(JobStoreError::Capacity),
            Err(_) => {
                state.fenced = true;
                Err(JobStoreError::Unavailable)
            }
        }
    }

    fn detached_candidate(memory: &InMemoryJobStore) -> Result<InMemoryJobStore, JobStoreError> {
        let inner = memory
            .inner
            .lock()
            .map_err(|_| JobStoreError::Unavailable)?
            .clone();
        Ok(InMemoryJobStore {
            inner: Arc::new(Mutex::new(inner)),
        })
    }
}

impl JobStore for SnapshotJobStore {
    fn insert(
        &self,
        record: JobRecord,
        dedupe: Option<(DedupeScope, TimestampMillis)>,
        now: TimestampMillis,
        queue_capacity: usize,
        record_capacity: usize,
    ) -> Result<InsertOutcome, JobStoreError> {
        let mut state = self.state.lock().map_err(|_| JobStoreError::Unavailable)?;
        if state.fenced {
            return Err(JobStoreError::Unavailable);
        }
        let sequence_exhausted = state
            .memory
            .inner
            .lock()
            .map_err(|_| JobStoreError::Unavailable)?
            .next_sequence
            == u64::MAX;
        let candidate = Self::detached_candidate(&state.memory)?;
        let outcome = candidate.insert(record, dedupe, now, queue_capacity, record_capacity)?;
        if matches!(outcome, InsertOutcome::Inserted(_)) {
            if sequence_exhausted {
                return Err(JobStoreError::Capacity);
            }
            self.commit_candidate(&mut state, candidate)?;
        }
        Ok(outcome)
    }

    fn get(&self, id: &JobId) -> Result<Option<JobRecord>, JobStoreError> {
        let state = self.state.lock().map_err(|_| JobStoreError::Unavailable)?;
        if state.fenced {
            return Err(JobStoreError::Unavailable);
        }
        state.memory.get(id)
    }

    fn list(&self) -> Result<Vec<JobRecord>, JobStoreError> {
        let state = self.state.lock().map_err(|_| JobStoreError::Unavailable)?;
        if state.fenced {
            return Err(JobStoreError::Unavailable);
        }
        state.memory.list()
    }

    fn compare_and_swap(
        &self,
        id: &JobId,
        expected_revision: u64,
        replacement: JobRecord,
    ) -> Result<JobRecord, JobStoreError> {
        let mut state = self.state.lock().map_err(|_| JobStoreError::Unavailable)?;
        if state.fenced {
            return Err(JobStoreError::Unavailable);
        }
        let candidate = Self::detached_candidate(&state.memory)?;
        let saved = candidate.compare_and_swap(id, expected_revision, replacement)?;
        self.commit_candidate(&mut state, candidate)?;
        Ok(saved)
    }

    fn claim(
        &self,
        id: &JobId,
        expected_revision: u64,
        now: TimestampMillis,
        global_limit: usize,
        per_owner_limit: usize,
    ) -> Result<Option<JobRecord>, JobStoreError> {
        let mut state = self.state.lock().map_err(|_| JobStoreError::Unavailable)?;
        if state.fenced {
            return Err(JobStoreError::Unavailable);
        }
        let candidate = Self::detached_candidate(&state.memory)?;
        let claimed = candidate.claim(id, expected_revision, now, global_limit, per_owner_limit)?;
        if claimed.is_some() {
            self.commit_candidate(&mut state, candidate)?;
        }
        Ok(claimed)
    }
}

/// Internal scheduler wrapper. It recovers before becoming available and
/// requires stable idempotency keys for enqueue retries after unknown replies.
pub(crate) struct LinuxDevelopmentRuntime {
    scheduler: Scheduler,
    store: Arc<SnapshotJobStore>,
    clock: Arc<RuntimeClock>,
}

impl LinuxDevelopmentRuntime {
    pub(crate) fn open_linux(
        path: &Path,
        authorization: Arc<dyn JobAuthorization>,
        diagnostics: Arc<dyn JobDiagnosticSink>,
        providers: Arc<JobProviderRegistry>,
        config: SchedulerConfig,
    ) -> Result<Self, DevelopmentOpenError> {
        if !cfg!(target_os = "linux") {
            return Err(DevelopmentOpenError::UnsupportedPlatform);
        }
        let clock = Arc::new(RuntimeClock::system_linux()?);
        Self::open_with_components(
            path,
            clock,
            authorization,
            diagnostics,
            providers,
            config,
            true,
        )
    }

    fn open_with_components(
        path: &Path,
        clock: Arc<RuntimeClock>,
        authorization: Arc<dyn JobAuthorization>,
        diagnostics: Arc<dyn JobDiagnosticSink>,
        providers: Arc<JobProviderRegistry>,
        config: SchedulerConfig,
        require_linux: bool,
    ) -> Result<Self, DevelopmentOpenError> {
        if require_linux && !cfg!(target_os = "linux") {
            return Err(DevelopmentOpenError::UnsupportedPlatform);
        }
        let store = Arc::new(if require_linux {
            SnapshotJobStore::open_linux(path, clock.clone())?
        } else {
            SnapshotJobStore::open_with_clock(
                path,
                clock.clone(),
                DEFAULT_MAX_SNAPSHOT_BYTES,
                FaultInjector::default(),
            )?
        });
        if store.has_unresolved_execution()? {
            return Err(DevelopmentOpenError::RecoveryRequired);
        }
        let scheduler = Scheduler::new_with_fallible_clock(
            store.clone(),
            clock.clone(),
            authorization,
            diagnostics,
            providers,
            config,
        )
        .map_err(DevelopmentOpenError::Scheduler)?;
        scheduler
            .recover_startup()
            .map_err(|_| DevelopmentOpenError::StartupRecovery)?;
        Ok(Self {
            scheduler,
            store,
            clock,
        })
    }

    pub(crate) fn enqueue_idempotent(
        &self,
        caller: &CallerRef,
        request: JobRequest,
    ) -> Result<EnqueueResult, DevelopmentEnqueueError> {
        if request.idempotency_key().is_none() {
            return Err(DevelopmentEnqueueError::IdempotencyKeyRequired);
        }
        self.scheduler
            .enqueue(caller, request)
            .map_err(DevelopmentEnqueueError::Scheduler)
    }

    pub(crate) fn dispatch_one(&self) -> Result<DispatchResult, SchedulerError> {
        self.scheduler.dispatch_one()
    }

    pub(crate) fn store(&self) -> &Arc<SnapshotJobStore> {
        &self.store
    }

    pub(crate) fn clock(&self) -> &Arc<RuntimeClock> {
        &self.clock
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FaultPoint {
    None = 0,
    AfterIntentDurable = 1,
    PartialSnapshot = 2,
    BeforeSnapshotRename = 3,
    AfterSnapshotRename = 4,
    AfterSnapshotDirectorySync = 5,
    Readback = 6,
    AfterIntentClear = 7,
}

#[derive(Default)]
struct FaultInjector {
    point: AtomicU8,
}

impl FaultInjector {
    fn trip(&self, point: FaultPoint) -> bool {
        self.point
            .compare_exchange(
                point as u8,
                FaultPoint::None as u8,
                Ordering::SeqCst,
                Ordering::SeqCst,
            )
            .is_ok()
    }

    #[cfg(test)]
    fn arm(&self, point: FaultPoint) {
        self.point.store(point as u8, Ordering::SeqCst);
    }
}

fn payload_from_inner(
    inner: &StoreInner,
    generation: u64,
    boot_id: String,
    last_commit_wall_ms: u64,
) -> Result<SnapshotPayload, DevelopmentOpenError> {
    if inner.records.len() > MAX_SNAPSHOT_RECORDS {
        return Err(DevelopmentOpenError::Capacity);
    }
    let mut records = Vec::with_capacity(inner.records.len());
    for record in inner.records.values() {
        let encoded = encode_job_record(record).map_err(|_| DevelopmentOpenError::Corrupt)?;
        records.push(String::from_utf8(encoded).map_err(|_| DevelopmentOpenError::Corrupt)?);
    }
    let mut dedupe = inner
        .dedupe
        .iter()
        .map(|(scope, entry)| PersistedDedupeEntry {
            scope: scope.clone(),
            job_id: entry.job_id.clone(),
            retained_until: entry.retained_until,
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|entry| {
            serde_json::to_vec(&entry.scope)
                .map(|bytes| (bytes, entry))
                .map_err(|_| DevelopmentOpenError::Corrupt)
        })
        .collect::<Result<Vec<_>, _>>()?;
    dedupe.sort_by(|left, right| left.0.cmp(&right.0));
    let dedupe = dedupe.into_iter().map(|(_, entry)| entry).collect();
    Ok(SnapshotPayload {
        schema_version: SNAPSHOT_SCHEMA_VERSION,
        generation,
        boot_id,
        last_commit_wall_ms,
        next_sequence: inner.next_sequence,
        records,
        dedupe,
    })
}

fn memory_from_payload(payload: &SnapshotPayload) -> Result<StoreInner, DevelopmentOpenError> {
    if payload.schema_version != SNAPSHOT_SCHEMA_VERSION {
        return Err(DevelopmentOpenError::UnsupportedSnapshotVersion);
    }
    validate_boot_id(&payload.boot_id)?;
    if payload.records.len() > MAX_SNAPSHOT_RECORDS {
        return Err(DevelopmentOpenError::Corrupt);
    }
    let mut inner = StoreInner::default();
    let mut previous_id: Option<JobId> = None;
    for encoded in &payload.records {
        let record =
            decode_job_record(encoded.as_bytes()).map_err(|_| DevelopmentOpenError::Corrupt)?;
        if encode_job_record(&record).map_err(|_| DevelopmentOpenError::Corrupt)?
            != encoded.as_bytes()
        {
            return Err(DevelopmentOpenError::Corrupt);
        }
        if previous_id
            .as_ref()
            .is_some_and(|previous| previous >= record.id())
        {
            return Err(DevelopmentOpenError::Corrupt);
        }
        previous_id = Some(record.id().clone());
        if record.enqueue_sequence() > payload.next_sequence
            || inner.records.insert(record.id().clone(), record).is_some()
        {
            return Err(DevelopmentOpenError::Corrupt);
        }
    }
    let mut previous_scope: Option<Vec<u8>> = None;
    for saved in &payload.dedupe {
        let encoded_scope =
            serde_json::to_vec(&saved.scope).map_err(|_| DevelopmentOpenError::Corrupt)?;
        if previous_scope
            .as_ref()
            .is_some_and(|previous| previous >= &encoded_scope)
        {
            return Err(DevelopmentOpenError::Corrupt);
        }
        previous_scope = Some(encoded_scope);
        let Some(record) = inner.records.get(&saved.job_id) else {
            return Err(DevelopmentOpenError::Corrupt);
        };
        let expected_scope = record
            .idempotency_key()
            .map(|key| DedupeScope::new(record.owner(), record.handler(), key.clone()));
        if expected_scope.as_ref() != Some(&saved.scope)
            || inner
                .dedupe
                .insert(
                    saved.scope.clone(),
                    DedupeEntry {
                        job_id: saved.job_id.clone(),
                        retained_until: saved.retained_until,
                    },
                )
                .is_some()
        {
            return Err(DevelopmentOpenError::Corrupt);
        }
    }
    inner.next_sequence = payload.next_sequence;
    Ok(inner)
}

fn has_unresolved_execution(inner: &StoreInner) -> bool {
    inner.records.values().any(|record| {
        matches!(
            record.state(),
            JobState::Running
                | JobState::CancelRequested
                | JobState::PauseRequested
                | JobState::Interrupted
        )
    })
}

fn encode_snapshot(payload: &SnapshotPayload) -> Result<Vec<u8>, DevelopmentOpenError> {
    let payload_bytes = serde_json::to_vec(payload).map_err(|_| DevelopmentOpenError::Corrupt)?;
    let digest = Sha256::digest(&payload_bytes);
    let envelope = SnapshotEnvelope {
        payload: payload.clone(),
        sha256: hex_digest(&digest),
    };
    serde_json::to_vec(&envelope).map_err(|_| DevelopmentOpenError::Corrupt)
}

fn decode_snapshot(bytes: &[u8]) -> Result<SnapshotPayload, DevelopmentOpenError> {
    let envelope: SnapshotEnvelope =
        serde_json::from_slice(bytes).map_err(|_| DevelopmentOpenError::Corrupt)?;
    let payload_bytes =
        serde_json::to_vec(&envelope.payload).map_err(|_| DevelopmentOpenError::Corrupt)?;
    let digest = Sha256::digest(&payload_bytes);
    if envelope.sha256 != hex_digest(&digest) || encode_snapshot(&envelope.payload)? != bytes {
        return Err(DevelopmentOpenError::Corrupt);
    }
    if envelope.payload.schema_version != SNAPSHOT_SCHEMA_VERSION {
        return Err(DevelopmentOpenError::UnsupportedSnapshotVersion);
    }
    let _ = memory_from_payload(&envelope.payload)?;
    Ok(envelope.payload)
}

fn hex_digest(digest: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        encoded.push(HEX[(byte >> 4) as usize] as char);
        encoded.push(HEX[(byte & 0x0f) as usize] as char);
    }
    encoded
}

fn write_snapshot_transaction(
    directory: &File,
    payload: &SnapshotPayload,
    max_bytes: usize,
    faults: &FaultInjector,
) -> Result<(), DevelopmentOpenError> {
    let bytes = encode_snapshot(payload)?;
    if bytes.len() > max_bytes || bytes.len() > HARD_MAX_SNAPSHOT_BYTES {
        return Err(DevelopmentOpenError::Capacity);
    }
    let intent = serde_json::to_vec(&PendingIntent {
        schema_version: SNAPSHOT_SCHEMA_VERSION,
        generation: payload.generation,
    })
    .map_err(|_| DevelopmentOpenError::Corrupt)?;
    let mut pending_temp = create_new_at(directory, INTENT_TEMP_NAME)?;
    pending_temp
        .write_all(&intent)
        .and_then(|()| pending_temp.sync_all())
        .map_err(|_| DevelopmentOpenError::CommitUnknown)?;
    renameat(directory, INTENT_TEMP_NAME, directory, INTENT_NAME)
        .map_err(|_| DevelopmentOpenError::CommitUnknown)?;
    directory
        .sync_all()
        .map_err(|_| DevelopmentOpenError::CommitUnknown)?;
    if faults.trip(FaultPoint::AfterIntentDurable) {
        return Err(DevelopmentOpenError::CommitUnknown);
    }

    let mut snapshot_temp = create_new_at(directory, SNAPSHOT_TEMP_NAME)?;
    if faults.trip(FaultPoint::PartialSnapshot) {
        let half = bytes.len() / 2;
        snapshot_temp
            .write_all(&bytes[..half])
            .and_then(|()| snapshot_temp.sync_all())
            .map_err(|_| DevelopmentOpenError::CommitUnknown)?;
        return Err(DevelopmentOpenError::CommitUnknown);
    }
    snapshot_temp
        .write_all(&bytes)
        .and_then(|()| snapshot_temp.sync_all())
        .map_err(|_| DevelopmentOpenError::CommitUnknown)?;
    if faults.trip(FaultPoint::BeforeSnapshotRename) {
        return Err(DevelopmentOpenError::CommitUnknown);
    }
    renameat(directory, SNAPSHOT_TEMP_NAME, directory, SNAPSHOT_NAME)
        .map_err(|_| DevelopmentOpenError::CommitUnknown)?;
    if faults.trip(FaultPoint::AfterSnapshotRename) {
        return Err(DevelopmentOpenError::CommitUnknown);
    }
    directory
        .sync_all()
        .map_err(|_| DevelopmentOpenError::CommitUnknown)?;
    if faults.trip(FaultPoint::AfterSnapshotDirectorySync) {
        return Err(DevelopmentOpenError::CommitUnknown);
    }
    if faults.trip(FaultPoint::Readback) {
        return Err(DevelopmentOpenError::CommitUnknown);
    }
    let readback = match read_named(directory, SNAPSHOT_NAME, max_bytes) {
        Ok(Some(bytes)) => bytes,
        _ => return Err(DevelopmentOpenError::CommitUnknown),
    };
    let decoded = match decode_snapshot(&readback) {
        Ok(payload) => payload,
        Err(_) => return Err(DevelopmentOpenError::CommitUnknown),
    };
    if decoded != *payload || readback != bytes {
        return Err(DevelopmentOpenError::CommitUnknown);
    }
    unlinkat(directory, INTENT_NAME, AtFlags::empty())
        .map_err(|_| DevelopmentOpenError::CommitUnknown)?;
    directory
        .sync_all()
        .map_err(|_| DevelopmentOpenError::CommitUnknown)?;
    if faults.trip(FaultPoint::AfterIntentClear) {
        return Err(DevelopmentOpenError::CommitUnknown);
    }
    Ok(())
}

fn open_lock_file(directory: &File) -> Result<File, DevelopmentOpenError> {
    let fd = openat(
        directory,
        LOCK_NAME,
        OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )
    .map_err(|_| DevelopmentOpenError::InvalidDirectory)?;
    let file = File::from(fd);
    if !file
        .metadata()
        .map_err(|_| DevelopmentOpenError::InvalidDirectory)?
        .is_file()
    {
        return Err(DevelopmentOpenError::InvalidDirectory);
    }
    fchmod(&file, Mode::from_raw_mode(0o600))
        .map_err(|_| DevelopmentOpenError::InvalidDirectory)?;
    Ok(file)
}

fn create_new_at(directory: &File, name: &str) -> Result<File, DevelopmentOpenError> {
    let fd = openat(
        directory,
        name,
        OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::from_raw_mode(0o600),
    )
    .map_err(|_| DevelopmentOpenError::CommitUnknown)?;
    Ok(File::from(fd))
}

fn named_file_exists(directory: &File, name: &str) -> Result<bool, DevelopmentOpenError> {
    match openat(
        directory,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    ) {
        Ok(fd) => {
            let file = File::from(fd);
            if !file
                .metadata()
                .map_err(|_| DevelopmentOpenError::Corrupt)?
                .is_file()
            {
                return Err(DevelopmentOpenError::Corrupt);
            }
            Ok(true)
        }
        Err(error) if error == Errno::NOENT => Ok(false),
        Err(_) => Err(DevelopmentOpenError::Corrupt),
    }
}

fn read_named(
    directory: &File,
    name: &str,
    max_bytes: usize,
) -> Result<Option<Vec<u8>>, DevelopmentOpenError> {
    let fd: OwnedFd = match openat(
        directory,
        name,
        OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
        Mode::empty(),
    ) {
        Ok(fd) => fd,
        Err(error) if error == Errno::NOENT => return Ok(None),
        Err(_) => return Err(DevelopmentOpenError::Corrupt),
    };
    let file = File::from(fd);
    if !file
        .metadata()
        .map_err(|_| DevelopmentOpenError::Corrupt)?
        .is_file()
    {
        return Err(DevelopmentOpenError::Corrupt);
    }
    let limit = max_bytes
        .checked_add(1)
        .ok_or(DevelopmentOpenError::Capacity)? as u64;
    let mut bytes = Vec::new();
    file.take(limit)
        .read_to_end(&mut bytes)
        .map_err(|_| DevelopmentOpenError::Corrupt)?;
    if bytes.len() > max_bytes {
        return Err(DevelopmentOpenError::Capacity);
    }
    Ok(Some(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authorization::{
        AuthorizationError, CallerRef, JobAction, JobOwner, OwnerProfileRef, PrincipalRef,
    };
    use crate::diagnostics::NoopJobDiagnosticSink;
    use crate::model::{HandlerRef, IdempotencyKey, OpaqueRef, RecoveryPolicy};
    use crate::scheduler::{HandlerContext, HandlerOutcome, JobHandler};
    use std::collections::VecDeque;
    use std::sync::atomic::AtomicU64;

    static TEST_DIRECTORY_ID: AtomicU64 = AtomicU64::new(1);

    struct TestDirectory(std::path::PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let id = TEST_DIRECTORY_ID.fetch_add(1, Ordering::SeqCst);
            let path =
                std::env::temp_dir().join(format!("nagi-job-snapshot-{}-{id}", std::process::id()));
            fs::create_dir(&path).expect("create private test directory");
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
                .expect("restrict private test directory");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[derive(Default)]
    struct ScriptedClockSource {
        values: Mutex<VecDeque<ClockReading>>,
        last: Mutex<Option<ClockReading>>,
    }

    impl ScriptedClockSource {
        fn new(values: impl IntoIterator<Item = ClockReading>) -> Self {
            Self {
                values: Mutex::new(values.into_iter().collect()),
                last: Mutex::new(None),
            }
        }
    }

    impl ClockSource for ScriptedClockSource {
        fn sample(&self) -> Result<ClockReading, ClockUnavailable> {
            if let Some(value) = self
                .values
                .lock()
                .map_err(|_| ClockUnavailable)?
                .pop_front()
            {
                *self.last.lock().map_err(|_| ClockUnavailable)? = Some(value.clone());
                return Ok(value);
            }
            self.last
                .lock()
                .map_err(|_| ClockUnavailable)?
                .clone()
                .ok_or(ClockUnavailable)
        }
    }

    struct UnavailableClockSource;

    impl ClockSource for UnavailableClockSource {
        fn sample(&self) -> Result<ClockReading, ClockUnavailable> {
            Err(ClockUnavailable)
        }
    }

    fn reading(wall_ms: u128, monotonic_nanos: u128, boot_id: &str) -> ClockReading {
        ClockReading {
            unix_millis: wall_ms,
            monotonic_nanos,
            boot_id: boot_id.to_owned(),
        }
    }

    fn runtime_clock(values: impl IntoIterator<Item = ClockReading>) -> Arc<RuntimeClock> {
        Arc::new(RuntimeClock::new(Arc::new(ScriptedClockSource::new(values))).unwrap())
    }

    fn stable_clock(boot_id: &str) -> Arc<RuntimeClock> {
        runtime_clock([
            reading(1_000, 0, boot_id),
            reading(1_000, 0, boot_id),
            reading(1_000, 0, boot_id),
            reading(1_000, 0, boot_id),
        ])
    }

    fn owner() -> JobOwner {
        JobOwner::from_authenticated_adapter(
            PrincipalRef::new("user.test").unwrap(),
            Some(OwnerProfileRef::new("profile.test").unwrap()),
        )
    }

    fn handler() -> HandlerRef {
        HandlerRef::new(
            OpaqueRef::new("provider.test").unwrap(),
            OpaqueRef::new("handler.test").unwrap(),
            1,
        )
        .unwrap()
    }

    fn record(id: &str, key: Option<&str>) -> JobRecord {
        let mut request = JobRequest::new(handler());
        if let Some(key) = key {
            request = request.with_idempotency_key(IdempotencyKey::new(key).unwrap());
        }
        JobRecord::new(
            JobId::new(id).unwrap(),
            owner(),
            request,
            TimestampMillis(1_000),
        )
    }

    fn open_store(directory: &TestDirectory, clock: Arc<RuntimeClock>) -> SnapshotJobStore {
        SnapshotJobStore::open_with_clock(
            &directory.0,
            clock,
            DEFAULT_MAX_SNAPSHOT_BYTES,
            FaultInjector::default(),
        )
        .unwrap()
    }

    fn open_error(result: Result<SnapshotJobStore, DevelopmentOpenError>) -> DevelopmentOpenError {
        match result {
            Err(error) => error,
            Ok(_) => panic!("expected development store open failure"),
        }
    }

    fn with_faults(store: SnapshotJobStore, faults: FaultInjector) -> SnapshotJobStore {
        SnapshotJobStore {
            directory: store.directory,
            _lock: store._lock,
            clock: store.clock,
            max_bytes: store.max_bytes,
            state: store.state,
            faults,
        }
    }

    fn insert_one(store: &SnapshotJobStore, id: &str, key: &str) -> JobRecord {
        let record = record(id, Some(key));
        let scope = DedupeScope::new(
            record.owner(),
            record.handler(),
            IdempotencyKey::new(key).unwrap(),
        );
        match store
            .insert(
                record,
                Some((scope, TimestampMillis(10_000))),
                TimestampMillis(1_000),
                32,
                128,
            )
            .unwrap()
        {
            InsertOutcome::Inserted(saved) => saved,
            InsertOutcome::Existing(_) => panic!("unexpected duplicate"),
        }
    }

    #[test]
    fn snapshot_reopens_records_and_idempotent_duplicate_returns_existing() {
        let directory = TestDirectory::new();
        let store = open_store(
            &directory,
            stable_clock("00000000-0000-0000-0000-000000000001"),
        );
        let first = insert_one(&store, "job-first", "request-1");
        drop(store);

        let reopened = open_store(
            &directory,
            stable_clock("00000000-0000-0000-0000-000000000001"),
        );
        assert_eq!(reopened.get(first.id()).unwrap(), Some(first.clone()));
        let scope = DedupeScope::new(
            first.owner(),
            first.handler(),
            IdempotencyKey::new("request-1").unwrap(),
        );
        let duplicate = reopened
            .insert(
                record("job-second", Some("request-1")),
                Some((scope, TimestampMillis(10_000))),
                TimestampMillis(1_001),
                32,
                128,
            )
            .unwrap();
        assert_eq!(duplicate, InsertOutcome::Existing(first));
        assert_eq!(reopened.list().unwrap().len(), 1);
    }

    #[test]
    fn open_rejects_valid_unsupported_snapshot_version_without_overwriting_it() {
        let directory = TestDirectory::new();
        let store = open_store(
            &directory,
            stable_clock("00000000-0000-0000-0000-000000000001"),
        );
        insert_one(&store, "job-versioned", "versioned-key");
        drop(store);

        let reopened = open_store(
            &directory,
            stable_clock("00000000-0000-0000-0000-000000000001"),
        );
        assert!(reopened
            .get(&JobId::new("job-versioned").unwrap())
            .unwrap()
            .is_some());
        drop(reopened);

        let snapshot_path = directory.0.join(SNAPSHOT_NAME);
        let valid_bytes = fs::read(&snapshot_path).unwrap();
        let valid_envelope: SnapshotEnvelope = serde_json::from_slice(&valid_bytes).unwrap();
        assert_eq!(
            valid_envelope.payload.schema_version,
            SNAPSHOT_SCHEMA_VERSION
        );
        assert_eq!(valid_envelope.payload.records.len(), 1);

        let mut unsupported_payload = valid_envelope.payload.clone();
        unsupported_payload.schema_version = SNAPSHOT_SCHEMA_VERSION + 1;
        let mut payload_with_only_version_reverted = unsupported_payload.clone();
        payload_with_only_version_reverted.schema_version = SNAPSHOT_SCHEMA_VERSION;
        assert_eq!(payload_with_only_version_reverted, valid_envelope.payload);

        // Re-encode so the envelope has a valid digest and canonical JSON. The
        // open path must reach the unsupported-version branch, not corruption.
        let unsupported_bytes = encode_snapshot(&unsupported_payload).unwrap();
        fs::write(&snapshot_path, &unsupported_bytes).unwrap();

        assert_eq!(
            open_error(SnapshotJobStore::open_with_clock(
                &directory.0,
                stable_clock("00000000-0000-0000-0000-000000000001"),
                DEFAULT_MAX_SNAPSHOT_BYTES,
                FaultInjector::default(),
            )),
            DevelopmentOpenError::UnsupportedSnapshotVersion
        );
        assert_eq!(fs::read(&snapshot_path).unwrap(), unsupported_bytes);
    }

    #[test]
    fn terminal_dedupe_entry_expires_only_after_the_retention_boundary() {
        let directory = TestDirectory::new();
        let store = open_store(
            &directory,
            stable_clock("00000000-0000-0000-0000-000000000001"),
        );
        let queued = insert_one(&store, "job-expiry", "expiry-key");
        let mut terminal = queued.clone();
        terminal
            .set_state(
                JobState::Failed(crate::model::JobFailureCode::Permanent),
                TimestampMillis(1_005),
            )
            .unwrap();
        terminal.mark_finished(TimestampMillis(1_005));
        terminal.set_revision(queued.revision() + 1);
        store
            .compare_and_swap(queued.id(), queued.revision(), terminal.clone())
            .unwrap();

        let scope = DedupeScope::new(
            terminal.owner(),
            terminal.handler(),
            IdempotencyKey::new("expiry-key").unwrap(),
        );
        let boundary = store
            .insert(
                record("job-at-boundary", Some("expiry-key")),
                Some((scope.clone(), TimestampMillis(20_000))),
                TimestampMillis(10_000),
                32,
                128,
            )
            .unwrap();
        assert_eq!(boundary, InsertOutcome::Existing(terminal));

        let after = store
            .insert(
                record("job-after-boundary", Some("expiry-key")),
                Some((scope, TimestampMillis(20_000))),
                TimestampMillis(10_001),
                32,
                128,
            )
            .unwrap();
        assert!(matches!(after, InsertOutcome::Inserted(_)));
    }

    #[test]
    fn file_lock_rejects_a_second_writer() {
        let directory = TestDirectory::new();
        let clock = stable_clock("00000000-0000-0000-0000-000000000001");
        let store = open_store(&directory, clock.clone());
        assert_eq!(
            open_error(SnapshotJobStore::open_with_clock(
                &directory.0,
                clock,
                DEFAULT_MAX_SNAPSHOT_BYTES,
                FaultInjector::default(),
            )),
            DevelopmentOpenError::AlreadyOpen
        );
        drop(store);
    }

    #[test]
    fn a_running_record_after_restart_fences_dispatch() {
        let directory = TestDirectory::new();
        let store = open_store(
            &directory,
            stable_clock("00000000-0000-0000-0000-000000000001"),
        );
        let queued = insert_one(&store, "job-running", "request-running");
        store
            .claim(queued.id(), queued.revision(), TimestampMillis(1_001), 1, 1)
            .unwrap()
            .expect("claim is committed");
        drop(store);

        let error = open_error(SnapshotJobStore::open_with_clock(
            &directory.0,
            stable_clock("00000000-0000-0000-0000-000000000002"),
            DEFAULT_MAX_SNAPSHOT_BYTES,
            FaultInjector::default(),
        ));
        assert_eq!(error, DevelopmentOpenError::RecoveryRequired);
    }

    #[test]
    fn failures_after_intent_never_publish_candidate_state_and_reopen_fences() {
        for point in [
            FaultPoint::AfterIntentDurable,
            FaultPoint::PartialSnapshot,
            FaultPoint::BeforeSnapshotRename,
            FaultPoint::AfterSnapshotRename,
            FaultPoint::AfterSnapshotDirectorySync,
            FaultPoint::Readback,
        ] {
            let directory = TestDirectory::new();
            let clock = stable_clock("00000000-0000-0000-0000-000000000001");
            let store = open_store(&directory, clock.clone());
            let faults = FaultInjector::default();
            faults.arm(point);
            let store = SnapshotJobStore {
                directory: store.directory,
                _lock: store._lock,
                clock: store.clock,
                max_bytes: store.max_bytes,
                state: store.state,
                faults,
            };
            assert_eq!(
                store
                    .insert(
                        record("job-fault", Some("fault-key")),
                        Some((
                            DedupeScope::new(
                                &owner(),
                                &handler(),
                                IdempotencyKey::new("fault-key").unwrap(),
                            ),
                            TimestampMillis(10_000),
                        )),
                        TimestampMillis(1_000),
                        32,
                        128,
                    )
                    .unwrap_err(),
                JobStoreError::Unavailable,
                "fault point {point:?}"
            );
            assert!(store.list().is_err(), "fenced at {point:?}");
            drop(store);
            assert_eq!(
                open_error(SnapshotJobStore::open_with_clock(
                    &directory.0,
                    clock,
                    DEFAULT_MAX_SNAPSHOT_BYTES,
                    FaultInjector::default(),
                )),
                DevelopmentOpenError::CommitUnknown,
                "reopen must fence at {point:?}"
            );
        }
    }

    #[test]
    fn failure_after_durable_intent_clear_reopens_the_committed_idempotent_record() {
        let directory = TestDirectory::new();
        let clock = stable_clock("00000000-0000-0000-0000-000000000001");
        let store = open_store(&directory, clock.clone());
        let faults = FaultInjector::default();
        faults.arm(FaultPoint::AfterIntentClear);
        let store = SnapshotJobStore {
            directory: store.directory,
            _lock: store._lock,
            clock: store.clock,
            max_bytes: store.max_bytes,
            state: store.state,
            faults,
        };
        let scope = DedupeScope::new(
            &owner(),
            &handler(),
            IdempotencyKey::new("uncertain-key").unwrap(),
        );
        assert_eq!(
            store
                .insert(
                    record("job-uncertain", Some("uncertain-key")),
                    Some((scope.clone(), TimestampMillis(10_000))),
                    TimestampMillis(1_000),
                    32,
                    128,
                )
                .unwrap_err(),
            JobStoreError::Unavailable
        );
        drop(store);
        let reopened = open_store(&directory, clock);
        let duplicate = reopened
            .insert(
                record("job-retried", Some("uncertain-key")),
                Some((scope, TimestampMillis(10_000))),
                TimestampMillis(1_001),
                32,
                128,
            )
            .unwrap();
        assert!(matches!(duplicate, InsertOutcome::Existing(_)));
        assert_eq!(reopened.list().unwrap().len(), 1);
    }

    #[test]
    fn unknown_claim_reply_never_restarts_a_handler_after_reopen() {
        let directory = TestDirectory::new();
        let clock = stable_clock("00000000-0000-0000-0000-000000000001");
        let store = open_store(&directory, clock.clone());
        let queued = insert_one(&store, "job-claim-unknown", "claim-unknown-key");
        let faults = FaultInjector::default();
        faults.arm(FaultPoint::AfterIntentClear);
        let store = with_faults(store, faults);
        assert_eq!(
            store
                .claim(queued.id(), queued.revision(), TimestampMillis(1_001), 1, 1)
                .unwrap_err(),
            JobStoreError::Unavailable
        );
        drop(store);

        assert_eq!(
            open_error(SnapshotJobStore::open_with_clock(
                &directory.0,
                clock,
                DEFAULT_MAX_SNAPSHOT_BYTES,
                FaultInjector::default(),
            )),
            DevelopmentOpenError::RecoveryRequired
        );
    }

    #[test]
    fn corruption_and_oversized_snapshot_fail_closed() {
        let directory = TestDirectory::new();
        let store = open_store(
            &directory,
            stable_clock("00000000-0000-0000-0000-000000000001"),
        );
        insert_one(&store, "job-corrupt", "key-corrupt");
        drop(store);
        let mut bytes = fs::read(directory.0.join(SNAPSHOT_NAME)).unwrap();
        let last = bytes.len() - 2;
        bytes[last] ^= 1;
        fs::write(directory.0.join(SNAPSHOT_NAME), bytes).unwrap();
        assert_eq!(
            open_error(SnapshotJobStore::open_with_clock(
                &directory.0,
                stable_clock("00000000-0000-0000-0000-000000000001"),
                DEFAULT_MAX_SNAPSHOT_BYTES,
                FaultInjector::default(),
            )),
            DevelopmentOpenError::Corrupt
        );

        let other = TestDirectory::new();
        let store = open_store(&other, stable_clock("00000000-0000-0000-0000-000000000001"));
        drop(store);
        fs::write(other.0.join(SNAPSHOT_NAME), vec![b'x'; 1_025]).unwrap();
        assert_eq!(
            open_error(SnapshotJobStore::open_with_clock(
                &other.0,
                stable_clock("00000000-0000-0000-0000-000000000001"),
                1024,
                FaultInjector::default(),
            )),
            DevelopmentOpenError::Capacity
        );
    }

    #[test]
    fn snapshot_capacity_is_rejected_before_creating_a_pending_transaction() {
        let directory = TestDirectory::new();
        let store = open_store(
            &directory,
            stable_clock("00000000-0000-0000-0000-000000000001"),
        );
        let oversized = SnapshotPayload {
            schema_version: SNAPSHOT_SCHEMA_VERSION,
            generation: 1,
            boot_id: "00000000-0000-0000-0000-000000000001".to_owned(),
            last_commit_wall_ms: 1_000,
            next_sequence: 0,
            records: vec!["x".repeat(2_000)],
            dedupe: Vec::new(),
        };
        assert_eq!(
            write_snapshot_transaction(
                &store.directory,
                &oversized,
                1_024,
                &FaultInjector::default(),
            ),
            Err(DevelopmentOpenError::Capacity)
        );
        assert!(!named_file_exists(&store.directory, INTENT_NAME).unwrap());
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn clock_source_error_and_store_sequence_generation_overflow_fail_closed() {
        assert_eq!(
            RuntimeClock::new(Arc::new(UnavailableClockSource)).err(),
            Some(DevelopmentOpenError::ClockUnavailable)
        );

        let directory = TestDirectory::new();
        let store = open_store(
            &directory,
            stable_clock("00000000-0000-0000-0000-000000000001"),
        );
        store.state.lock().unwrap().generation = u64::MAX;
        assert_eq!(
            store
                .insert(
                    record("job-generation-overflow", Some("generation-key")),
                    None,
                    TimestampMillis(1_000),
                    32,
                    128,
                )
                .unwrap_err(),
            JobStoreError::Capacity
        );

        {
            let mut state = store.state.lock().unwrap();
            state.generation = 0;
            state.memory.inner.lock().unwrap().next_sequence = u64::MAX;
        }
        assert_eq!(
            store
                .insert(
                    record("job-sequence-overflow", Some("sequence-key")),
                    None,
                    TimestampMillis(1_000),
                    32,
                    128,
                )
                .unwrap_err(),
            JobStoreError::Capacity
        );
        assert!(!named_file_exists(&store.directory, INTENT_NAME).unwrap());
        assert!(store.list().unwrap().is_empty());
    }

    #[test]
    fn clock_rejects_overflow_backward_jump_forward_jump_and_boot_change() {
        let overflow = RuntimeClock::new(Arc::new(ScriptedClockSource::new([reading(
            u64::MAX as u128 + 1,
            0,
            "00000000-0000-0000-0000-000000000001",
        )])));
        assert_eq!(overflow.err(), Some(DevelopmentOpenError::ClockUnavailable));

        for values in [
            vec![
                reading(1_000, 0, "00000000-0000-0000-0000-000000000001"),
                reading(999, 1_000_000, "00000000-0000-0000-0000-000000000001"),
            ],
            vec![
                reading(1_000, 0, "00000000-0000-0000-0000-000000000001"),
                reading(5_000, 1_000_000, "00000000-0000-0000-0000-000000000001"),
            ],
            vec![
                reading(1_000, 0, "00000000-0000-0000-0000-000000000001"),
                reading(1_001, 1_000_000, "00000000-0000-0000-0000-000000000002"),
            ],
        ] {
            let source = Arc::new(ScriptedClockSource::new(values));
            let clock = RuntimeClock::new(source).unwrap();
            assert!(clock.try_now().is_err());
            assert!(clock.try_now().is_err(), "clock failure remains fenced");
        }
    }

    #[test]
    fn boot_change_with_clean_snapshot_is_recorded_and_clock_rollback_is_rejected() {
        let directory = TestDirectory::new();
        let original = stable_clock("00000000-0000-0000-0000-000000000001");
        let store = open_store(&directory, original);
        drop(store);

        let changed_boot = stable_clock("00000000-0000-0000-0000-000000000002");
        let reopened = open_store(&directory, changed_boot);
        let state = reopened.state.lock().unwrap();
        assert_eq!(state.boot_id, "00000000-0000-0000-0000-000000000002");
        assert_eq!(state.generation, 1);
        drop(state);
        drop(reopened);

        let rollback_clock = runtime_clock([
            reading(900, 0, "00000000-0000-0000-0000-000000000002"),
            reading(900, 0, "00000000-0000-0000-0000-000000000002"),
        ]);
        assert_eq!(
            open_error(SnapshotJobStore::open_with_clock(
                &directory.0,
                rollback_clock,
                DEFAULT_MAX_SNAPSHOT_BYTES,
                FaultInjector::default(),
            )),
            DevelopmentOpenError::ClockRollback
        );
    }

    #[derive(Default)]
    struct AllowAuthority;

    impl JobAuthorization for AllowAuthority {
        fn owner_for(&self, caller: &CallerRef) -> Result<JobOwner, AuthorizationError> {
            Ok(JobOwner::from_authenticated_adapter(
                caller.principal().clone(),
                caller.profile().cloned(),
            ))
        }

        fn authorize_user(
            &self,
            _caller: &CallerRef,
            _owner: &JobOwner,
            _action: JobAction,
            _job_id: Option<&JobId>,
        ) -> Result<(), AuthorizationError> {
            Ok(())
        }

        fn authorize_priority(
            &self,
            _caller: &CallerRef,
            _owner: &JobOwner,
            _handler: &HandlerRef,
            _priority: crate::model::JobPriority,
        ) -> Result<(), AuthorizationError> {
            Ok(())
        }

        fn authorize_handler(
            &self,
            _owner: &JobOwner,
            _handler: &HandlerRef,
        ) -> Result<(), AuthorizationError> {
            Ok(())
        }
    }

    struct CountingHandler {
        calls: AtomicU64,
    }

    impl JobHandler for CountingHandler {
        fn handler_ref(&self) -> HandlerRef {
            handler()
        }

        fn recovery_policy(&self, _record: &JobRecord) -> RecoveryPolicy {
            RecoveryPolicy::NonIdempotent
        }

        fn execute(&self, _context: &mut HandlerContext) -> HandlerOutcome {
            self.calls.fetch_add(1, Ordering::SeqCst);
            HandlerOutcome::Succeeded
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_runtime_uses_system_clock_and_persists_idempotent_execution() {
        let directory = TestDirectory::new();
        let handler_calls = Arc::new(CountingHandler {
            calls: AtomicU64::new(0),
        });
        let providers = Arc::new(JobProviderRegistry::new());
        providers.register(handler_calls.clone()).unwrap();
        let runtime = LinuxDevelopmentRuntime::open_linux(
            &directory.0,
            Arc::new(AllowAuthority),
            Arc::new(NoopJobDiagnosticSink),
            providers,
            SchedulerConfig::default(),
        )
        .unwrap();
        let system_time_before = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        let runtime_clock_sample = runtime.clock().try_now().unwrap();
        let system_time_after = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis();
        assert!((system_time_before..=system_time_after)
            .contains(&u128::from(runtime_clock_sample.get())));

        let caller =
            CallerRef::from_authenticated_adapter(PrincipalRef::new("user.test").unwrap(), None);
        let request = JobRequest::new(handler())
            .with_idempotency_key(IdempotencyKey::new("linux-runtime-request").unwrap());
        let first = runtime
            .enqueue_idempotent(&caller, request.clone())
            .unwrap();
        assert!(!first.deduplicated);
        let duplicate = runtime
            .enqueue_idempotent(&caller, request.clone())
            .unwrap();
        assert!(duplicate.deduplicated);
        assert_eq!(first.record.id(), duplicate.record.id());
        assert!(matches!(
            runtime.dispatch_one().unwrap(),
            DispatchResult::Completed(_)
        ));
        assert_eq!(handler_calls.calls.load(Ordering::SeqCst), 1);
        drop(runtime);

        let providers = Arc::new(JobProviderRegistry::new());
        providers.register(handler_calls.clone()).unwrap();
        let reopened = LinuxDevelopmentRuntime::open_linux(
            &directory.0,
            Arc::new(AllowAuthority),
            Arc::new(NoopJobDiagnosticSink),
            providers,
            SchedulerConfig::default(),
        )
        .unwrap();
        let reopened_records = reopened.store().list().unwrap();
        assert_eq!(reopened_records.len(), 1);
        assert_eq!(reopened_records[0].id(), first.record.id());
        assert_eq!(reopened_records[0].state(), &JobState::Succeeded);
        let after_restart_duplicate = reopened.enqueue_idempotent(&caller, request).unwrap();
        assert!(after_restart_duplicate.deduplicated);
        assert_eq!(after_restart_duplicate.record.id(), first.record.id());
        assert_eq!(handler_calls.calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn runtime_requires_idempotency_key_and_keeps_authorization_injected() {
        let directory = TestDirectory::new();
        let clock = stable_clock("00000000-0000-0000-0000-000000000001");
        let handler_calls = Arc::new(CountingHandler {
            calls: AtomicU64::new(0),
        });
        let providers = Arc::new(JobProviderRegistry::new());
        providers.register(handler_calls.clone()).unwrap();
        let runtime = LinuxDevelopmentRuntime::open_with_components(
            &directory.0,
            clock.clone(),
            Arc::new(AllowAuthority),
            Arc::new(NoopJobDiagnosticSink),
            providers,
            SchedulerConfig::default(),
            false,
        )
        .unwrap();
        let caller =
            CallerRef::from_authenticated_adapter(PrincipalRef::new("user.test").unwrap(), None);
        let missing = runtime.enqueue_idempotent(&caller, JobRequest::new(handler()));
        assert_eq!(
            missing,
            Err(DevelopmentEnqueueError::IdempotencyKeyRequired)
        );
        let request = JobRequest::new(handler())
            .with_idempotency_key(IdempotencyKey::new("runtime-request").unwrap());
        let first = runtime
            .enqueue_idempotent(&caller, request.clone())
            .unwrap();
        assert!(!first.deduplicated);
        let duplicate = runtime.enqueue_idempotent(&caller, request).unwrap();
        assert!(duplicate.deduplicated);
        assert_eq!(first.record.id(), duplicate.record.id());
        assert_eq!(runtime.store().list().unwrap().len(), 1);
        assert_eq!(runtime.clock().try_now().unwrap(), TimestampMillis(1_000));
        assert!(matches!(
            runtime.dispatch_one().unwrap(),
            DispatchResult::Completed(_)
        ));
        assert_eq!(handler_calls.calls.load(Ordering::SeqCst), 1);
        drop(runtime);

        let providers = Arc::new(JobProviderRegistry::new());
        providers.register(handler_calls.clone()).unwrap();
        let reopened = LinuxDevelopmentRuntime::open_with_components(
            &directory.0,
            clock,
            Arc::new(AllowAuthority),
            Arc::new(NoopJobDiagnosticSink),
            providers,
            SchedulerConfig::default(),
            false,
        )
        .unwrap();
        assert_eq!(
            reopened.store().list().unwrap()[0].state(),
            &JobState::Succeeded
        );
        assert_eq!(handler_calls.calls.load(Ordering::SeqCst), 1);
    }
}
