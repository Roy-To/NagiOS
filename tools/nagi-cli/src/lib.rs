pub mod acceptance;
pub(crate) mod cc_nagi;
pub(crate) mod cli_output;
pub mod commands;
pub mod config;
pub mod development;
pub mod diagnostics;
pub mod doctor;
pub mod fat32;
pub(crate) mod fetch_progress;
pub(crate) mod fonts;
pub(crate) mod freetype_sys;
pub mod gpt;
pub(crate) mod hyper_util_servo;
pub mod image;
pub(crate) mod libc_servo;
pub(crate) mod llama_cpp;
pub(crate) mod llvm_libcxx;
#[cfg(test)]
mod login_password_change_contract_tests;
#[cfg(test)]
mod m17_storage_contract_tests;
pub(crate) mod m18_acceptance;
#[path = "../../../tests/fixtures/m20_model_store_reader.rs"]
pub(crate) mod m20_model_store_fixture;
#[cfg(test)]
mod m29_language_contract_tests;
pub(crate) mod mesa;
pub(crate) mod mio_servo;
pub(crate) mod model_artifact;
pub(crate) mod mozjs_sys_nagi;
pub mod paths;
pub(crate) mod registry_source;
pub mod run_receipt;
pub(crate) mod rust_std_source;
pub(crate) mod servo;
pub(crate) mod socket2_servo;
pub(crate) mod surfman;
pub(crate) mod tempfile_nagi;
#[cfg(test)]
mod third_party_notices;
pub(crate) mod tokio_servo;
pub(crate) mod whisper_cpp;

#[cfg(test)]
fn read_frozen_legacy_pid1_file(root: &std::path::Path, name: &str) -> String {
    // Keep this revision aligned with the frozen legacy PID 1 inputs in CI.
    const REVISION: &str = "956a104683cb4163e1e77fc2606607d5c3f033e7";
    let output = std::process::Command::new("git")
        .current_dir(root)
        .args(["show", &format!("{REVISION}:user/nagi-init/{name}")])
        .output()
        .expect("run Git to read frozen legacy PID 1 inputs");
    assert!(
        output.status.success(),
        "cannot read frozen legacy PID 1 {name} at {REVISION}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("frozen legacy PID 1 Cargo file is UTF-8")
}
