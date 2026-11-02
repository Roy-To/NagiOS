//! Append-only receipts for opt-in long-running CLI runs.
//!
//! A missing terminal receipt is never reconstructed from logs or guest markers.
//! If the recorded CLI PID is gone, the result is inconclusive; SIGKILL and power
//! loss can prevent the terminal receipt from being written.
//! A signal that terminates the process before terminal.json is published
//! can leave only started.json; no handler infers a command result.
//! Publication uses a same-directory hard link for create-only semantics.
//! Filesystems without hard-link support fail closed rather than falling back to
//! a weaker overwrite-prone publication method.
//! Opt in through NAGI_RUN_RECEIPT_DIR, NAGI_RUN_ID, and NAGI_RUN_STAGE.
//! The environment settings must be consumed at single-threaded CLI startup so
//! nested CLI processes do not accidentally reuse the parent's receipt ID.

use std::env;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

const RECEIPT_DIR_ENV: &str = "NAGI_RUN_RECEIPT_DIR";
const RUN_ID_ENV: &str = "NAGI_RUN_ID";
const RUN_STAGE_ENV: &str = "NAGI_RUN_STAGE";

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunReceiptConfig {
    directory: PathBuf,
    run_id: String,
    stage: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReconnectionState {
    Terminal {
        command_result: i32,
        source: SourceIdentity,
    },
    Inconclusive {
        reason: InconclusiveReason,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ReceiptWriteError {
    NotPublished { path: PathBuf, reason: String },
    PublishedDurabilityUnknown { path: PathBuf, reason: String },
}

impl fmt::Display for ReceiptWriteError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotPublished { path, reason } => write!(
                formatter,
                "receipt not published at {}: {reason}",
                path.display()
            ),
            Self::PublishedDurabilityUnknown { path, reason } => write!(
                formatter,
                "receipt published at {}, but directory durability is unknown: {reason}",
                path.display()
            ),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum InconclusiveReason {
    MissingStartedReceipt,
    InvalidStartedReceipt,
    StartedReceiptMismatch,
    MissingTerminalProcessAbsent,
    ProcessStateUnknown,
    InvalidTerminalReceipt,
    TerminalReceiptUnreadable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerminalObservation {
    Missing,
    RecordedCommandResult {
        command_result: i32,
        source: SourceIdentity,
    },
    Invalid,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProcessObservation {
    Present,
    Absent,
    Unknown,
}

/// Source state captured once at run start and repeated in the terminal record.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SourceIdentity {
    pub head: String,
    pub worktree_dirty: bool,
}

/// The run a reconnecting caller expects to find in a receipt directory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpectedRun {
    run_id: String,
    stage: String,
    source_head: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct StartedRecord {
    run_id: String,
    source: SourceIdentity,
    stage: String,
    pid: u32,
    started_unix_nanos: u128,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TerminalRecord {
    run_id: String,
    source: SourceIdentity,
    stage: String,
    pid: u32,
    finished_unix_nanos: u128,
    command_result: i32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StartedRun {
    directory: PathBuf,
    record: StartedRecord,
}

impl ExpectedRun {
    pub fn new(
        run_id: impl Into<String>,
        stage: impl Into<String>,
        source_head: impl Into<String>,
    ) -> Result<Self, String> {
        let run_id = run_id.into();
        let stage = stage.into();
        let source_head = source_head.into();
        validate_label("run id", &run_id)?;
        validate_label("stage", &stage)?;
        validate_source_head(&source_head)?;
        Ok(Self {
            run_id,
            stage,
            source_head,
        })
    }

    fn matches_started(&self, started: &StartedRecord) -> bool {
        self.run_id == started.run_id
            && self.stage == started.stage
            && self.source_head == started.source.head
    }
}

impl RunReceiptConfig {
    /// Enable receipt writing only when all three opt-in variables are present.
    /// The directory may be absolute or relative to the repository root.
    fn from_env() -> Result<Option<Self>, String> {
        let directory = env::var_os(RECEIPT_DIR_ENV);
        let run_id = env::var_os(RUN_ID_ENV);
        let stage = env::var_os(RUN_STAGE_ENV);

        if directory.is_none() && run_id.is_none() && stage.is_none() {
            return Ok(None);
        }
        let (Some(directory), Some(run_id), Some(stage)) = (directory, run_id, stage) else {
            return Err(format!(
                "set {RECEIPT_DIR_ENV}, {RUN_ID_ENV}, and {RUN_STAGE_ENV} together"
            ));
        };
        let run_id = run_id
            .into_string()
            .map_err(|_| format!("{RUN_ID_ENV} must be UTF-8"))?;
        let stage = stage
            .into_string()
            .map_err(|_| format!("{RUN_STAGE_ENV} must be UTF-8"))?;
        Self::new(PathBuf::from(directory), run_id, stage).map(Some)
    }

    /// Read the opt-in settings once, then remove them from this process's
    /// environment before commands can spawn children. Call only during
    /// single-threaded CLI startup; the returned config remains available to
    /// write this process's own receipts.
    pub fn take_from_env_at_startup() -> Result<Option<Self>, String> {
        let config = Self::from_env();
        env::remove_var(RECEIPT_DIR_ENV);
        env::remove_var(RUN_ID_ENV);
        env::remove_var(RUN_STAGE_ENV);
        config
    }

    pub fn new(
        directory: PathBuf,
        run_id: impl Into<String>,
        stage: impl Into<String>,
    ) -> Result<Self, String> {
        let run_id = run_id.into();
        let stage = stage.into();
        if directory.as_os_str().is_empty() {
            return Err("run receipt directory must not be empty".to_owned());
        }
        validate_label("run id", &run_id)?;
        validate_label("stage", &stage)?;
        Ok(Self {
            directory,
            run_id,
            stage,
        })
    }

    /// Persist started.json before running the command. A pre-existing receipt
    /// is an error; a run ID is never silently reused or overwritten.
    pub fn start(&self, repository_root: &Path) -> Result<StartedRun, String> {
        let source = capture_source_identity(repository_root)?;
        let directory = if self.directory.is_absolute() {
            self.directory.clone()
        } else {
            repository_root.join(&self.directory)
        };
        Self {
            directory,
            run_id: self.run_id.clone(),
            stage: self.stage.clone(),
        }
        .begin(source)
    }

    fn begin(&self, source: SourceIdentity) -> Result<StartedRun, String> {
        let record = StartedRecord {
            run_id: self.run_id.clone(),
            source,
            stage: self.stage.clone(),
            pid: std::process::id(),
            started_unix_nanos: unix_nanos(),
        };
        fs::create_dir_all(&self.directory).map_err(|error| {
            format!(
                "cannot create run receipt directory {}: {error}",
                self.directory.display()
            )
        })?;
        let started_path = self.directory.join("started.json");
        atomic_create_new(&started_path, &record.to_json()).map_err(|error| error.to_string())?;
        Ok(StartedRun {
            directory: self.directory.clone(),
            record,
        })
    }
}

impl StartedRun {
    /// Save the command result code that main immediately passes to
    /// process::exit. This cannot recover an OS wait status after abrupt death.
    pub fn finish(&self, command_result: i32) -> Result<(), ReceiptWriteError> {
        let record = TerminalRecord {
            run_id: self.record.run_id.clone(),
            source: self.record.source.clone(),
            stage: self.record.stage.clone(),
            pid: self.record.pid,
            finished_unix_nanos: unix_nanos(),
            command_result,
        };
        let terminal_path = self.directory.join("terminal.json");
        atomic_create_new(&terminal_path, &record.to_json())
    }
}

/// Read receipts without changing them. The started receipt must name the
/// expected run ID, stage and source revision; any mismatch is inconclusive.
/// A matching terminal receipt wins and reports the recorded source metadata.
/// Without one, a missing terminal plus an absent PID is inconclusive, never a
/// guessed pass or a guessed exit code. PID presence by itself does not prove
/// that the process is the original run.
pub fn inspect_reconnection(directory: &Path, expected: &ExpectedRun) -> ReconnectionState {
    let started_path = directory.join("started.json");
    let started_text = match fs::read_to_string(&started_path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return ReconnectionState::Inconclusive {
                reason: InconclusiveReason::MissingStartedReceipt,
            }
        }
        Err(_) => {
            return ReconnectionState::Inconclusive {
                reason: InconclusiveReason::InvalidStartedReceipt,
            }
        }
    };
    let started = match StartedRecord::parse(&started_text) {
        Some(record) => record,
        None => {
            return ReconnectionState::Inconclusive {
                reason: InconclusiveReason::InvalidStartedReceipt,
            }
        }
    };
    if !expected.matches_started(&started) {
        return ReconnectionState::Inconclusive {
            reason: InconclusiveReason::StartedReceiptMismatch,
        };
    }

    let terminal_path = directory.join("terminal.json");
    match fs::read_to_string(&terminal_path) {
        Ok(text) => match TerminalRecord::parse(&text) {
            Some(terminal) if terminal.matches_started(&started) => {
                return ReconnectionState::Terminal {
                    command_result: terminal.command_result,
                    source: terminal.source,
                }
            }
            _ => {
                return ReconnectionState::Inconclusive {
                    reason: InconclusiveReason::InvalidTerminalReceipt,
                }
            }
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(_) => {
            return ReconnectionState::Inconclusive {
                reason: InconclusiveReason::TerminalReceiptUnreadable,
            }
        }
    }

    match process_is_present(started.pid) {
        Ok(false) => ReconnectionState::Inconclusive {
            reason: InconclusiveReason::MissingTerminalProcessAbsent,
        },
        Ok(true) => ReconnectionState::Inconclusive {
            reason: InconclusiveReason::ProcessStateUnknown,
        },
        Err(()) => ReconnectionState::Inconclusive {
            reason: InconclusiveReason::ProcessStateUnknown,
        },
    }
}

/// Pure classifier also used by host tests and reconnect callers with an
/// externally obtained process observation.
pub fn classify_reconnection(
    terminal: TerminalObservation,
    process: ProcessObservation,
) -> ReconnectionState {
    match terminal {
        TerminalObservation::RecordedCommandResult {
            command_result,
            source,
        } => ReconnectionState::Terminal {
            command_result,
            source,
        },
        TerminalObservation::Invalid => ReconnectionState::Inconclusive {
            reason: InconclusiveReason::InvalidTerminalReceipt,
        },
        TerminalObservation::Missing => match process {
            ProcessObservation::Present | ProcessObservation::Unknown => {
                ReconnectionState::Inconclusive {
                    reason: InconclusiveReason::ProcessStateUnknown,
                }
            }
            ProcessObservation::Absent => ReconnectionState::Inconclusive {
                reason: InconclusiveReason::MissingTerminalProcessAbsent,
            },
        },
    }
}

fn capture_source_identity(repository_root: &Path) -> Result<SourceIdentity, String> {
    let head = Command::new("git")
        .arg("-C")
        .arg(repository_root)
        .args(["rev-parse", "--verify", "HEAD"])
        .output()
        .map_err(|error| format!("cannot query source revision: {error}"))?;
    if !head.status.success() {
        return Err("cannot query source revision for run receipt".to_owned());
    }
    let head = String::from_utf8(head.stdout)
        .map_err(|_| "source revision is not UTF-8".to_owned())?
        .trim()
        .to_owned();
    validate_source_head(&head)?;

    let status = Command::new("git")
        .arg("-C")
        .arg(repository_root)
        .args(["status", "--porcelain", "--untracked-files=normal"])
        .output()
        .map_err(|error| format!("cannot query source worktree state: {error}"))?;
    if !status.status.success() {
        return Err("cannot query source worktree state for run receipt".to_owned());
    }
    Ok(SourceIdentity {
        head,
        worktree_dirty: !status.stdout.is_empty(),
    })
}

fn validate_label(name: &str, value: &str) -> Result<(), String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(format!(
            "run receipt {name} must contain 1-128 ASCII letters, digits, '.', '_' or '-'"
        ));
    }
    Ok(())
}

fn validate_source_head(value: &str) -> Result<(), String> {
    if !matches!(value.len(), 40 | 64) || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err("source revision is not a 40- or 64-digit hexadecimal commit id".to_owned());
    }
    Ok(())
}

fn unix_nanos() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

fn atomic_create_new(path: &Path, contents: &str) -> Result<(), ReceiptWriteError> {
    atomic_create_new_with_sync(path, contents, sync_parent_directory)
}

fn atomic_create_new_with_sync(
    path: &Path,
    contents: &str,
    sync_parent: impl FnOnce(&Path) -> io::Result<()>,
) -> Result<(), ReceiptWriteError> {
    let parent = path
        .parent()
        .ok_or_else(|| not_published(path, "receipt has no parent"))?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| not_published(path, "receipt name is not UTF-8"))?;
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temporary = parent.join(format!(
        ".{file_name}.tmp.{}.{}.{}",
        std::process::id(),
        unix_nanos(),
        sequence
    ));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| not_published(path, error))?;
    if let Err(error) = file
        .write_all(contents.as_bytes())
        .and_then(|()| file.sync_all())
    {
        drop(file);
        let _ = fs::remove_file(&temporary);
        return Err(not_published(path, error));
    }
    drop(file);

    if let Err(error) = fs::hard_link(&temporary, path) {
        let _ = fs::remove_file(&temporary);
        return Err(not_published(path, error));
    }
    fs::remove_file(&temporary).map_err(|error| published_durability_unknown(path, error))?;
    sync_parent(parent).map_err(|error| published_durability_unknown(path, error))
}

fn not_published(path: &Path, reason: impl fmt::Display) -> ReceiptWriteError {
    ReceiptWriteError::NotPublished {
        path: path.to_path_buf(),
        reason: reason.to_string(),
    }
}

fn published_durability_unknown(path: &Path, reason: impl fmt::Display) -> ReceiptWriteError {
    ReceiptWriteError::PublishedDurabilityUnknown {
        path: path.to_path_buf(),
        reason: reason.to_string(),
    }
}

#[cfg(unix)]
fn sync_parent_directory(parent: &Path) -> io::Result<()> {
    File::open(parent)?.sync_all()
}

#[cfg(not(unix))]
fn sync_parent_directory(_parent: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn process_is_present(pid: u32) -> Result<bool, ()> {
    use std::os::raw::c_int;

    if pid == 0 || pid > c_int::MAX as u32 {
        return Err(());
    }
    extern "C" {
        fn kill(pid: c_int, signal: c_int) -> c_int;
    }
    // Signal 0 checks existence/permission without sending a signal.
    let result = unsafe { kill(pid as c_int, 0) };
    if result == 0 {
        return Ok(true);
    }
    let error = io::Error::last_os_error();
    match error.raw_os_error() {
        // ESRCH and EPERM have these POSIX values on the supported macOS host
        // and common Unix targets. Permission denial still means the PID is
        // present; every other failure remains unknown.
        Some(3) => Ok(false),
        Some(1) => Ok(true),
        _ => match error.kind() {
            io::ErrorKind::NotFound => Ok(false),
            io::ErrorKind::PermissionDenied => Ok(true),
            _ => Err(()),
        },
    }
}

#[cfg(not(unix))]
fn process_is_present(_pid: u32) -> Result<bool, ()> {
    Err(())
}

impl StartedRecord {
    fn to_json(&self) -> String {
        format!(
            "{{\n  \"schema_version\": 2,\n  \"record\": \"started\",\n  \"run_id\": \"{}\",\n  \"source_head\": \"{}\",\n  \"source_worktree_dirty\": {},\n  \"stage\": \"{}\",\n  \"pid\": {},\n  \"started_unix_nanos\": {}\n}}\n",
            self.run_id,
            self.source.head,
            self.source.worktree_dirty,
            self.stage,
            self.pid,
            self.started_unix_nanos
        )
    }

    fn parse(json: &str) -> Option<Self> {
        if number_field(json, "schema_version")? != 2 || string_field(json, "record")? != "started"
        {
            return None;
        }
        let run_id = string_field(json, "run_id")?.to_owned();
        let source_head = string_field(json, "source_head")?.to_owned();
        let source_worktree_dirty = bool_field(json, "source_worktree_dirty")?;
        let stage = string_field(json, "stage")?.to_owned();
        let pid = u32::try_from(number_field(json, "pid")?).ok()?;
        let started_unix_nanos = u128::try_from(number_field(json, "started_unix_nanos")?).ok()?;
        validate_label("run id", &run_id).ok()?;
        validate_label("stage", &stage).ok()?;
        validate_source_head(&source_head).ok()?;
        (pid > 0).then_some(())?;
        let record = Self {
            run_id,
            source: SourceIdentity {
                head: source_head,
                worktree_dirty: source_worktree_dirty,
            },
            stage,
            pid,
            started_unix_nanos,
        };
        (record.to_json() == json).then_some(record)
    }
}

impl TerminalRecord {
    fn to_json(&self) -> String {
        format!(
            "{{\n  \"schema_version\": 2,\n  \"record\": \"terminal\",\n  \"run_id\": \"{}\",\n  \"source_head\": \"{}\",\n  \"source_worktree_dirty\": {},\n  \"stage\": \"{}\",\n  \"pid\": {},\n  \"finished_unix_nanos\": {},\n  \"command_result\": {}\n}}\n",
            self.run_id,
            self.source.head,
            self.source.worktree_dirty,
            self.stage,
            self.pid,
            self.finished_unix_nanos,
            self.command_result
        )
    }

    fn parse(json: &str) -> Option<Self> {
        if number_field(json, "schema_version")? != 2 || string_field(json, "record")? != "terminal"
        {
            return None;
        }
        let run_id = string_field(json, "run_id")?.to_owned();
        let source_head = string_field(json, "source_head")?.to_owned();
        let source_worktree_dirty = bool_field(json, "source_worktree_dirty")?;
        let stage = string_field(json, "stage")?.to_owned();
        let pid = u32::try_from(number_field(json, "pid")?).ok()?;
        let finished_unix_nanos =
            u128::try_from(number_field(json, "finished_unix_nanos")?).ok()?;
        let command_result = i32::try_from(number_field(json, "command_result")?).ok()?;
        validate_label("run id", &run_id).ok()?;
        validate_label("stage", &stage).ok()?;
        validate_source_head(&source_head).ok()?;
        (pid > 0).then_some(())?;
        let record = Self {
            run_id,
            source: SourceIdentity {
                head: source_head,
                worktree_dirty: source_worktree_dirty,
            },
            stage,
            pid,
            finished_unix_nanos,
            command_result,
        };
        (record.to_json() == json).then_some(record)
    }

    fn matches_started(&self, started: &StartedRecord) -> bool {
        self.run_id == started.run_id
            && self.source == started.source
            && self.stage == started.stage
            && self.pid == started.pid
            && self.finished_unix_nanos >= started.started_unix_nanos
    }
}

fn field_value<'a>(json: &'a str, name: &str) -> Option<&'a str> {
    let needle = format!("\"{name}\": ");
    let mut matches = json.match_indices(&needle);
    let (offset, _) = matches.next()?;
    if matches.next().is_some() {
        return None;
    }
    Some(&json[offset + needle.len()..])
}

fn string_field<'a>(json: &'a str, name: &str) -> Option<&'a str> {
    let value = field_value(json, name)?.strip_prefix('"')?;
    let end = value.find('"')?;
    let result = &value[..end];
    if result.contains('\\') || result.contains('\n') || result.contains('\r') {
        return None;
    }
    Some(result)
}

fn number_field(json: &str, name: &str) -> Option<i128> {
    let value = field_value(json, name)?;
    let len = value
        .bytes()
        .take_while(|byte| byte.is_ascii_digit() || *byte == b'-')
        .count();
    if len == 0 {
        return None;
    }
    value[..len].parse().ok()
}

fn bool_field(json: &str, name: &str) -> Option<bool> {
    let value = field_value(json, name)?;
    if value.starts_with("true") {
        Some(true)
    } else if value.starts_with("false") {
        Some(false)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new() -> Self {
            let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
            let path = env::temp_dir().join(format!(
                "nagi-run-receipt-test-{}-{}-{sequence}",
                std::process::id(),
                unix_nanos()
            ));
            fs::create_dir(&path).expect("create isolated test directory");
            Self(path)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn test_config(directory: &Path) -> RunReceiptConfig {
        RunReceiptConfig::new(directory.to_path_buf(), "run-17", "m20-normal-session")
            .expect("valid test config")
    }

    fn test_source() -> SourceIdentity {
        SourceIdentity {
            head: "0123456789abcdef0123456789abcdef01234567".to_owned(),
            worktree_dirty: false,
        }
    }

    fn test_expected() -> ExpectedRun {
        ExpectedRun::new("run-17", "m20-normal-session", test_source().head)
            .expect("valid expected run")
    }

    fn begin_test_run(directory: &Path) -> StartedRun {
        let config = test_config(directory);
        config.begin(test_source()).expect("write started receipt")
    }

    #[test]
    fn started_receipt_records_run_source_stage_and_pid() {
        let directory = TestDirectory::new();
        let started = begin_test_run(&directory.0);
        let contents =
            fs::read_to_string(directory.0.join("started.json")).expect("read started receipt");
        assert!(contents.contains("\"run_id\": \"run-17\""));
        assert!(contents.contains("\"source_head\": \"0123456789abcdef0123456789abcdef01234567\""));
        assert!(contents.contains("\"stage\": \"m20-normal-session\""));
        assert!(contents.contains(&format!("\"pid\": {}", started.record.pid)));
    }

    #[test]
    fn public_start_captures_git_source_before_command_execution() {
        let directory = TestDirectory::new();
        let repository_root = env::current_dir().expect("current repository directory");
        let config =
            RunReceiptConfig::new(directory.0.clone(), "run-git-source", "host-receipt-test")
                .expect("valid test config");
        config
            .start(&repository_root)
            .expect("capture source and write start receipt");
        let contents =
            fs::read_to_string(directory.0.join("started.json")).expect("read started receipt");
        let parsed = StartedRecord::parse(&contents).expect("parse started receipt");
        assert_eq!(parsed.run_id, "run-git-source");
        assert_eq!(parsed.stage, "host-receipt-test");
        assert!(matches!(parsed.source.head.len(), 40 | 64));
    }

    #[test]
    fn terminal_receipt_preserves_success_rc_zero() {
        let directory = TestDirectory::new();
        let started = begin_test_run(&directory.0);
        started.finish(0).expect("write terminal receipt");
        assert_eq!(
            inspect_reconnection(&directory.0, &test_expected()),
            ReconnectionState::Terminal {
                command_result: 0,
                source: test_source()
            }
        );
    }

    #[test]
    fn terminal_receipt_preserves_nonzero_failure_rc() {
        let directory = TestDirectory::new();
        let started = begin_test_run(&directory.0);
        started.finish(23).expect("write terminal receipt");
        assert_eq!(
            inspect_reconnection(&directory.0, &test_expected()),
            ReconnectionState::Terminal {
                command_result: 23,
                source: test_source()
            }
        );
    }

    #[test]
    fn started_without_terminal_and_pid_identity_unknown_is_inconclusive() {
        let directory = TestDirectory::new();
        let _started = begin_test_run(&directory.0);
        assert_eq!(
            inspect_reconnection(&directory.0, &test_expected()),
            ReconnectionState::Inconclusive {
                reason: InconclusiveReason::ProcessStateUnknown
            }
        );
    }

    #[test]
    fn missing_terminal_and_absent_process_is_inconclusive_without_rc() {
        assert_eq!(
            classify_reconnection(TerminalObservation::Missing, ProcessObservation::Absent),
            ReconnectionState::Inconclusive {
                reason: InconclusiveReason::MissingTerminalProcessAbsent
            }
        );
    }

    #[test]
    #[cfg(unix)]
    fn reconnected_absent_process_without_terminal_is_inconclusive() {
        let directory = TestDirectory::new();
        let mut child = Command::new("/usr/bin/true")
            .spawn()
            .expect("spawn short-lived host fixture");
        let pid = child.id();
        assert!(child.wait().expect("wait for host fixture").success());

        let record = StartedRecord {
            run_id: "run-17".to_owned(),
            source: test_source(),
            stage: "m20-normal-session".to_owned(),
            pid,
            started_unix_nanos: unix_nanos(),
        };
        fs::write(
            directory.0.join("started.json"),
            record.to_json().as_bytes(),
        )
        .expect("write started receipt for exited process");
        assert_eq!(
            inspect_reconnection(&directory.0, &test_expected()),
            ReconnectionState::Inconclusive {
                reason: InconclusiveReason::MissingTerminalProcessAbsent
            }
        );
    }

    #[test]
    fn missing_terminal_and_present_pid_is_not_terminal() {
        assert_eq!(
            classify_reconnection(TerminalObservation::Missing, ProcessObservation::Present),
            ReconnectionState::Inconclusive {
                reason: InconclusiveReason::ProcessStateUnknown
            }
        );
    }

    #[test]
    fn invalid_terminal_never_uses_process_or_logs_to_guess_result() {
        assert_eq!(
            classify_reconnection(TerminalObservation::Invalid, ProcessObservation::Absent),
            ReconnectionState::Inconclusive {
                reason: InconclusiveReason::InvalidTerminalReceipt
            }
        );
    }

    #[test]
    fn malformed_terminal_receipt_is_inconclusive() {
        let directory = TestDirectory::new();
        let _started = begin_test_run(&directory.0);
        fs::write(
            directory.0.join("terminal.json"),
            "{\"record\":\"terminal\"}\n",
        )
        .expect("write malformed terminal fixture");
        assert_eq!(
            inspect_reconnection(&directory.0, &test_expected()),
            ReconnectionState::Inconclusive {
                reason: InconclusiveReason::InvalidTerminalReceipt
            }
        );
    }

    #[test]
    fn legacy_exit_code_field_is_not_misread_as_command_result() {
        let directory = TestDirectory::new();
        let started = begin_test_run(&directory.0);
        let legacy = format!(
            "{{\n  \"schema_version\": 1,\n  \"record\": \"terminal\",\n  \"run_id\": \"{}\",\n  \"source_head\": \"{}\",\n  \"source_worktree_dirty\": {},\n  \"stage\": \"{}\",\n  \"pid\": {},\n  \"finished_unix_nanos\": {},\n  \"exit_code\": 0\n}}\n",
            started.record.run_id,
            started.record.source.head,
            started.record.source.worktree_dirty,
            started.record.stage,
            started.record.pid,
            unix_nanos()
        );
        fs::write(directory.0.join("terminal.json"), legacy).expect("write legacy terminal format");
        assert_eq!(
            inspect_reconnection(&directory.0, &test_expected()),
            ReconnectionState::Inconclusive {
                reason: InconclusiveReason::InvalidTerminalReceipt
            }
        );
    }

    #[test]
    fn started_and_terminal_receipts_are_create_only() {
        let directory = TestDirectory::new();
        let started = begin_test_run(&directory.0);
        let started_before = fs::read(directory.0.join("started.json")).expect("read start");
        assert!(test_config(&directory.0).begin(test_source()).is_err());
        assert_eq!(
            fs::read(directory.0.join("started.json")).expect("reread start"),
            started_before
        );

        started.finish(7).expect("write first terminal");
        let terminal_before = fs::read(directory.0.join("terminal.json")).expect("read terminal");
        assert!(started.finish(0).is_err());
        assert_eq!(
            fs::read(directory.0.join("terminal.json")).expect("reread terminal"),
            terminal_before
        );
        assert_eq!(
            inspect_reconnection(&directory.0, &test_expected()),
            ReconnectionState::Terminal {
                command_result: 7,
                source: test_source()
            }
        );
    }

    #[test]
    fn failure_before_publish_is_reported_without_overwriting_existing_receipt() {
        let directory = TestDirectory::new();
        let path = directory.0.join("terminal.json");
        fs::write(&path, "existing\n").expect("create existing receipt");
        let error = atomic_create_new(&path, "replacement\n").expect_err("must be create-only");
        assert!(matches!(error, ReceiptWriteError::NotPublished { .. }));
        assert_eq!(
            fs::read_to_string(path).expect("read original"),
            "existing\n"
        );
    }

    #[test]
    fn failure_after_publish_reports_visible_receipt_with_unknown_durability() {
        let directory = TestDirectory::new();
        let path = directory.0.join("terminal.json");
        let error = atomic_create_new_with_sync(&path, "complete receipt\n", |_| {
            Err(io::Error::other("injected parent directory sync failure"))
        })
        .expect_err("directory sync failure must be surfaced");
        assert!(matches!(
            error,
            ReceiptWriteError::PublishedDurabilityUnknown { .. }
        ));
        assert_eq!(
            fs::read_to_string(path).expect("published receipt remains visible"),
            "complete receipt\n"
        );
    }

    #[test]
    fn started_receipt_for_another_run_stage_or_source_is_inconclusive() {
        let directory = TestDirectory::new();
        begin_test_run(&directory.0)
            .finish(0)
            .expect("write terminal receipt");
        let head = test_source().head;
        let other_head = "1123456789abcdef0123456789abcdef01234567";
        for expected in [
            ExpectedRun::new("run-18", "m20-normal-session", head.clone()),
            ExpectedRun::new("run-17", "m20-other-stage", head.clone()),
            ExpectedRun::new("run-17", "m20-normal-session", other_head),
        ] {
            assert_eq!(
                inspect_reconnection(&directory.0, &expected.expect("valid expected run")),
                ReconnectionState::Inconclusive {
                    reason: InconclusiveReason::StartedReceiptMismatch
                }
            );
        }
    }

    #[test]
    fn terminal_for_another_run_source_stage_pid_or_time_is_rejected() {
        let directory = TestDirectory::new();
        let started = begin_test_run(&directory.0);
        let matching = TerminalRecord {
            run_id: started.record.run_id.clone(),
            source: started.record.source.clone(),
            stage: started.record.stage.clone(),
            pid: started.record.pid,
            finished_unix_nanos: started.record.started_unix_nanos,
            command_result: 0,
        };
        let mut other_run = matching.clone();
        other_run.run_id = "run-18".to_owned();
        let mut other_head = matching.clone();
        other_head.source.head = "1123456789abcdef0123456789abcdef01234567".to_owned();
        let mut other_dirty = matching.clone();
        other_dirty.source.worktree_dirty = !matching.source.worktree_dirty;
        let mut other_stage = matching.clone();
        other_stage.stage = "m20-other-stage".to_owned();
        let mut other_pid = matching.clone();
        other_pid.pid = matching.pid.wrapping_add(1).max(1);
        let mut before_start = matching.clone();
        before_start.finished_unix_nanos = matching.finished_unix_nanos - 1;
        for terminal in [
            other_run,
            other_head,
            other_dirty,
            other_stage,
            other_pid,
            before_start,
        ] {
            fs::write(directory.0.join("terminal.json"), terminal.to_json())
                .expect("write mismatched terminal fixture");
            assert_eq!(
                inspect_reconnection(&directory.0, &test_expected()),
                ReconnectionState::Inconclusive {
                    reason: InconclusiveReason::InvalidTerminalReceipt
                }
            );
        }
    }

    #[test]
    fn terminal_result_reports_recorded_dirty_source() {
        let directory = TestDirectory::new();
        let dirty = SourceIdentity {
            worktree_dirty: true,
            ..test_source()
        };
        test_config(&directory.0)
            .begin(dirty.clone())
            .expect("write started receipt")
            .finish(5)
            .expect("write terminal receipt");
        assert_eq!(
            inspect_reconnection(&directory.0, &test_expected()),
            ReconnectionState::Terminal {
                command_result: 5,
                source: dirty
            }
        );
    }

    #[test]
    fn missing_or_corrupt_started_receipt_is_inconclusive() {
        let directory = TestDirectory::new();
        assert_eq!(
            inspect_reconnection(&directory.0, &test_expected()),
            ReconnectionState::Inconclusive {
                reason: InconclusiveReason::MissingStartedReceipt
            }
        );
        fs::write(
            directory.0.join("started.json"),
            "{\"record\":\"started\"}\n",
        )
        .expect("write corrupt started fixture");
        assert_eq!(
            inspect_reconnection(&directory.0, &test_expected()),
            ReconnectionState::Inconclusive {
                reason: InconclusiveReason::InvalidStartedReceipt
            }
        );
    }

    #[test]
    fn expected_run_rejects_invalid_identity() {
        let head = test_source().head;
        assert!(ExpectedRun::new("../run", "m20", head.clone()).is_err());
        assert!(ExpectedRun::new("run", "m20/session", head).is_err());
        assert!(ExpectedRun::new("run", "m20", "not-a-commit").is_err());
    }

    #[test]
    fn invalid_labels_fail_before_creating_any_receipt() {
        let directory = TestDirectory::new();
        assert!(RunReceiptConfig::new(directory.0.clone(), "../run", "m20").is_err());
        assert!(RunReceiptConfig::new(directory.0.clone(), "run", "m20/session").is_err());
        assert!(RunReceiptConfig::new(PathBuf::new(), "run", "m20").is_err());
        assert_eq!(
            fs::read_dir(&directory.0).expect("list temp dir").count(),
            0
        );
    }
}

#[cfg(test)]
mod source_identity_validation_tests {
    use super::*;

    #[test]
    fn only_commit_id_shapes_are_accepted() {
        assert!(validate_source_head("0123456789abcdef0123456789abcdef01234567").is_ok());
        assert!(validate_source_head(
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
        )
        .is_ok());
        assert!(validate_source_head("not-a-commit").is_err());
    }
}
