pub mod acceptance;
pub(crate) mod cli_output;
pub mod commands;
pub mod config;
pub mod development;
pub mod diagnostics;
pub mod doctor;
pub(crate) mod fetch_progress;
pub(crate) mod fonts;
#[cfg(test)]
mod login_password_change_contract_tests;
#[cfg(test)]
mod m17_storage_contract_tests;
#[cfg(test)]
mod m29_language_contract_tests;
pub mod paths;
pub mod run_receipt;
#[cfg(test)]
mod third_party_notices;

#[cfg(test)]
fn read_frozen_legacy_file(root: &std::path::Path, path: &str) -> String {
    // Keep this revision aligned with the frozen legacy checkout in CI.
    const REVISION: &str = "956a104683cb4163e1e77fc2606607d5c3f033e7";
    let output = std::process::Command::new("git")
        .current_dir(root)
        .args(["show", &format!("{REVISION}:{path}")])
        .output()
        .expect("run Git to read frozen legacy source");
    assert!(
        output.status.success(),
        "cannot read frozen legacy source {path} at {REVISION}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("frozen legacy source file is UTF-8")
}
