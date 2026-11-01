//! Opt-in run receipts exercised through the real `nagi` binary. These tests
//! only run `help` and an unknown command; they never build, fetch or start a VM.
#![cfg(unix)]

use std::env;
use std::ffi::OsString;
use std::fs;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

use nagi_cli::commands::{EXIT_CONFIG_ERROR, EXIT_SUCCESS, EXIT_USAGE};
use nagi_cli::run_receipt::{
    inspect_reconnection, ExpectedRun, InconclusiveReason, ReconnectionState, SourceIdentity,
};

const RECEIPT_DIR: &str = "NAGI_RUN_RECEIPT_DIR";
const RUN_ID: &str = "NAGI_RUN_ID";
const RUN_STAGE: &str = "NAGI_RUN_STAGE";
const UNKNOWN_COMMAND: &str = "receipt-test-unknown-command";

static SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct Scratch(PathBuf);

impl Scratch {
    fn new(name: &str) -> Self {
        let path = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!(
            "run-receipt-cli-{name}-{}-{}",
            std::process::id(),
            SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(&path).expect("create isolated receipt test directory");
        Self(path)
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn repository_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("repository root")
}

fn nagi(args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_nagi"));
    command
        .args(args)
        .current_dir(repository_root())
        .env_remove(RECEIPT_DIR)
        .env_remove(RUN_ID)
        .env_remove(RUN_STAGE)
        .stdin(Stdio::null());
    command
}

fn nagi_with_receipt(args: &[&str], directory: &Path, run_id: &str, stage: &str) -> Command {
    let mut command = nagi(args);
    command
        .env(RECEIPT_DIR, directory)
        .env(RUN_ID, run_id)
        .env(RUN_STAGE, stage);
    command
}

fn run(mut command: Command) -> Output {
    command.output().expect("run nagi")
}

fn git(args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(repository_root())
        .args(args)
        .output()
        .expect("run git");
    assert!(output.status.success(), "git {args:?} failed");
    String::from_utf8(output.stdout).expect("git output is UTF-8")
}

fn current_source() -> SourceIdentity {
    SourceIdentity {
        head: git(&["rev-parse", "--verify", "HEAD"]).trim().to_owned(),
        worktree_dirty: !git(&["status", "--porcelain", "--untracked-files=normal"]).is_empty(),
    }
}

fn expected(run_id: &str, stage: &str, head: &str) -> ExpectedRun {
    ExpectedRun::new(run_id, stage, head).expect("valid expected run")
}

fn other_head(head: &str) -> String {
    let replacement = if head.starts_with('0') { "1" } else { "0" };
    format!("{replacement}{}", &head[1..])
}

/// Assert a matching terminal receipt. Dirty state is compared only when the
/// worktree did not change while the CLI ran.
fn assert_terminal(
    directory: &Path,
    expected: &ExpectedRun,
    command_result: i32,
    before: &SourceIdentity,
) {
    let after = current_source();
    match inspect_reconnection(directory, expected) {
        ReconnectionState::Terminal {
            command_result: recorded,
            source,
        } => {
            assert_eq!(recorded, command_result);
            assert_eq!(source.head, before.head);
            if before == &after {
                assert_eq!(&source, before);
            }
        }
        other => panic!(
            "expected terminal receipt in {}: {other:?}",
            directory.display()
        ),
    }
}

fn assert_inconclusive(directory: &Path, expected: &ExpectedRun, reason: InconclusiveReason) {
    assert_eq!(
        inspect_reconnection(directory, expected),
        ReconnectionState::Inconclusive { reason }
    );
}

fn directory_entries(directory: &Path) -> Vec<OsString> {
    let mut entries: Vec<OsString> = fs::read_dir(directory)
        .expect("list receipt directory")
        .map(|entry| entry.expect("read receipt entry").file_name())
        .collect();
    entries.sort();
    entries
}

#[test]
fn closed_output_pipes_keep_command_result_and_terminal_receipt() {
    let scratch = Scratch::new("closed-pipes");
    let source = current_source();
    for (index, (args, command_result)) in [
        (&["help"][..], EXIT_SUCCESS),
        (&[UNKNOWN_COMMAND][..], EXIT_USAGE),
    ]
    .into_iter()
    .enumerate()
    {
        let receipts = scratch.0.join(format!("receipts-{index}"));
        let run_id = format!("closed-pipes-{index}");
        let (stdout_reader, stdout_writer) = io::pipe().expect("stdout pipe");
        let (stderr_reader, stderr_writer) = io::pipe().expect("stderr pipe");
        // Close both read ends before the CLI starts, so every write fails.
        drop(stdout_reader);
        drop(stderr_reader);
        let status = nagi_with_receipt(args, &receipts, &run_id, "receipt-cli-test")
            .stdout(stdout_writer)
            .stderr(stderr_writer)
            .status()
            .expect("run nagi with closed output pipes");
        assert_eq!(status.code(), Some(command_result), "{args:?}");
        assert_terminal(
            &receipts,
            &expected(&run_id, "receipt-cli-test", &source.head),
            command_result,
            &source,
        );

        // Without receipts, a closed pipe must not replace the result either.
        let (stdout_reader, stdout_writer) = io::pipe().expect("stdout pipe");
        let (stderr_reader, stderr_writer) = io::pipe().expect("stderr pipe");
        drop(stdout_reader);
        drop(stderr_reader);
        let status = nagi(args)
            .stdout(stdout_writer)
            .stderr(stderr_writer)
            .status()
            .expect("run nagi without receipts");
        assert_eq!(status.code(), Some(command_result), "{args:?}");
    }
}

#[test]
fn receipt_settings_do_not_leak_to_cli_children_and_explicit_ids_stay_separate() {
    let scratch = Scratch::new("isolation");
    let source = current_source();
    let original_path = env::var_os("PATH").expect("PATH");
    let real_git = env::split_paths(&original_path)
        .map(|directory| directory.join("git"))
        .find(|candidate| candidate.is_file())
        .expect("git on PATH");

    // The CLI spawns git after consuming receipt settings; this shim records
    // whether any NAGI_RUN_* value reached that child.
    let shim_directory = scratch.0.join("bin");
    fs::create_dir(&shim_directory).expect("create shim directory");
    let shim = shim_directory.join("git");
    fs::write(
        &shim,
        "#!/bin/sh\n\
         echo invoked >> \"$NAGI_RECEIPT_TEST_LOG\"\n\
         env | grep '^NAGI_RUN_' >> \"$NAGI_RECEIPT_TEST_LOG\"\n\
         exec \"$NAGI_RECEIPT_TEST_REAL_GIT\" \"$@\"\n",
    )
    .expect("write git shim");
    fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).expect("make shim executable");
    let log = scratch.0.join("child-environment.log");
    let mut path = vec![shim_directory];
    path.extend(env::split_paths(&original_path));
    let path = env::join_paths(path).expect("join PATH");

    let parent_dir = scratch.0.join("parent-run");
    let mut parent = nagi_with_receipt(
        &[UNKNOWN_COMMAND],
        &parent_dir,
        "parent-run-01",
        "receipt-cli-parent",
    );
    parent
        .env("PATH", &path)
        .env("NAGI_RECEIPT_TEST_LOG", &log)
        .env("NAGI_RECEIPT_TEST_REAL_GIT", &real_git);
    assert_eq!(run(parent).status.code(), Some(EXIT_USAGE));
    let child_environment = fs::read_to_string(&log).expect("read child environment log");
    assert!(
        !child_environment.is_empty() && child_environment.lines().all(|line| line == "invoked"),
        "receipt settings reached a CLI child: {child_environment:?}"
    );

    let parent_expected = expected("parent-run-01", "receipt-cli-parent", &source.head);
    assert_terminal(&parent_dir, &parent_expected, EXIT_USAGE, &source);
    let parent_started = fs::read(parent_dir.join("started.json")).expect("read parent start");
    let parent_terminal = fs::read(parent_dir.join("terminal.json")).expect("read parent terminal");

    // An explicitly configured second run keeps its own ID and directory.
    let child_dir = scratch.0.join("explicit-child-run");
    let child = nagi_with_receipt(&["help"], &child_dir, "child-run-02", "receipt-cli-child");
    assert_eq!(run(child).status.code(), Some(EXIT_SUCCESS));
    let child_expected = expected("child-run-02", "receipt-cli-child", &source.head);
    assert_terminal(&child_dir, &child_expected, EXIT_SUCCESS, &source);
    assert_inconclusive(
        &parent_dir,
        &child_expected,
        InconclusiveReason::StartedReceiptMismatch,
    );

    // Reusing the parent's directory is refused before the command runs and
    // leaves the parent's receipts byte-for-byte unchanged.
    let reuse = nagi_with_receipt(&["help"], &parent_dir, "child-run-03", "receipt-cli-child");
    assert_eq!(run(reuse).status.code(), Some(EXIT_CONFIG_ERROR));
    assert_eq!(
        fs::read(parent_dir.join("started.json")).expect("reread parent start"),
        parent_started
    );
    assert_eq!(
        fs::read(parent_dir.join("terminal.json")).expect("reread parent terminal"),
        parent_terminal
    );
    assert_eq!(
        directory_entries(&parent_dir),
        [
            OsString::from("started.json"),
            OsString::from("terminal.json")
        ]
    );
    assert_terminal(&parent_dir, &parent_expected, EXIT_USAGE, &source);
}

#[test]
fn mismatched_partial_or_mixed_receipts_are_never_terminal() {
    let scratch = Scratch::new("mismatch");
    let source = current_source();
    let run_a = scratch.0.join("run-a");
    let run_b = scratch.0.join("run-b");
    let stage = "receipt-cli-mismatch";
    assert_eq!(
        run(nagi_with_receipt(
            &[UNKNOWN_COMMAND],
            &run_a,
            "run-a",
            stage
        ))
        .status
        .code(),
        Some(EXIT_USAGE)
    );
    assert_eq!(
        run(nagi_with_receipt(&["help"], &run_b, "run-b", stage))
            .status
            .code(),
        Some(EXIT_SUCCESS)
    );
    let expected_a = expected("run-a", stage, &source.head);
    assert_terminal(&run_a, &expected_a, EXIT_USAGE, &source);

    for wrong in [
        expected("run-b", stage, &source.head),
        expected("run-a", "receipt-cli-other", &source.head),
        expected("run-a", stage, &other_head(&source.head)),
    ] {
        assert_inconclusive(&run_a, &wrong, InconclusiveReason::StartedReceiptMismatch);
    }
    // A complete, valid receipt pair for another run is not run A's result.
    assert_inconclusive(
        &run_b,
        &expected_a,
        InconclusiveReason::StartedReceiptMismatch,
    );

    let started_a = fs::read(run_a.join("started.json")).expect("read run A start");
    let terminal_a = fs::read(run_a.join("terminal.json")).expect("read run A terminal");
    let terminal_b = fs::read(run_b.join("terminal.json")).expect("read run B terminal");

    let mixed = scratch.0.join("mixed");
    fs::create_dir(&mixed).expect("create mixed directory");
    fs::write(mixed.join("started.json"), &started_a).expect("copy run A start");
    fs::write(mixed.join("terminal.json"), &terminal_b).expect("copy run B terminal");
    assert_inconclusive(
        &mixed,
        &expected_a,
        InconclusiveReason::InvalidTerminalReceipt,
    );

    let truncated = scratch.0.join("truncated");
    fs::create_dir(&truncated).expect("create truncated directory");
    fs::write(truncated.join("started.json"), &started_a).expect("copy run A start");
    fs::write(
        truncated.join("terminal.json"),
        &terminal_a[..terminal_a.len() / 2],
    )
    .expect("write truncated terminal");
    assert_inconclusive(
        &truncated,
        &expected_a,
        InconclusiveReason::InvalidTerminalReceipt,
    );

    // The recorded PID has exited (or may have been reused); either way the
    // result stays inconclusive without a terminal receipt.
    let start_only = scratch.0.join("start-only");
    fs::create_dir(&start_only).expect("create start-only directory");
    fs::write(start_only.join("started.json"), &started_a).expect("copy run A start");
    assert!(matches!(
        inspect_reconnection(&start_only, &expected_a),
        ReconnectionState::Inconclusive {
            reason: InconclusiveReason::MissingTerminalProcessAbsent
                | InconclusiveReason::ProcessStateUnknown
        }
    ));

    let missing = scratch.0.join("missing");
    fs::create_dir(&missing).expect("create empty directory");
    assert_inconclusive(
        &missing,
        &expected_a,
        InconclusiveReason::MissingStartedReceipt,
    );
}

#[test]
fn competing_starts_publish_one_run_and_never_overwrite() {
    let scratch = Scratch::new("race");
    let source = current_source();
    let receipts = scratch.0.join("receipts");
    let children: Vec<Child> = (0..8)
        .map(|_| {
            nagi_with_receipt(&[UNKNOWN_COMMAND], &receipts, "race-01", "receipt-cli-race")
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .expect("spawn competing nagi")
        })
        .collect();
    let results: Vec<(u32, Option<i32>)> = children
        .into_iter()
        .map(|mut child| {
            let pid = child.id();
            (pid, child.wait().expect("wait for competing nagi").code())
        })
        .collect();

    let winners: Vec<u32> = results
        .iter()
        .filter(|(_, code)| *code == Some(EXIT_USAGE))
        .map(|(pid, _)| *pid)
        .collect();
    assert_eq!(winners.len(), 1, "{results:?}");
    assert!(
        results
            .iter()
            .all(|(_, code)| *code == Some(EXIT_USAGE) || *code == Some(EXIT_CONFIG_ERROR)),
        "{results:?}"
    );
    let started = fs::read_to_string(receipts.join("started.json")).expect("read start");
    assert!(started.contains(&format!("\"pid\": {},\n", winners[0])));
    assert_terminal(
        &receipts,
        &expected("race-01", "receipt-cli-race", &source.head),
        EXIT_USAGE,
        &source,
    );
    assert_eq!(
        directory_entries(&receipts),
        [
            OsString::from("started.json"),
            OsString::from("terminal.json")
        ]
    );
}
