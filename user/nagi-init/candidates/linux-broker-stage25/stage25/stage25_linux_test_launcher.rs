use std::process::{Command, ExitCode};

const TEST_BINARY: &str = "/stage25-test-binary";
const FILTERS: &[&str] = &[
    "task24_spawn_identity_cannot_be_overwritten_and_is_retained_in_entry",
    "durable_authority_fingerprint_uses_fresh_run_nonce",
    "ending_run_clears_request_ledger_without_making_inflight_request_retryable",
    "actual_task24_owner_trait_matches_task41_spawn_nonce_and_record",
    "exact_reap_and_record_retire_nonclone_lease_once",
    "serialized_package_owner_records_one_shot_generation_receipt_only",
    "revoking_32_resolved_receipts_releases_capacity_for_a_new_request",
    "revoke_all_reclaims_known_outcomes_but_keeps_unknown_fail_closed",
    "supervisor_retirement_revokes_registered_package_receipts_and_releases_terminal_slots",
    "run_end_revokes_registered_package_receipts_but_keeps_unknown_tombstones",
    "public_owner_resolver_rejects_same_request_after_new_pid1_run",
];

fn main() -> ExitCode {
    for filter in FILTERS {
        println!("STAGE25_TEST_START={filter}");
        let result = Command::new(TEST_BINARY)
            .env("NAGI_DEV_PREVIEW_PACKAGE", "/task25-preview.xapp")
            .arg(filter)
            .arg("--nocapture")
            .status();
        match result {
            Ok(status) if status.success() => {}
            Ok(status) => {
                eprintln!("STAGE25_TEST_FAIL={filter} status={status}");
                return ExitCode::FAILURE;
            }
            Err(error) => {
                eprintln!("STAGE25_TEST_EXEC_ERROR={filter} error={error}");
                return ExitCode::FAILURE;
            }
        }
    }
    println!("STAGE25_LINUX_TESTS=PASS count={}", FILTERS.len());

    println!("STAGE25_PIDFD_RUNTIME_START");
    match Command::new("/stage25-runtime-fixture").status() {
        Ok(status) if status.success() => {}
        Ok(status) => {
            eprintln!("STAGE25_PIDFD_RUNTIME_FAIL status={status}");
            return ExitCode::FAILURE;
        }
        Err(error) => {
            eprintln!("STAGE25_PIDFD_RUNTIME_EXEC_ERROR error={error}");
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}
