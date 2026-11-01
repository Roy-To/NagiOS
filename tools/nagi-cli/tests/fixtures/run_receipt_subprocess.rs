#[path = "../../src/run_receipt.rs"]
#[allow(dead_code)]
mod run_receipt;

use run_receipt::RunReceiptConfig;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::{SystemTime, UNIX_EPOCH};

const RECEIPT_DIR: &str = "NAGI_RUN_RECEIPT_DIR";
const RUN_ID: &str = "NAGI_RUN_ID";
const RUN_STAGE: &str = "NAGI_RUN_STAGE";
const CHILD_DIR: &str = "NAGI_RUN_RECEIPT_TEST_CHILD_DIR";

struct Cleanup(PathBuf);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn main() -> ExitCode {
    let mode = env::args().nth(1).unwrap_or_else(|| "outer".to_owned());
    let result = match mode.as_str() {
        "outer" => run_outer(),
        "parent" => run_parent(),
        "plain-child" => run_plain_child(),
        "explicit-child" => run_explicit_child(),
        _ => Err(format!("unknown harness mode: {mode}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("FAIL receipt subprocess harness: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run_outer() -> Result<(), String> {
    let base = env::temp_dir().join(format!(
        "nagi-receipt-subprocess-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    ));
    fs::create_dir(&base).map_err(|error| format!("create harness directory: {error}"))?;
    let _cleanup = Cleanup(base.clone());
    let parent_dir = base.join("parent-run");
    let child_dir = base.join("explicit-child-run");
    let status = Command::new(env::current_exe().map_err(|error| error.to_string())?)
        .arg("parent")
        .env(RECEIPT_DIR, &parent_dir)
        .env(RUN_ID, "parent-run-01")
        .env(RUN_STAGE, "m20-parent")
        .env(CHILD_DIR, &child_dir)
        .status()
        .map_err(|error| format!("spawn parent probe: {error}"))?;
    if !status.success() {
        return Err(format!("parent probe exited with {status}"));
    }

    assert_receipts(&parent_dir, "parent-run-01", "m20-parent")?;
    assert_receipts(&child_dir, "child-run-02", "m20-child")?;
    println!("PASS parent receipt retained; plain child had no inherited receipt; explicit child used a separate ID");
    Ok(())
}

fn run_parent() -> Result<(), String> {
    let child_dir =
        PathBuf::from(env::var_os(CHILD_DIR).ok_or_else(|| format!("missing {CHILD_DIR}"))?);
    let config = RunReceiptConfig::take_from_env_at_startup()
        .map_err(|error| format!("take parent receipt config: {error}"))?
        .ok_or_else(|| "parent receipt config was not present".to_owned())?;
    ensure_receipt_env_absent()?;
    let started = config
        .start(&env::current_dir().map_err(|error| error.to_string())?)
        .map_err(|error| format!("start parent receipt: {error}"))?;

    let binary = env::current_exe().map_err(|error| error.to_string())?;
    let ordinary_child = Command::new(&binary)
        .arg("plain-child")
        .env(CHILD_DIR, &child_dir)
        .status()
        .map_err(|error| format!("spawn plain nested CLI: {error}"))?;
    if !ordinary_child.success() {
        return Err(format!("plain nested CLI exited with {ordinary_child}"));
    }

    let explicit_child = Command::new(binary)
        .arg("explicit-child")
        .env(RECEIPT_DIR, &child_dir)
        .env(RUN_ID, "child-run-02")
        .env(RUN_STAGE, "m20-child")
        .status()
        .map_err(|error| format!("spawn explicitly recorded child CLI: {error}"))?;
    if !explicit_child.success() {
        return Err(format!("explicit child CLI exited with {explicit_child}"));
    }

    started
        .finish(0)
        .map_err(|error| format!("finish parent receipt: {error}"))
}

fn run_plain_child() -> Result<(), String> {
    ensure_receipt_env_absent()
}

fn run_explicit_child() -> Result<(), String> {
    let config = RunReceiptConfig::take_from_env_at_startup()
        .map_err(|error| format!("take explicit child receipt config: {error}"))?
        .ok_or_else(|| "explicit child receipt config was not present".to_owned())?;
    ensure_receipt_env_absent()?;
    let started = config
        .start(&env::current_dir().map_err(|error| error.to_string())?)
        .map_err(|error| format!("start explicit child receipt: {error}"))?;

    let nested = Command::new(env::current_exe().map_err(|error| error.to_string())?)
        .arg("plain-child")
        .status()
        .map_err(|error| format!("spawn child of explicitly recorded CLI: {error}"))?;
    if !nested.success() {
        return Err(format!("nested plain CLI exited with {nested}"));
    }

    started
        .finish(0)
        .map_err(|error| format!("finish explicit child receipt: {error}"))
}

fn ensure_receipt_env_absent() -> Result<(), String> {
    for name in [RECEIPT_DIR, RUN_ID, RUN_STAGE] {
        if env::var_os(name).is_some() {
            return Err(format!("{name} leaked to child process"));
        }
    }
    Ok(())
}

fn assert_receipts(directory: &Path, run_id: &str, stage: &str) -> Result<(), String> {
    let started = fs::read_to_string(directory.join("started.json"))
        .map_err(|error| format!("read started receipt: {error}"))?;
    let terminal = fs::read_to_string(directory.join("terminal.json"))
        .map_err(|error| format!("read terminal receipt: {error}"))?;
    for (label, contents) in [("started", &started), ("terminal", &terminal)] {
        if !contents.contains(&format!("\"run_id\": \"{run_id}\""))
            || !contents.contains(&format!("\"stage\": \"{stage}\""))
        {
            return Err(format!("{label} receipt did not retain {run_id}/{stage}"));
        }
    }
    if !terminal.contains("\"command_result\": 0") {
        return Err(format!(
            "terminal receipt for {run_id} did not preserve RC 0"
        ));
    }
    Ok(())
}
