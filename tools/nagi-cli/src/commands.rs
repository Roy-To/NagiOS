use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command as ProcessCommand, Stdio};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

use crate::cc_nagi::ensure_cc_nagi_checkout;
use crate::config::{load_toolchain_requirements, validate_project};
use crate::diagnostics::{
    ClosureHealthCheck, DiagnosticEvent, DiagnosticsBundle, EvidenceKind, FailureClass,
    HealthCheckRegistry, OverallStatus, PrivacyClass, Severity, VerificationCheckResult,
    VerificationReport, DIAGNOSTIC_BUNDLE_SCHEMA_VERSION,
};
use crate::doctor::{
    ovmf_pair_is_allowed, run_doctor_with_requirements, CheckState, DoctorPolicy, HostProbe,
};
use crate::image::signed_update_bundle;
use crate::image::{
    ensure_persistent_disk, initialize_ovmf_vars, run_qemu, run_qemu_gui,
    run_qemu_gui_reusing_ovmf_vars_with_events,
    run_qemu_gui_reusing_ovmf_vars_with_events_and_serial_input,
    run_qemu_gui_reusing_ovmf_vars_with_read_only_boot_disk_and_events_and_serial_input,
    run_qemu_gui_with_events, run_qemu_gui_with_events_and_screenshot,
    run_qemu_gui_with_read_only_boot_disk_and_events_and_serial_input,
    run_qemu_gui_with_read_only_boot_disk_and_staged_events_and_failure_marker_and_screenshot,
    run_qemu_interactive, run_qemu_reusing_ovmf_vars,
    run_qemu_reusing_ovmf_vars_with_read_only_boot_disk, run_qemu_until_any_acceptance_marker,
    run_qemu_until_any_acceptance_marker_reusing_ovmf_vars,
    run_qemu_until_any_acceptance_marker_with_read_only_boot_disk,
    run_qemu_with_read_only_boot_disk, validate_reference_disk_qcow2, write_fat12_image,
    write_isolated_apps_fat12_image, write_m17_fat12_image,
    write_m20_model_store_fixture_reference_disk_qcow2, write_m27_broken_slot_image,
    write_m27_gpt_broken_system_b_qcow2, write_m27_healthy_slot_image, write_m27_recovery_image,
    write_reference_disk_qcow2, write_reference_disk_qcow2_with_external_model_store_file,
    ImageLayout, QemuConfig, QmpEventStage, GUEST_ACCEPTANCE_MARKER, NAGI_WRITE_MARKER,
};
use crate::image::{
    qmp_screendump_command, run_qemu_gui_with_staged_events_and_screenshot, validate_screenshot,
};
use crate::llama_cpp::ensure_llama_cpp_checkout;
use crate::mesa::ensure_mesa_checkout;
use crate::model_artifact::{validate_m26_model_lock, validate_m26_model_manifest, M26Model};
use crate::mozjs_sys_nagi::ensure_mozjs_sys_nagi_checkout;
use crate::paths::{clean_owned_outputs, ensure_owned_directory};
use crate::servo::ensure_servo_checkout;
use crate::surfman::ensure_surfman_checkout;
use crate::tempfile_nagi::ensure_tempfile_nagi_checkout;
use crate::whisper_cpp::ensure_whisper_cpp_checkout;

pub const EXIT_SUCCESS: i32 = 0;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_NOT_IMPLEMENTED: i32 = 3;
pub const EXIT_CONFIG_ERROR: i32 = 4;
pub const EXIT_DOCTOR_FAILURE: i32 = 10;
pub const EXIT_VERIFY_FAILURE: i32 = 11;

type ImageWriter = fn(&Path, &[u8], &[u8], &[u8], Option<&[u8]>) -> Result<ImageLayout, String>;

#[derive(Clone, Copy, Default)]
struct ImageBuildFeatures<'a> {
    kernel: &'a [&'a str],
    loader: &'a [&'a str],
}

struct ImageBuildRequest<'a> {
    image_name: &'a str,
    cargo_env: &'a [(&'a str, &'a Path)],
    recovery_init: Option<&'a [u8]>,
    image_writer: ImageWriter,
    external_model_store_file: Option<(&'a str, &'a Path)>,
    build_features: ImageBuildFeatures<'a>,
}

struct DesktopAcceptanceConfig {
    label: &'static str,
    features: &'static str,
    image_name: &'static str,
    persistent_disk_name: &'static str,
    vars_name: &'static str,
    first_log_name: &'static str,
    run_log_name: &'static str,
    evidence_prefix: &'static str,
    screenshot_name: &'static str,
    acceptance_marker: &'static str,
    required_markers: &'static [&'static str],
    restart_marker: Option<&'static str>,
    restart_log_name: Option<&'static str>,
    /// What the restart marker proves, for the PASS summary.
    restart_summary: &'static str,
    unique_run_artifacts: bool,
    /// Build init with the signed acceptance packages (ADR 0049).
    acceptance_packages: bool,
    /// Also capture the first desktop frame, before any input is sent.
    ready_screenshot_name: Option<&'static str>,
    /// QMP input for the restart boot (for example, signing in).
    restart_events: Vec<String>,
    /// Input sent only after a guest marker: `(marker, events)`.
    later_stages: Vec<(&'static str, Vec<String>)>,
    /// Capture `ready_screenshot_name` at the start of the stage with this
    /// marker instead of before the first input.
    ready_screenshot_stage: Option<&'static str>,
}

/// Clipboard steps, each sent after the guest reports the previous one:
/// Ctrl+C in the selected source field; a click on the destination field
/// (pointer moves from the address-bar click at y=30 to y=132); Ctrl+V.
const M18_CLIPBOARD_COPY_EVENTS: [&str; 1] = [r#"{
        "execute":"input-send-event",
        "arguments":{"events":[
            {"type":"key","data":{"down":true,"key":{"type":"qcode","data":"ctrl"}}},
            {"type":"key","data":{"down":true,"key":{"type":"qcode","data":"c"}}},
            {"type":"key","data":{"down":false,"key":{"type":"qcode","data":"c"}}},
            {"type":"key","data":{"down":false,"key":{"type":"qcode","data":"ctrl"}}}
        ]}
    }"#];
const M18_CLIPBOARD_FOCUS_EVENTS: [&str; 1] = [r#"{
        "execute":"input-send-event",
        "arguments":{"events":[
            {"type":"rel","data":{"axis":"y","value":102}},
            {"type":"btn","data":{"button":"left","down":true}},
            {"type":"btn","data":{"button":"left","down":false}}
        ]}
    }"#];
const M18_CLIPBOARD_PASTE_EVENTS: [&str; 1] = [r#"{
        "execute":"input-send-event",
        "arguments":{"events":[
            {"type":"key","data":{"down":true,"key":{"type":"qcode","data":"ctrl"}}},
            {"type":"key","data":{"down":true,"key":{"type":"qcode","data":"v"}}},
            {"type":"key","data":{"down":false,"key":{"type":"qcode","data":"v"}}},
            {"type":"key","data":{"down":false,"key":{"type":"qcode","data":"ctrl"}}}
        ]}
    }"#];

/// After the guest reports the focused IME field: Ctrl+Space switches to
/// hiragana, `nihongo` composes にほんご, and Enter commits it.
const M18_IME_EVENTS: [&str; 3] = [
    r#"{
        "execute":"input-send-event",
        "arguments":{"events":[
            {"type":"key","data":{"down":true,"key":{"type":"qcode","data":"ctrl"}}},
            {"type":"key","data":{"down":true,"key":{"type":"qcode","data":"spc"}}},
            {"type":"key","data":{"down":false,"key":{"type":"qcode","data":"spc"}}},
            {"type":"key","data":{"down":false,"key":{"type":"qcode","data":"ctrl"}}}
        ]}
    }"#,
    r#"{
        "execute":"input-send-event",
        "arguments":{"events":[
            {"type":"key","data":{"down":true,"key":{"type":"qcode","data":"n"}}},
            {"type":"key","data":{"down":false,"key":{"type":"qcode","data":"n"}}},
            {"type":"key","data":{"down":true,"key":{"type":"qcode","data":"i"}}},
            {"type":"key","data":{"down":false,"key":{"type":"qcode","data":"i"}}},
            {"type":"key","data":{"down":true,"key":{"type":"qcode","data":"h"}}},
            {"type":"key","data":{"down":false,"key":{"type":"qcode","data":"h"}}},
            {"type":"key","data":{"down":true,"key":{"type":"qcode","data":"o"}}},
            {"type":"key","data":{"down":false,"key":{"type":"qcode","data":"o"}}},
            {"type":"key","data":{"down":true,"key":{"type":"qcode","data":"n"}}},
            {"type":"key","data":{"down":false,"key":{"type":"qcode","data":"n"}}},
            {"type":"key","data":{"down":true,"key":{"type":"qcode","data":"g"}}},
            {"type":"key","data":{"down":false,"key":{"type":"qcode","data":"g"}}},
            {"type":"key","data":{"down":true,"key":{"type":"qcode","data":"o"}}},
            {"type":"key","data":{"down":false,"key":{"type":"qcode","data":"o"}}}
        ]}
    }"#,
    r#"{
        "execute":"input-send-event",
        "arguments":{"events":[
            {"type":"key","data":{"down":true,"key":{"type":"qcode","data":"ret"}}},
            {"type":"key","data":{"down":false,"key":{"type":"qcode","data":"ret"}}}
        ]}
    }"#,
];

/// Upload steps: click the page's file input (pointer moves from the
/// clipboard click at y=132 to y=90, page y=28), then Enter in Albert's
/// trusted picker.
const M18_UPLOAD_CLICK_EVENTS: [&str; 1] = [r#"{
        "execute":"input-send-event",
        "arguments":{"events":[
            {"type":"rel","data":{"axis":"y","value":-42}},
            {"type":"btn","data":{"button":"left","down":true}},
            {"type":"btn","data":{"button":"left","down":false}}
        ]}
    }"#];
const M18_UPLOAD_CHOOSE_EVENTS: [&str; 1] = [r#"{
        "execute":"input-send-event",
        "arguments":{"events":[
            {"type":"key","data":{"down":true,"key":{"type":"qcode","data":"ret"}}},
            {"type":"key","data":{"down":false,"key":{"type":"qcode","data":"ret"}}}
        ]}
    }"#];

/// Download step: click the page's download link where the upload input was
/// (page y=28); the pointer is already there.
const M18_DOWNLOAD_CLICK_EVENTS: [&str; 1] = [r#"{
        "execute":"input-send-event",
        "arguments":{"events":[
            {"type":"btn","data":{"button":"left","down":true}},
            {"type":"btn","data":{"button":"left","down":false}}
        ]}
    }"#];

/// Permission step: click Allow on Albert's prompt at (253,145) — pinned by
/// nagi-albert's `m18_harness_allow_click_lands_on_allow` — from the
/// address-bar click at (100,30), then return the pointer there so later
/// relative moves are unchanged.
const M18_PERMISSION_ALLOW_EVENTS: [&str; 2] = [
    r#"{
        "execute":"input-send-event",
        "arguments":{"events":[
            {"type":"rel","data":{"axis":"x","value":153}},
            {"type":"rel","data":{"axis":"y","value":115}},
            {"type":"btn","data":{"button":"left","down":true}},
            {"type":"btn","data":{"button":"left","down":false}}
        ]}
    }"#,
    r#"{
        "execute":"input-send-event",
        "arguments":{"events":[
            {"type":"rel","data":{"axis":"x","value":-153}},
            {"type":"rel","data":{"axis":"y","value":-115}}
        ]}
    }"#,
];

const M18_INPUT_EVENTS: [&str; 2] = [
    r#"{
        "execute":"input-send-event",
        "arguments":{"events":[
            {"type":"rel","data":{"axis":"x","value":100}},
            {"type":"rel","data":{"axis":"y","value":30}},
            {"type":"btn","data":{"button":"left","down":true}},
            {"type":"btn","data":{"button":"left","down":false}}
        ]}
    }"#,
    r#"{
        "execute":"input-send-event",
        "arguments":{"events":[
            {"type":"key","data":{"down":true,"key":{"type":"qcode","data":"e"}}},
            {"type":"key","data":{"down":false,"key":{"type":"qcode","data":"e"}}},
            {"type":"key","data":{"down":true,"key":{"type":"qcode","data":"x"}}},
            {"type":"key","data":{"down":false,"key":{"type":"qcode","data":"x"}}},
            {"type":"key","data":{"down":true,"key":{"type":"qcode","data":"a"}}},
            {"type":"key","data":{"down":false,"key":{"type":"qcode","data":"a"}}},
            {"type":"key","data":{"down":true,"key":{"type":"qcode","data":"m"}}},
            {"type":"key","data":{"down":false,"key":{"type":"qcode","data":"m"}}},
            {"type":"key","data":{"down":true,"key":{"type":"qcode","data":"p"}}},
            {"type":"key","data":{"down":false,"key":{"type":"qcode","data":"p"}}},
            {"type":"key","data":{"down":true,"key":{"type":"qcode","data":"l"}}},
            {"type":"key","data":{"down":false,"key":{"type":"qcode","data":"l"}}},
            {"type":"key","data":{"down":true,"key":{"type":"qcode","data":"e"}}},
            {"type":"key","data":{"down":false,"key":{"type":"qcode","data":"e"}}},
            {"type":"key","data":{"down":true,"key":{"type":"qcode","data":"dot"}}},
            {"type":"key","data":{"down":false,"key":{"type":"qcode","data":"dot"}}},
            {"type":"key","data":{"down":true,"key":{"type":"qcode","data":"c"}}},
            {"type":"key","data":{"down":false,"key":{"type":"qcode","data":"c"}}},
            {"type":"key","data":{"down":true,"key":{"type":"qcode","data":"o"}}},
            {"type":"key","data":{"down":false,"key":{"type":"qcode","data":"o"}}},
            {"type":"key","data":{"down":true,"key":{"type":"qcode","data":"m"}}},
            {"type":"key","data":{"down":false,"key":{"type":"qcode","data":"m"}}},
            {"type":"key","data":{"down":true,"key":{"type":"qcode","data":"ret"}}},
            {"type":"key","data":{"down":false,"key":{"type":"qcode","data":"ret"}}}
        ]}
    }"#,
];

const M27_SYSTEM_A_MENU_EVENTS: [&str; 2] = [
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"a"}}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"a"}}}]}}"#,
];

const M27_BOOTSTRAP_COMPLETION_MARKER: &str = "Nagi M7 reboot required PASS";

const M27_RECOVERY_MENU_EVENTS: [&str; 2] = [
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"r"}}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"r"}}}]}}"#,
];

const M30_UNSTAGED_SYSTEM_B_MENU_EVENTS: [&str; 4] = [
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"b"}}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"b"}}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"a"}}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"a"}}}]}}"#,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Help,
    Doctor,
    Diagnostics,
    Verify,
    Smoke,
    Fetch,
    Build,
    Image,
    Run,
    Shell,
    Gui,
    Desktop,
    Security,
    Network,
    Posix,
    Std,
    M13,
    M14,
    M15,
    M16,
    M17,
    M18,
    M19,
    M29,
    M22,
    M25,
    M27,
    M30,
    M30Update,
    IsolatedProcess,
    Consent,
    Login,
    M20Granite,
    M20GraniteInference,
    M20LlamaSmoke,
    M26Qwen,
    M26Gemma,
    M25Whisper,
    M25WhisperInference,
    Test,
    Acceptance,
    Clean,
    Fmt,
    Lint,
    Dev,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CliError {
    message: String,
    code: i32,
}

impl CliError {
    pub fn new(message: impl Into<String>, code: i32) -> Self {
        Self {
            message: message.into(),
            code,
        }
    }

    pub fn exit_code(&self) -> i32 {
        self.code
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for CliError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandResult {
    pub exit_code: i32,
    pub lines: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ReportFormat {
    Text,
    Json,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ReportOptions {
    format: ReportFormat,
    output: Option<PathBuf>,
    scope: Option<String>,
    vm_smoke: bool,
}

fn parse_report_options(
    command: &str,
    args: &[String],
    allow_scope: bool,
    allow_smoke_mode: bool,
) -> Result<ReportOptions, CliError> {
    let mut options = ReportOptions {
        format: ReportFormat::Text,
        output: None,
        scope: None,
        vm_smoke: false,
    };
    let mut format_seen = false;
    let mut output_seen = false;
    let mut scope_seen = false;
    let mut smoke_mode_seen = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--json" => {
                if format_seen {
                    return Err(CliError::new(
                        format!("{command}: choose one output format"),
                        EXIT_USAGE,
                    ));
                }
                format_seen = true;
                options.format = ReportFormat::Json;
                index += 1;
            }
            "--format" => {
                if format_seen || index + 1 >= args.len() {
                    return Err(CliError::new(
                        format!("{command}: --format requires one value"),
                        EXIT_USAGE,
                    ));
                }
                format_seen = true;
                options.format = match args[index + 1].as_str() {
                    "text" => ReportFormat::Text,
                    "json" => ReportFormat::Json,
                    _ => {
                        return Err(CliError::new(
                            format!("{command}: format must be text or json"),
                            EXIT_USAGE,
                        ));
                    }
                };
                index += 2;
            }
            "--output" => {
                if output_seen || index + 1 >= args.len() || args[index + 1].is_empty() {
                    return Err(CliError::new(
                        format!("{command}: --output requires one path"),
                        EXIT_USAGE,
                    ));
                }
                output_seen = true;
                options.output = Some(PathBuf::from(&args[index + 1]));
                index += 2;
            }
            "--scope" => {
                if !allow_scope || scope_seen || index + 1 >= args.len() {
                    return Err(CliError::new(
                        format!("{command}: invalid or missing --scope value"),
                        EXIT_USAGE,
                    ));
                }
                let scope = args[index + 1].trim();
                if scope.is_empty()
                    || scope.len() > 96
                    || !scope.bytes().all(|byte| {
                        byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')
                    })
                {
                    return Err(CliError::new(
                        format!("{command}: scope must be a short identifier"),
                        EXIT_USAGE,
                    ));
                }
                scope_seen = true;
                options.scope = Some(scope.to_owned());
                index += 2;
            }
            "--vm" if allow_smoke_mode => {
                if smoke_mode_seen {
                    return Err(CliError::new(
                        "smoke: choose only one of --vm or --host-only",
                        EXIT_USAGE,
                    ));
                }
                smoke_mode_seen = true;
                options.vm_smoke = true;
                index += 1;
            }
            "--host-only" if allow_smoke_mode => {
                if smoke_mode_seen {
                    return Err(CliError::new(
                        "smoke: choose only one of --vm or --host-only",
                        EXIT_USAGE,
                    ));
                }
                smoke_mode_seen = true;
                index += 1;
            }
            flag => {
                return Err(CliError::new(
                    format!("{command}: unsupported option `{flag}`"),
                    EXIT_USAGE,
                ));
            }
        }
    }
    Ok(options)
}

pub fn parse_command(args: &[String]) -> Result<Command, CliError> {
    let Some(name) = args.first().map(String::as_str) else {
        return Ok(Command::Help);
    };

    let command = match name {
        "help" | "--help" | "-h" => Command::Help,
        "doctor" => Command::Doctor,
        "diagnostics" => Command::Diagnostics,
        "verify" => Command::Verify,
        "smoke" => Command::Smoke,
        "fetch" => Command::Fetch,
        "build" => Command::Build,
        "image" => Command::Image,
        "run" => Command::Run,
        "shell" => Command::Shell,
        "gui" => Command::Gui,
        "desktop" => Command::Desktop,
        "security" => Command::Security,
        "network" => Command::Network,
        "posix" => Command::Posix,
        "std" => Command::Std,
        "m13" => Command::M13,
        "m14" => Command::M14,
        "m15" => Command::M15,
        "m16" => Command::M16,
        "m17" => Command::M17,
        "test" if args.get(1).is_some_and(|arg| arg == "--acceptance") => Command::Acceptance,
        "m18" => Command::M18,
        "m19" => Command::M19,
        "m29" => Command::M29,
        "m22" => Command::M22,
        "m25" => Command::M25,
        "m27" => Command::M27,
        "m30" => Command::M30,
        "m30-update" => Command::M30Update,
        "isolated-process" => Command::IsolatedProcess,
        "consent" => Command::Consent,
        "login" => Command::Login,
        "m20-granite" => Command::M20Granite,
        "m20-granite-inference" => Command::M20GraniteInference,
        "m20-llama-smoke" => Command::M20LlamaSmoke,
        "m26-qwen" => Command::M26Qwen,
        "m26-gemma" => Command::M26Gemma,
        "m25-whisper" => Command::M25Whisper,
        "m25-whisper-inference" => Command::M25WhisperInference,
        "test" => Command::Test,
        "clean" => Command::Clean,
        "fmt" => Command::Fmt,
        "lint" => Command::Lint,
        "dev" => Command::Dev,
        other => {
            return Err(CliError::new(
                format!("unknown command `{other}`"),
                EXIT_USAGE,
            ));
        }
    };

    match command {
        Command::Diagnostics => {
            parse_report_options(name, &args[1..], true, false)?;
        }
        Command::Verify => {
            parse_report_options(name, &args[1..], true, false)?;
        }
        Command::Smoke => {
            parse_report_options(name, &args[1..], false, true)?;
        }
        _ => {}
    }

    let valid_arity = match command {
        Command::Doctor => {
            args.len() == 1 || args.get(1).is_some_and(|arg| arg == "--allow-missing")
        }
        Command::Dev => args.len() >= 2,
        Command::Test => args.len() == 1,
        Command::Acceptance => args.len() >= 2,
        Command::M20Granite => args.len() == 2,
        Command::M20GraniteInference => args.len() == 2,
        Command::M20LlamaSmoke => args.len() == 1,
        Command::M26Qwen => args.len() == 2,
        Command::M26Gemma => args.len() == 3 && args[2] == "--accept-gemma-terms",
        Command::M25Whisper => args.len() == 2,
        Command::M25WhisperInference => args.len() == 4,
        Command::Help
        | Command::Fetch
        | Command::Build
        | Command::Image
        | Command::Run
        | Command::Shell
        | Command::Gui
        | Command::Desktop
        | Command::Security
        | Command::Network
        | Command::Posix
        | Command::Std
        | Command::M13
        | Command::M14
        | Command::M15
        | Command::M16
        | Command::M17
        | Command::M18
        | Command::M19
        | Command::M29
        | Command::M22
        | Command::M25
        | Command::M27
        | Command::M30
        | Command::M30Update
        | Command::IsolatedProcess
        | Command::Consent
        | Command::Login
        | Command::Clean
        | Command::Fmt
        | Command::Lint => args.len() == 1,
        Command::Diagnostics | Command::Verify | Command::Smoke => true,
    };
    if !valid_arity {
        return Err(CliError::new(
            format!("unexpected argument for `{name}`"),
            EXIT_USAGE,
        ));
    }

    Ok(command)
}

pub fn host_workspace_args(command: &str) -> Vec<&'static str> {
    host_workspace_args_for_arch(command, std::env::consts::ARCH)
}

pub fn host_workspace_args_for_arch(command: &str, host_arch: &str) -> Vec<&'static str> {
    let command_name = match command {
        "build" => "build",
        "test" => "test",
        "clippy" => "clippy",
        _ => panic!("unsupported host workspace command: {command}"),
    };
    let mut args = vec![command_name];
    if host_arch == "x86_64" {
        args.push("--workspace");
    } else {
        // Nagi user-space syscall stubs currently use the x86_64 register ABI.
        // On other hosts, check the host-compatible workspace crates while
        // leaving those target-only crates to the Nagi target build.
        for package in [
            "nagi-cli",
            "nagi-idl",
            "nagi-bootinfo",
            "nagi-abi",
            "nagi-model",
            "nagi-i18n",
            "nagi-audio",
            "nagi-history",
            "nagi-package",
            "nagi-model-manager",
            "nagi-ui",
            "nagi-localization",
            "nagi-search",
            "nagi-ai",
            "nagi-servo-adapter",
        ] {
            args.extend(["--package", package]);
        }
    }

    if command == "clippy" {
        args.push("--all-targets");
    }

    if host_arch == "x86_64" {
        args.extend(["--exclude", "nagi-kernel"]);
    }

    args.push("--locked");
    if command == "clippy" {
        args.extend(["--", "-D", "warnings"]);
    }

    args
}

pub fn host_format_commands() -> Vec<(&'static str, Vec<&'static str>)> {
    let mut workspace_args = vec!["fmt"];
    for package in [
        "nagi-cli",
        "nagi-idl",
        "nagi-bootinfo",
        "nagi-abi",
        "nagi-model",
        "nagi-i18n",
        "nagi-kernel",
        "libnagi",
        "nagi-net",
        "nagi-pal",
        "nagi-posix",
        "nagi-audio",
        "nagi-history",
        "nagi-model-manager",
        "nagi-ui",
        "nagi-localization",
        "nagi-search",
        "nagi-ai",
        "nagi-servo-adapter",
        "nagi-init",
        "nagi-package",
        "nagi-sdk",
    ] {
        workspace_args.extend(["--package", package]);
    }
    workspace_args.extend(["--", "--check"]);

    vec![
        ("cargo", workspace_args),
        (
            "cargo",
            vec![
                "fmt",
                "--manifest-path",
                "tools/nagi-pkg/Cargo.toml",
                "--",
                "--check",
            ],
        ),
        (
            "cargo",
            vec![
                "fmt",
                "--manifest-path",
                "loader/Cargo.toml",
                "--",
                "--check",
            ],
        ),
        ("rustfmt", vec!["--check", "user/nagi-albert/src/lib.rs"]),
    ]
}

pub fn execute(args: &[String], root: &Path, probe: &dyn HostProbe) -> CommandResult {
    let command = match parse_command(args) {
        Ok(command) => command,
        Err(error) => return failure(error.exit_code(), error.to_string()),
    };

    if command == Command::Help {
        return help();
    }

    if let Err(error) = validate_project(root) {
        return failure(EXIT_CONFIG_ERROR, format!("project: {error}"));
    }

    match command {
        Command::Help => help(),
        Command::Doctor => execute_doctor(&args[1..], root, probe),
        Command::Diagnostics => execute_diagnostics(&args[1..], root, probe),
        Command::Verify => execute_verify(&args[1..], root, probe),
        Command::Smoke => execute_smoke(&args[1..], root, probe),
        Command::Fetch => execute_fetch(root),
        Command::Build => run_cargo(root, "build", &host_workspace_args("build")),
        Command::Test => run_cargo(root, "test", &host_workspace_args("test")),
        Command::Acceptance => crate::acceptance::execute(&args[2..], root),
        Command::Fmt => execute_format(root),
        Command::Lint => run_cargo(root, "lint", &host_workspace_args("clippy")),
        Command::Clean => execute_clean(root),
        Command::Image => execute_image(root),
        Command::Run => execute_run(root, probe),
        Command::Shell => execute_shell(root, probe),
        Command::Gui => execute_gui(root, probe),
        Command::Desktop => execute_desktop(root, probe),
        Command::M29 => execute_m29(root, probe),
        Command::Security => execute_security(root, probe),
        Command::Network => execute_network(root, probe),
        Command::Posix => execute_posix(root, probe),
        Command::Std => execute_std(root, probe),
        Command::M13 => execute_m13(root, probe),
        Command::M14 => execute_m14(root, probe),
        Command::M15 => execute_m15(root, probe),
        Command::M16 => execute_m16(root, probe),
        Command::M17 => execute_m17(root, probe),
        Command::M18 => execute_m18(root, probe),
        Command::M19 => execute_m19(root, probe),
        Command::M22 => execute_m22(root, probe),
        Command::IsolatedProcess => execute_isolated_process(root, probe),
        Command::Consent => execute_consent(root, probe),
        Command::Login => execute_login(root, probe),
        Command::M25 => execute_m25(root, probe),
        Command::M27 => execute_m27(root, probe),
        Command::M30 => execute_m30(root, probe),
        Command::M30Update => execute_m30_update(root, probe),
        Command::M20Granite => execute_m20_granite(&args[1..], root, probe),
        Command::M20GraniteInference => execute_m20_granite_inference(&args[1..], root, probe),
        Command::M20LlamaSmoke => execute_m20_llama_smoke(root, probe),
        Command::M26Qwen => execute_m26_model(&args[1..], root, probe, M26Model::Qwen),
        Command::M26Gemma => execute_m26_model(&args[1..], root, probe, M26Model::Gemma),
        Command::M25Whisper => execute_m25_whisper(&args[1..], root, probe),
        Command::M25WhisperInference => execute_m25_whisper_inference(&args[1..], root, probe),
        Command::Dev => execute_dev(&args[1..], root),
    }
}

fn execute_dev(args: &[String], root: &Path) -> CommandResult {
    match crate::development::execute(args, root) {
        Ok(lines) => CommandResult {
            exit_code: EXIT_SUCCESS,
            lines,
        },
        Err(error) => failure(error.exit_code(), error.to_string()),
    }
}

fn execute_doctor(args: &[String], root: &Path, probe: &dyn HostProbe) -> CommandResult {
    let policy = match args {
        [] => DoctorPolicy::Strict,
        [flag] if flag == "--allow-missing" => DoctorPolicy::AllowMissing,
        _ => return failure(EXIT_USAGE, "doctor accepts only --allow-missing"),
    };
    let requirements = match load_toolchain_requirements(root) {
        Ok(requirements) => requirements,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("toolchain: {error}")),
    };
    let report = run_doctor_with_requirements(probe, policy, &requirements);
    let summary = report.summary();
    let mut lines: Vec<String> = report
        .checks
        .into_iter()
        .map(|check| check.render())
        .collect();
    lines.push(summary);
    CommandResult {
        exit_code: report.exit_code,
        lines,
    }
}

fn execute_diagnostics(args: &[String], root: &Path, probe: &dyn HostProbe) -> CommandResult {
    let options = match parse_report_options("diagnostics", args, true, false) {
        Ok(options) => options,
        Err(error) => return failure(error.exit_code(), error.to_string()),
    };
    let source_commit = read_source_commit(root);
    let verification =
        run_verification_report(root, probe, options.scope.clone(), source_commit.clone());
    let event = match DiagnosticEvent::new(
        Severity::Info,
        "nagi-cli",
        "DIAGNOSTICS.REPORT_CREATED",
        "local diagnostics report generated",
    )
    .and_then(|event| {
        event.with_field(
            "verification_outcome",
            verification.outcome.as_str(),
            PrivacyClass::Public,
        )
    }) {
        Ok(event) => event,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("diagnostics: {error}")),
    };
    let bundle = DiagnosticsBundle {
        schema_version: DIAGNOSTIC_BUNDLE_SCHEMA_VERSION,
        generated_at_unix_ms: verification.generated_at_unix_ms,
        source_commit,
        host_os: std::env::consts::OS.to_owned(),
        host_arch: std::env::consts::ARCH.to_owned(),
        verification,
        events: vec![event.safe_view()],
    };
    let json = match bundle.to_json() {
        Ok(json) => json,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("diagnostics: {error}")),
    };
    report_command_result(
        options.format,
        options.output,
        json,
        bundle.render_human(),
        bundle.verification.outcome,
    )
}

fn execute_verify(args: &[String], root: &Path, probe: &dyn HostProbe) -> CommandResult {
    let options = match parse_report_options("verify", args, true, false) {
        Ok(options) => options,
        Err(error) => return failure(error.exit_code(), error.to_string()),
    };
    let report = run_verification_report(root, probe, options.scope, read_source_commit(root));
    let json = match report.to_json() {
        Ok(json) => json,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("verify: {error}")),
    };
    report_command_result(
        options.format,
        options.output,
        json,
        report.render_human(),
        report.outcome,
    )
}

fn execute_smoke(args: &[String], root: &Path, probe: &dyn HostProbe) -> CommandResult {
    let options = match parse_report_options("smoke", args, false, true) {
        Ok(options) => options,
        Err(error) => return failure(error.exit_code(), error.to_string()),
    };
    let mut report = run_verification_report(root, probe, None, read_source_commit(root));
    if options.vm_smoke {
        let vm_result = execute_run(root, probe);
        let evidence = vm_result
            .lines
            .iter()
            .take(crate::diagnostics::MAX_FIELD_COUNT)
            .cloned()
            .collect::<Vec<_>>();
        let summary = if vm_result.lines.is_empty() {
            format!(
                "existing QEMU boot acceptance exited with {}",
                vm_result.exit_code
            )
        } else {
            vm_result.lines.join("; ")
        };
        let vm_check = if vm_result.exit_code == EXIT_SUCCESS {
            VerificationCheckResult::pass(
                "m7-qemu-smoke",
                "vm",
                "M1/M7 guest boot acceptance",
                EvidenceKind::Vm,
                summary,
                evidence,
            )
        } else {
            VerificationCheckResult::fail(
                "m7-qemu-smoke",
                "vm",
                "M1/M7 guest boot acceptance",
                EvidenceKind::Vm,
                FailureClass::Acceptance,
                summary,
                evidence,
            )
        };
        report.checks.push(vm_check);
        report = VerificationReport::new(
            report.checks,
            report.requested_scope,
            report.source_commit,
            report.generated_at_unix_ms,
        );
    }
    let json = match report.to_json() {
        Ok(json) => json,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("smoke: {error}")),
    };
    report_command_result(
        options.format,
        options.output,
        json,
        report.render_human(),
        report.outcome,
    )
}

fn run_verification_report(
    root: &Path,
    probe: &dyn HostProbe,
    scope: Option<String>,
    source_commit: Option<String>,
) -> VerificationReport {
    let mut registry = HealthCheckRegistry::default();
    let registrations = [
        (
            "repository-layout",
            "repository",
            check_repository_layout as fn(&Path) -> VerificationCheckResult,
        ),
        (
            "diagnostics-contract",
            "diagnostics",
            check_diagnostics_contract as fn(&Path) -> VerificationCheckResult,
        ),
        (
            "workstream-state",
            "workstreams",
            check_workstream_state_boundary as fn(&Path) -> VerificationCheckResult,
        ),
    ];
    for (id, check_scope, check_fn) in registrations {
        let check = ClosureHealthCheck::new(
            id,
            check_scope,
            move |context: &crate::diagnostics::VerificationContext| {
                vec![check_fn(&context.repository_root)]
            },
        );
        if let Err(error) = registry.register(check) {
            return VerificationReport::new(
                vec![VerificationCheckResult::fail(
                    "health-check-registry",
                    "diagnostics",
                    "Health-check registration",
                    EvidenceKind::Host,
                    FailureClass::Unknown,
                    error.to_string(),
                    Vec::new(),
                )],
                scope,
                source_commit,
                crate::diagnostics::now_unix_ms(),
            );
        }
    }

    let mut checks = registry
        .execute(root, scope.clone(), source_commit.clone())
        .checks;
    let wants_host = scope.as_deref().is_none_or(|selected| {
        selected == "host" || selected == "host-toolchain" || selected.starts_with("host-")
    });
    if wants_host {
        checks.extend(host_doctor_results(root, probe, scope.as_deref()));
    }
    if checks.is_empty() {
        checks.push(VerificationCheckResult::fail(
            "unregistered-scope",
            "diagnostics",
            "Verification scope",
            EvidenceKind::Host,
            FailureClass::Source,
            format!(
                "no checks are registered for scope `{}`",
                scope.as_deref().unwrap_or("(none)")
            ),
            Vec::new(),
        ));
    }
    VerificationReport::new(
        checks,
        scope,
        source_commit,
        crate::diagnostics::now_unix_ms(),
    )
}

fn check_repository_layout(root: &Path) -> VerificationCheckResult {
    let required = [
        "Cargo.toml",
        "Cargo.lock",
        "AGENTS.md",
        "docs/Nagi_OS_0.1_Codex_Implementation_Spec.md",
        "docs/implementation_status.md",
    ];
    let missing = required
        .iter()
        .filter(|path| !root.join(path).is_file())
        .copied()
        .collect::<Vec<_>>();
    let evidence = required
        .iter()
        .map(|path| {
            format!(
                "{} {}",
                if root.join(path).is_file() {
                    "present:"
                } else {
                    "missing:"
                },
                path
            )
        })
        .collect::<Vec<_>>();
    if missing.is_empty() {
        VerificationCheckResult::pass(
            "repository-required-files",
            "repository",
            "Required repository files",
            EvidenceKind::Host,
            "repository manifests and authority documents are present",
            evidence,
        )
    } else {
        VerificationCheckResult::fail(
            "repository-required-files",
            "repository",
            "Required repository files",
            EvidenceKind::Host,
            FailureClass::Source,
            format!("missing required files: {}", missing.join(", ")),
            evidence,
        )
    }
}

fn check_diagnostics_contract(root: &Path) -> VerificationCheckResult {
    let schema_path = root.join("docs/testing/diagnostic-report.schema.json");
    let schema_text = match fs::read_to_string(&schema_path) {
        Ok(text) => text,
        Err(error) => {
            return VerificationCheckResult::fail(
                "diagnostics-report-schema",
                "diagnostics",
                "Diagnostics report schema",
                EvidenceKind::Host,
                FailureClass::Source,
                format!("cannot read report schema: {error}"),
                vec!["docs/testing/diagnostic-report.schema.json".into()],
            );
        }
    };
    let schema: serde_json::Value = match serde_json::from_str(&schema_text) {
        Ok(schema) => schema,
        Err(error) => {
            return VerificationCheckResult::fail(
                "diagnostics-report-schema",
                "diagnostics",
                "Diagnostics report schema",
                EvidenceKind::Host,
                FailureClass::Source,
                format!("report schema is invalid JSON: {error}"),
                vec!["docs/testing/diagnostic-report.schema.json".into()],
            );
        }
    };
    let schema_matches = schema["properties"]["schema_version"]["const"] == 1
        && schema["$defs"]["verificationReport"]["properties"]["schema_version"]["const"] == 1
        && schema["$defs"]["safeDiagnosticEvent"]["properties"]["schema_version"]["const"] == 1;
    let event_round_trip = DiagnosticEvent::new(
        Severity::Info,
        "diagnostics",
        "DIAGNOSTICS.SCHEMA_CHECK",
        "schema contract self-check",
    )
    .and_then(|event| event.to_json())
    .and_then(|json| DiagnosticEvent::from_json(&json).map(|_| ()))
    .is_ok();
    if schema_matches && event_round_trip {
        VerificationCheckResult::pass(
            "diagnostics-report-schema",
            "diagnostics",
            "Diagnostics report schema",
            EvidenceKind::Host,
            "versioned bundle schema parses and event serialization round-trips",
            vec!["docs/testing/diagnostic-report.schema.json".into()],
        )
    } else {
        VerificationCheckResult::fail(
            "diagnostics-report-schema",
            "diagnostics",
            "Diagnostics report schema",
            EvidenceKind::Host,
            FailureClass::Source,
            "schema version or event round-trip contract does not match the implementation",
            vec!["docs/testing/diagnostic-report.schema.json".into()],
        )
    }
}

fn check_workstream_state_boundary(root: &Path) -> VerificationCheckResult {
    if !root.join(".dev/workstreams.json").is_file() {
        return VerificationCheckResult::skipped(
            "workstream-state-registry",
            "workstreams",
            "DF-01 workstream state",
            EvidenceKind::Host,
            "DF-01 registry is not present in this checkout",
        );
    }

    match crate::development::execute(&["verify".into()], root) {
        Ok(lines) => VerificationCheckResult::pass(
            "workstream-state-registry",
            "workstreams",
            "DF-01 workstream state",
            EvidenceKind::Host,
            lines.join("; "),
            vec!["./nagi dev verify".into(), ".dev/workstreams.json".into()],
        ),
        Err(error) => VerificationCheckResult::fail(
            "workstream-state-registry",
            "workstreams",
            "DF-01 workstream state",
            EvidenceKind::Host,
            FailureClass::Source,
            format!("DF-01 state verification failed: {error}"),
            vec!["./nagi dev verify".into(), ".dev/workstreams.json".into()],
        ),
    }
}

fn host_doctor_results(
    root: &Path,
    probe: &dyn HostProbe,
    selected_scope: Option<&str>,
) -> Vec<VerificationCheckResult> {
    let requirements = match load_toolchain_requirements(root) {
        Ok(requirements) => requirements,
        Err(error) => {
            return vec![VerificationCheckResult::fail(
                "host-toolchain-config",
                "host",
                "Host toolchain requirements",
                EvidenceKind::Host,
                FailureClass::Source,
                format!("cannot load toolchain requirements: {error}"),
                vec!["nagi.toml".into()],
            )];
        }
    };
    run_doctor_with_requirements(probe, DoctorPolicy::Strict, &requirements)
        .checks
        .into_iter()
        .map(|check| {
            let id = format!("host-{}", check_name_slug(check.name));
            let detail = safe_doctor_detail(check.name, &check.detail, check.state);
            match check.state {
                CheckState::Pass => VerificationCheckResult::pass(
                    id,
                    "host",
                    check.name,
                    EvidenceKind::Host,
                    detail,
                    vec![format!("nagi doctor: {}", check.name)],
                ),
                CheckState::Warn => VerificationCheckResult::skipped(
                    id,
                    "host",
                    check.name,
                    EvidenceKind::Host,
                    detail,
                ),
                CheckState::Fail => VerificationCheckResult::fail(
                    id,
                    "host",
                    check.name,
                    EvidenceKind::Host,
                    FailureClass::HostEnv,
                    detail,
                    vec![format!("nagi doctor: {}", check.name)],
                ),
            }
        })
        .filter(|check| {
            selected_scope.is_none_or(|scope| {
                scope == "host" || scope == "host-toolchain" || scope == check.id
            })
        })
        .collect()
}

fn check_name_slug(name: &str) -> String {
    let mut slug = String::new();
    let mut separator = false;
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            if separator && !slug.is_empty() {
                slug.push('-');
            }
            slug.push(character.to_ascii_lowercase());
            separator = false;
        } else {
            separator = true;
        }
    }
    slug
}

fn safe_doctor_detail(name: &str, detail: &str, state: CheckState) -> String {
    let lowered = detail.to_ascii_lowercase();
    if lowered.contains("not found") {
        return "required host dependency was not found".into();
    }
    if state == CheckState::Pass {
        if name == "OVMF CODE/VARS" {
            return "compatible, allow-listed OVMF CODE/VARS pair found".into();
        }
        if let Some((_, version)) = detail.rsplit_once(" (") {
            return format!("available: {}", version.trim_end_matches(')'));
        }
        return "required host dependency is available".into();
    }
    if lowered.contains("incompatible") {
        "OVMF CODE/VARS firmware pair is incompatible".into()
    } else if lowered.contains("not listed") {
        "OVMF CODE/VARS pair is not in the project allow-list".into()
    } else if lowered.contains("unsupported") || lowered.contains("minimum") {
        "host tool identity or minimum version check failed".into()
    } else if lowered.contains("did not identify") {
        "version probe returned unexpected program output".into()
    } else if lowered.contains("failed with exit code") {
        "version probe exited unsuccessfully".into()
    } else {
        format!("{name} host check failed")
    }
}

fn read_source_commit(root: &Path) -> Option<String> {
    let output = ProcessCommand::new("git")
        .current_dir(root)
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let commit = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    (commit.len() == 40 && commit.bytes().all(|byte| byte.is_ascii_hexdigit())).then_some(commit)
}

fn report_command_result(
    format: ReportFormat,
    output: Option<PathBuf>,
    json: String,
    human: String,
    outcome: OverallStatus,
) -> CommandResult {
    let payload = match format {
        ReportFormat::Text => human,
        ReportFormat::Json => json,
    };
    let exit_code = if outcome == OverallStatus::Pass {
        EXIT_SUCCESS
    } else {
        EXIT_VERIFY_FAILURE
    };
    if let Some(path) = output {
        if let Err(error) = fs::write(&path, payload) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("cannot write report {}: {error}", path.display()),
            );
        }
        return CommandResult {
            exit_code,
            lines: vec![
                format!("{} report: {}", outcome.as_str(), path.display()),
                format!("report written to {}", path.display()),
            ],
        };
    }
    CommandResult {
        exit_code,
        lines: payload.lines().map(str::to_owned).collect(),
    }
}

fn execute_fetch(root: &Path) -> CommandResult {
    let lock = root.join("third_party").join("sources.lock");
    let lock_contents = match fs::read_to_string(&lock) {
        Ok(contents) => contents,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "fetch: cannot read third_party/sources.lock; source fetching is not reproducible: {error}"
                ),
            );
        }
    };
    for required in [
        "[sources.smoltcp]",
        "version = \"0.12.0\"",
        "revision = \"d2d647090d544b1e7c142571da9d55f7280f664b\"",
        "source_hash = \"sha256:dad095989c1533c1c266d9b1e8d70a1329dd3723c3edac6d03bbd67e7bf6f4bb\"",
        "license = \"0BSD\"",
    ] {
        if !lock_contents.lines().any(|line| line.trim() == required) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "fetch: third_party/sources.lock is missing pinned smoltcp field `{required}`"
                ),
            );
        }
    }
    let llama_cpp = match ensure_llama_cpp_checkout(root) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("fetch: {error}")),
    };
    let whisper_cpp = match ensure_whisper_cpp_checkout(root) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("fetch: {error}")),
    };
    let surfman = match ensure_surfman_checkout(root) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("fetch: {error}")),
    };
    let tempfile_nagi = match ensure_tempfile_nagi_checkout(root) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("fetch: {error}")),
    };
    let mozjs_sys_nagi = match ensure_mozjs_sys_nagi_checkout(root) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("fetch: {error}")),
    };
    let cc_nagi = match ensure_cc_nagi_checkout(root) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("fetch: {error}")),
    };
    let servo = match ensure_servo_checkout(root) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("fetch: {error}")),
    };
    if let Err(error) = crate::fonts::ensure_font_cache(root) {
        return failure(EXIT_CONFIG_ERROR, format!("fetch: fonts: {error}"));
    }
    let servo_relative = servo
        .strip_prefix(root)
        .unwrap_or(Path::new("third_party/servo"));
    let servo_fetch = run_cargo_in(root, servo_relative, "Servo fetch", &["fetch", "--locked"]);
    if servo_fetch.exit_code != EXIT_SUCCESS {
        return servo_fetch;
    }
    let mesa = match ensure_mesa_checkout(root) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("fetch: {error}")),
    };
    // The parent workspace has a pinned path patch for Servo's generated libc.
    // It is materialized above and the committed root lock records that path
    // source, so every subsequent locked build resolves the same Nagi-owned
    // checkout rather than falling back to the registry copy.
    let mesa_relative = mesa
        .strip_prefix(root)
        .unwrap_or(Path::new("third_party/mesa"));
    CommandResult {
        exit_code: EXIT_SUCCESS,
        lines: vec![format!(
            "PASS fetch: Cargo registry sources fetched; pinned smoltcp, Surfman, tempfile, mozjs_sys, cc, Servo, Mesa/Softpipe, and llama.cpp sources validated; pinned whisper.cpp source and Nagi patch validated with Whisper small model metadata ({}, {}, {}, {}, {}, {}, {}, {})",
            llama_cpp
                .strip_prefix(root)
                .unwrap_or(Path::new("third_party/llama.cpp"))
                .display(),
            surfman
                .strip_prefix(root)
                .unwrap_or(Path::new("third_party/surfman"))
                .display(),
            tempfile_nagi
                .strip_prefix(root)
                .unwrap_or(Path::new("third_party/tempfile-nagi"))
                .display(),
            mozjs_sys_nagi
                .strip_prefix(root)
                .unwrap_or(Path::new("third_party/mozjs-sys-nagi"))
                .display(),
            cc_nagi
                .strip_prefix(root)
                .unwrap_or(Path::new("third_party/cc-nagi"))
                .display(),
            servo_relative.display(),
            mesa_relative.display(),
            whisper_cpp
                .strip_prefix(root)
                .unwrap_or(Path::new("out/cache/whisper-cpp-nagi"))
                .display()
        )],
    }
}

fn execute_clean(root: &Path) -> CommandResult {
    match clean_owned_outputs(root) {
        Ok(removed) => CommandResult {
            exit_code: EXIT_SUCCESS,
            lines: vec![format!(
                "PASS clean: removed {} repository-owned output path(s)",
                removed
            )],
        },
        Err(error) => failure(EXIT_CONFIG_ERROR, format!("clean: {error}")),
    }
}

fn execute_image(root: &Path) -> CommandResult {
    execute_image_with_features(root, None, "nagi-0.1-m1.img")
}

fn execute_image_with_mode(root: &Path, shell_mode: bool) -> CommandResult {
    if shell_mode {
        execute_image_with_features(root, Some("m8-shell"), "nagi-0.1-m8-shell.img")
    } else {
        execute_image_with_features(root, None, "nagi-0.1-m1.img")
    }
}

fn execute_image_with_features(
    root: &Path,
    init_feature: Option<&str>,
    image_name: &str,
) -> CommandResult {
    let init_args = match init_feature {
        Some("m19-search") => vec![
            "build",
            "-p",
            "nagi-init",
            "--features",
            "m19-search",
            "--target",
            "targets/x86_64-unknown-nagi-user.json",
            "-Zbuild-std=core,alloc,compiler_builtins",
            "--release",
        ],
        Some("m22-history") => vec![
            "build",
            "-p",
            "nagi-init",
            "--features",
            "m22-history",
            "--target",
            "targets/x86_64-unknown-nagi-user.json",
            "-Zbuild-std=core,alloc,compiler_builtins",
            "--release",
        ],
        Some(feature) => vec![
            "build",
            "-p",
            "nagi-init",
            "--features",
            feature,
            "--target",
            "targets/x86_64-unknown-nagi-user.json",
            "-Zbuild-std=core,alloc,compiler_builtins",
            "--release",
        ],
        None => vec![
            "build",
            "-p",
            "nagi-init",
            "--target",
            "targets/x86_64-unknown-nagi-user.json",
            "-Zbuild-std=core,alloc,compiler_builtins",
            "--release",
        ],
    };
    execute_image_with_init_build(root, &init_args, None, image_name)
}

fn execute_image_with_std(root: &Path, image_name: &str) -> CommandResult {
    let source_root = match prepare_nagi_rust_std_source(root) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("rust std: {error}")),
    };
    let init_args = [
        "build",
        "-p",
        "nagi-init",
        "--features",
        "m13-std",
        "--target",
        "targets/x86_64-unknown-nagi-user.json",
        "-Zbuild-std=std,panic_abort",
        "--release",
        "--locked",
        "--offline",
    ];
    execute_image_with_init_build(root, &init_args, Some(&source_root), image_name)
}

fn execute_image_with_init_build(
    root: &Path,
    init_args: &[&str],
    rust_std_source: Option<&Path>,
    image_name: &str,
) -> CommandResult {
    execute_image_with_init_build_env(root, init_args, rust_std_source, image_name, &[])
}

fn execute_image_with_init_build_env(
    root: &Path,
    init_args: &[&str],
    rust_std_source: Option<&Path>,
    image_name: &str,
    cargo_env: &[(&str, &Path)],
) -> CommandResult {
    execute_image_with_init_build_env_using_writer(
        root,
        init_args,
        rust_std_source,
        image_name,
        cargo_env,
        write_fat12_image,
        ImageBuildFeatures::default(),
    )
}

/// Whether `init_args` build the Servo-enabled init (M17/M18 features).
fn servo_enabled_init(init_args: &[&str]) -> bool {
    init_args.windows(2).any(|pair| {
        pair[0] == "--features"
            && pair[1]
                .split(',')
                .any(|feature| matches!(feature.trim(), "m17-servo" | "m18-acceptance"))
    })
}

/// Write `<init>.image`, the init ELF without symbol tables, using the
/// `llvm-objcopy` the Servo/Mesa build already requires.
fn strip_init_for_image(init_path: &Path) -> Result<PathBuf, String> {
    let stripped = init_path.with_extension("image");
    let output = ProcessCommand::new("llvm-objcopy")
        .arg("--strip-all")
        .arg(init_path)
        .arg(&stripped)
        .output()
        .map_err(|error| format!("cannot start llvm-objcopy to strip the init ELF: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "llvm-objcopy could not strip {}: {}",
            init_path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(stripped)
}

fn execute_image_with_init_build_env_using_writer(
    root: &Path,
    init_args: &[&str],
    rust_std_source: Option<&Path>,
    image_name: &str,
    cargo_env: &[(&str, &Path)],
    image_writer: ImageWriter,
    build_features: ImageBuildFeatures<'_>,
) -> CommandResult {
    execute_image_with_init_build_env_using_writer_and_recovery(
        root,
        init_args,
        rust_std_source,
        ImageBuildRequest {
            image_name,
            cargo_env,
            recovery_init: None,
            image_writer,
            external_model_store_file: None,
            build_features,
        },
    )
}

fn execute_image_with_init_build_env_using_writer_and_recovery(
    root: &Path,
    init_args: &[&str],
    rust_std_source: Option<&Path>,
    request: ImageBuildRequest<'_>,
) -> CommandResult {
    let ImageBuildRequest {
        image_name,
        cargo_env,
        recovery_init,
        image_writer,
        external_model_store_file,
        build_features,
    } = request;
    let init_build = match rust_std_source {
        Some(source) => {
            run_cargo_with_rust_std_source(root, "user init", init_args, source, cargo_env)
        }
        None => run_cargo_with_env(root, "user init", init_args, cargo_env),
    };
    if init_build.exit_code != EXIT_SUCCESS {
        return init_build;
    }
    let kernel_features = build_features.kernel.join(",");
    let mut kernel_args = vec!["build", "-p", "nagi-kernel"];
    if !kernel_features.is_empty() {
        kernel_args.extend(["--features", kernel_features.as_str()]);
    }
    kernel_args.extend([
        "--target",
        "targets/x86_64-unknown-nagi.json",
        "-Zbuild-std=core,compiler_builtins",
        "--release",
    ]);
    let kernel_build = run_cargo(root, "kernel", &kernel_args);
    if kernel_build.exit_code != EXIT_SUCCESS {
        return kernel_build;
    }
    let loader_features = build_features.loader.join(",");
    let mut loader_args = vec!["build", "--manifest-path", "loader/Cargo.toml"];
    if !loader_features.is_empty() {
        loader_args.extend(["--features", loader_features.as_str()]);
    }
    loader_args.extend(["--target", "x86_64-unknown-uefi", "--release", "--locked"]);
    let loader_build = run_cargo(root, "loader", &loader_args);
    if loader_build.exit_code != EXIT_SUCCESS {
        return loader_build;
    }

    let kernel_path = root
        .join("target")
        .join("x86_64-unknown-nagi")
        .join("release")
        .join("nagi-kernel");
    let init_path = root
        .join("target")
        .join("x86_64-unknown-nagi-user")
        .join("release")
        .join("nagi-init");
    let loader_path = root
        .join("loader")
        .join("target")
        .join("x86_64-unknown-uefi")
        .join("release")
        .join("nagi-loader.efi");
    let kernel = match fs::read(&kernel_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("image: cannot read {}: {error}", kernel_path.display()),
            );
        }
    };
    let loader = match fs::read(&loader_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("image: cannot read {}: {error}", loader_path.display()),
            );
        }
    };
    // Servo-enabled init ELFs carry ~20 MiB of symbol tables the guest never
    // reads. Boot images get a stripped copy so they stay inside the FAT12
    // per-file limit; the symbol-bearing ELF stays in target/ for debugging.
    let init_path = if servo_enabled_init(init_args) {
        match strip_init_for_image(&init_path) {
            Ok(path) => path,
            Err(error) => return failure(EXIT_CONFIG_ERROR, format!("image: {error}")),
        }
    } else {
        init_path
    };
    let init = match fs::read(&init_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("image: cannot read {}: {error}", init_path.display()),
            );
        }
    };

    let artifacts = match ensure_owned_directory(root, Path::new("out").join("artifacts")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("image: {error}")),
    };
    let image_path = artifacts.join(image_name);
    let layout_result = match external_model_store_file {
        Some((file_name, source_path)) => {
            write_reference_disk_qcow2_with_external_model_store_file(
                &image_path,
                &loader,
                &kernel,
                &init,
                recovery_init,
                file_name,
                source_path,
            )
        }
        None => image_writer(&image_path, &loader, &kernel, &init, recovery_init),
    };
    let layout = match layout_result {
        Ok(layout) => layout,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("image: {error}")),
    };
    CommandResult {
        exit_code: EXIT_SUCCESS,
        lines: vec![format!(
            "PASS image: {} ({} bytes; loader clusters {}..{}; kernel clusters {}..{}; init clusters {}..{})",
            image_path.display(),
            fs::metadata(&image_path)
                .map(|metadata| metadata.len())
                .unwrap_or_default(),
            layout.bootloader_start_cluster,
            layout.bootloader_start_cluster + layout.bootloader_clusters as u16 - 1,
            layout.kernel_start_cluster,
            layout.kernel_start_cluster + layout.kernel_clusters as u16 - 1,
            layout.init_start_cluster,
            layout.init_start_cluster + layout.init_clusters as u16 - 1,
        )],
    }
}

struct ModelStoreArtifactProfile {
    command_name: &'static str,
    model_id: String,
    artifact_id: String,
    source_uri: String,
    source_revision: String,
    file_name: String,
    format: String,
    size_bytes: u64,
    sha256: String,
    init_feature: &'static str,
    kernel_features: &'static [&'static str],
    image_prefix: &'static str,
    evidence_prefix: &'static str,
    vars_name: &'static str,
    serial_name: &'static str,
    digest_marker: &'static str,
}

struct ModelStoreEvidenceInput<'a> {
    environment: Option<&'a str>,
    source: &'a Path,
    evidence_name: &'a str,
}

struct ModelStoreAcceptanceOptions<'a> {
    acceptance_marker: &'a str,
    required_markers: &'a [&'a str],
    early_exit_markers: &'a [&'a str],
    cargo_env: &'a [(&'a str, &'a Path)],
    evidence_inputs: &'a [ModelStoreEvidenceInput<'a>],
    timeout: Duration,
    claims: &'a str,
    success_summary: &'a str,
}

fn execute_m20_granite(args: &[String], root: &Path, probe: &dyn HostProbe) -> CommandResult {
    let manifest = match pinned_granite_manifest(root) {
        Ok(manifest) => manifest,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m20-granite: {error}")),
    };
    let Some(source) = manifest.source.as_ref() else {
        return failure(
            EXIT_CONFIG_ERROR,
            "m20-granite: manifest source metadata is missing",
        );
    };
    let (
        nagi_model_manager::ArtifactReference::ModelStore { artifact_id },
        Some(size_bytes),
        Some(integrity),
    ) = (
        &manifest.artifact.reference,
        manifest.artifact.size_bytes,
        manifest.artifact.integrity.as_ref(),
    )
    else {
        return failure(
            EXIT_CONFIG_ERROR,
            "m20-granite: pinned manifest lacks Model Store size or integrity metadata",
        );
    };
    execute_model_store_artifact(
        args,
        root,
        probe,
        ModelStoreArtifactProfile {
            command_name: "m20-granite",
            model_id: manifest.model_id.as_str().to_owned(),
            artifact_id: artifact_id.as_str().to_owned(),
            source_uri: source.uri.clone(),
            source_revision: source.revision.clone(),
            file_name: source.file_name.clone(),
            format: manifest.artifact.format.as_str().to_owned(),
            size_bytes,
            sha256: integrity.digest.clone(),
            init_feature: "m20-granite-artifact-acceptance",
            kernel_features: &[],
            image_prefix: "nagi-0.1-m20-granite",
            evidence_prefix: "m20-granite-artifact",
            vars_name: "granite-OVMF_VARS.fd",
            serial_name: "m20-granite-qemu.log",
            digest_marker: "Nagi M20 Granite artifact digest PASS",
        },
    )
}

fn execute_m20_granite_inference(
    args: &[String],
    root: &Path,
    probe: &dyn HostProbe,
) -> CommandResult {
    let manifest = match pinned_granite_manifest(root) {
        Ok(manifest) => manifest,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m20-granite-inference: {error}")),
    };
    let Some(source) = manifest.source.as_ref() else {
        return failure(
            EXIT_CONFIG_ERROR,
            "m20-granite-inference: manifest source metadata is missing",
        );
    };
    let (
        nagi_model_manager::ArtifactReference::ModelStore { artifact_id },
        Some(size_bytes),
        Some(integrity),
    ) = (
        &manifest.artifact.reference,
        manifest.artifact.size_bytes,
        manifest.artifact.integrity.as_ref(),
    )
    else {
        return failure(
            EXIT_CONFIG_ERROR,
            "m20-granite-inference: pinned manifest lacks Model Store size or integrity metadata",
        );
    };

    let run_id = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_nanos().to_string(),
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m20-granite-inference: system clock: {error}"),
            )
        }
    };
    let target_evidence = root
        .join("out/evidence")
        .join(format!("m20-granite-inference-target-{run_id}"));
    if let Err(error) = fs::create_dir_all(&target_evidence) {
        return failure(
            EXIT_CONFIG_ERROR,
            format!("m20-granite-inference: create target evidence directory: {error}"),
        );
    }

    let llama_source = match ensure_llama_cpp_checkout(root) {
        Ok(path) => path,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m20-granite-inference: pinned llama.cpp source: {error}"),
            )
        }
    };
    let target_clang = match resolve_m20_target_clang() {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m20-granite-inference: {error}")),
    };
    let cxx_headers = match resolve_m20_cxx_headers(&target_clang) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m20-granite-inference: {error}")),
    };
    let relibc_headers = std::env::var_os("NAGI_RELIBC_HEADERS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            root.join("out/m17-mesa/relibc-target/x86_64-unknown-nagi-user/include")
        });
    if !relibc_headers.join("pthread.h").is_file() {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "m20-granite-inference: generated Nagi relibc headers are missing at {}; complete the target header generation first",
                relibc_headers.display()
            ),
        );
    }
    let llvm_bin = target_clang.parent().unwrap_or_else(|| Path::new("."));
    let llvm_ar = match resolve_m20_llvm_tool("NAGI_LLVM_AR", "llvm-ar", llvm_bin) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m20-granite-inference: {error}")),
    };
    let llvm_ranlib = match resolve_m20_llvm_tool("NAGI_LLVM_RANLIB", "llvm-ranlib", llvm_bin) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m20-granite-inference: {error}")),
    };

    let build_dir = target_evidence.join("target-build");
    let build_log = target_evidence.join("target-build.log");
    let mut build = ProcessCommand::new("bash");
    build
        .args(["tools/llama/build-nagi-target.sh"])
        .current_dir(root)
        .env("NAGI_LLAMA_BUILD", &build_dir)
        .env("NAGI_TARGET_CLANG", &target_clang)
        .env("NAGI_CXX_HEADERS", &cxx_headers)
        .env("NAGI_RELIBC_HEADERS", &relibc_headers)
        .env("NAGI_LLVM_AR", &llvm_ar)
        .env("NAGI_LLVM_RANLIB", &llvm_ranlib);
    let build_output = match build.output() {
        Ok(output) => output,
        Err(error) => {
            let detail = format!("cannot start target archive build: {error}");
            let _ = fs::write(&build_log, &detail);
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m20-granite-inference: {detail} (evidence {})",
                    target_evidence.display()
                ),
            );
        }
    };
    let build_detail = command_output(&build_output);
    if let Err(error) = fs::write(&build_log, &build_detail) {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "m20-granite-inference: cannot write {}: {error}",
                build_log.display()
            ),
        );
    }
    if !build_output.status.success() {
        let _ = fs::write(
            target_evidence.join("README.md"),
            format!(
                "# M20 Granite inference target archives\n\nStatus: BLOCKED\nPinned llama.cpp source: {}\nTarget archive build exited with {}.\nBuild log: target-build.log\n",
                llama_source.display(),
                build_output.status
            ),
        );
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "m20-granite-inference: target archive build failed ({}); log {}; evidence {}",
                build_output.status,
                build_log.display(),
                target_evidence.display()
            ),
        );
    }
    let pre_run = target_evidence.join("pre-run-target-artifacts");
    if let Err(error) = fs::create_dir(&pre_run) {
        return failure(
            EXIT_CONFIG_ERROR,
            format!("m20-granite-inference: create target artifact archive: {error}"),
        );
    }
    for (relative, name) in [
        (
            "target/x86_64-unknown-nagi-user/release/nagi-init",
            "nagi-init",
        ),
        (
            "target/x86_64-unknown-nagi/release/nagi-kernel",
            "nagi-kernel",
        ),
        (
            "loader/target/x86_64-unknown-uefi/release/nagi-loader.efi",
            "nagi-loader.efi",
        ),
    ] {
        let source = root.join(relative);
        match fs::symlink_metadata(&source) {
            Ok(metadata) if metadata.file_type().is_file() => {
                if let Err(error) = fs::copy(&source, pre_run.join(name)) {
                    return failure(
                        EXIT_CONFIG_ERROR,
                        format!(
                            "m20-granite-inference: preserve {}: {error}",
                            source.display()
                        ),
                    );
                }
            }
            Ok(_) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!(
                        "m20-granite-inference: refusing non-regular generated artifact {}",
                        source.display()
                    ),
                )
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!(
                        "m20-granite-inference: inspect {}: {error}",
                        source.display()
                    ),
                )
            }
        }
    }
    if let Err(error) = fs::write(
        target_evidence.join("README.md"),
        format!(
            "# M20 Granite inference target archives\n\nStatus: PASS\nPinned llama.cpp source: {}\nTarget archive directory: target-build\nTarget compiler: {}\nNagi C++ headers: {}\nNagi relibc headers: {}\nBuild log: target-build.log\nPrior generated target artifacts: pre-run-target-artifacts/\n",
            llama_source.display(),
            target_clang.display(),
            cxx_headers.display(),
            relibc_headers.display()
        ),
    ) {
        return failure(
            EXIT_CONFIG_ERROR,
            format!("m20-granite-inference: cannot write target build README: {error}"),
        );
    }

    let cargo_env = [
        ("NAGI_LLAMA_BUILD", build_dir.as_path()),
        ("NAGI_LLAMA_SOURCE", llama_source.as_path()),
        ("NAGI_TARGET_CLANG", target_clang.as_path()),
        ("NAGI_CXX_HEADERS", cxx_headers.as_path()),
        ("NAGI_RELIBC_HEADERS", relibc_headers.as_path()),
        ("NAGI_LLVM_AR", llvm_ar.as_path()),
        ("NAGI_LLVM_RANLIB", llvm_ranlib.as_path()),
    ];
    let evidence_inputs = [ModelStoreEvidenceInput {
        environment: None,
        source: &build_log,
        evidence_name: "llama-target-build.log",
    }];
    let required_markers = ["Nagi M20 Granite structured inference PASS"];
    let early_exit_markers = [
        "Nagi M20 Model Store capability FAIL",
        "Nagi M20 Granite structured inference FAIL",
    ];
    let options = ModelStoreAcceptanceOptions {
        acceptance_marker: "Nagi M20 Granite structured inference PASS",
        required_markers: &required_markers,
        early_exit_markers: &early_exit_markers,
        cargo_env: &cargo_env,
        evidence_inputs: &evidence_inputs,
        timeout: Duration::from_secs(21_600),
        claims: "The guest ModelRuntime verifies the pinned Granite artifact before loading it through a seekable read-only Model Store descriptor into the target llama.cpp CPU backend. It generates a schema-constrained JSON response inside Nagi, then ModelRuntime validates that response. No host inference is used.",
        success_summary: "guest Model Store load and structured Granite inference acceptance",
    };
    execute_model_store_artifact_with_options(
        args,
        root,
        probe,
        ModelStoreArtifactProfile {
            command_name: "m20-granite-inference",
            model_id: manifest.model_id.as_str().to_owned(),
            artifact_id: artifact_id.as_str().to_owned(),
            source_uri: source.uri.clone(),
            source_revision: source.revision.clone(),
            file_name: source.file_name.clone(),
            format: manifest.artifact.format.as_str().to_owned(),
            size_bytes,
            sha256: integrity.digest.clone(),
            init_feature: "m20-llama-inference-acceptance",
            kernel_features: &["m20-llama-memory"],
            image_prefix: "nagi-0.1-m20-granite-inference",
            evidence_prefix: "m20-granite-inference",
            vars_name: "granite-inference-OVMF_VARS.fd",
            serial_name: "granite-inference-qemu.log",
            digest_marker: "Nagi M20 Model Store capability PASS",
        },
        &options,
    )
}

fn execute_m20_llama_smoke(root: &Path, probe: &dyn HostProbe) -> CommandResult {
    let run_id = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_nanos().to_string(),
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m20-llama-smoke: system clock: {error}"),
            )
        }
    };
    let evidence = root
        .join("out/evidence")
        .join(format!("m20-llama-link-smoke-{run_id}"));
    if let Err(error) = fs::create_dir_all(evidence.parent().expect("evidence parent")) {
        return failure(
            EXIT_CONFIG_ERROR,
            format!("m20-llama-smoke: create evidence parent: {error}"),
        );
    }
    if let Err(error) = fs::create_dir(&evidence) {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "m20-llama-smoke: create evidence directory {}: {error}",
                evidence.display()
            ),
        );
    }

    let llama_source = match ensure_llama_cpp_checkout(root) {
        Ok(path) => path,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m20-llama-smoke: pinned llama.cpp source: {error}"),
            );
        }
    };
    let target_clang = match resolve_m20_target_clang() {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m20-llama-smoke: {error}")),
    };
    let cxx_headers = match resolve_m20_cxx_headers(&target_clang) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m20-llama-smoke: {error}")),
    };
    let relibc_headers = std::env::var_os("NAGI_RELIBC_HEADERS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            root.join("out/m17-mesa/relibc-target/x86_64-unknown-nagi-user/include")
        });
    if !relibc_headers.join("pthread.h").is_file() {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "m20-llama-smoke: generated Nagi relibc headers are missing at {}; complete the target header generation first",
                relibc_headers.display()
            ),
        );
    }
    let llvm_bin = target_clang.parent().unwrap_or_else(|| Path::new("."));
    let llvm_ar = match resolve_m20_llvm_tool("NAGI_LLVM_AR", "llvm-ar", llvm_bin) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m20-llama-smoke: {error}")),
    };
    let llvm_ranlib = match resolve_m20_llvm_tool("NAGI_LLVM_RANLIB", "llvm-ranlib", llvm_bin) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m20-llama-smoke: {error}")),
    };

    let build_dir = evidence.join("target-build");
    let build_log = evidence.join("target-build.log");
    let pre_run = evidence.join("pre-run-target-artifacts");
    if let Err(error) = fs::create_dir(&pre_run) {
        return failure(
            EXIT_CONFIG_ERROR,
            format!("m20-llama-smoke: create pre-run archive: {error}"),
        );
    }
    let mut pre_run_files = Vec::new();
    for (relative, name) in [
        (
            "target/x86_64-unknown-nagi-user/release/nagi-init",
            "nagi-init",
        ),
        (
            "target/x86_64-unknown-nagi/release/nagi-kernel",
            "nagi-kernel",
        ),
        (
            "loader/target/x86_64-unknown-uefi/release/nagi-loader.efi",
            "nagi-loader.efi",
        ),
    ] {
        let source = root.join(relative);
        match fs::symlink_metadata(&source) {
            Ok(metadata) if metadata.file_type().is_file() => {
                let destination = pre_run.join(name);
                if let Err(error) = fs::copy(&source, &destination) {
                    return failure(
                        EXIT_CONFIG_ERROR,
                        format!("m20-llama-smoke: preserve {}: {error}", source.display()),
                    );
                }
                pre_run_files.push((destination, format!("pre-run-target-artifacts/{name}")));
            }
            Ok(_) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!(
                        "m20-llama-smoke: refusing non-regular generated artifact {}",
                        source.display()
                    ),
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!("m20-llama-smoke: inspect {}: {error}", source.display()),
                );
            }
        }
    }

    let mut build = ProcessCommand::new("bash");
    build
        .args(["tools/llama/build-nagi-target.sh"])
        .current_dir(root)
        .env("NAGI_LLAMA_BUILD", &build_dir)
        .env("NAGI_TARGET_CLANG", &target_clang)
        .env("NAGI_CXX_HEADERS", &cxx_headers)
        .env("NAGI_RELIBC_HEADERS", &relibc_headers)
        .env("NAGI_LLVM_AR", &llvm_ar)
        .env("NAGI_LLVM_RANLIB", &llvm_ranlib);
    let build_output = match build.output() {
        Ok(output) => output,
        Err(error) => {
            let detail = format!("cannot start target archive build: {error}");
            let _ = fs::write(&build_log, &detail);
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m20-llama-smoke: {detail} (evidence {})",
                    evidence.display()
                ),
            );
        }
    };
    let build_detail = command_output(&build_output);
    if let Err(error) = fs::write(&build_log, &build_detail) {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "m20-llama-smoke: cannot write {}: {error}",
                build_log.display()
            ),
        );
    }
    if !build_output.status.success() {
        let _ = write_m20_llama_smoke_readme(
            &evidence,
            &format!(
                "Status: BLOCKED\nTarget archive build failed with {}.\nPinned source: {}\nBuild log: {}\n",
                build_output.status,
                llama_source.display(),
                build_log.display()
            ),
        );
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "m20-llama-smoke: target archive build failed ({}); log {}; evidence {}",
                build_output.status,
                build_log.display(),
                evidence.display()
            ),
        );
    }

    let image_name = format!("nagi-0.1-m20-llama-smoke-{run_id}.img");
    let init_args = [
        "build",
        "-p",
        "nagi-init",
        "--features",
        "m20-llama-link-smoke",
        "--target",
        "targets/x86_64-unknown-nagi-user.json",
        "-Zbuild-std=core,alloc,compiler_builtins",
        "--release",
        "--locked",
    ];
    let cargo_env = [
        ("NAGI_LLAMA_BUILD", build_dir.as_path()),
        ("NAGI_LLAMA_SOURCE", llama_source.as_path()),
        ("NAGI_TARGET_CLANG", target_clang.as_path()),
        ("NAGI_CXX_HEADERS", cxx_headers.as_path()),
        ("NAGI_RELIBC_HEADERS", relibc_headers.as_path()),
        ("NAGI_LLVM_AR", llvm_ar.as_path()),
        ("NAGI_LLVM_RANLIB", llvm_ranlib.as_path()),
    ];
    let image_result = execute_image_with_init_build_env_using_writer(
        root,
        &init_args,
        None,
        &image_name,
        &cargo_env,
        write_m17_fat12_image,
        ImageBuildFeatures::default(),
    );
    let image_result_log = evidence.join("image-build.log");
    if let Err(error) = fs::write(&image_result_log, image_result.lines.join("\n")) {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "m20-llama-smoke: cannot write {}: {error}",
                image_result_log.display()
            ),
        );
    }
    let mut target_elf_files = Vec::new();
    let target_artifacts_built = image_result.exit_code == EXIT_SUCCESS
        || image_result
            .lines
            .iter()
            .any(|line| line.starts_with("FAIL image:"));
    if target_artifacts_built {
        let target_elf_dir = evidence.join("target-elf");
        if let Err(error) = fs::create_dir(&target_elf_dir) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m20-llama-smoke: create target ELF evidence directory: {error}"),
            );
        }
        for (relative, name) in [
            (
                "target/x86_64-unknown-nagi-user/release/nagi-init",
                "nagi-init",
            ),
            (
                "target/x86_64-unknown-nagi/release/nagi-kernel",
                "nagi-kernel",
            ),
            (
                "loader/target/x86_64-unknown-uefi/release/nagi-loader.efi",
                "nagi-loader.efi",
            ),
        ] {
            let source = root.join(relative);
            match fs::symlink_metadata(&source) {
                Ok(metadata) if metadata.file_type().is_file() => {
                    let destination = target_elf_dir.join(name);
                    if let Err(error) = fs::copy(&source, &destination) {
                        return failure(
                            EXIT_CONFIG_ERROR,
                            format!(
                                "m20-llama-smoke: preserve built target artifact {}: {error}",
                                source.display()
                            ),
                        );
                    }
                    target_elf_files.push((destination, format!("target-elf/{name}")));
                }
                Ok(_) => {
                    return failure(
                        EXIT_CONFIG_ERROR,
                        format!(
                            "m20-llama-smoke: refusing non-regular built target artifact {}",
                            source.display()
                        ),
                    );
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    return failure(
                        EXIT_CONFIG_ERROR,
                        format!("m20-llama-smoke: inspect built target artifact: {error}"),
                    );
                }
            }
        }
    }
    if image_result.exit_code != EXIT_SUCCESS {
        let _ = write_m20_llama_smoke_readme(
            &evidence,
            &format!(
                "Status: BLOCKED\nTarget llama.cpp archives built, but Nagi target image/link failed with exit code {}.\nPinned source: {}\nBuild log: {}\nImage build log: {}\n",
                image_result.exit_code,
                llama_source.display(),
                build_log.display(),
                image_result_log.display()
            ),
        );
        let mut result = image_result;
        result
            .lines
            .push(format!("M20 target-build evidence: {}", evidence.display()));
        return result;
    }

    let host = match resolve_qemu_host(root, probe, "m20-llama-smoke") {
        Ok(host) => host,
        Err(error) => {
            let _ = write_m20_llama_smoke_readme(
                &evidence,
                &format!(
                    "Status: PARTIAL\nNagi target link completed; QEMU unavailable: {error}\n"
                ),
            );
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "{error}; target ELF and archives are preserved at {}",
                    evidence.display()
                ),
            );
        }
    };
    let artifacts = match ensure_owned_directory(root, Path::new("out/artifacts")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m20-llama-smoke: {error}")),
    };
    let image_path = artifacts.join(&image_name);
    let image_evidence = evidence.join(&image_name);
    if let Err(error) = fs::copy(&image_path, &image_evidence) {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "m20-llama-smoke: preserve {}: {error}",
                image_path.display()
            ),
        );
    }
    let persistent_disk = evidence.join(format!("user-data-{run_id}.img"));
    if let Err(error) = ensure_persistent_disk(&persistent_disk) {
        return failure(
            EXIT_CONFIG_ERROR,
            format!("m20-llama-smoke: initialize User Data image: {error}"),
        );
    }
    let vars_copy = evidence.join("OVMF_VARS.fd");
    if let Err(error) = initialize_ovmf_vars(&host.ovmf_vars, &vars_copy) {
        return failure(
            EXIT_CONFIG_ERROR,
            format!("m20-llama-smoke: initialize OVMF variables: {error}"),
        );
    }
    let serial_log = evidence.join("qemu-serial.log");
    let config = QemuConfig {
        qemu: &host.qemu,
        ovmf_code: &host.ovmf_code,
        ovmf_vars_template: &host.ovmf_vars,
        disk_image: &image_path,
        persistent_disk: &persistent_disk,
        vars_copy: &vars_copy,
        serial_log: &serial_log,
        acceptance_marker: "Nagi M20 llama backend init PASS",
        timeout: Duration::from_secs(180),
    };
    let qemu_status = match run_qemu_until_any_acceptance_marker_with_read_only_boot_disk(
        &config,
        &[
            "Nagi M20 llama backend init PASS",
            "Nagi M20 llama backend init FAIL",
            "Nagi M7 VirtIO Block FAIL",
        ],
    ) {
        Ok(status) => status,
        Err(error) => {
            let _ = write_m20_llama_smoke_readme(
                &evidence,
                &format!(
                    "Status: BLOCKED\nTarget link completed, but QEMU did not reach the backend init marker: {error}\nQEMU log: {}\n",
                    serial_log.display()
                ),
            );
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m20-llama-smoke: QEMU did not reach backend init marker: {error} (log {})",
                    serial_log.display()
                ),
            );
        }
    };
    let serial = match fs::read_to_string(&serial_log) {
        Ok(serial) => serial,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m20-llama-smoke: cannot read {}: {error}",
                    serial_log.display()
                ),
            );
        }
    };
    if !serial.contains("Nagi M20 llama backend init PASS") {
        let _ = write_m20_llama_smoke_readme(
            &evidence,
            &format!(
                "Status: FAIL\nQEMU exit: {qemu_status}\nGuest did not register the CPU backend.\nSerial log: {}\n",
                serial_log.display()
            ),
        );
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "m20-llama-smoke: guest backend init failed (QEMU exit {qemu_status}); log {}; evidence {}",
                serial_log.display(),
                evidence.display()
            ),
        );
    }
    for marker in ["Nagi Kernel started", "Nagi M20 llama backend init PASS"] {
        if !serial.contains(marker) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m20-llama-smoke: guest log lacks `{marker}` (log {})",
                    serial_log.display()
                ),
            );
        }
    }

    let readme = evidence.join("README.md");
    if let Err(error) = write_m20_llama_smoke_readme(
        &evidence,
        &format!(
            "# M20 llama.cpp target link/init smoke\n\nStatus: PASS\nRun: {run_id}\nPinned patched source: {}\nTarget build: {}\nQEMU exit: {qemu_status}\nSerial acceptance: `llama_backend_init()` returned and the actual ggml CPU backend registry contained `CPU`.\n\nThis checks target archive build, static target linking, process startup, and backend initialization only. It does not load a model, validate Model Store integration, or run inference.\n",
            llama_source.display(),
            build_dir.display()
        ),
    ) {
        return failure(EXIT_CONFIG_ERROR, format!("m20-llama-smoke: write README: {error}"));
    }
    let mut manifest_files = vec![
        (readme.as_path(), "README.md".to_owned()),
        (build_log.as_path(), "target-build.log".to_owned()),
        (image_evidence.as_path(), image_name.clone()),
        (persistent_disk.as_path(), format!("user-data-{run_id}.img")),
        (vars_copy.as_path(), "OVMF_VARS.fd".to_owned()),
        (serial_log.as_path(), "qemu-serial.log".to_owned()),
    ];
    manifest_files.extend(
        pre_run_files
            .iter()
            .map(|(path, relative)| (path.as_path(), relative.clone())),
    );
    manifest_files.extend(
        target_elf_files
            .iter()
            .map(|(path, relative)| (path.as_path(), relative.clone())),
    );
    let llama_archive = build_dir.join("src/libllama.a");
    let ggml_archive = build_dir.join("ggml/src/libggml.a");
    let ggml_cpu_archive = build_dir.join("ggml/src/libggml-cpu.a");
    let ggml_base_archive = build_dir.join("ggml/src/libggml-base.a");
    manifest_files.extend([
        (
            llama_archive.as_path(),
            "target-build/src/libllama.a".to_owned(),
        ),
        (
            ggml_archive.as_path(),
            "target-build/ggml/src/libggml.a".to_owned(),
        ),
        (
            ggml_cpu_archive.as_path(),
            "target-build/ggml/src/libggml-cpu.a".to_owned(),
        ),
        (
            ggml_base_archive.as_path(),
            "target-build/ggml/src/libggml-base.a".to_owned(),
        ),
    ]);
    if let Err(error) = write_sha256_manifest(&evidence.join("SHA256SUMS"), &manifest_files) {
        return failure(
            EXIT_CONFIG_ERROR,
            format!("m20-llama-smoke: write SHA256SUMS: {error}"),
        );
    }
    CommandResult {
        exit_code: EXIT_SUCCESS,
        lines: vec![format!(
            "PASS M20 llama.cpp target link/init smoke: actual CPU backend registered in Nagi/QEMU (exit {qemu_status}; evidence {})",
            evidence.display()
        )],
    }
}

fn write_m20_llama_smoke_readme(evidence: &Path, content: &str) -> Result<(), String> {
    fs::write(evidence.join("README.md"), content).map_err(|error| {
        format!(
            "cannot write M20 link-smoke evidence in {}: {error}",
            evidence.display()
        )
    })
}

fn resolve_m20_target_clang() -> Result<PathBuf, String> {
    let configured = std::env::var_os("NAGI_TARGET_CLANG").map(PathBuf::from);
    if let Some(path) = configured.as_ref() {
        // Resolve a bare command name to LLVM 19 first when it is installed.
        // Nagi's current no-exceptions llama archive was built with this
        // compiler and its matching libc++ headers; an unqualified `clang`
        // can otherwise select the macOS SDK's unrelated libc++ headers.
        if path.components().count() == 1 {
            let executable = path.file_name().unwrap_or_default();
            for prefix in ["/opt/homebrew/opt/llvm@19", "/usr/local/opt/llvm@19"] {
                let candidate = PathBuf::from(prefix).join("bin").join(executable);
                if candidate.is_file() {
                    return Ok(candidate);
                }
            }
        }
        return Ok(path.clone());
    }
    for candidate in [
        PathBuf::from("/opt/homebrew/opt/llvm@19/bin/clang"),
        PathBuf::from("/usr/local/opt/llvm@19/bin/clang"),
    ] {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Ok(PathBuf::from("clang"))
}

fn resolve_m20_cxx_headers(compiler: &Path) -> Result<PathBuf, String> {
    if let Some(configured) = std::env::var_os("NAGI_CXX_HEADERS") {
        let path = PathBuf::from(configured);
        if path.join("cstddef").is_file() {
            return Ok(path);
        }
        return Err(format!(
            "NAGI_CXX_HEADERS lacks cstddef: {}",
            path.display()
        ));
    }
    if let Some(candidate) = compiler
        .parent()
        .and_then(Path::parent)
        .map(|prefix| prefix.join("include/c++/v1"))
    {
        if candidate.join("cstddef").is_file() {
            return Ok(candidate);
        }
    }
    let output = ProcessCommand::new(compiler)
        .args(["-E", "-x", "c++", "-", "-v"])
        .stdin(Stdio::null())
        .output()
        .map_err(|error| {
            format!(
                "cannot inspect libc++ headers using {}: {error}",
                compiler.display()
            )
        })?;
    for line in String::from_utf8_lossy(&output.stderr).lines() {
        let candidate = Path::new(line.trim());
        if candidate.ends_with("c++/v1") && candidate.join("cstddef").is_file() {
            return Ok(candidate.to_path_buf());
        }
    }
    Err(format!(
        "cannot find libc++ headers for {}; set NAGI_CXX_HEADERS",
        compiler.display()
    ))
}

fn resolve_m20_llvm_tool(
    override_name: &str,
    tool_name: &str,
    compiler_dir: &Path,
) -> Result<PathBuf, String> {
    if let Some(configured) = std::env::var_os(override_name) {
        return Ok(PathBuf::from(configured));
    }
    let sibling = compiler_dir.join(tool_name);
    if sibling.is_file() {
        return Ok(sibling);
    }
    for directory in std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()) {
        let candidate = directory.join(tool_name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(format!(
        "{tool_name} was not found beside the configured clang or on PATH"
    ))
}

fn execute_m25_whisper(args: &[String], root: &Path, probe: &dyn HostProbe) -> CommandResult {
    let model = match crate::whisper_cpp::validate_whisper_model_artifact_lock(root) {
        Ok(model) => model,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m25-whisper: {error}")),
    };
    execute_model_store_artifact(
        args,
        root,
        probe,
        ModelStoreArtifactProfile {
            command_name: "m25-whisper",
            model_id: model.model_id,
            artifact_id: model.artifact_id,
            source_uri: model.repository,
            source_revision: model.revision,
            file_name: model.file_name,
            format: model.format,
            size_bytes: model.size_bytes,
            sha256: model.sha256,
            init_feature: "m25-whisper-artifact-acceptance",
            kernel_features: &[],
            image_prefix: "nagi-0.1-m25-whisper",
            evidence_prefix: "m25-whisper-artifact",
            vars_name: "whisper-OVMF_VARS.fd",
            serial_name: "m25-whisper-qemu.log",
            digest_marker: "Nagi M25 Whisper artifact digest PASS",
        },
    )
}

fn execute_m25_whisper_inference(
    args: &[String],
    root: &Path,
    probe: &dyn HostProbe,
) -> CommandResult {
    let model = match crate::whisper_cpp::validate_whisper_model_artifact_lock(root) {
        Ok(model) => model,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m25-whisper-inference: {error}")),
    };
    let requested_pcm_path = PathBuf::from(&args[1]);
    let pcm_path = if requested_pcm_path.is_absolute() {
        requested_pcm_path
    } else {
        root.join(requested_pcm_path)
    };
    let pcm_metadata = match fs::symlink_metadata(&pcm_path) {
        Ok(metadata) if metadata.file_type().is_file() => metadata,
        Ok(_) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m25-whisper-inference: PCM input must be a regular file: {}",
                    pcm_path.display()
                ),
            )
        }
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m25-whisper-inference: cannot inspect {}: {error}",
                    pcm_path.display()
                ),
            )
        }
    };
    if pcm_metadata.len() == 0 || pcm_metadata.len() > 1_048_576 || pcm_metadata.len() % 2 != 0 {
        return failure(
            EXIT_CONFIG_ERROR,
            "m25-whisper-inference: input must be raw mono S16LE at 16 kHz, nonempty, even-sized, and at most 1 MiB",
        );
    }
    let expected_text = &args[2];
    if expected_text.is_empty()
        || expected_text.len() > 1024
        || expected_text
            .chars()
            .any(|character| matches!(character, '\n' | '\r' | '\0'))
    {
        return failure(
            EXIT_USAGE,
            "m25-whisper-inference: expected text must be 1–1024 UTF-8 bytes without line breaks",
        );
    }

    let whisper_source = match ensure_whisper_cpp_checkout(root) {
        Ok(path) => path,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m25-whisper-inference: pinned whisper.cpp source: {error}"),
            )
        }
    };
    let target_clang = match resolve_m20_target_clang() {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m25-whisper-inference: {error}")),
    };
    let cxx_headers = match resolve_m20_cxx_headers(&target_clang) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m25-whisper-inference: {error}")),
    };
    let relibc_headers = std::env::var_os("NAGI_RELIBC_HEADERS")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            root.join("out/m17-mesa/relibc-target/x86_64-unknown-nagi-user/include")
        });
    if !relibc_headers.join("pthread.h").is_file() {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "m25-whisper-inference: generated Nagi relibc headers are missing at {}",
                relibc_headers.display()
            ),
        );
    }
    let llvm_bin = target_clang.parent().unwrap_or_else(|| Path::new("."));
    let llvm_ar = match resolve_m20_llvm_tool("NAGI_LLVM_AR", "llvm-ar", llvm_bin) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m25-whisper-inference: {error}")),
    };
    let llvm_ranlib = match resolve_m20_llvm_tool("NAGI_LLVM_RANLIB", "llvm-ranlib", llvm_bin) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m25-whisper-inference: {error}")),
    };

    let run_id = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_nanos().to_string(),
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m25-whisper-inference: system clock: {error}"),
            )
        }
    };
    let logs = match ensure_owned_directory(root, Path::new("out/logs")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m25-whisper-inference: {error}")),
    };
    let whisper_build_log = logs.join(format!("m25-whisper-target-build-{run_id}.log"));
    let build_dir = root.join("out/m25-whisper-target");
    let mut build = ProcessCommand::new("bash");
    build
        .arg("tools/whisper/build-nagi-target.sh")
        .current_dir(root)
        .env("NAGI_WHISPER_SOURCE", &whisper_source)
        .env("NAGI_WHISPER_BUILD", &build_dir)
        .env("NAGI_TARGET_CLANG", &target_clang)
        .env("NAGI_CXX_HEADERS", &cxx_headers)
        .env("NAGI_RELIBC_HEADERS", &relibc_headers)
        .env("NAGI_LLVM_AR", &llvm_ar)
        .env("NAGI_LLVM_RANLIB", &llvm_ranlib);
    let build_output = match build.output() {
        Ok(output) => output,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m25-whisper-inference: cannot start target build: {error}"),
            )
        }
    };
    let build_detail = command_output(&build_output);
    if let Err(error) = fs::write(&whisper_build_log, &build_detail) {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "m25-whisper-inference: cannot write target build log {}: {error}",
                whisper_build_log.display()
            ),
        );
    }
    if !build_output.status.success() {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "m25-whisper-inference: Nagi-target whisper.cpp build failed ({}); log {}",
                build_output.status,
                whisper_build_log.display()
            ),
        );
    }

    let expected_text_path = logs.join(format!("m25-whisper-expected-text-{run_id}.txt"));
    let mut expected_output = match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&expected_text_path)
    {
        Ok(file) => file,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m25-whisper-inference: cannot create expected-text fixture {}: {error}",
                    expected_text_path.display()
                ),
            )
        }
    };
    if let Err(error) = expected_output.write_all(expected_text.as_bytes()) {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "m25-whisper-inference: cannot write expected-text fixture {}: {error}",
                expected_text_path.display()
            ),
        );
    }
    drop(expected_output);

    let cargo_env = [
        ("NAGI_WHISPER_BUILD", build_dir.as_path()),
        ("NAGI_WHISPER_SOURCE", whisper_source.as_path()),
        ("NAGI_TARGET_CLANG", target_clang.as_path()),
        ("NAGI_CXX_HEADERS", cxx_headers.as_path()),
        ("NAGI_RELIBC_HEADERS", relibc_headers.as_path()),
        ("NAGI_LLVM_AR", llvm_ar.as_path()),
        ("NAGI_LLVM_RANLIB", llvm_ranlib.as_path()),
    ];
    let evidence_inputs = [
        ModelStoreEvidenceInput {
            environment: Some("NAGI_M25_WHISPER_PCM_FIXTURE"),
            source: &pcm_path,
            evidence_name: "whisper-input-16khz-mono-s16le.pcm",
        },
        ModelStoreEvidenceInput {
            environment: Some("NAGI_M25_WHISPER_EXPECTED_TEXT_FILE"),
            source: &expected_text_path,
            evidence_name: "expected-transcript.txt",
        },
        ModelStoreEvidenceInput {
            environment: None,
            source: &whisper_build_log,
            evidence_name: "whisper-target-build.log",
        },
    ];
    let required_markers = ["Nagi M25 Whisper Japanese fixture inference PASS"];
    let early_exit_markers = [
        "Nagi M20 Model Store capability FAIL",
        "Nagi M25 Whisper artifact digest FAIL",
        "Nagi M25 Whisper Japanese fixture inference FAIL",
    ];
    let options = ModelStoreAcceptanceOptions {
        acceptance_marker: "Nagi M25 Whisper Japanese fixture inference PASS",
        required_markers: &required_markers,
        early_exit_markers: &early_exit_markers,
        cargo_env: &cargo_env,
        evidence_inputs: &evidence_inputs,
        timeout: Duration::from_secs(3600),
        claims: "The guest loads the pinned model through its read-only Model Store capability and runs whisper.cpp against the checksummed raw mono 16 kHz S16LE fixture. It verifies that inference includes the supplied expected Japanese text. This does not use a microphone, grant site or app authority, or execute the transcript.",
        success_summary: "guest Model Store load and Japanese Whisper fixture inference acceptance",
    };
    let result = execute_model_store_artifact_with_options(
        args,
        root,
        probe,
        ModelStoreArtifactProfile {
            command_name: "m25-whisper-inference",
            model_id: model.model_id,
            artifact_id: model.artifact_id,
            source_uri: model.repository,
            source_revision: model.revision,
            file_name: model.file_name,
            format: model.format,
            size_bytes: model.size_bytes,
            sha256: model.sha256,
            init_feature: "m25-whisper-inference-acceptance",
            kernel_features: &["m25-whisper-memory"],
            image_prefix: "nagi-0.1-m25-whisper-inference",
            evidence_prefix: "m25-whisper-inference",
            vars_name: "whisper-inference-OVMF_VARS.fd",
            serial_name: "whisper-inference-qemu.log",
            digest_marker: "Nagi M25 Whisper artifact digest PASS",
        },
        &options,
    );
    let _ = fs::remove_file(expected_text_path);
    result
}

fn execute_m26_model(
    args: &[String],
    root: &Path,
    probe: &dyn HostProbe,
    model: M26Model,
) -> CommandResult {
    let command_name = match model {
        M26Model::Qwen => "m26-qwen",
        M26Model::Gemma => "m26-gemma",
    };
    let pin = match validate_m26_model_lock(root, model) {
        Ok(pin) => pin,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("{command_name}: {error}")),
    };
    if let Err(error) = validate_m26_model_manifest(root, model, pin) {
        return failure(EXIT_CONFIG_ERROR, format!("{command_name}: {error}"));
    }
    let init_feature = match model {
        M26Model::Qwen => "m26-qwen-artifact-acceptance",
        M26Model::Gemma => "m26-gemma-artifact-acceptance",
    };
    let artifact_args = match model {
        M26Model::Qwen => args,
        M26Model::Gemma if args.get(1).map(String::as_str) == Some("--accept-gemma-terms") => {
            &args[..1]
        }
        M26Model::Gemma => {
            return failure(
                EXIT_USAGE,
                "m26-gemma: user must explicitly pass --accept-gemma-terms after reviewing the Gemma Terms of Use",
            )
        }
    };
    execute_model_store_artifact(
        artifact_args,
        root,
        probe,
        ModelStoreArtifactProfile {
            command_name,
            model_id: pin.model_id.to_owned(),
            artifact_id: pin.artifact_id.to_owned(),
            source_uri: pin.repository.to_owned(),
            source_revision: pin.revision.to_owned(),
            file_name: pin.file_name.to_owned(),
            format: pin.format.to_owned(),
            size_bytes: pin.size_bytes,
            sha256: pin.sha256.to_owned(),
            init_feature,
            kernel_features: &[],
            image_prefix: match model {
                M26Model::Qwen => "nagi-0.1-m26-qwen",
                M26Model::Gemma => "nagi-0.1-m26-gemma",
            },
            evidence_prefix: match model {
                M26Model::Qwen => "m26-qwen-artifact",
                M26Model::Gemma => "m26-gemma-artifact",
            },
            vars_name: match model {
                M26Model::Qwen => "qwen-OVMF_VARS.fd",
                M26Model::Gemma => "gemma-OVMF_VARS.fd",
            },
            serial_name: match model {
                M26Model::Qwen => "m26-qwen-qemu.log",
                M26Model::Gemma => "m26-gemma-qemu.log",
            },
            digest_marker: match model {
                M26Model::Qwen => "Nagi M26 Qwen artifact digest PASS",
                M26Model::Gemma => "Nagi M26 Gemma artifact digest PASS",
            },
        },
    )
}

fn execute_model_store_artifact(
    args: &[String],
    root: &Path,
    probe: &dyn HostProbe,
    profile: ModelStoreArtifactProfile,
) -> CommandResult {
    let options = ModelStoreAcceptanceOptions {
        acceptance_marker: "Nagi M20 Model Store capability PASS",
        required_markers: &[],
        early_exit_markers: &[],
        cargo_env: &[],
        evidence_inputs: &[],
        timeout: Duration::from_secs(1800),
        claims: "The guest verifies the pinned artifact bytes through its read-only Model Store capability. No backend is loaded and no inference is performed.",
        success_summary: "guest Model Store artifact digest acceptance",
    };
    execute_model_store_artifact_with_options(args, root, probe, profile, &options)
}

fn execute_model_store_artifact_with_options(
    args: &[String],
    root: &Path,
    probe: &dyn HostProbe,
    profile: ModelStoreArtifactProfile,
    options: &ModelStoreAcceptanceOptions<'_>,
) -> CommandResult {
    let requested_path = PathBuf::from(&args[0]);
    let artifact_path = if requested_path.is_absolute() {
        requested_path
    } else {
        root.join(requested_path)
    };
    if let Err(error) =
        verify_external_artifact(&artifact_path, profile.size_bytes, &profile.sha256)
    {
        return failure(
            EXIT_CONFIG_ERROR,
            format!("{}: {error}", profile.command_name),
        );
    }
    let artifact_id = match nagi_model_manager::ArtifactId::new(profile.artifact_id.clone()) {
        Ok(artifact_id) => artifact_id,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "{}: invalid pinned artifact ID: {error}",
                    profile.command_name
                ),
            )
        }
    };
    let short_name = nagi_model_manager::model_store_short_name(&artifact_id);
    let file_name = match (
        std::str::from_utf8(&short_name[..8]),
        std::str::from_utf8(&short_name[8..]),
    ) {
        (Ok(base), Ok(extension)) => format!("{base}.{extension}"),
        _ => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("{}: invalid FAT32 Model Store name", profile.command_name),
            )
        }
    };
    let host = match resolve_qemu_host(root, probe, profile.command_name) {
        Ok(host) => host,
        Err(error) => return failure(EXIT_CONFIG_ERROR, error),
    };
    let run_id = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_nanos().to_string(),
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("{}: system clock: {error}", profile.command_name),
            )
        }
    };
    let artifacts = match ensure_owned_directory(root, Path::new("out").join("artifacts")) {
        Ok(path) => path,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("{}: {error}", profile.command_name),
            )
        }
    };
    let evidence = match ensure_owned_directory(
        root,
        Path::new("out")
            .join("evidence")
            .join(format!("{}-{}", profile.evidence_prefix, run_id)),
    ) {
        Ok(path) => path,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("{}: {error}", profile.command_name),
            )
        }
    };
    let mut copied_evidence_inputs = Vec::new();
    let mut owned_cargo_env: Vec<(String, PathBuf)> = options
        .cargo_env
        .iter()
        .map(|(name, path)| ((*name).to_owned(), (*path).to_path_buf()))
        .collect();
    for input in options.evidence_inputs {
        let destination = evidence.join(input.evidence_name);
        if let Err(error) = fs::copy(input.source, &destination) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "{}: preserve evidence input {}: {error}",
                    profile.command_name,
                    input.source.display()
                ),
            );
        }
        if let Some(environment) = input.environment {
            owned_cargo_env.push((environment.to_owned(), destination.clone()));
        }
        copied_evidence_inputs.push((destination, input.evidence_name.to_owned()));
    }
    let borrowed_cargo_env: Vec<(&str, &Path)> = owned_cargo_env
        .iter()
        .map(|(name, path)| (name.as_str(), path.as_path()))
        .collect();
    let image_name = format!("{}-{}.qcow2", profile.image_prefix, run_id);
    let image_path = artifacts.join(&image_name);
    let evidence_readme = evidence.join("README.md");
    let initial_readme = format!(
        "# {} Model Store acceptance\n\nModel ID: {}\nArtifact ID: {}\nSource: {} at {}\nLocked filename: {}\nModel Store filename: {}\nExternal artifact: {}\nFormat: {}\nExpected size: {} bytes\nExpected SHA-256: {}\n\nThe host verifies the external artifact against its source lock before streaming it into a separate GPT Model Store. {}\n",
        profile.command_name,
        profile.model_id,
        profile.artifact_id,
        profile.source_uri,
        profile.source_revision,
        profile.file_name,
        file_name,
        artifact_path.display(),
        profile.format,
        profile.size_bytes,
        profile.sha256,
        options.claims
    );
    if let Err(error) = fs::write(&evidence_readme, initial_readme) {
        return failure(
            EXIT_CONFIG_ERROR,
            format!("{}: cannot write README: {error}", profile.command_name),
        );
    }

    let recovery_init_args = [
        "build",
        "-p",
        "nagi-init",
        "--features",
        "m27-recovery",
        "--target",
        "targets/x86_64-unknown-nagi-user.json",
        "-Zbuild-std=core,alloc,compiler_builtins",
        "--release",
        "--locked",
        "--offline",
    ];
    let recovery_build = run_cargo(root, "Model Store Recovery init", &recovery_init_args);
    if recovery_build.exit_code != EXIT_SUCCESS {
        return recovery_build;
    }
    let recovery_init_path = root.join("target/x86_64-unknown-nagi-user/release/nagi-init");
    let recovery_init = match fs::read(&recovery_init_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "{}: cannot read {}: {error}",
                    profile.command_name,
                    recovery_init_path.display()
                ),
            );
        }
    };
    let init_args = [
        "build",
        "-p",
        "nagi-init",
        "--features",
        profile.init_feature,
        "--target",
        "targets/x86_64-unknown-nagi-user.json",
        "-Zbuild-std=core,alloc,compiler_builtins",
        "--release",
        "--locked",
        "--offline",
    ];
    let image_result = execute_image_with_init_build_env_using_writer_and_recovery(
        root,
        &init_args,
        None,
        ImageBuildRequest {
            image_name: &image_name,
            cargo_env: &borrowed_cargo_env,
            recovery_init: Some(&recovery_init),
            image_writer: write_reference_disk_qcow2,
            external_model_store_file: Some((&file_name, &artifact_path)),
            build_features: ImageBuildFeatures {
                kernel: profile.kernel_features,
                loader: &["m27-ab-slot-boot-control"],
            },
        },
    );
    if image_result.exit_code != EXIT_SUCCESS {
        return image_result;
    }

    let vars_copy = evidence.join(profile.vars_name);
    if let Err(error) = initialize_ovmf_vars(&host.ovmf_vars, &vars_copy) {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "{}: initialize OVMF variables: {error}",
                profile.command_name
            ),
        );
    }
    let serial_log = evidence.join(profile.serial_name);
    let first_boot_log_name = format!("{}-first-boot.log", profile.evidence_prefix);
    let first_boot_log = evidence.join(&first_boot_log_name);
    let config = QemuConfig {
        qemu: &host.qemu,
        ovmf_code: &host.ovmf_code,
        ovmf_vars_template: &host.ovmf_vars,
        disk_image: &image_path,
        persistent_disk: &image_path,
        vars_copy: &vars_copy,
        serial_log: &serial_log,
        acceptance_marker: options.acceptance_marker,
        timeout: options.timeout,
    };
    let mut qemu_stop_markers = vec![options.acceptance_marker];
    qemu_stop_markers.extend_from_slice(options.early_exit_markers);
    qemu_stop_markers.push("Nagi M5 process exit FAIL");
    let mut first_boot_stop_markers = qemu_stop_markers.clone();
    first_boot_stop_markers.push("Nagi M7 reboot required PASS");
    let first_boot_config = QemuConfig {
        serial_log: &first_boot_log,
        ..config
    };
    let first_boot_status =
        match run_qemu_until_any_acceptance_marker(&first_boot_config, &first_boot_stop_markers) {
            Ok(status) => status,
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!(
                        "{}: QEMU acceptance failed; log {}: {error}",
                        profile.command_name,
                        first_boot_log.display()
                    ),
                )
            }
        };
    let first_boot_serial = match fs::read_to_string(&first_boot_log) {
        Ok(serial) => serial,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "{}: cannot read {}: {error}",
                    profile.command_name,
                    first_boot_log.display()
                ),
            )
        }
    };
    let mut first_boot_log_for_manifest = None;
    let (qemu_status, serial) = if first_boot_serial.contains("Nagi M7 reboot required PASS") {
        for marker in [
            "Nagi M30 GPT partition boot: System A PASS",
            "Nagi M20 Model Store capability PASS",
            profile.digest_marker,
            "Nagi M7 ext2 format PASS",
            "Nagi M7 persistent write PASS",
        ] {
            if !first_boot_serial.contains(marker) {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!(
                        "{}: User Data bootstrap did not print `{marker}` (log {})",
                        profile.command_name,
                        first_boot_log.display()
                    ),
                );
            }
        }
        let restart_config = QemuConfig {
            serial_log: &serial_log,
            ..config
        };
        let status = match run_qemu_until_any_acceptance_marker_reusing_ovmf_vars(
                &restart_config,
                &qemu_stop_markers,
            ) {
                Ok(status) => status,
                Err(error) => {
                    return failure(
                        EXIT_CONFIG_ERROR,
                        format!(
                            "{}: QEMU restart acceptance failed; first boot log {}, restart log {}: {error}",
                            profile.command_name,
                            first_boot_log.display(),
                            serial_log.display()
                        ),
                    )
                }
            };
        let serial = match fs::read_to_string(&serial_log) {
            Ok(serial) => serial,
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!(
                        "{}: cannot read {}: {error}",
                        profile.command_name,
                        serial_log.display()
                    ),
                )
            }
        };
        first_boot_log_for_manifest = Some(first_boot_log_name.clone());
        (status, serial)
    } else {
        if let Err(error) = fs::copy(&first_boot_log, &serial_log) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "{}: preserve QEMU log {}: {error}",
                    profile.command_name,
                    serial_log.display()
                ),
            );
        }
        if let Err(error) = fs::remove_file(&first_boot_log) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "{}: remove temporary first-boot log {}: {error}",
                    profile.command_name,
                    first_boot_log.display()
                ),
            );
        }
        (first_boot_status, first_boot_serial)
    };
    let mut required_markers = vec![
        "Nagi M30 GPT partition boot: System A PASS",
        "Nagi M20 Model Store capability PASS",
        profile.digest_marker,
    ];
    required_markers.extend_from_slice(options.required_markers);
    for marker in required_markers {
        if !serial.contains(marker) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "{}: guest did not print {} (QEMU exit {qemu_status}; log {})",
                    profile.command_name,
                    marker,
                    serial_log.display()
                ),
            );
        }
    }
    let image_check = ProcessCommand::new("qemu-img")
        .args(["check", "-f", "qcow2"])
        .arg(&image_path)
        .output();
    match image_check {
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "{}: qemu-img check failed: {}",
                    profile.command_name,
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            )
        }
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("{}: qemu-img check: {error}", profile.command_name),
            )
        }
    }
    let bootstrap_note = first_boot_log_for_manifest.as_ref().map_or_else(
        || "The host acceptance runner did not need a separate User Data bootstrap restart.".to_owned(),
        |first_boot_log_name| {
            format!(
                "The host acceptance runner relaunched QEMU after User Data bootstrap, reusing the same qcow2 image and OVMF variables; first-boot log: {first_boot_log_name}."
            )
        },
    );
    let final_readme = format!(
        "# {} Model Store acceptance\n\nStatus: PASS\nModel ID: {}\nArtifact ID: {}\nSource: {} at {}\nLocked filename: {}\nModel Store filename: {}\nExternal artifact: {}\nFormat: {}\nSize: {} bytes\nSHA-256: {}\n\nThe guest booted System A, read the complete artifact through the separate read-only Model Store FAT32 reader, and verified its actual bytes against the locked SHA-256. qemu-img check passed for {}. {} {} QEMU log: {}.\n",
        profile.command_name,
        profile.model_id,
        profile.artifact_id,
        profile.source_uri,
        profile.source_revision,
        profile.file_name,
        file_name,
        artifact_path.display(),
        profile.format,
        profile.size_bytes,
        profile.sha256,
        image_path.display(),
        bootstrap_note,
        options.claims,
        profile.serial_name
    );
    if let Err(error) = fs::write(&evidence_readme, final_readme) {
        return failure(
            EXIT_CONFIG_ERROR,
            format!("{}: cannot update README: {error}", profile.command_name),
        );
    }
    let mut manifest_paths = vec![
        (evidence_readme.as_path(), "README.md".to_owned()),
        (
            image_path.as_path(),
            format!("../../artifacts/{image_name}"),
        ),
        (vars_copy.as_path(), profile.vars_name.to_owned()),
        (serial_log.as_path(), profile.serial_name.to_owned()),
    ];
    manifest_paths.extend(
        copied_evidence_inputs
            .iter()
            .map(|(path, relative)| (path.as_path(), relative.clone())),
    );
    if let Some(first_boot_log_name) = first_boot_log_for_manifest.as_ref() {
        manifest_paths.push((first_boot_log.as_path(), first_boot_log_name.clone()));
    }
    if let Err(error) = write_sha256_manifest(&evidence.join("SHA256SUMS"), &manifest_paths) {
        return failure(
            EXIT_CONFIG_ERROR,
            format!("{}: write evidence manifest: {error}", profile.command_name),
        );
    }
    CommandResult {
        exit_code: EXIT_SUCCESS,
        lines: vec![format!(
            "PASS {} {} (image {}; artifact {}; log {}; evidence {})",
            profile.command_name,
            options.success_summary,
            image_path.display(),
            artifact_path.display(),
            serial_log.display(),
            evidence.display()
        )],
    }
}

fn pinned_granite_manifest(root: &Path) -> Result<nagi_model_manager::ModelManifest, String> {
    let manifest_path = root.join("user/nagi-model-manager/tests/fixtures/granite-4.2-3b.json");
    let manifest_bytes = fs::read(&manifest_path)
        .map_err(|error| format!("cannot read {}: {error}", manifest_path.display()))?;
    let manifest = nagi_model_manager::ModelManifest::parse_json(&manifest_bytes)
        .map_err(|error| format!("Granite manifest is invalid: {error}"))?;
    if manifest.model_id.as_str() != "ibm.granite-4.2-3b" {
        return Err("Granite manifest has an unexpected model ID".to_owned());
    }
    let (artifact_id, source, integrity, size) = match (
        &manifest.artifact.reference,
        manifest.source.as_ref(),
        manifest.artifact.integrity.as_ref(),
        manifest.artifact.size_bytes,
    ) {
        (
            nagi_model_manager::ArtifactReference::ModelStore { artifact_id },
            Some(source),
            Some(integrity),
            Some(size),
        ) => (artifact_id.as_str(), source, integrity, size),
        _ => return Err("Granite manifest is missing pinned Model Store metadata".to_owned()),
    };
    if integrity.algorithm != "sha256" || manifest.artifact.format.as_str() != "gguf" {
        return Err("Granite manifest has an unsupported artifact contract".to_owned());
    }
    let lock = fs::read_to_string(root.join("third_party/models.lock"))
        .map_err(|error| format!("cannot read third_party/models.lock: {error}"))?;
    let locked = |key: &str| {
        crate::llama_cpp::lock_value(&lock, "models.granite_4_2_3b", key)
            .ok_or_else(|| format!("third_party/models.lock is missing Granite field `{key}`"))
    };
    let notice = manifest
        .license
        .notices
        .iter()
        .find(|notice| notice.notice_id == "apache-2.0")
        .ok_or_else(|| "Granite manifest is missing the Apache-2.0 notice".to_owned())?;
    let fields = [
        ("model_id", manifest.model_id.as_str().to_owned()),
        ("repository", source.uri.clone()),
        ("revision", source.revision.clone()),
        ("file_name", source.file_name.clone()),
        ("format", manifest.artifact.format.as_str().to_owned()),
        ("size_bytes", size.to_string()),
        ("sha256", integrity.digest.clone()),
        ("license", manifest.license.identifier.clone()),
        (
            "license_reference",
            manifest.license.terms_reference.clone().unwrap_or_default(),
        ),
        ("notice_id", notice.notice_id.clone()),
        ("notice_reference", notice.reference.clone()),
        (
            "acknowledgement_required",
            manifest.license.acknowledgement_required.to_string(),
        ),
        ("artifact_id", artifact_id.to_owned()),
        ("storage", "model_store".to_owned()),
    ];
    for (key, expected) in fields {
        let actual = locked(key)?;
        if actual != expected {
            return Err(format!(
                "Granite manifest field `{key}` does not match third_party/models.lock"
            ));
        }
    }
    Ok(manifest)
}

fn verify_external_artifact(
    path: &Path,
    expected_size: u64,
    expected_digest: &str,
) -> Result<(), String> {
    let mut file = fs::File::open(path).map_err(|error| {
        format!(
            "cannot open external model artifact {}: {error}",
            path.display()
        )
    })?;
    let metadata = file.metadata().map_err(|error| {
        format!(
            "cannot inspect external model artifact {}: {error}",
            path.display()
        )
    })?;
    if !metadata.is_file() || metadata.len() != expected_size {
        return Err(format!(
            "external model artifact {} has the wrong file type or size (expected {expected_size} bytes)",
            path.display()
        ));
    }
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 1024 * 1024];
    loop {
        let read = file.read(&mut buffer).map_err(|error| {
            format!("cannot hash external artifact {}: {error}", path.display())
        })?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(read as u64)
            .ok_or_else(|| "external model size overflow".to_owned())?;
        hasher.update(&buffer[..read]);
    }
    if total != expected_size {
        return Err(format!(
            "external model artifact {} changed size during verification",
            path.display()
        ));
    }
    let actual_digest = format!("{:x}", hasher.finalize());
    if actual_digest != expected_digest {
        return Err(format!(
            "external model artifact {} has SHA-256 {actual_digest}, expected {expected_digest}",
            path.display()
        ));
    }
    Ok(())
}

fn write_sha256_manifest(paths: &Path, files: &[(&Path, String)]) -> Result<(), String> {
    let mut manifest = String::new();
    for (path, relative_name) in files {
        let digest = m30_image_sha256(path)?;
        manifest.push_str(&format!("{digest}  {relative_name}\n"));
    }
    fs::write(paths, manifest).map_err(|error| format!("cannot write {}: {error}", paths.display()))
}

/// ADR 0062: install a signed system update from update media into System B
/// from a running System A, trial it through the loader's re-verification,
/// and confirm it after guest readiness. A separate image proves a tampered
/// bundle is refused before anything is written.
fn execute_m30_update(root: &Path, probe: &dyn HostProbe) -> CommandResult {
    match run_m30_update(root, probe) {
        Ok(lines) => CommandResult {
            exit_code: EXIT_SUCCESS,
            lines,
        },
        Err(error) => failure(EXIT_CONFIG_ERROR, format!("m30-update: {error}")),
    }
}

const M30_UPDATE_BUNDLE_NAME: &str = "NAGIUPD.BIN";
const M30_UPDATE_ROLLBACK_INDEX: u64 = 2;

fn m30_update_init_args(features: &str) -> [&str; 10] {
    [
        "build",
        "-p",
        "nagi-init",
        "--features",
        features,
        "--target",
        "targets/x86_64-unknown-nagi-user.json",
        "-Zbuild-std=core,alloc,compiler_builtins",
        "--release",
        "--locked",
    ]
}

fn build_and_read(
    root: &Path,
    label: &str,
    args: &[&str],
    output: &Path,
) -> Result<Vec<u8>, String> {
    let result = run_cargo(root, label, args);
    if result.exit_code != EXIT_SUCCESS {
        return Err(format!("{label} build failed: {}", result.lines.join("; ")));
    }
    fs::read(output).map_err(|error| format!("read {}: {error}", output.display()))
}

fn run_m30_update(root: &Path, probe: &dyn HostProbe) -> Result<Vec<String>, String> {
    let run_id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock: {error}"))?
        .as_nanos()
        .to_string();
    let host = resolve_qemu_host(root, probe, "m30-update")?;
    let evidence = ensure_owned_directory(
        root,
        Path::new("out")
            .join("evidence")
            .join(format!("m30-update-{run_id}")),
    )?;
    let init_path = root.join("target/x86_64-unknown-nagi-user/release/nagi-init");
    let kernel_path = root.join("target/x86_64-unknown-nagi/release/nagi-kernel");

    // The update payload: the release kernel and a desktop init that
    // reports readiness, signed with a higher rollback index.
    let recovery_init = build_and_read(
        root,
        "M30 update Recovery init",
        &m30_update_init_args("m27-recovery"),
        &init_path,
    )?;
    let update_init = build_and_read(
        root,
        "M30 update payload init",
        // ADR 0063: the updated system is confirmed only after sign-in.
        &m30_update_init_args("desktop-login"),
        &init_path,
    )?;
    let kernel = build_and_read(
        root,
        "M30 update kernel",
        &[
            "build",
            "-p",
            "nagi-kernel",
            "--target",
            "targets/x86_64-unknown-nagi.json",
            "-Zbuild-std=core,compiler_builtins",
            "--release",
        ],
        &kernel_path,
    )?;
    let bundle = signed_update_bundle(
        &kernel,
        &update_init,
        M30_UPDATE_ROLLBACK_INDEX,
        &nagi_slot_manifest::DEVELOPER_PREVIEW_SIGNING_SECRET,
    )?;
    let mut tampered = bundle.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 0x01;
    let bundle_path = evidence.join("signed-update.bin");
    let tampered_path = evidence.join("tampered-update.bin");
    fs::write(&bundle_path, &bundle).map_err(|error| format!("write bundle: {error}"))?;
    fs::write(&tampered_path, &tampered).map_err(|error| format!("write bundle: {error}"))?;

    let mut lines = Vec::new();
    for (label, bundle_path, image_name) in [
        (
            "signed",
            bundle_path.as_path(),
            format!("nagi-0.1-m30-update-signed-{run_id}.qcow2"),
        ),
        (
            "tampered",
            tampered_path.as_path(),
            format!("nagi-0.1-m30-update-tampered-{run_id}.qcow2"),
        ),
    ] {
        let init_args = m30_update_init_args("m10-desktop,m30-update-install");
        let image = execute_image_with_init_build_env_using_writer_and_recovery(
            root,
            &init_args,
            None,
            ImageBuildRequest {
                image_name: &image_name,
                cargo_env: &[],
                recovery_init: Some(&recovery_init),
                image_writer: write_reference_disk_qcow2,
                external_model_store_file: Some((M30_UPDATE_BUNDLE_NAME, bundle_path)),
                build_features: ImageBuildFeatures {
                    kernel: &[],
                    loader: &["m27-ab-slot-boot-control"],
                },
            },
        );
        if image.exit_code != EXIT_SUCCESS {
            return Err(format!("build {label} image: {}", image.lines.join("; ")));
        }
        let image_path = root.join("out").join("artifacts").join(&image_name);
        let vars = evidence.join(format!("{label}-OVMF_VARS.fd"));
        let boot = |name: &str, marker: &'static str, reuse_vars: bool| -> Result<String, String> {
            let log = evidence.join(format!("{label}-{name}.log"));
            let config = QemuConfig {
                qemu: &host.qemu,
                ovmf_code: &host.ovmf_code,
                ovmf_vars_template: &host.ovmf_vars,
                disk_image: &image_path,
                persistent_disk: &image_path,
                vars_copy: &vars,
                serial_log: &log,
                acceptance_marker: marker,
                timeout: Duration::from_secs(180),
            };
            let status = if reuse_vars {
                run_qemu_reusing_ovmf_vars(&config)
            } else {
                run_qemu(&config)
            }
            .map_err(|error| format!("{label} {name} boot: {error} (log {})", log.display()))?;
            let serial = fs::read_to_string(&log)
                .map_err(|error| format!("read {}: {error}", log.display()))?;
            if !serial.contains(marker) {
                return Err(format!(
                    "{label} {name} boot did not print `{marker}` (QEMU exit {status}; log {})",
                    log.display()
                ));
            }
            Ok(serial)
        };
        let require = |phase: &str, serial: &str, markers: &[&str], absent: &[&str]| {
            for marker in markers {
                if !serial.contains(marker) {
                    return Err(format!("{label} {phase} did not print `{marker}`"));
                }
            }
            for marker in absent {
                if serial.contains(marker) {
                    return Err(format!("{label} {phase} unexpectedly printed `{marker}`"));
                }
            }
            Ok(())
        };

        let install = boot("install", "Nagi M7 reboot required PASS", false)?;
        if label == "tampered" {
            require(
                "install",
                &install,
                &[
                    "Nagi slot manifest verified slot=A rollback-index=1 PASS",
                    "Nagi update slot claimed slot=B",
                    "Nagi update bundle REJECTED",
                ],
                &["Nagi update written", "Nagi update stage request persisted"],
            )?;
            let next = boot("after-rejection", "Nagi M10 desktop READY", true)?;
            require(
                "after-rejection",
                &next,
                &[
                    "Nagi M27 persistence decision: confirmed slot=A",
                    "Nagi slot manifest verified slot=A rollback-index=1 PASS",
                ],
                &["Nagi M27 update stage request accepted"],
            )?;
            lines.push(format!(
                "PASS m30-update: a tampered bundle was refused before any write and System A stayed confirmed (evidence {})",
                evidence.display()
            ));
            continue;
        }
        require(
            "install",
            &install,
            &[
                "Nagi slot manifest verified slot=A rollback-index=1 PASS",
                "Nagi update slot claimed slot=B",
                "Nagi update bundle verified PASS",
                "Nagi update written slot=B PASS",
                "Nagi update readback verified slot=B PASS",
                "Nagi update stage request persisted slot=B",
                "Nagi update install PASS slot=B",
            ],
            &[],
        )?;
        // The updated system reports readiness only after the owner signs
        // in; create the owner through the OS-owned login screen.
        let trial_log = evidence.join(format!("{label}-trial.log"));
        let trial_config = QemuConfig {
            qemu: &host.qemu,
            ovmf_code: &host.ovmf_code,
            ovmf_vars_template: &host.ovmf_vars,
            disk_image: &image_path,
            persistent_disk: &image_path,
            vars_copy: &vars,
            serial_log: &trial_log,
            acceptance_marker: "Nagi M27 readiness persisted slot=B attempt=1",
            timeout: Duration::from_secs(180),
        };
        let sign_in = qmp_first_run_sign_in("owner", "nagi1");
        let sign_in: Vec<&str> = sign_in.iter().map(String::as_str).collect();
        run_qemu_gui_reusing_ovmf_vars_with_events(
            &trial_config,
            "Nagi login READY mode=create",
            &sign_in,
        )
        .map_err(|error| format!("{label} trial boot: {error} (log {})", trial_log.display()))?;
        let trial = fs::read_to_string(&trial_log)
            .map_err(|error| format!("read {}: {error}", trial_log.display()))?;
        require(
            "trial",
            &trial,
            &[
                "Nagi M27 update stage request accepted slot=B PASS",
                "Nagi M27 persistence decision: trial attempt=1 slot=B",
                "Nagi M30 GPT partition boot: System B PASS",
                "Nagi slot manifest verified slot=B rollback-index=2 PASS",
                "Nagi login owner created PASS name=owner",
                "Nagi M27 readiness persisted slot=B attempt=1",
            ],
            &["Nagi update slot claimed"],
        )?;
        if !m27_readiness_persisted_after_sign_in(&trial) {
            return Err(format!(
                "{label} trial persisted readiness before the owner signed in (log {})",
                trial_log.display()
            ));
        }
        let confirmed = boot("confirmed", "Nagi M10 desktop READY", true)?;
        require(
            "confirmed",
            &confirmed,
            &[
                "Nagi M27 readiness record consumed slot=B PASS",
                "Nagi M27 persistence decision: confirmed slot=B",
                "Nagi slot manifest verified slot=B rollback-index=2 PASS",
            ],
            &[],
        )?;
        lines.push(format!(
            "PASS m30-update: System A installed a signed update into System B, the loader re-verified and trialled it, and System B was confirmed after readiness (evidence {})",
            evidence.display()
        ));
    }
    Ok(lines)
}

fn execute_m30(root: &Path, probe: &dyn HostProbe) -> CommandResult {
    let host = match resolve_qemu_host(root, probe, "m30") {
        Ok(host) => host,
        Err(error) => return failure(EXIT_CONFIG_ERROR, error),
    };
    let source_revision = match m30_clean_source_revision(root) {
        Ok(revision) => revision,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m30: {error}")),
    };
    let run_id = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_nanos().to_string(),
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m30: system clock: {error}")),
    };
    let artifacts = match ensure_owned_directory(root, Path::new("out").join("artifacts")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m30: {error}")),
    };
    let evidence = match ensure_owned_directory(
        root,
        Path::new("out")
            .join("evidence")
            .join(format!("m30-release-{run_id}")),
    ) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m30: {error}")),
    };
    let image_name = "Nagi-OS-0.1-devpreview.qcow2";
    let image_path = artifacts.join(image_name);
    let vars_copy = artifacts.join(format!("nagi-0.1-m30-vars-{run_id}.fd"));
    let image_is_new = match fs::symlink_metadata(&image_path) {
        Ok(metadata) if metadata.file_type().is_file() => {
            if let Err(error) = validate_reference_disk_qcow2(&image_path) {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!("m30: existing release image is invalid: {error}"),
                );
            }
            if let Err(error) = verify_m30_image_build_info(&image_path, &source_revision) {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!(
                        "m30: refusing to accept an image without matching current-source provenance ({error}); preserve the existing image, move it and its `.build-info` file out of `out/artifacts`, then rerun `./nagi m30`"
                    ),
                );
            }
            false
        }
        Ok(_) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m30: release image path is not a regular file: {}",
                    image_path.display()
                ),
            );
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m30: cannot inspect release image {}: {error}",
                    image_path.display()
                ),
            );
        }
    };

    if image_is_new {
        let recovery_init_args = [
            "build",
            "-p",
            "nagi-init",
            "--features",
            "m27-recovery",
            "--target",
            "targets/x86_64-unknown-nagi-user.json",
            "-Zbuild-std=core,alloc,compiler_builtins",
            "--release",
            "--locked",
        ];
        let recovery_init_build = run_cargo(root, "M30 Recovery init", &recovery_init_args);
        if recovery_init_build.exit_code != EXIT_SUCCESS {
            return recovery_init_build;
        }
        let recovery_init_path = root
            .join("target")
            .join("x86_64-unknown-nagi-user")
            .join("release")
            .join("nagi-init");
        let recovery_init = match fs::read(&recovery_init_path) {
            Ok(bytes) => bytes,
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!("m30: cannot read {}: {error}", recovery_init_path.display()),
                );
            }
        };
        let client_env = match isolated_client_env(root) {
            Ok(env) => env,
            Err(result) => return result,
        };
        let client_env_refs = client_env
            .each_ref()
            .map(|(key, path)| (*key, path.as_path()));
        let init_args = [
            "build",
            "-p",
            "nagi-init",
            "--features",
            "m10-desktop,m19-search,m20-model-store-acceptance,m22-history,m21-action-ipc",
            "--target",
            "targets/x86_64-unknown-nagi-user.json",
            "-Zbuild-std=core,alloc,compiler_builtins",
            "--release",
            "--locked",
        ];
        let image_result = execute_image_with_init_build_env_using_writer_and_recovery(
            root,
            &init_args,
            None,
            ImageBuildRequest {
                image_name,
                cargo_env: &client_env_refs,
                recovery_init: Some(&recovery_init),
                image_writer: write_reference_disk_qcow2,
                external_model_store_file: None,
                build_features: ImageBuildFeatures {
                    kernel: &[],
                    loader: &["m27-ab-slot-boot-control"],
                },
            },
        );
        if image_result.exit_code != EXIT_SUCCESS {
            return image_result;
        }
        if let Err(error) = write_m30_image_build_info(&image_path, &source_revision) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m30: cannot write current-source image provenance: {error}"),
            );
        }
    }

    let qemu_test_image = evidence.join("reference-disk-qemu-acceptance-copy.qcow2");
    if let Err(error) = fs::copy(&image_path, &qemu_test_image) {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "m30: cannot create QEMU acceptance copy {} from {}: {error}",
                qemu_test_image.display(),
                image_path.display()
            ),
        );
    }

    let first_log = evidence.join("reference-disk-first-boot.log");
    let first_config = QemuConfig {
        qemu: &host.qemu,
        ovmf_code: &host.ovmf_code,
        ovmf_vars_template: &host.ovmf_vars,
        disk_image: &qemu_test_image,
        persistent_disk: &qemu_test_image,
        vars_copy: &vars_copy,
        serial_log: &first_log,
        acceptance_marker: "Nagi M7 acceptance PASS",
        timeout: Duration::from_secs(180),
    };
    let first_status = match run_qemu_until_any_acceptance_marker(
        &first_config,
        &["Nagi M7 reboot required PASS", "Nagi M7 acceptance PASS"],
    ) {
        Ok(status) => status,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m30: initial QEMU boot: {error}"),
            );
        }
    };
    let first_serial = match fs::read_to_string(&first_log) {
        Ok(serial) => serial,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m30: cannot read {}: {error}", first_log.display()),
            );
        }
    };
    for marker in [
        "Nagi M30 GPT partition boot: System A PASS",
        "Nagi M20 Model Store capability PASS",
        "Nagi M27 persistence decision: confirmed slot=A",
        "Nagi M27 UEFI variable journal persistence PASS",
        "Nagi Kernel started",
        "Nagi M7 VirtIO Block PASS",
    ] {
        if !first_serial.contains(marker) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m30: initial boot did not print `{marker}` (QEMU exit {first_status}; log {})",
                    first_log.display()
                ),
            );
        }
    }
    if first_serial.contains("Nagi M7 reboot required PASS") {
        for marker in ["Nagi M7 ext2 format PASS", "Nagi M7 persistent write PASS"] {
            if !first_serial.contains(marker) {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!(
                        "m30: User Data bootstrap did not print `{marker}` (log {})",
                        first_log.display()
                    ),
                );
            }
        }
    } else if !first_serial.contains("Nagi M7 persistent read PASS") {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "m30: existing User Data did not print the persistent read marker (log {})",
                first_log.display()
            ),
        );
    }

    let serial_log = evidence.join("reference-disk-restart.log");
    let restart_config = QemuConfig {
        serial_log: &serial_log,
        acceptance_marker: "Nagi M13 acceptance PASS",
        ..first_config
    };
    let qemu_status = match run_m13_qemu_with_http_fixture(root, &restart_config) {
        Ok(status) => status,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m30: persistent restart boot: {error}"),
            );
        }
    };
    let serial = match fs::read_to_string(&serial_log) {
        Ok(serial) => serial,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m30: cannot read {}: {error}", serial_log.display()),
            );
        }
    };
    for marker in [
        "Nagi M30 GPT partition boot: System A PASS",
        "Nagi M20 Model Store capability PASS",
        "Nagi M27 persistence decision: confirmed slot=A",
        "Nagi M27 UEFI variable journal persistence PASS",
        "Nagi Kernel started",
        "Nagi M7 VirtIO Block PASS",
        "Nagi M7 ext2 mount PASS",
        "Nagi M7 persistent read PASS",
        "Nagi M7 acceptance PASS",
        "Nagi bootstrap Channel wait/wake PASS",
        "Nagi M19 live VFS file ObjectId rename/restart PASS",
        "Nagi M19 guest search persistence PASS",
        "Nagi M22 file.search Activity Ledger PASS",
        "Nagi M22 AI Activity Ledger committed PASS",
        "Nagi M21 file.move Plan Validate Execute PASS",
        "Nagi M22 move group persisted in guest VFS PASS",
        "Nagi M21 file.copy Plan Validate Execute PASS",
        "Nagi M22 file.copy prepared transaction persisted PASS",
        "Nagi M13 acceptance PASS",
    ] {
        if !serial.contains(marker) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m30: release guest did not print `{marker}` (QEMU exit {qemu_status}; log {})",
                    serial_log.display()
                ),
            );
        }
    }

    let recovery_log = evidence.join("reference-disk-recovery-boot.log");
    let recovery_config = QemuConfig {
        serial_log: &recovery_log,
        acceptance_marker: "Nagi M27 Recovery command help PASS",
        timeout: Duration::from_secs(180),
        ..restart_config
    };
    let recovery_status = match run_qemu_gui_reusing_ovmf_vars_with_events_and_serial_input(
        &recovery_config,
        "Nagi M27 Recovery boot menu READY",
        &M27_RECOVERY_MENU_EVENTS,
        "Nagi M27 Recovery console READY",
        b"check\nfiles\nhelp\n",
    ) {
        Ok(status) => status,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m30: Recovery QEMU boot: {error}"),
            );
        }
    };
    let recovery_serial = match fs::read_to_string(&recovery_log) {
        Ok(serial) => serial,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m30: cannot read {}: {error}", recovery_log.display()),
            );
        }
    };
    for marker in [
        "Nagi M27 boot menu: confirmed=A pending=none",
        "Nagi M27 manual selection: Recovery; boot journal unchanged PASS",
        "Nagi M30 GPT partition boot: Recovery PASS",
        "Nagi M27 Recovery Environment START",
        "Nagi M27 Recovery VFS check PASS files=",
        "Nagi M27 Recovery command help PASS",
    ] {
        if !recovery_serial.contains(marker) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m30: Recovery boot did not print `{marker}` (QEMU exit {recovery_status}; log {})",
                    recovery_log.display()
                ),
            );
        }
    }
    if recovery_serial.contains("Nagi M27 persistence decision:")
        || recovery_serial.contains("Nagi M27 readiness persisted")
    {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "m30: Recovery changed the A/B boot decision or reported trial readiness (log {})",
                recovery_log.display()
            ),
        );
    }

    let unstaged_b_log = evidence.join("reference-disk-unstaged-system-b.log");
    let unstaged_b_config = QemuConfig {
        serial_log: &unstaged_b_log,
        acceptance_marker: "Nagi M30 GPT partition boot: System A PASS",
        ..restart_config
    };
    let unstaged_b_status = match run_qemu_gui_reusing_ovmf_vars_with_events(
        &unstaged_b_config,
        "Nagi M27 Recovery boot menu READY",
        &M30_UNSTAGED_SYSTEM_B_MENU_EVENTS,
    ) {
        Ok(status) => status,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m30: unstaged System B selection QEMU: {error}"),
            );
        }
    };
    let unstaged_b_serial = match fs::read_to_string(&unstaged_b_log) {
        Ok(serial) => serial,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m30: cannot read {}: {error}", unstaged_b_log.display()),
            );
        }
    };
    for marker in [
        "Nagi M27 boot menu: confirmed=A pending=none",
        "Nagi M27 boot menu: System B unavailable (no staged image)",
        "Nagi M27 manual selection: confirmed slot=A (pending trial preserved) PASS",
        "Nagi M27 UEFI variable journal persistence PASS",
        "Nagi M30 GPT partition boot: System A PASS",
    ] {
        if !unstaged_b_serial.contains(marker) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m30: unstaged System B selection did not print `{marker}` (QEMU exit {unstaged_b_status}; log {})",
                    unstaged_b_log.display()
                ),
            );
        }
    }
    if unstaged_b_serial.contains("Nagi M27 persistence decision:")
        || unstaged_b_serial.contains("Nagi M27 readiness persisted")
    {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "m30: an unstaged System B selection changed boot policy state (log {})",
                unstaged_b_log.display()
            ),
        );
    }

    let post_recovery_log = evidence.join("reference-disk-post-recovery-boot.log");
    let post_recovery_config = QemuConfig {
        serial_log: &post_recovery_log,
        ..restart_config
    };
    let post_recovery_status = match run_m13_qemu_with_http_fixture(root, &post_recovery_config) {
        Ok(status) => status,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m30: post-Recovery System A restart: {error}"),
            );
        }
    };
    let post_recovery_serial = match fs::read_to_string(&post_recovery_log) {
        Ok(serial) => serial,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m30: cannot read {}: {error}", post_recovery_log.display()),
            );
        }
    };
    for marker in [
        "Nagi M30 GPT partition boot: System A PASS",
        "Nagi M27 persistence decision: confirmed slot=A",
        "Nagi M27 UEFI variable journal persistence PASS",
        "Nagi M7 persistent read PASS",
        "Nagi M7 acceptance PASS",
        "Nagi bootstrap Channel wait/wake PASS",
        "Nagi M19 live VFS file ObjectId rename/restart PASS",
        "Nagi M19 guest search persistence PASS",
        "Nagi M22 file.search Activity Ledger PASS",
        "Nagi M22 AI Activity Ledger undo result PASS",
        "Nagi M22 composite undo applied and persisted PASS",
        "Nagi M13 acceptance PASS",
    ] {
        if !post_recovery_serial.contains(marker) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m30: post-Recovery boot did not print `{marker}` (QEMU exit {post_recovery_status}; log {})",
                    post_recovery_log.display()
                ),
            );
        }
    }

    let evidence_vars_copy = evidence.join("reference-disk-OVMF_VARS.fd");
    if let Err(error) = fs::copy(&vars_copy, &evidence_vars_copy) {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "m30: cannot preserve final OVMF variables at {}: {error}",
                evidence_vars_copy.display()
            ),
        );
    }

    let recovery_init_args = [
        "build",
        "-p",
        "nagi-init",
        "--features",
        "m27-recovery",
        "--target",
        "targets/x86_64-unknown-nagi-user.json",
        "-Zbuild-std=core,alloc,compiler_builtins",
        "--release",
        "--locked",
    ];
    let recovery_init_build = run_cargo(root, "M20 fixture Recovery init", &recovery_init_args);
    if recovery_init_build.exit_code != EXIT_SUCCESS {
        return recovery_init_build;
    }
    let recovery_init_path = root
        .join("target")
        .join("x86_64-unknown-nagi-user")
        .join("release")
        .join("nagi-init");
    let recovery_init = match fs::read(&recovery_init_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m30: cannot read {}: {error}", recovery_init_path.display()),
            );
        }
    };
    let fixture_image_name = format!("nagi-0.1-m20-reader-{run_id}.qcow2");
    let fixture_client_env = match isolated_client_env(root) {
        Ok(env) => env,
        Err(result) => return result,
    };
    let fixture_client_env_refs = fixture_client_env
        .each_ref()
        .map(|(key, path)| (*key, path.as_path()));
    let fixture_init_args = [
        "build",
        "-p",
        "nagi-init",
        "--features",
        "m10-desktop,m19-search,m20-fixture-acceptance,m22-history,m21-action-ipc",
        "--target",
        "targets/x86_64-unknown-nagi-user.json",
        "-Zbuild-std=core,alloc,compiler_builtins",
        "--release",
        "--locked",
    ];
    let fixture_image_result = execute_image_with_init_build_env_using_writer_and_recovery(
        root,
        &fixture_init_args,
        None,
        ImageBuildRequest {
            image_name: &fixture_image_name,
            cargo_env: &fixture_client_env_refs,
            recovery_init: Some(&recovery_init),
            image_writer: write_m20_model_store_fixture_reference_disk_qcow2,
            external_model_store_file: None,
            build_features: ImageBuildFeatures {
                kernel: &[],
                loader: &["m27-ab-slot-boot-control"],
            },
        },
    );
    if fixture_image_result.exit_code != EXIT_SUCCESS {
        return fixture_image_result;
    }
    let fixture_image_artifact = artifacts.join(&fixture_image_name);
    let fixture_image = evidence.join("m20-model-store-reader-fixture.qcow2");
    if let Err(error) = fs::copy(&fixture_image_artifact, &fixture_image) {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "m30: cannot preserve M20 reader fixture {}: {error}",
                fixture_image.display()
            ),
        );
    }
    let fixture_vars_copy = evidence.join("m20-model-store-reader-fixture-vars.fd");
    if let Err(error) = initialize_ovmf_vars(&host.ovmf_vars, &fixture_vars_copy) {
        return failure(
            EXIT_CONFIG_ERROR,
            format!("m30: initialize M20 fixture OVMF variables: {error}"),
        );
    }
    let fixture_serial_log = evidence.join("m20-model-store-reader-fixture.log");
    let fixture_config = QemuConfig {
        qemu: &host.qemu,
        ovmf_code: &host.ovmf_code,
        ovmf_vars_template: &host.ovmf_vars,
        disk_image: &fixture_image,
        persistent_disk: &fixture_image,
        vars_copy: &fixture_vars_copy,
        serial_log: &fixture_serial_log,
        acceptance_marker: "Nagi M20 FAT32 fixture read PASS",
        timeout: Duration::from_secs(180),
    };
    let fixture_status = match run_qemu_until_any_acceptance_marker(
        &fixture_config,
        &["Nagi M20 FAT32 fixture read PASS"],
    ) {
        Ok(status) => status,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m30: M20 Model Store fixture QEMU boot: {error}"),
            );
        }
    };
    let fixture_serial = match fs::read_to_string(&fixture_serial_log) {
        Ok(serial) => serial,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m30: cannot read {}: {error}", fixture_serial_log.display()),
            );
        }
    };
    for marker in [
        "Nagi M30 GPT partition boot: System A PASS",
        "Nagi M20 Model Store capability PASS",
        "Nagi M20 FAT32 fixture read PASS",
    ] {
        if !fixture_serial.contains(marker) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m30: M20 fixture guest did not print `{marker}` (QEMU exit {fixture_status}; log {})",
                    fixture_serial_log.display()
                ),
            );
        }
    }
    for checked_image in [&image_path, &qemu_test_image, &fixture_image] {
        let image_check = ProcessCommand::new("qemu-img")
            .args(["check", "-f", "qcow2"])
            .arg(checked_image)
            .output();
        match image_check {
            Ok(output) if output.status.success() => {}
            Ok(output) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!(
                        "m30: qemu-img check failed for {}: {}",
                        checked_image.display(),
                        String::from_utf8_lossy(&output.stderr).trim()
                    ),
                );
            }
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!("m30: qemu-img check {}: {error}", checked_image.display()),
                );
            }
        }
    }
    CommandResult {
        exit_code: EXIT_SUCCESS,
        lines: vec![format!(
            "PASS M30 64 GiB GPT qcow2 passed System A, User Data persistence, Recovery, unstaged System B rejection, and post-Recovery restart acceptance; separate M20 guest FAT32 fixture read passed (image {}; QEMU copy {}; System A log {}; Recovery log {}; unstaged System B log {}; post-Recovery log {}; M20 fixture {}; M20 log {})",
            image_path.display(),
            qemu_test_image.display(),
            serial_log.display(),
            recovery_log.display(),
            unstaged_b_log.display(),
            post_recovery_log.display(),
            fixture_image.display(),
            fixture_serial_log.display()
        )],
    }
}

fn m30_clean_source_revision(root: &Path) -> Result<String, String> {
    let revision = ProcessCommand::new("git")
        .args(["rev-parse", "--verify", "HEAD"])
        .current_dir(root)
        .output()
        .map_err(|error| format!("cannot read Git source revision: {error}"))?;
    if !revision.status.success() {
        return Err(format!(
            "cannot read Git source revision: {}",
            String::from_utf8_lossy(&revision.stderr).trim()
        ));
    }
    let revision = String::from_utf8(revision.stdout)
        .map_err(|error| format!("Git source revision is not UTF-8: {error}"))?;
    let revision = revision.trim();
    if !valid_m30_source_revision(revision) {
        return Err("Git returned an invalid full source revision".to_owned());
    }

    let status = ProcessCommand::new("git")
        .args(["status", "--porcelain=v1", "--untracked-files=all"])
        .current_dir(root)
        .output()
        .map_err(|error| format!("cannot verify Git source tree cleanliness: {error}"))?;
    if !status.status.success() {
        return Err(format!(
            "cannot verify Git source tree cleanliness: {}",
            String::from_utf8_lossy(&status.stderr).trim()
        ));
    }
    if !status.stdout.is_empty() {
        return Err("M30 image acceptance requires a clean committed source tree".to_owned());
    }
    Ok(revision.to_owned())
}

fn valid_m30_source_revision(revision: &str) -> bool {
    matches!(revision.len(), 40 | 64)
        && revision
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn m30_image_build_info_path(image_path: &Path) -> Result<PathBuf, String> {
    let Some(file_name) = image_path.file_name() else {
        return Err(format!(
            "image path has no filename: {}",
            image_path.display()
        ));
    };
    let mut build_info_name = file_name.to_os_string();
    build_info_name.push(".build-info");
    Ok(image_path.with_file_name(build_info_name))
}

fn m30_image_sha256(image_path: &Path) -> Result<String, String> {
    let mut image = fs::File::open(image_path)
        .map_err(|error| format!("cannot open {}: {error}", image_path.display()))?;
    let mut digest = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let length = image
            .read(&mut buffer)
            .map_err(|error| format!("cannot hash {}: {error}", image_path.display()))?;
        if length == 0 {
            break;
        }
        digest.update(&buffer[..length]);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn m30_image_build_info_matches(contents: &str, source_revision: &str, image_sha256: &str) -> bool {
    valid_m30_source_revision(source_revision)
        && image_sha256.len() == 64
        && image_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        && contents
            == format!(
                "format_version=1\nsource_revision={source_revision}\nimage_sha256={image_sha256}\n"
            )
}

fn verify_m30_image_build_info(image_path: &Path, source_revision: &str) -> Result<(), String> {
    let image_sha256 = m30_image_sha256(image_path)?;
    let build_info_path = m30_image_build_info_path(image_path)?;
    let metadata = fs::symlink_metadata(&build_info_path)
        .map_err(|error| format!("cannot inspect {}: {error}", build_info_path.display()))?;
    if !metadata.file_type().is_file() {
        return Err(format!(
            "{} is not a regular file",
            build_info_path.display()
        ));
    }
    let contents = fs::read_to_string(&build_info_path)
        .map_err(|error| format!("cannot read {}: {error}", build_info_path.display()))?;
    if !m30_image_build_info_matches(&contents, source_revision, &image_sha256) {
        return Err(format!(
            "{} does not match source revision {source_revision} and image SHA-256 {image_sha256}",
            build_info_path.display()
        ));
    }
    Ok(())
}

fn write_m30_image_build_info(image_path: &Path, source_revision: &str) -> Result<(), String> {
    if !valid_m30_source_revision(source_revision) {
        return Err("invalid source revision".to_owned());
    }
    let image_sha256 = m30_image_sha256(image_path)?;
    let build_info_path = m30_image_build_info_path(image_path)?;
    match fs::symlink_metadata(&build_info_path) {
        Ok(metadata) if metadata.file_type().is_file() => {}
        Ok(_) => {
            return Err(format!(
                "{} is not a regular file",
                build_info_path.display()
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "cannot inspect {}: {error}",
                build_info_path.display()
            ));
        }
    }
    fs::write(
        &build_info_path,
        format!(
            "format_version=1\nsource_revision={source_revision}\nimage_sha256={image_sha256}\n"
        ),
    )
    .map_err(|error| format!("cannot write {}: {error}", build_info_path.display()))
}

struct QemuHost {
    qemu: PathBuf,
    ovmf_code: PathBuf,
    ovmf_vars: PathBuf,
}

fn resolve_qemu_host(root: &Path, probe: &dyn HostProbe, label: &str) -> Result<QemuHost, String> {
    let Some(qemu) = probe.command(
        &["qemu-system-x86_64", "qemu-system-x86_64.exe"],
        &["--version"],
    ) else {
        return Err(format!("{label}: QEMU was not found"));
    };
    if qemu.exit_code != Some(0) {
        return Err(format!(
            "{label}: QEMU probe failed: {}",
            qemu.output.trim()
        ));
    }
    let Some(ovmf) = probe.ovmf() else {
        return Err(format!("{label}: compatible OVMF CODE/VARS was not found"));
    };
    if !ovmf.is_compatible() {
        return Err(format!(
            "{label}: incompatible OVMF CODE/VARS: {} / {}",
            ovmf.code, ovmf.vars
        ));
    }
    let requirements = load_toolchain_requirements(root)
        .map_err(|error| format!("{label}: toolchain: {error}"))?;
    if !ovmf_pair_is_allowed(&ovmf, &requirements) {
        return Err(format!(
            "{label}: OVMF CODE/VARS pair is not listed in nagi.toml: {} / {}",
            ovmf.code, ovmf.vars
        ));
    }
    Ok(QemuHost {
        qemu: PathBuf::from(qemu.path),
        ovmf_code: PathBuf::from(ovmf.code),
        ovmf_vars: PathBuf::from(ovmf.vars),
    })
}

const M10_DESKTOP_EVENTS: [&str; 8] = [
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"btn","data":{"button":"left","down":true}},{"type":"btn","data":{"button":"left","down":false}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"rel","data":{"axis":"x","value":100}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"btn","data":{"button":"left","down":true}},{"type":"btn","data":{"button":"left","down":false}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"a"}}},{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"a"}}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"rel","data":{"axis":"x","value":-100}},{"type":"rel","data":{"axis":"y","value":70}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"btn","data":{"button":"left","down":true}},{"type":"btn","data":{"button":"left","down":false}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"rel","data":{"axis":"x","value":100}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"btn","data":{"button":"left","down":true}},{"type":"btn","data":{"button":"left","down":false}}]}}"#,
];

const M29_DESKTOP_FOCUS_EVENTS: [&str; 11] = [
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"tab"}}},{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"tab"}}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"tab"}}},{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"tab"}}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"ret"}}},{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"ret"}}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"tab"}}},{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"tab"}}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"ret"}}},{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"ret"}}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"a"}}},{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"a"}}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"tab"}}},{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"tab"}}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"ret"}}},{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"ret"}}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"tab"}}},{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"tab"}}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"ret"}}},{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"ret"}}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"tab"}}},{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"tab"}}}]}}"#,
];

const M29_SETTINGS_EVENTS: [&str; 10] = [
    // Move the pointer away from the language controls before activating the
    // final locale; the accepted Desktop may stop polling input immediately.
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"rel","data":{"axis":"x","value":100}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"rel","data":{"axis":"y","value":38}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"tab"}}},{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"tab"}}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"ret"}}},{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"ret"}}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"esc"}}},{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"esc"}}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"ret"}}},{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"ret"}}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"down"}}},{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"down"}}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"up"}}},{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"up"}}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"down"}}},{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"down"}}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"spc"}}},{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"spc"}}}]}}"#,
];

/// ADR 0060 dialog input. The pointer starts at (80, 58) inside the dialog.
/// A press on Deny released elsewhere must decide nothing; then Tab moves
/// focus Deny -> Allow once -> Allow and Enter (press and release) allows.
const CONSENT_DIALOG_EVENTS: [&str; 7] = [
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"rel","data":{"axis":"y","value":78}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"btn","data":{"button":"left","down":true}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"rel","data":{"axis":"y","value":-40}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"btn","data":{"button":"left","down":false}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"tab"}}},{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"tab"}}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"tab"}}},{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"tab"}}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"ret"}}},{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"ret"}}}]}}"#,
];

const CONSENT_OWNER_NAME: &str = "owner";
const CONSENT_OWNER_PASSWORD: &str = "nagi1";

const CONSENT_DIALOG_REQUIRED_MARKERS: &[&str] = &[
    "Nagi boot lock READY",
    "Nagi M10 desktop READY",
    "Nagi login READY mode=create",
    "Nagi login owner created PASS name=owner",
    "Nagi login unlocked PASS",
    "Nagi consent dialog SHOWN app=org.nagi.acceptance.faulting-app capability=acceptance.consent-probe",
    "Nagi consent dialog decision PASS decision=allow",
    "Nagi consent decision persisted PASS",
    "Nagi consent dialog acceptance PASS",
];

const M10_DESKTOP_REQUIRED_MARKERS: &[&str] = &[
    "Nagi boot stage PLATFORM 15",
    "Nagi boot stage CORE_SERVICES 30",
    "Nagi boot stage STORAGE 50",
    "Nagi boot stage GRAPHICS 70",
    "Nagi boot stage SESSION 90",
    "Nagi boot lock READY",
    "Nagi boot collapse COMPLETE",
    "Nagi boot frame checksum=",
    "Nagi boot lock checksum=",
    "Nagi M10 desktop READY",
    "Nagi M10 surface checksum=",
    "Nagi M10 Calculator focus PASS",
    "Nagi M10 Notes focus PASS",
    "Nagi M10 Japanese input PASS",
    "Nagi M10 Files focus PASS",
    "Nagi M10 GUI Terminal focus PASS",
    "Nagi M29 keyboard locale selection PASS locale=ja-JP",
    "Nagi M10 acceptance PASS",
];

const M29_SETTINGS_REQUIRED_MARKERS: &[&str] = &[
    "Nagi boot stage PLATFORM 15",
    "Nagi boot stage CORE_SERVICES 30",
    "Nagi boot stage STORAGE 50",
    "Nagi boot stage GRAPHICS 70",
    "Nagi boot stage SESSION 90",
    "Nagi boot lock READY",
    "Nagi boot collapse COMPLETE",
    "Nagi boot frame checksum=",
    "Nagi boot lock checksum=",
    "Nagi M10 desktop READY",
    "Nagi M10 surface checksum=",
    "Nagi M10 Calculator focus PASS",
    "Nagi M10 Notes focus PASS",
    "Nagi M10 Japanese input PASS",
    "Nagi M10 Files focus PASS",
    "Nagi M10 GUI Terminal focus PASS",
    "Nagi M29 desktop keyboard focus PASS",
    "Nagi M29 settings locale persisted PASS locale=ja-JP",
    "Nagi M29 settings locale PASS locale=ja-JP",
    "Nagi M10 acceptance PASS",
    "Nagi M29 settings acceptance PASS",
];

fn execute_gui(root: &Path, probe: &dyn HostProbe) -> CommandResult {
    let image_result =
        execute_image_with_features(root, Some("m9-window"), "nagi-0.1-m9-window.img");
    if image_result.exit_code != EXIT_SUCCESS {
        return image_result;
    }
    let host = match resolve_qemu_host(root, probe, "gui") {
        Ok(host) => host,
        Err(error) => return failure(EXIT_CONFIG_ERROR, error),
    };
    let artifacts = match ensure_owned_directory(root, Path::new("out").join("artifacts")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("gui: {error}")),
    };
    let logs = match ensure_owned_directory(root, Path::new("out").join("logs")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("gui: {error}")),
    };
    let image_path = artifacts.join("nagi-0.1-m9-window.img");
    let persistent_disk = artifacts.join("nagi-0.1-user-data.img");
    let vars_copy = artifacts.join("nagi-0.1-m9-window-vars.fd");
    let first_log = logs.join("m9-first-boot.log");
    let gui_log = logs.join("m9-gui.log");
    let had_persistent_disk = match ensure_persistent_disk(&persistent_disk) {
        Ok(existing) => existing,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("gui: {error}")),
    };
    let timeout = Duration::from_secs(45);
    if !had_persistent_disk {
        let first_config = QemuConfig {
            qemu: &host.qemu,
            ovmf_code: &host.ovmf_code,
            ovmf_vars_template: &host.ovmf_vars,
            disk_image: &image_path,
            persistent_disk: &persistent_disk,
            vars_copy: &vars_copy,
            serial_log: &first_log,
            acceptance_marker: NAGI_WRITE_MARKER,
            timeout,
        };
        if let Err(error) = run_qemu(&first_config) {
            return failure(EXIT_CONFIG_ERROR, format!("gui: first boot: {error}"));
        }
        let first_serial = match fs::read_to_string(&first_log) {
            Ok(serial) => serial,
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!("gui: cannot read {}: {error}", first_log.display()),
                );
            }
        };
        if !first_serial.contains(NAGI_WRITE_MARKER) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("gui: first boot did not print `{NAGI_WRITE_MARKER}`"),
            );
        }
    }
    let config = QemuConfig {
        qemu: &host.qemu,
        ovmf_code: &host.ovmf_code,
        ovmf_vars_template: &host.ovmf_vars,
        disk_image: &image_path,
        persistent_disk: &persistent_disk,
        vars_copy: &vars_copy,
        serial_log: &gui_log,
        acceptance_marker: "Nagi M9 acceptance PASS",
        timeout,
    };
    let status = match run_qemu_gui(&config) {
        Ok(status) => status,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("gui: {error}")),
    };
    let serial = match fs::read_to_string(&gui_log) {
        Ok(serial) => serial,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("gui: cannot read {}: {error}", gui_log.display()),
            );
        }
    };
    for marker in [
        "Nagi M9 window READY",
        "Nagi M9 mouse move PASS",
        "Nagi M9 state x=",
        "Nagi M9 focus PASS",
        "Nagi M9 keyboard PASS",
        "Nagi M9 acceptance PASS",
    ] {
        if !serial.contains(marker) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "gui: guest did not print `{marker}` (QEMU exit {status}; log {})",
                    gui_log.display()
                ),
            );
        }
    }
    CommandResult {
        exit_code: EXIT_SUCCESS,
        lines: vec![format!(
            "PASS gui: QEMU guest moved and focused the Nagi window (exit {status}; log {})",
            gui_log.display()
        )],
    }
}

fn execute_desktop(root: &Path, probe: &dyn HostProbe) -> CommandResult {
    execute_desktop_acceptance(
        root,
        probe,
        DesktopAcceptanceConfig {
            label: "desktop",
            features: "m10-desktop",
            image_name: "nagi-0.1-m10-desktop.img",
            persistent_disk_name: "nagi-0.1-user-data.img",
            vars_name: "nagi-0.1-m10-desktop-vars.fd",
            first_log_name: "m10-first-boot.log",
            run_log_name: "m10-desktop.log",
            evidence_prefix: "m29-desktop",
            screenshot_name: "nagi-m10-desktop.png",
            acceptance_marker: "Nagi M10 acceptance PASS",
            required_markers: M10_DESKTOP_REQUIRED_MARKERS,
            restart_marker: None,
            restart_log_name: None,
            restart_summary: "",
            unique_run_artifacts: false,
            acceptance_packages: false,
            ready_screenshot_name: None,
            restart_events: Vec::new(),
            later_stages: Vec::new(),
            ready_screenshot_stage: None,
        },
        &M10_DESKTOP_EVENTS,
    )
}

fn execute_m29(root: &Path, probe: &dyn HostProbe) -> CommandResult {
    let mut events = M29_DESKTOP_FOCUS_EVENTS.to_vec();
    events.extend_from_slice(&M10_DESKTOP_EVENTS);
    events.extend_from_slice(&M29_SETTINGS_EVENTS);
    execute_desktop_acceptance(
        root,
        probe,
        DesktopAcceptanceConfig {
            label: "m29",
            features: "m10-desktop,m29-settings-acceptance",
            image_name: "nagi-0.1-m29-settings-persistent.img",
            persistent_disk_name: "nagi-0.1-m29-settings-persistent-user-data.img",
            vars_name: "nagi-0.1-m29-settings-persistent-vars.fd",
            first_log_name: "m29-settings-persistent-first-boot.log",
            run_log_name: "m29-settings-persistent.log",
            evidence_prefix: "m29-settings",
            screenshot_name: "nagi-m29-settings-ja-jp.png",
            acceptance_marker: "Nagi M29 settings acceptance PASS",
            required_markers: M29_SETTINGS_REQUIRED_MARKERS,
            restart_marker: Some("Nagi M29 settings preference restored PASS locale=ja-JP"),
            restart_log_name: Some("m29-settings-persistent-restart.log"),
            restart_summary: "the selected system language was restored",
            unique_run_artifacts: true,
            acceptance_packages: false,
            ready_screenshot_name: None,
            restart_events: Vec::new(),
            later_stages: Vec::new(),
            ready_screenshot_stage: None,
        },
        &events,
    )
}

/// ADR 0060: the desktop's OS-owned consent dialog answers a signed
/// application's grant request through real QMP input, and the persisted
/// `Allow` holds after a restart without a new prompt.
fn execute_consent(root: &Path, probe: &dyn HostProbe) -> CommandResult {
    // ADR 0063: decisions belong to the signed-in owner. Create the owner
    // first; the dialog appears after sign-in.
    // `events` signs in; the dialog input is a later stage.
    let events = qmp_first_run_sign_in(CONSENT_OWNER_NAME, CONSENT_OWNER_PASSWORD);
    execute_desktop_acceptance(
        root,
        probe,
        DesktopAcceptanceConfig {
            label: "consent",
            features: "consent-dialog-acceptance,desktop-login",
            image_name: "nagi-0.1-consent-dialog.img",
            persistent_disk_name: "nagi-0.1-consent-dialog-user-data.img",
            vars_name: "nagi-0.1-consent-dialog-vars.fd",
            first_log_name: "consent-dialog-first-boot.log",
            run_log_name: "consent-dialog.log",
            evidence_prefix: "consent-dialog",
            screenshot_name: "nagi-consent-dialog-answered.png",
            acceptance_marker: "Nagi consent dialog acceptance PASS",
            required_markers: CONSENT_DIALOG_REQUIRED_MARKERS,
            restart_marker: Some("Nagi consent decision restored PASS decision=allow"),
            restart_log_name: Some("consent-dialog-restart.log"),
            restart_summary: "the persisted Allow decision was restored without a prompt",
            unique_run_artifacts: true,
            acceptance_packages: true,
            ready_screenshot_name: Some("nagi-consent-dialog-shown.png"),
            restart_events: qmp_typed_keys(CONSENT_OWNER_PASSWORD, "ret"),
            // Answer the dialog, and capture it, only once it is shown.
            later_stages: vec![(
                "Nagi consent dialog SHOWN",
                CONSENT_DIALOG_EVENTS
                    .iter()
                    .map(|event| (*event).to_owned())
                    .collect(),
            )],
            ready_screenshot_stage: Some("Nagi consent dialog SHOWN"),
        },
        &events.iter().map(String::as_str).collect::<Vec<_>>(),
    )
}

/// Build a desktop init image that embeds the signed acceptance packages.
fn execute_image_with_acceptance_packages(
    root: &Path,
    features: &str,
    image_name: &str,
) -> CommandResult {
    let packages = match build_isolated_apps(root) {
        Ok(directory) => directory,
        Err(result) => return result,
    };
    let init_args = [
        "build",
        "-p",
        "nagi-init",
        "--features",
        features,
        "--target",
        "targets/x86_64-unknown-nagi-user.json",
        "-Zbuild-std=core,alloc,compiler_builtins",
        "--release",
        "--locked",
    ];
    execute_image_with_init_build_env_using_writer(
        root,
        &init_args,
        None,
        image_name,
        &[("NAGI_ACCEPTANCE_PACKAGES", packages.as_path())],
        write_isolated_apps_fat12_image,
        ImageBuildFeatures::default(),
    )
}

/// QMP commands that type `text` (lowercase letters, digits, `-`), then
/// press `finish` (`ret` or `tab`), one key press/release per command.
fn qmp_typed_keys(text: &str, finish: &str) -> Vec<String> {
    text.chars()
        .map(|character| match character {
            '-' => "minus".to_owned(),
            other => other.to_string(),
        })
        .chain(std::iter::once(finish.to_owned()))
        .map(|key| {
            format!(
                r#"{{"execute":"input-send-event","arguments":{{"events":[{{"type":"key","data":{{"down":true,"key":{{"type":"qcode","data":"{key}"}}}}}},{{"type":"key","data":{{"down":false,"key":{{"type":"qcode","data":"{key}"}}}}}}]}}}}"#
            )
        })
        .collect()
}

/// First-run input for the OS-owned login (ADR 0063): keep the offered
/// default language, then create the owner `name` with `password`.
fn qmp_first_run_sign_in(name: &str, password: &str) -> Vec<String> {
    let mut events = qmp_typed_keys("", "ret");
    events.extend(qmp_typed_keys(name, "ret"));
    events.extend(qmp_typed_keys(password, "ret"));
    events.extend(qmp_typed_keys(password, "ret"));
    events
}

#[cfg(test)]
#[test]
fn first_run_sign_in_answers_the_language_step_first() {
    let events = qmp_first_run_sign_in("ab", "cd");
    // Language Enter, then "ab"+Enter, then "cd"+Enter twice.
    assert_eq!(events.len(), 1 + 3 + 3 + 3);
    assert!(events[0].contains(r#""data":"ret""#));
    assert!(events[1].contains(r#""data":"a""#));
}

/// ADR 0063: first run creates the owner account through the OS-owned
/// login screen; after a restart a wrong password is refused and the right
/// one signs in. Readiness and the desktop follow only a sign-in.
fn execute_login(root: &Path, probe: &dyn HostProbe) -> CommandResult {
    match run_login_acceptance(root, probe) {
        Ok(lines) => CommandResult {
            exit_code: EXIT_SUCCESS,
            lines,
        },
        Err(error) => failure(EXIT_CONFIG_ERROR, format!("login: {error}")),
    }
}

fn run_login_acceptance(root: &Path, probe: &dyn HostProbe) -> Result<Vec<String>, String> {
    let run_id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock: {error}"))?
        .as_nanos()
        .to_string();
    let image_name = format!("nagi-0.1-login-{run_id}.img");
    let image = execute_image_with_features(root, Some("desktop-login-acceptance"), &image_name);
    if image.exit_code != EXIT_SUCCESS {
        return Err(image.lines.join("; "));
    }
    let host = resolve_qemu_host(root, probe, "login")?;
    let evidence = ensure_owned_directory(
        root,
        Path::new("out")
            .join("evidence")
            .join(format!("login-{run_id}")),
    )?;
    let image_path = root.join("out").join("artifacts").join(&image_name);
    let disk = evidence.join("user-data.img");
    let vars = evidence.join("OVMF_VARS.fd");
    ensure_persistent_disk(&disk)?;
    fn login_config<'a>(
        host: &'a QemuHost,
        paths: [&'a Path; 4],
        marker: &'static str,
    ) -> QemuConfig<'a> {
        let [image, disk, vars, log] = paths;
        QemuConfig {
            qemu: &host.qemu,
            ovmf_code: &host.ovmf_code,
            ovmf_vars_template: &host.ovmf_vars,
            disk_image: image,
            persistent_disk: disk,
            vars_copy: vars,
            serial_log: log,
            acceptance_marker: marker,
            timeout: Duration::from_secs(60),
        }
    }
    let format_log = evidence.join("format.log");
    run_qemu(&login_config(
        &host,
        [&image_path, &disk, &vars, &format_log],
        NAGI_WRITE_MARKER,
    ))?;
    let read = |log: &Path| {
        fs::read_to_string(log).map_err(|error| format!("read {}: {error}", log.display()))
    };
    if !read(&format_log)?.contains(NAGI_WRITE_MARKER) {
        return Err(format!(
            "User Data format boot did not print `{NAGI_WRITE_MARKER}`"
        ));
    }

    // M29 onboarding: choose 日本語 (second option) before the account.
    let mut create_events = qmp_typed_keys("", "down");
    create_events.extend(qmp_typed_keys("", "ret"));
    create_events.extend(qmp_typed_keys("owner", "ret"));
    create_events.extend(qmp_typed_keys("nagi1", "ret"));
    create_events.extend(qmp_typed_keys("nagi1", "ret"));
    let mut unlock_events = qmp_typed_keys("wrong1", "ret");
    unlock_events.extend(qmp_typed_keys("nagi1", "ret"));
    let mut lines = Vec::new();
    for (phase, events, markers, absent) in [
        (
            "create",
            create_events,
            &[
                "Nagi M10 desktop READY",
                "Nagi login READY mode=create",
                "Nagi onboarding language PASS locale=ja-JP",
                "Nagi login owner created PASS name=owner",
                "Nagi login unlocked PASS",
                "Nagi login acceptance PASS",
            ][..],
            &[
                "Nagi login unlock REJECTED",
                "Nagi M27 readiness persistence FAIL",
            ][..],
        ),
        (
            "unlock",
            unlock_events,
            &[
                "Nagi M10 desktop READY",
                "Nagi login READY mode=unlock",
                "Nagi M29 settings preference restored PASS locale=ja-JP",
                "Nagi login unlock REJECTED",
                "Nagi login unlocked PASS",
                "Nagi login acceptance PASS",
            ][..],
            &["Nagi login owner created"][..],
        ),
    ] {
        let log = evidence.join(format!("{phase}.log"));
        let screenshot = evidence.join(format!("{phase}-signed-in.png"));
        let shown = evidence.join(format!("{phase}-login-screen.png"));
        let mut commands = vec![qmp_screendump_command(&shown)?];
        commands.extend(events);
        let commands: Vec<&str> = commands.iter().map(String::as_str).collect();
        let outcome = run_qemu_gui_with_events_and_screenshot(
            &login_config(
                &host,
                [&image_path, &disk, &vars, &log],
                "Nagi login acceptance PASS",
            ),
            "Nagi M10 desktop READY",
            &commands,
            &screenshot,
        )?;
        let serial = read(&log)?;
        let mut position = 0;
        for marker in markers {
            let Some(found) = serial[position..].find(marker) else {
                return Err(format!(
                    "{phase} did not print ordered marker `{marker}` (QEMU exit {}; log {})",
                    outcome.exit_status,
                    log.display()
                ));
            };
            position += found + marker.len();
        }
        for marker in absent {
            if serial.contains(marker) {
                return Err(format!(
                    "{phase} unexpectedly printed `{marker}` (log {})",
                    log.display()
                ));
            }
        }
        validate_screenshot(&shown)?;
        lines.push(format!(
            "PASS login: {phase} (log {}; screenshots {}, {})",
            log.display(),
            shown.display(),
            screenshot.display()
        ));
    }
    Ok(lines)
}

fn execute_desktop_acceptance(
    root: &Path,
    probe: &dyn HostProbe,
    acceptance: DesktopAcceptanceConfig,
    events: &[&str],
) -> CommandResult {
    let screenshot_run_id = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_nanos().to_string(),
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("{}: system clock: {error}", acceptance.label),
            );
        }
    };
    let image_name = scoped_artifact_name(
        acceptance.image_name,
        &screenshot_run_id,
        acceptance.unique_run_artifacts,
    );
    let persistent_disk_name = scoped_artifact_name(
        acceptance.persistent_disk_name,
        &screenshot_run_id,
        acceptance.unique_run_artifacts,
    );
    let vars_name = scoped_artifact_name(
        acceptance.vars_name,
        &screenshot_run_id,
        acceptance.unique_run_artifacts,
    );
    let first_log_name = scoped_artifact_name(
        acceptance.first_log_name,
        &screenshot_run_id,
        acceptance.unique_run_artifacts,
    );
    let run_log_name = scoped_artifact_name(
        acceptance.run_log_name,
        &screenshot_run_id,
        acceptance.unique_run_artifacts,
    );
    let restart_log_name = acceptance.restart_log_name.map(|name| {
        scoped_artifact_name(name, &screenshot_run_id, acceptance.unique_run_artifacts)
    });
    let image_result = if acceptance.acceptance_packages {
        execute_image_with_acceptance_packages(root, acceptance.features, &image_name)
    } else {
        execute_image_with_features(root, Some(acceptance.features), &image_name)
    };
    if image_result.exit_code != EXIT_SUCCESS {
        return image_result;
    }
    let host = match resolve_qemu_host(root, probe, acceptance.label) {
        Ok(host) => host,
        Err(error) => return failure(EXIT_CONFIG_ERROR, error),
    };
    let artifacts = match ensure_owned_directory(root, Path::new("out").join("artifacts")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("{}: {error}", acceptance.label)),
    };
    let logs = match ensure_owned_directory(root, Path::new("out").join("logs")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("{}: {error}", acceptance.label)),
    };
    let screenshot_directory = match ensure_owned_directory(
        root,
        Path::new("out").join("evidence").join(format!(
            "{}-{screenshot_run_id}",
            acceptance.evidence_prefix
        )),
    ) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("{}: {error}", acceptance.label)),
    };
    let screenshot_path = screenshot_directory.join(acceptance.screenshot_name);
    let ready_screenshot_path = acceptance
        .ready_screenshot_name
        .map(|name| screenshot_directory.join(name));
    let mut event_commands: Vec<String> = events.iter().map(|event| (*event).to_owned()).collect();
    let mut stage_commands: Vec<(&'static str, Vec<String>)> = acceptance.later_stages.clone();
    if let Some(path) = &ready_screenshot_path {
        let command = match qmp_screendump_command(path) {
            Ok(command) => command,
            Err(error) => {
                return failure(EXIT_CONFIG_ERROR, format!("{}: {error}", acceptance.label))
            }
        };
        match acceptance.ready_screenshot_stage.and_then(|marker| {
            stage_commands
                .iter_mut()
                .find(|(stage, _)| *stage == marker)
        }) {
            Some((_, events)) => events.insert(0, command),
            None => event_commands.insert(0, command),
        }
    }
    let event_commands: Vec<&str> = event_commands.iter().map(String::as_str).collect();
    let stage_events: Vec<Vec<&str>> = stage_commands
        .iter()
        .map(|(_, events)| events.iter().map(String::as_str).collect())
        .collect();
    let stages: Vec<QmpEventStage<'_>> = stage_commands
        .iter()
        .zip(&stage_events)
        .map(|((marker, _), events)| QmpEventStage { marker, events })
        .collect();
    let image_path = artifacts.join(image_name);
    let persistent_disk = artifacts.join(persistent_disk_name);
    let vars_copy = artifacts.join(vars_name);
    let first_log = logs.join(first_log_name);
    let desktop_log = logs.join(run_log_name);
    let had_persistent_disk = match ensure_persistent_disk(&persistent_disk) {
        Ok(existing) => existing,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("{}: {error}", acceptance.label)),
    };
    let timeout = Duration::from_secs(45);
    if !had_persistent_disk {
        let first_config = QemuConfig {
            qemu: &host.qemu,
            ovmf_code: &host.ovmf_code,
            ovmf_vars_template: &host.ovmf_vars,
            disk_image: &image_path,
            persistent_disk: &persistent_disk,
            vars_copy: &vars_copy,
            serial_log: &first_log,
            acceptance_marker: NAGI_WRITE_MARKER,
            timeout,
        };
        if let Err(error) = run_qemu(&first_config) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("{}: first boot: {error}", acceptance.label),
            );
        }
        let first_serial = match fs::read_to_string(&first_log) {
            Ok(serial) => serial,
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!(
                        "{}: cannot read {}: {error}",
                        acceptance.label,
                        first_log.display()
                    ),
                );
            }
        };
        if !first_serial.contains(NAGI_WRITE_MARKER) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "{}: first boot did not print `{NAGI_WRITE_MARKER}`",
                    acceptance.label
                ),
            );
        }
    }
    let config = QemuConfig {
        qemu: &host.qemu,
        ovmf_code: &host.ovmf_code,
        ovmf_vars_template: &host.ovmf_vars,
        disk_image: &image_path,
        persistent_disk: &persistent_disk,
        vars_copy: &vars_copy,
        serial_log: &desktop_log,
        acceptance_marker: acceptance.acceptance_marker,
        timeout,
    };
    let outcome = match run_qemu_gui_with_staged_events_and_screenshot(
        &config,
        "Nagi M10 desktop READY",
        &event_commands,
        &stages,
        &screenshot_path,
    ) {
        Ok(status) => status,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("{}: {error}", acceptance.label)),
    };
    let serial = match fs::read_to_string(&desktop_log) {
        Ok(serial) => serial,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "{}: cannot read {}: {error}",
                    acceptance.label,
                    desktop_log.display()
                ),
            );
        }
    };
    let mut last_marker_end = 0;
    for marker in acceptance.required_markers {
        let Some(relative) = serial[last_marker_end..].find(marker) else {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "{}: guest did not print ordered marker `{marker}` (QEMU exit {}; log {})",
                    acceptance.label,
                    outcome.exit_status,
                    desktop_log.display()
                ),
            );
        };
        last_marker_end += relative + marker.len();
    }
    if !outcome.acceptance_reached {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "{}: guest markers are present but QEMU did not reach its acceptance marker (exit {}; log {})",
                acceptance.label,
                outcome.exit_status,
                desktop_log.display()
            ),
        );
    }
    if let Some(path) = &ready_screenshot_path {
        if let Err(error) = validate_screenshot(path) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("{}: first-frame screenshot: {error}", acceptance.label),
            );
        }
    }
    let Some(ready_after) = outcome.ready_after else {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "{}: QEMU acceptance completed without observing the READY marker (log {})",
                acceptance.label,
                desktop_log.display()
            ),
        );
    };
    let restart_log_path = match (acceptance.restart_marker, restart_log_name) {
        (Some(restart_marker), Some(restart_log_name)) => {
            let restart_log = logs.join(restart_log_name);
            let restart_config = QemuConfig {
                qemu: &host.qemu,
                ovmf_code: &host.ovmf_code,
                ovmf_vars_template: &host.ovmf_vars,
                disk_image: &image_path,
                persistent_disk: &persistent_disk,
                vars_copy: &vars_copy,
                serial_log: &restart_log,
                acceptance_marker: restart_marker,
                timeout,
            };
            let restart_commands: Vec<&str> = acceptance
                .restart_events
                .iter()
                .map(String::as_str)
                .collect();
            let restarted = if restart_commands.is_empty() {
                run_qemu(&restart_config).map(|_| ())
            } else {
                run_qemu_gui_with_events(
                    &restart_config,
                    "Nagi M10 desktop READY",
                    &restart_commands,
                )
                .map(|_| ())
            };
            if let Err(error) = restarted {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!("{}: persistence restart: {error}", acceptance.label),
                );
            }
            let restart_serial = match fs::read_to_string(&restart_log) {
                Ok(serial) => serial,
                Err(error) => {
                    return failure(
                        EXIT_CONFIG_ERROR,
                        format!(
                            "{}: cannot read {}: {error}",
                            acceptance.label,
                            restart_log.display()
                        ),
                    );
                }
            };
            let mut marker_end = 0;
            for marker in ["Nagi M10 desktop READY", restart_marker] {
                let Some(relative) = restart_serial[marker_end..].find(marker) else {
                    return failure(
                        EXIT_CONFIG_ERROR,
                        format!(
                            "{}: persistence restart did not print ordered marker `{marker}` (log {})",
                            acceptance.label,
                            restart_log.display()
                        ),
                    );
                };
                marker_end += relative + marker.len();
            }
            Some(restart_log)
        }
        (None, None) => None,
        _ => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "{}: incomplete persistence restart configuration",
                    acceptance.label
                ),
            );
        }
    };
    let mut lines = vec![format!(
        "PASS {}: QEMU guest rendered and interacted with the Nagi desktop (exit {}; guest READY after {} ms; log {}; screenshot {})",
        acceptance.label,
        outcome.exit_status,
        ready_after.as_millis(),
        desktop_log.display(),
        screenshot_path.display(),
    )];
    if let Some(path) = &ready_screenshot_path {
        lines.push(format!(
            "PASS {}: screen captured before the acceptance input (screenshot {})",
            acceptance.label,
            path.display()
        ));
    }
    if let Some(restart_log) = restart_log_path {
        lines.push(format!(
            "PASS {}: {} after a QEMU restart (log {})",
            acceptance.label,
            acceptance.restart_summary,
            restart_log.display(),
        ));
    }
    CommandResult {
        exit_code: EXIT_SUCCESS,
        lines,
    }
}

fn scoped_artifact_name(name: &str, run_id: &str, unique_run: bool) -> String {
    if !unique_run {
        return name.to_owned();
    }
    let path = Path::new(name);
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or(name);
    match path.extension().and_then(|extension| extension.to_str()) {
        Some(extension) => format!("{stem}-{run_id}.{extension}"),
        None => format!("{name}-{run_id}"),
    }
}

fn execute_security(root: &Path, probe: &dyn HostProbe) -> CommandResult {
    let image_result =
        execute_image_with_features(root, Some("m11-security"), "nagi-0.1-m11-security.img");
    if image_result.exit_code != EXIT_SUCCESS {
        return image_result;
    }
    let host = match resolve_qemu_host(root, probe, "security") {
        Ok(host) => host,
        Err(error) => return failure(EXIT_CONFIG_ERROR, error),
    };
    let artifacts = match ensure_owned_directory(root, Path::new("out").join("artifacts")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("security: {error}")),
    };
    let logs = match ensure_owned_directory(root, Path::new("out").join("logs")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("security: {error}")),
    };
    let image_path = artifacts.join("nagi-0.1-m11-security.img");
    let persistent_disk = artifacts.join("nagi-0.1-user-data.img");
    let vars_copy = artifacts.join("nagi-0.1-m11-security-vars.fd");
    let first_log = logs.join("m11-first-boot.log");
    let security_log = logs.join("m11-security.log");
    let had_persistent_disk = match ensure_persistent_disk(&persistent_disk) {
        Ok(existing) => existing,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("security: {error}")),
    };
    let timeout = Duration::from_secs(45);
    if !had_persistent_disk {
        let first_config = QemuConfig {
            qemu: &host.qemu,
            ovmf_code: &host.ovmf_code,
            ovmf_vars_template: &host.ovmf_vars,
            disk_image: &image_path,
            persistent_disk: &persistent_disk,
            vars_copy: &vars_copy,
            serial_log: &first_log,
            acceptance_marker: NAGI_WRITE_MARKER,
            timeout,
        };
        if let Err(error) = run_qemu(&first_config) {
            return failure(EXIT_CONFIG_ERROR, format!("security: first boot: {error}"));
        }
        let first_serial = match fs::read_to_string(&first_log) {
            Ok(serial) => serial,
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!("security: cannot read {}: {error}", first_log.display()),
                );
            }
        };
        if !first_serial.contains(NAGI_WRITE_MARKER) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("security: first boot did not print `{NAGI_WRITE_MARKER}`"),
            );
        }
    }
    let config = QemuConfig {
        qemu: &host.qemu,
        ovmf_code: &host.ovmf_code,
        ovmf_vars_template: &host.ovmf_vars,
        disk_image: &image_path,
        persistent_disk: &persistent_disk,
        vars_copy: &vars_copy,
        serial_log: &security_log,
        acceptance_marker: "Nagi M11 acceptance PASS",
        timeout,
    };
    let status = match run_qemu(&config) {
        Ok(status) => status,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("security: {error}")),
    };
    let serial = match fs::read_to_string(&security_log) {
        Ok(serial) => serial,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("security: cannot read {}: {error}", security_log.display()),
            );
        }
    };
    for marker in [
        "Nagi Kernel started",
        "Nagi M2 acceptance PASS",
        "Nagi M3 acceptance PASS",
        "Nagi M4 acceptance PASS",
        "Nagi M7 VirtIO Block PASS",
        "Nagi M5 user process START",
        "Nagi M6 echo@1 call PASS",
        "Nagi M7 ext2 mount PASS",
        "Nagi M7 persistent read PASS",
        "Nagi M5 syscall PASS",
        "Nagi M6 acceptance PASS",
        "Nagi M7 acceptance PASS",
        "Nagi M11 local login PASS",
        "Nagi M11 lock screen PASS",
        "Nagi M11 Developer Mode PASS",
        "Nagi M11 trusted dialog ASK PASS",
        "Nagi M11 malicious file DENIED",
        "Nagi M11 malicious microphone DENIED",
        "Nagi M11 acceptance PASS",
    ] {
        if !serial.contains(marker) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "security: guest did not print `{marker}` (QEMU exit {status}; log {})",
                    security_log.display()
                ),
            );
        }
    }
    CommandResult {
        exit_code: EXIT_SUCCESS,
        lines: vec![format!(
            "PASS security: QEMU guest enforced login, lock screen, trusted dialog, and malicious app denial (exit {status}; log {})",
            security_log.display()
        )],
    }
}

fn execute_network(root: &Path, probe: &dyn HostProbe) -> CommandResult {
    let image_result =
        execute_image_with_features(root, Some("m12-network"), "nagi-0.1-m12-network.img");
    if image_result.exit_code != EXIT_SUCCESS {
        return image_result;
    }
    let host = match resolve_qemu_host(root, probe, "network") {
        Ok(host) => host,
        Err(error) => return failure(EXIT_CONFIG_ERROR, error),
    };
    let artifacts = match ensure_owned_directory(root, Path::new("out").join("artifacts")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("network: {error}")),
    };
    let logs = match ensure_owned_directory(root, Path::new("out").join("logs")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("network: {error}")),
    };
    let image_path = artifacts.join("nagi-0.1-m12-network.img");
    let persistent_disk = artifacts.join("nagi-0.1-user-data.img");
    let vars_copy = artifacts.join("nagi-0.1-m12-network-vars.fd");
    let first_log = logs.join("m12-first-boot.log");
    let network_log = logs.join("m12-network.log");
    let had_persistent_disk = match ensure_persistent_disk(&persistent_disk) {
        Ok(existing) => existing,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("network: {error}")),
    };
    let timeout = Duration::from_secs(45);
    if !had_persistent_disk {
        let first_config = QemuConfig {
            qemu: &host.qemu,
            ovmf_code: &host.ovmf_code,
            ovmf_vars_template: &host.ovmf_vars,
            disk_image: &image_path,
            persistent_disk: &persistent_disk,
            vars_copy: &vars_copy,
            serial_log: &first_log,
            acceptance_marker: NAGI_WRITE_MARKER,
            timeout,
        };
        if let Err(error) = run_qemu(&first_config) {
            return failure(EXIT_CONFIG_ERROR, format!("network: first boot: {error}"));
        }
        let first_serial = match fs::read_to_string(&first_log) {
            Ok(serial) => serial,
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!("network: cannot read {}: {error}", first_log.display()),
                );
            }
        };
        if !first_serial.contains(NAGI_WRITE_MARKER) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("network: first boot did not print `{NAGI_WRITE_MARKER}`"),
            );
        }
    }
    let config = QemuConfig {
        qemu: &host.qemu,
        ovmf_code: &host.ovmf_code,
        ovmf_vars_template: &host.ovmf_vars,
        disk_image: &image_path,
        persistent_disk: &persistent_disk,
        vars_copy: &vars_copy,
        serial_log: &network_log,
        acceptance_marker: "Nagi M12 acceptance PASS",
        timeout,
    };
    let status = match run_qemu(&config) {
        Ok(status) => status,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("network: {error}")),
    };
    let serial = match fs::read_to_string(&network_log) {
        Ok(serial) => serial,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("network: cannot read {}: {error}", network_log.display()),
            );
        }
    };
    for marker in [
        "Nagi Kernel started",
        "Nagi M2 acceptance PASS",
        "Nagi M3 acceptance PASS",
        "Nagi M4 acceptance PASS",
        "Nagi M7 VirtIO Block PASS",
        "Nagi M12 VirtIO Net PASS",
        "Nagi M5 user process START",
        "Nagi M6 echo@1 call PASS",
        "Nagi M7 ext2 mount PASS",
        "Nagi M7 persistent read PASS",
        "Nagi M5 syscall PASS",
        "Nagi M6 acceptance PASS",
        "Nagi M7 acceptance PASS",
        "Nagi M12 network READY",
        "Nagi M12 DHCP PASS",
        "Nagi M12 ICMP PASS",
        "Nagi M12 UDP/DNS PASS",
        "Nagi M12 ARP PASS",
        "Nagi M12 TCP handshake PASS",
        "Nagi M12 HTTP response PASS",
        "Nagi M12 acceptance PASS",
    ] {
        if !serial.contains(marker) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "network: guest did not print `{marker}` (QEMU exit {status}; log {})",
                    network_log.display()
                ),
            );
        }
    }
    CommandResult {
        exit_code: EXIT_SUCCESS,
        lines: vec![format!(
            "PASS network: QEMU guest performed DHCP, ICMP, UDP/DNS, ARP, TCP, and HTTP through nagi-net (exit {status}; log {})",
            network_log.display()
        )],
    }
}

fn execute_posix(root: &Path, probe: &dyn HostProbe) -> CommandResult {
    let image_result =
        execute_image_with_features(root, Some("m13-posix"), "nagi-0.1-m13-posix.img");
    if image_result.exit_code != EXIT_SUCCESS {
        return image_result;
    }
    let host = match resolve_qemu_host(root, probe, "posix") {
        Ok(host) => host,
        Err(error) => return failure(EXIT_CONFIG_ERROR, error),
    };
    let artifacts = match ensure_owned_directory(root, Path::new("out").join("artifacts")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("posix: {error}")),
    };
    let logs = match ensure_owned_directory(root, Path::new("out").join("logs")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("posix: {error}")),
    };
    let image_path = artifacts.join("nagi-0.1-m13-posix.img");
    let persistent_disk = artifacts.join("nagi-0.1-user-data.img");
    let vars_copy = artifacts.join("nagi-0.1-m13-posix-vars.fd");
    let first_log = logs.join("m13-first-boot.log");
    let posix_log = logs.join("m13-posix.log");
    let had_persistent_disk = match ensure_persistent_disk(&persistent_disk) {
        Ok(existing) => existing,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("posix: {error}")),
    };
    let timeout = Duration::from_secs(45);
    if !had_persistent_disk {
        let first_config = QemuConfig {
            qemu: &host.qemu,
            ovmf_code: &host.ovmf_code,
            ovmf_vars_template: &host.ovmf_vars,
            disk_image: &image_path,
            persistent_disk: &persistent_disk,
            vars_copy: &vars_copy,
            serial_log: &first_log,
            acceptance_marker: NAGI_WRITE_MARKER,
            timeout,
        };
        if let Err(error) = run_qemu(&first_config) {
            return failure(EXIT_CONFIG_ERROR, format!("posix: first boot: {error}"));
        }
        let first_serial = match fs::read_to_string(&first_log) {
            Ok(serial) => serial,
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!("posix: cannot read {}: {error}", first_log.display()),
                );
            }
        };
        if !first_serial.contains(NAGI_WRITE_MARKER) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("posix: first boot did not print `{NAGI_WRITE_MARKER}`"),
            );
        }
    }
    let config = QemuConfig {
        qemu: &host.qemu,
        ovmf_code: &host.ovmf_code,
        ovmf_vars_template: &host.ovmf_vars,
        disk_image: &image_path,
        persistent_disk: &persistent_disk,
        vars_copy: &vars_copy,
        serial_log: &posix_log,
        acceptance_marker: "Nagi M13 acceptance PASS",
        timeout,
    };
    let status = match run_qemu(&config) {
        Ok(status) => status,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("posix: {error}")),
    };
    let serial = match fs::read_to_string(&posix_log) {
        Ok(serial) => serial,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("posix: cannot read {}: {error}", posix_log.display()),
            );
        }
    };
    for marker in [
        "Nagi Kernel started",
        "Nagi M2 acceptance PASS",
        "Nagi M3 acceptance PASS",
        "Nagi M4 acceptance PASS",
        "Nagi M7 VirtIO Block PASS",
        "Nagi M12 VirtIO Net PASS",
        "Nagi M5 user process START",
        "Nagi M6 echo@1 call PASS",
        "Nagi M7 ext2 mount PASS",
        "Nagi M7 persistent read PASS",
        "Nagi M5 syscall PASS",
        "Nagi M6 acceptance PASS",
        "Nagi M7 acceptance PASS",
        "Nagi M13 Rust PAL PASS",
        "Nagi M13 C POSIX PASS",
        "Nagi M13 socket/DNS PASS",
        "Nagi M13 mmap PASS",
        "Nagi M13 time/sleep PASS",
        "Nagi M13 poll PASS",
        "Nagi M13 thread/TLS PASS",
        "Nagi M13 native spawn PASS",
        "Nagi M13 OSS library PASS",
        "Nagi M13 acceptance PASS",
    ] {
        if !serial.contains(marker) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "posix: guest did not print `{marker}` (QEMU exit {status}; log {})",
                    posix_log.display()
                ),
            );
        }
    }
    CommandResult {
        exit_code: EXIT_SUCCESS,
        lines: vec![format!(
            "PASS posix: guest exercised Nagi PAL, C POSIX ABI, and itoa through real VFS/network paths (exit {status}; log {})",
            posix_log.display()
        )],
    }
}

fn execute_std(root: &Path, probe: &dyn HostProbe) -> CommandResult {
    let image_result = execute_image_with_std(root, "nagi-0.1-m13-std.img");
    if image_result.exit_code != EXIT_SUCCESS {
        return image_result;
    }
    let host = match resolve_qemu_host(root, probe, "std") {
        Ok(host) => host,
        Err(error) => return failure(EXIT_CONFIG_ERROR, error),
    };
    let artifacts = match ensure_owned_directory(root, Path::new("out").join("artifacts")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("std: {error}")),
    };
    let logs = match ensure_owned_directory(root, Path::new("out").join("logs")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("std: {error}")),
    };
    let image_path = artifacts.join("nagi-0.1-m13-std.img");
    let persistent_disk = artifacts.join("nagi-0.1-user-data.img");
    let vars_copy = artifacts.join("nagi-0.1-m13-std-vars.fd");
    let first_log = logs.join("m13-std-first-boot.log");
    let std_log = logs.join("m13-std.log");
    let had_persistent_disk = match ensure_persistent_disk(&persistent_disk) {
        Ok(existing) => existing,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("std: {error}")),
    };
    let timeout = Duration::from_secs(45);
    if !had_persistent_disk {
        let first_config = QemuConfig {
            qemu: &host.qemu,
            ovmf_code: &host.ovmf_code,
            ovmf_vars_template: &host.ovmf_vars,
            disk_image: &image_path,
            persistent_disk: &persistent_disk,
            vars_copy: &vars_copy,
            serial_log: &first_log,
            acceptance_marker: "Nagi M13 Rust std PASS",
            timeout,
        };
        if let Err(error) = run_qemu(&first_config) {
            return failure(EXIT_CONFIG_ERROR, format!("std: first boot: {error}"));
        }
        let first_serial = match fs::read_to_string(&first_log) {
            Ok(serial) => serial,
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!("std: cannot read {}: {error}", first_log.display()),
                );
            }
        };
        if !first_serial.contains("Nagi M13 Rust std PASS") {
            return failure(
                EXIT_CONFIG_ERROR,
                "std: first boot did not print `Nagi M13 Rust std PASS`".to_owned(),
            );
        }
    }
    let config = QemuConfig {
        qemu: &host.qemu,
        ovmf_code: &host.ovmf_code,
        ovmf_vars_template: &host.ovmf_vars,
        disk_image: &image_path,
        persistent_disk: &persistent_disk,
        vars_copy: &vars_copy,
        serial_log: &std_log,
        acceptance_marker: "Nagi M13 Rust std PASS",
        timeout,
    };
    let status = match run_qemu(&config) {
        Ok(status) => status,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("std: {error}")),
    };
    let serial = match fs::read_to_string(&std_log) {
        Ok(serial) => serial,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("std: cannot read {}: {error}", std_log.display()),
            );
        }
    };
    for marker in [
        "Nagi Kernel started",
        "Nagi M2 acceptance PASS",
        "Nagi M3 acceptance PASS",
        "Nagi M4 acceptance PASS",
        "Nagi M7 VirtIO Block PASS",
        "Nagi M12 VirtIO Net PASS",
        "Nagi M9 display setup PASS",
        "Nagi M9 input setup PASS",
        "Nagi M5 user process START",
        "Nagi M13 Rust std relibc PASS",
        "Nagi M13 Rust std allocator PASS",
        "Nagi M13 Rust std clock PASS",
        "Nagi M13 Rust std network PASS",
        "Nagi M13 Rust std thread/TLS PASS",
        "Nagi M13 Rust std sync PASS",
        "Nagi M13 Rust std VFS PASS",
        "Nagi M13 Rust std PASS",
    ] {
        if !serial.contains(marker) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "std: guest did not print `{marker}` (QEMU exit {status}; log {})",
                    std_log.display()
                ),
            );
        }
    }
    CommandResult {
        exit_code: EXIT_SUCCESS,
        lines: vec![format!(
            "PASS std: real Nagi Rust std, relibc, guest timer, synchronization, allocator, and VFS tests passed (exit {status}; log {})",
            std_log.display()
        )],
    }
}

fn execute_m13(root: &Path, probe: &dyn HostProbe) -> CommandResult {
    let mut fixture = match start_m13_http_fixture(root) {
        Ok(child) => child,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m13: {error}")),
    };
    let result = (|| {
        let posix = execute_posix(root, probe);
        if posix.exit_code != EXIT_SUCCESS {
            return posix;
        }
        let std = execute_std(root, probe);
        if std.exit_code != EXIT_SUCCESS {
            return std;
        }
        let mut lines = posix.lines;
        lines.extend(std.lines);
        lines.push(
            "PASS M13 unified acceptance: real QEMU POSIX/relibc and Rust std gates passed"
                .to_owned(),
        );
        CommandResult {
            exit_code: EXIT_SUCCESS,
            lines,
        }
    })();
    let _ = fixture.kill();
    let _ = fixture.wait();
    result
}

fn execute_m14(root: &Path, probe: &dyn HostProbe) -> CommandResult {
    let mut fixture = match start_m13_http_fixture(root) {
        Ok(child) => child,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m14: {error}")),
    };
    let result = execute_m14_inner(root, probe);
    let _ = fixture.kill();
    let _ = fixture.wait();
    result
}

fn execute_m14_inner(root: &Path, probe: &dyn HostProbe) -> CommandResult {
    let image_result =
        execute_image_with_features(root, Some("m14-audio"), "nagi-0.1-m14-audio.img");
    if image_result.exit_code != EXIT_SUCCESS {
        return image_result;
    }
    let host = match resolve_qemu_host(root, probe, "m14") {
        Ok(host) => host,
        Err(error) => return failure(EXIT_CONFIG_ERROR, error),
    };
    let artifacts = match ensure_owned_directory(root, Path::new("out").join("artifacts")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m14: {error}")),
    };
    let logs = match ensure_owned_directory(root, Path::new("out").join("logs")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m14: {error}")),
    };
    let image_path = artifacts.join("nagi-0.1-m14-audio.img");
    let persistent_disk = artifacts.join("nagi-0.1-m14-audio-user-data.img");
    let vars_copy = artifacts.join("nagi-0.1-m14-audio-vars.fd");
    let first_log = logs.join("m14-audio-first-boot.log");
    let audio_log = logs.join("m14-audio.log");
    let had_persistent_disk = match ensure_persistent_disk(&persistent_disk) {
        Ok(existing) => existing,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m14: {error}")),
    };
    let timeout = Duration::from_secs(60);
    if !had_persistent_disk {
        let first_config = QemuConfig {
            qemu: &host.qemu,
            ovmf_code: &host.ovmf_code,
            ovmf_vars_template: &host.ovmf_vars,
            disk_image: &image_path,
            persistent_disk: &persistent_disk,
            vars_copy: &vars_copy,
            serial_log: &first_log,
            acceptance_marker: NAGI_WRITE_MARKER,
            timeout,
        };
        if let Err(error) = run_qemu(&first_config) {
            return failure(EXIT_CONFIG_ERROR, format!("m14: first boot: {error}"));
        }
        let first_serial = match fs::read_to_string(&first_log) {
            Ok(serial) => serial,
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!("m14: cannot read {}: {error}", first_log.display()),
                );
            }
        };
        if !first_serial.contains(NAGI_WRITE_MARKER) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m14: first boot did not print `{NAGI_WRITE_MARKER}`"),
            );
        }
    }
    let config = QemuConfig {
        qemu: &host.qemu,
        ovmf_code: &host.ovmf_code,
        ovmf_vars_template: &host.ovmf_vars,
        disk_image: &image_path,
        persistent_disk: &persistent_disk,
        vars_copy: &vars_copy,
        serial_log: &audio_log,
        acceptance_marker: "Nagi M14 audio service PASS",
        timeout,
    };
    let status = match run_qemu(&config) {
        Ok(status) => status,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m14: {error}")),
    };
    let serial = match fs::read_to_string(&audio_log) {
        Ok(serial) => serial,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m14: cannot read {}: {error}", audio_log.display()),
            );
        }
    };
    for marker in [
        "Nagi Kernel started",
        "Nagi M2 acceptance PASS",
        "Nagi M3 acceptance PASS",
        "Nagi M4 acceptance PASS",
        "Nagi M7 VirtIO Block PASS",
        "Nagi M12 VirtIO Net PASS",
        "Nagi M14 VirtIO Sound PASS",
        "Nagi M9 display setup PASS",
        "Nagi M9 input setup PASS",
        "Nagi M5 user process START",
        "Nagi M13 Rust PAL PASS",
        "Nagi M13 C POSIX PASS",
        "Nagi M13 OSS library PASS",
        "Nagi M14 playback PASS",
        "Nagi M14 capture PASS",
        "Nagi M14 capture signal PASS",
        "Nagi M14 capability denial PASS",
        "Nagi M14 mixer PASS",
        "Nagi M14 volume/mute PASS",
        "Nagi M14 sessions PASS",
        "Nagi M14 audio service PASS",
    ] {
        if !serial.contains(marker) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m14: guest did not print `{marker}` (QEMU exit {status}; log {})",
                    audio_log.display()
                ),
            );
        }
    }
    CommandResult {
        exit_code: EXIT_SUCCESS,
        lines: vec![format!(
            "PASS M14 audio: real VirtIO Sound playback, capture, mixer, volume/mute, and session gates passed (exit {status}; log {})",
            audio_log.display()
        )],
    }
}

fn execute_m15(root: &Path, probe: &dyn HostProbe) -> CommandResult {
    let mut fixture = match start_m13_http_fixture(root) {
        Ok(child) => child,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m15: {error}")),
    };
    let result = execute_m15_inner(root, probe);
    let _ = fixture.kill();
    let _ = fixture.wait();
    result
}

fn execute_m25(root: &Path, probe: &dyn HostProbe) -> CommandResult {
    let run_id = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_nanos().to_string(),
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m25: system clock: {error}")),
    };
    let host = match resolve_qemu_host(root, probe, "m25") {
        Ok(host) => host,
        Err(error) => return failure(EXIT_CONFIG_ERROR, error),
    };
    let artifacts = match ensure_owned_directory(root, Path::new("out").join("artifacts")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m25: {error}")),
    };
    let logs = match ensure_owned_directory(root, Path::new("out").join("logs")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m25: {error}")),
    };
    let image_name = format!("nagi-0.1-m25-voice-{run_id}.img");
    let image_path = artifacts.join(&image_name);
    let persistent_disk = artifacts.join(format!("nagi-0.1-m25-voice-user-data-{run_id}.img"));
    let vars_copy = artifacts.join(format!("nagi-0.1-m25-voice-vars-{run_id}.fd"));
    let bootstrap_log = logs.join(format!("m25-voice-bootstrap-{run_id}.log"));
    let voice_log = logs.join(format!("m25-voice-{run_id}.log"));
    for path in [
        &image_path,
        &persistent_disk,
        &vars_copy,
        &bootstrap_log,
        &voice_log,
    ] {
        match fs::symlink_metadata(path) {
            Ok(_) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!(
                        "m25: refusing to overwrite existing run artifact {}",
                        path.display()
                    ),
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!("m25: cannot inspect {}: {error}", path.display()),
                );
            }
        }
    }
    let image_result = execute_image_with_features(root, Some("m25-voice-acceptance"), &image_name);
    if image_result.exit_code != EXIT_SUCCESS {
        return image_result;
    }
    let had_persistent_disk = match ensure_persistent_disk(&persistent_disk) {
        Ok(existing) => existing,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m25: {error}")),
    };
    if had_persistent_disk {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "m25: unique User Data path unexpectedly existed: {}",
                persistent_disk.display()
            ),
        );
    }
    if let Err(error) = initialize_ovmf_vars(&host.ovmf_vars, &vars_copy) {
        return failure(
            EXIT_CONFIG_ERROR,
            format!("m25: initialize OVMF variables: {error}"),
        );
    }
    let timeout = Duration::from_secs(90);
    if !had_persistent_disk {
        let bootstrap_config = QemuConfig {
            qemu: &host.qemu,
            ovmf_code: &host.ovmf_code,
            ovmf_vars_template: &host.ovmf_vars,
            disk_image: &image_path,
            persistent_disk: &persistent_disk,
            vars_copy: &vars_copy,
            serial_log: &bootstrap_log,
            acceptance_marker: NAGI_WRITE_MARKER,
            timeout,
        };
        let status = match run_qemu(&bootstrap_config) {
            Ok(status) => status,
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!("m25: storage bootstrap: {error}"),
                );
            }
        };
        let serial = match fs::read_to_string(&bootstrap_log) {
            Ok(serial) => serial,
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!("m25: cannot read {}: {error}", bootstrap_log.display()),
                );
            }
        };
        for marker in [NAGI_WRITE_MARKER, "Nagi M7 reboot required PASS"] {
            if !serial.contains(marker) {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!(
                        "m25: storage bootstrap did not print `{marker}` (QEMU exit {status}; log {})",
                        bootstrap_log.display()
                    ),
                );
            }
        }
    }

    let config = QemuConfig {
        qemu: &host.qemu,
        ovmf_code: &host.ovmf_code,
        ovmf_vars_template: &host.ovmf_vars,
        disk_image: &image_path,
        persistent_disk: &persistent_disk,
        vars_copy: &vars_copy,
        serial_log: &voice_log,
        acceptance_marker: "Nagi M25 voice orchestration PASS",
        timeout,
    };
    let status = match run_qemu(&config) {
        Ok(status) => status,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m25: QEMU: {error}")),
    };
    let serial = match fs::read_to_string(&voice_log) {
        Ok(serial) => serial,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m25: cannot read {}: {error}", voice_log.display()),
            );
        }
    };
    for marker in [
        "Nagi Kernel started",
        "Nagi M2 acceptance PASS",
        "Nagi M3 acceptance PASS",
        "Nagi M4 acceptance PASS",
        "Nagi M5 user process START",
        "Nagi M6 acceptance PASS",
        "Nagi M7 ext2 mount PASS",
        "Nagi M7 persistent read PASS",
        "Nagi M25 permission fail-closed PASS",
        "Nagi M25 indicator-before-provider PASS",
        "Nagi M25 bounded PCM forwarding PASS",
        "Nagi M25 unavailable cleanup PASS",
        "Nagi M25 empty transcript rejected PASS",
        "Nagi M25 fixture transcript delivery PASS",
        "Nagi M25 TTS provider contract PASS",
        "Nagi M25 voice orchestration PASS",
    ] {
        if !serial.contains(marker) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m25: guest did not print `{marker}` (QEMU exit {status}; log {})",
                    voice_log.display()
                ),
            );
        }
    }
    CommandResult {
        exit_code: EXIT_SUCCESS,
        lines: vec![format!(
            "PASS M25 guest voice orchestration: bounded fixture capture and TTS PCM, permission/indicator ordering, provider cleanup, empty-result rejection, and fixed Japanese fixture transcript handoff passed; no real audio device, STT model, or TTS engine was used (log {})",
            voice_log.display()
        )],
    }
}

fn execute_m15_inner(root: &Path, probe: &dyn HostProbe) -> CommandResult {
    let image_result =
        execute_image_with_features(root, Some("m15-history"), "nagi-0.1-m15-history.img");
    if image_result.exit_code != EXIT_SUCCESS {
        return image_result;
    }
    let host = match resolve_qemu_host(root, probe, "m15") {
        Ok(host) => host,
        Err(error) => return failure(EXIT_CONFIG_ERROR, error),
    };
    let artifacts = match ensure_owned_directory(root, Path::new("out").join("artifacts")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m15: {error}")),
    };
    let logs = match ensure_owned_directory(root, Path::new("out").join("logs")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m15: {error}")),
    };
    let image_path = artifacts.join("nagi-0.1-m15-history.img");
    let persistent_disk = artifacts.join("nagi-0.1-m15-history-user-data.img");
    let vars_copy = artifacts.join("nagi-0.1-m15-history-vars.fd");
    let first_log = logs.join("m15-history-first-boot.log");
    let history_log = logs.join("m15-history.log");
    let had_persistent_disk = match ensure_persistent_disk(&persistent_disk) {
        Ok(existing) => existing,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m15: {error}")),
    };
    let timeout = Duration::from_secs(75);
    if !had_persistent_disk {
        let first_config = QemuConfig {
            qemu: &host.qemu,
            ovmf_code: &host.ovmf_code,
            ovmf_vars_template: &host.ovmf_vars,
            disk_image: &image_path,
            persistent_disk: &persistent_disk,
            vars_copy: &vars_copy,
            serial_log: &first_log,
            acceptance_marker: NAGI_WRITE_MARKER,
            timeout,
        };
        if let Err(error) = run_qemu(&first_config) {
            return failure(EXIT_CONFIG_ERROR, format!("m15: first boot: {error}"));
        }
        let first_serial = match fs::read_to_string(&first_log) {
            Ok(serial) => serial,
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!("m15: cannot read {}: {error}", first_log.display()),
                );
            }
        };
        if !first_serial.contains(NAGI_WRITE_MARKER) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m15: first boot did not print `{NAGI_WRITE_MARKER}`"),
            );
        }
    }
    let config = QemuConfig {
        qemu: &host.qemu,
        ovmf_code: &host.ovmf_code,
        ovmf_vars_template: &host.ovmf_vars,
        disk_image: &image_path,
        persistent_disk: &persistent_disk,
        vars_copy: &vars_copy,
        serial_log: &history_log,
        acceptance_marker: "Nagi M15 acceptance PASS",
        timeout,
    };
    let status = match run_qemu(&config) {
        Ok(status) => status,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m15: {error}")),
    };
    let serial = match fs::read_to_string(&history_log) {
        Ok(serial) => serial,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m15: cannot read {}: {error}", history_log.display()),
            );
        }
    };
    for marker in [
        "Nagi Kernel started",
        "Nagi M2 acceptance PASS",
        "Nagi M3 acceptance PASS",
        "Nagi M4 acceptance PASS",
        "Nagi M7 VirtIO Block PASS",
        "Nagi M12 VirtIO Net PASS",
        "Nagi M14 VirtIO Sound PASS",
        "Nagi M5 user process START",
        "Nagi M13 Rust PAL PASS",
        "Nagi M13 C POSIX PASS",
        "Nagi M14 audio service PASS",
        "Nagi M15 create PASS",
        "Nagi M15 edit/version PASS",
        "Nagi M15 move PASS",
        "Nagi M15 delete/trash PASS",
        "Nagi M15 restore PASS",
        "Nagi M15 undo PASS",
        "Nagi M15 transaction ledger PASS",
        "Nagi M15 History Service PASS",
        "Nagi M15 acceptance PASS",
    ] {
        if !serial.contains(marker) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m15: guest did not print `{marker}` (QEMU exit {status}; log {})",
                    history_log.display()
                ),
            );
        }
    }
    CommandResult {
        exit_code: EXIT_SUCCESS,
        lines: vec![format!(
            "PASS M15 history: real guest create/edit/move/delete/restore/undo and persistent ledger passed (exit {status}; log {})",
            history_log.display()
        )],
    }
}

fn execute_m16(root: &Path, probe: &dyn HostProbe) -> CommandResult {
    let sample = execute_m16_sample_build(root);
    if sample.exit_code != EXIT_SUCCESS {
        return sample;
    }
    let mut fixture = match start_m13_http_fixture(root) {
        Ok(child) => child,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m16: {error}")),
    };
    let result = execute_m16_inner(root, probe);
    let _ = fixture.kill();
    let _ = fixture.wait();
    result
}

fn execute_m17(root: &Path, probe: &dyn HostProbe) -> CommandResult {
    let cxx_headers = match resolve_m17_cxx_headers() {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m17: C++ headers: {error}")),
    };

    let fetch = execute_fetch(root);
    if fetch.exit_code != EXIT_SUCCESS {
        return fetch;
    }
    let sample = execute_m16_sample_build(root);
    if sample.exit_code != EXIT_SUCCESS {
        return sample;
    }

    let rust_std_source = match prepare_nagi_rust_std_source(root) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m17: rust std: {error}")),
    };
    let mesa_build = ProcessCommand::new("bash")
        .args(["tools/mesa/build.sh"])
        .current_dir(root)
        .output();
    match mesa_build {
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m17: Mesa/Softpipe build failed: {}",
                    command_output(&output)
                ),
            );
        }
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m17: cannot start tools/mesa/build.sh through bash: {error}"),
            );
        }
    }

    let package_path = root.join("out").join("artifacts").join("hello-nagi.xapp");
    let mesa_build_path = root.join("out").join("m17-mesa").join("mesa-build");
    let font_cache_path = root.join("out").join("cache").join("fonts");
    let target_compiler_wrapper = root.join("tools").join("nagi-target-cc.sh");
    let init_args = [
        "build",
        "-p",
        "nagi-init",
        "--features",
        "m17-servo",
        "--target",
        "targets/x86_64-unknown-nagi-user.json",
        "-Zbuild-std=std,panic_abort",
        "--release",
        "--locked",
        "--offline",
    ];
    let mut init_build_env = vec![
        ("NAGI_M16_PACKAGE", package_path.as_path()),
        ("NAGI_MESA_BUILD", mesa_build_path.as_path()),
        ("NAGI_FONT_DIR", font_cache_path.as_path()),
        ("NAGI_CXX_HEADERS", cxx_headers.as_path()),
        // MozJS builds host-side configure helpers as well as Nagi
        // target objects; keep those host probes off the target wrapper.
        ("HOST_CC", Path::new("cc")),
        ("HOST_CXX", Path::new("c++")),
        (
            "CC_x86_64_unknown_nagi_user",
            target_compiler_wrapper.as_path(),
        ),
        (
            "CXX_x86_64_unknown_nagi_user",
            target_compiler_wrapper.as_path(),
        ),
    ];
    append_nagi_target_archive_tools(&mut init_build_env, std::env::consts::OS);
    let image_result = execute_image_with_init_build_env_using_writer(
        root,
        &init_args,
        Some(&rust_std_source),
        "nagi-0.1-m17-servo.img",
        &init_build_env,
        write_m17_fat12_image,
        ImageBuildFeatures::default(),
    );
    if image_result.exit_code != EXIT_SUCCESS {
        return image_result;
    }

    let host = match resolve_qemu_host(root, probe, "m17") {
        Ok(host) => host,
        Err(error) => return failure(EXIT_CONFIG_ERROR, error),
    };
    let artifacts = match ensure_owned_directory(root, Path::new("out").join("artifacts")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m17: {error}")),
    };
    let logs = match ensure_owned_directory(root, Path::new("out").join("logs")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m17: {error}")),
    };
    let image_path = artifacts.join("nagi-0.1-m17-servo.img");
    let persistent_disk = artifacts.join("nagi-0.1-m17-user-data.img");
    let vars_copy = artifacts.join("nagi-0.1-m17-vars.fd");
    let first_log = logs.join("m17-first-boot.log");
    let log_path = logs.join("m17-servo.log");
    let had_persistent_disk = match ensure_persistent_disk(&persistent_disk) {
        Ok(existing) => existing,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m17: {error}")),
    };
    let timeout = Duration::from_secs(120);
    if !had_persistent_disk {
        let first_config = QemuConfig {
            qemu: &host.qemu,
            ovmf_code: &host.ovmf_code,
            ovmf_vars_template: &host.ovmf_vars,
            disk_image: &image_path,
            persistent_disk: &persistent_disk,
            vars_copy: &vars_copy,
            serial_log: &first_log,
            acceptance_marker: NAGI_WRITE_MARKER,
            timeout,
        };
        if let Err(error) = run_qemu_with_read_only_boot_disk(&first_config) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m17: first boot: {error}\nserial log tail:\n{}",
                    serial_log_tail(&first_log, 64)
                ),
            );
        }
        let first_serial = match fs::read_to_string(&first_log) {
            Ok(serial) => serial,
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!("m17: cannot read {}: {error}", first_log.display()),
                );
            }
        };
        if !first_serial.contains(NAGI_WRITE_MARKER) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m17: first boot did not print `{NAGI_WRITE_MARKER}` (log {})",
                    first_log.display()
                ),
            );
        }
    }
    let config = QemuConfig {
        qemu: &host.qemu,
        ovmf_code: &host.ovmf_code,
        ovmf_vars_template: &host.ovmf_vars,
        disk_image: &image_path,
        persistent_disk: &persistent_disk,
        vars_copy: &vars_copy,
        serial_log: &log_path,
        acceptance_marker: "Nagi M17 first web pixel PASS",
        timeout,
    };
    let status = match run_qemu_with_read_only_boot_disk(&config) {
        Ok(status) => status,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m17: QEMU: {error}\nM17 trace excerpt:\n{}\nserial log tail:\n{}",
                    serial_log_m17_trace_excerpt(&log_path, 256),
                    serial_log_tail(&log_path, 64),
                ),
            );
        }
    };
    let serial = match fs::read_to_string(&log_path) {
        Ok(serial) => serial,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m17: cannot read {}: {error}", log_path.display()),
            );
        }
    };
    for marker in [
        "Nagi Kernel started",
        "Nagi M17 first web pixel checksum=0x",
        "Nagi M17 first web pixel PASS",
    ] {
        if !serial.contains(marker) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m17: guest did not print `{marker}` (QEMU exit {status}; log {})",
                    log_path.display()
                ),
            );
        }
    }
    CommandResult {
        exit_code: EXIT_SUCCESS,
        lines: vec![format!(
            "PASS M17 first web pixel: real Servo/Mesa Softpipe frame reached Nagi Surface and QEMU (exit {status}; log {})",
            log_path.display()
        )],
    }
}

fn serial_log_tail(path: &Path, max_lines: usize) -> String {
    let serial = match fs::read_to_string(path) {
        Ok(serial) => serial,
        Err(error) => return format!("(could not read {}: {error})", path.display()),
    };
    last_serial_lines(&serial, max_lines)
}

fn serial_log_m17_trace_excerpt(path: &Path, max_lines: usize) -> String {
    let serial = match fs::read_to_string(path) {
        Ok(serial) => serial,
        Err(error) => return format!("(could not read {}: {error})", path.display()),
    };
    m17_trace_excerpt(&serial, max_lines)
}

fn m17_trace_excerpt(serial: &str, max_lines: usize) -> String {
    let trace_lines = serial
        .lines()
        .filter(|line| line.contains("Nagi M17 trace:"))
        .collect::<Vec<_>>();
    if trace_lines.is_empty() {
        return "(no Nagi M17 trace markers)".into();
    }
    if trace_lines.len() <= max_lines {
        return trace_lines.join("\n");
    }

    let head_count = max_lines.div_ceil(2);
    let tail_count = max_lines - head_count;
    let omitted_count = trace_lines.len() - max_lines;
    let mut excerpt = trace_lines[..head_count].join("\n");
    excerpt.push_str(&format!(
        "\n[omitted {omitted_count} M17 trace lines]\n{}",
        trace_lines[trace_lines.len() - tail_count..].join("\n")
    ));
    excerpt
}

fn last_serial_lines(serial: &str, max_lines: usize) -> String {
    let mut lines = serial.lines().rev().take(max_lines).collect::<Vec<_>>();
    lines.reverse();
    lines.join("\n")
}

fn resolve_m17_cxx_headers() -> Result<PathBuf, String> {
    if let Some(configured) = std::env::var_os("NAGI_CXX_HEADERS") {
        let path = PathBuf::from(configured);
        if path.join("cstddef").is_file() {
            return Ok(path);
        }
        return Err(format!(
            "NAGI_CXX_HEADERS does not contain libc++ cstddef: {}",
            path.display()
        ));
    }

    let configured_clang = std::env::var_os("NAGI_TARGET_CLANG")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("clang"));
    for compiler in [configured_clang, PathBuf::from("clang++")] {
        let output = match ProcessCommand::new(&compiler)
            .args(["-E", "-x", "c++", "-", "-v"])
            .stdin(Stdio::null())
            .output()
        {
            Ok(output) if output.status.success() => output,
            _ => continue,
        };
        for line in String::from_utf8_lossy(&output.stderr).lines() {
            let candidate = Path::new(line.trim());
            if candidate.ends_with("c++/v1") && candidate.join("cstddef").is_file() {
                return Ok(candidate.to_path_buf());
            }
        }
    }

    Err("could not find libc++ headers through clang++; set NAGI_CXX_HEADERS to a libc++ include directory containing cstddef".to_owned())
}

fn execute_m16_sample_build(root: &Path) -> CommandResult {
    let sample = ProcessCommand::new("cargo")
        .args([
            "build",
            "--manifest-path",
            "samples/hello-nagi/Cargo.toml",
            "--offline",
            "--locked",
        ])
        .current_dir(root)
        .output();
    let sample = match sample {
        Ok(output) if output.status.success() => output,
        Ok(output) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m16: sample build failed: {}", command_output(&output)),
            );
        }
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m16: sample build: {error}")),
    };
    let napp_path = root.join("out").join("artifacts").join("hello-nagi.napp");
    let sample_artifact = ProcessCommand::new("cargo")
        .args([
            "run",
            "--manifest-path",
            "samples/hello-nagi/Cargo.toml",
            "--bin",
            "hello-nagi-package",
            "--offline",
            "--locked",
            "--",
        ])
        .arg(&napp_path)
        .current_dir(root)
        .output();
    let sample_artifact = match sample_artifact {
        Ok(output) if output.status.success() => output,
        Ok(output) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m16: sample SDK artifact failed: {}",
                    command_output(&output)
                ),
            );
        }
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m16: sample SDK artifact: {error}"),
            );
        }
    };
    let generated_dir = root.join("out").join("generated").join("m16");
    let idl = ProcessCommand::new("cargo")
        .args([
            "run",
            "-p",
            "nagi-idl",
            "--offline",
            "--locked",
            "--",
            "generate",
            "idl/application.nidl",
        ])
        .arg(&generated_dir)
        .current_dir(root)
        .output();
    let idl = match idl {
        Ok(output) if output.status.success() => output,
        Ok(output) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m16: IDL generation failed: {}", command_output(&output)),
            );
        }
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m16: IDL generation: {error}")),
    };
    let generated_matches_sdk = [
        (
            generated_dir.join("rust").join("application.rs"),
            root.join("sdk")
                .join("rust")
                .join("src")
                .join("generated.rs"),
        ),
        (
            generated_dir.join("c").join("nagi_sdk.h"),
            root.join("sdk")
                .join("c")
                .join("include")
                .join("nagi_sdk.h"),
        ),
        (
            generated_dir.join("c").join("nagi_sdk.c"),
            root.join("sdk").join("c").join("src").join("nagi_sdk.c"),
        ),
    ]
    .iter()
    .all(|(generated, checked_in)| fs::read(generated).ok() == fs::read(checked_in).ok());
    if !generated_matches_sdk || !command_output(&idl).contains("PASS nagi-idl generate:") {
        return failure(
            EXIT_CONFIG_ERROR,
            "m16: generated IDL bindings differ from the SDK sources".to_owned(),
        );
    }
    let package_path = root.join("out").join("artifacts").join("hello-nagi.xapp");
    let package = ProcessCommand::new("cargo")
        .args([
            "run",
            "--manifest-path",
            "tools/nagi-pkg/Cargo.toml",
            "--offline",
            "--locked",
            "--",
            "build-hello",
        ])
        .arg(&napp_path)
        .arg(&package_path)
        .current_dir(root)
        .output();
    let package = match package {
        Ok(output) if output.status.success() => output,
        Ok(output) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m16: package build failed: {}", command_output(&output)),
            );
        }
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m16: package build: {error}")),
    };
    if !napp_path.is_file()
        || !package_path.is_file()
        || !command_output(&sample_artifact).contains("PASS hello-nagi SDK artifact:")
        || !command_output(&package).contains("PASS nagi-pkg build:")
    {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "m16: package artifact was not produced at {}",
                package_path.display()
            ),
        );
    }
    let _ = sample;
    CommandResult {
        exit_code: EXIT_SUCCESS,
        lines: vec![format!(
            "PASS M16 out-of-tree sample build/package: {}",
            package_path.display()
        )],
    }
}

fn execute_m16_inner(root: &Path, probe: &dyn HostProbe) -> CommandResult {
    let package_path = root.join("out").join("artifacts").join("hello-nagi.xapp");
    let init_args = [
        "build",
        "-p",
        "nagi-init",
        "--features",
        "m16-package",
        "--target",
        "targets/x86_64-unknown-nagi-user.json",
        "-Zbuild-std=core,alloc,compiler_builtins",
        "--release",
    ];
    let image_result = execute_image_with_init_build_env(
        root,
        &init_args,
        None,
        "nagi-0.1-m16-package.img",
        &[("NAGI_M16_PACKAGE", package_path.as_path())],
    );
    if image_result.exit_code != EXIT_SUCCESS {
        return image_result;
    }
    let host = match resolve_qemu_host(root, probe, "m16") {
        Ok(host) => host,
        Err(error) => return failure(EXIT_CONFIG_ERROR, error),
    };
    let artifacts = match ensure_owned_directory(root, Path::new("out").join("artifacts")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m16: {error}")),
    };
    let logs = match ensure_owned_directory(root, Path::new("out").join("logs")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m16: {error}")),
    };
    let image_path = artifacts.join("nagi-0.1-m16-package.img");
    let persistent_disk = artifacts.join("nagi-0.1-m16-package-user-data.img");
    let vars_copy = artifacts.join("nagi-0.1-m16-package-vars.fd");
    let first_log = logs.join("m16-package-first-boot.log");
    let package_log = logs.join("m16-package.log");
    let had_persistent_disk = match ensure_persistent_disk(&persistent_disk) {
        Ok(existing) => existing,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m16: {error}")),
    };
    let timeout = Duration::from_secs(90);
    if !had_persistent_disk {
        let first_config = QemuConfig {
            qemu: &host.qemu,
            ovmf_code: &host.ovmf_code,
            ovmf_vars_template: &host.ovmf_vars,
            disk_image: &image_path,
            persistent_disk: &persistent_disk,
            vars_copy: &vars_copy,
            serial_log: &first_log,
            acceptance_marker: NAGI_WRITE_MARKER,
            timeout,
        };
        if let Err(error) = run_qemu(&first_config) {
            return failure(EXIT_CONFIG_ERROR, format!("m16: first boot: {error}"));
        }
        let first_serial = match fs::read_to_string(&first_log) {
            Ok(serial) => serial,
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!("m16: cannot read {}: {error}", first_log.display()),
                );
            }
        };
        if !first_serial.contains(NAGI_WRITE_MARKER) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m16: first boot did not print `{NAGI_WRITE_MARKER}`"),
            );
        }
    }
    let config = QemuConfig {
        qemu: &host.qemu,
        ovmf_code: &host.ovmf_code,
        ovmf_vars_template: &host.ovmf_vars,
        disk_image: &image_path,
        persistent_disk: &persistent_disk,
        vars_copy: &vars_copy,
        serial_log: &package_log,
        acceptance_marker: "Nagi M16 acceptance PASS",
        timeout,
    };
    let status = match run_qemu(&config) {
        Ok(status) => status,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m16: {error}")),
    };
    let serial = match fs::read_to_string(&package_log) {
        Ok(serial) => serial,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m16: cannot read {}: {error}", package_log.display()),
            );
        }
    };
    for marker in [
        "Nagi Kernel started",
        "Nagi M2 acceptance PASS",
        "Nagi M3 acceptance PASS",
        "Nagi M4 acceptance PASS",
        "Nagi M7 VirtIO Block PASS",
        "Nagi M12 VirtIO Net PASS",
        "Nagi M14 VirtIO Sound PASS",
        "Nagi M14 audio service PASS",
        "Nagi M15 History Service PASS",
        "Nagi M16 SDK identity PASS",
        "Nagi M16 package install PASS",
        "Nagi M16 package list/info PASS",
        "Hello from out-of-tree Nagi app",
        "Nagi M16 Hello app launch PASS",
        "Nagi M16 package atomic update PASS",
        "Nagi M16 package remove PASS",
        "Nagi M16 Package Service PASS",
        "Nagi M16 acceptance PASS",
    ] {
        if !serial.contains(marker) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m16: guest did not print `{marker}` (QEMU exit {status}; log {})",
                    package_log.display()
                ),
            );
        }
    }
    CommandResult {
        exit_code: EXIT_SUCCESS,
        lines: vec![format!(
            "PASS M16 package/SDK: out-of-tree sample built, `.xapp` installed, launched, updated atomically, and removed in the guest (exit {status}; log {})",
            package_log.display()
        )],
    }
}

fn start_m13_http_fixture(root: &Path) -> Result<Child, String> {
    let fixture_root = root.join("tests").join("fixtures").join("m12");
    for executable in ["python.exe", "python3", "python"] {
        let result = ProcessCommand::new(executable)
            .args([
                "-m",
                "http.server",
                "18080",
                "--bind",
                "0.0.0.0",
                "--directory",
            ])
            .arg(&fixture_root)
            .spawn();
        if let Ok(child) = result {
            thread::sleep(Duration::from_millis(500));
            return Ok(child);
        }
    }
    Err(format!(
        "cannot start the M13 HTTP fixture at {}",
        fixture_root.display()
    ))
}

fn execute_shell(root: &Path, probe: &dyn HostProbe) -> CommandResult {
    let image_result = execute_image_with_mode(root, true);
    if image_result.exit_code != EXIT_SUCCESS {
        return image_result;
    }
    let Some(qemu) = probe.command(
        &["qemu-system-x86_64", "qemu-system-x86_64.exe"],
        &["--version"],
    ) else {
        return failure(EXIT_CONFIG_ERROR, "shell: QEMU was not found");
    };
    if qemu.exit_code != Some(0) {
        return failure(
            EXIT_CONFIG_ERROR,
            format!("shell: QEMU probe failed: {}", qemu.output.trim()),
        );
    }
    let Some(ovmf) = probe.ovmf() else {
        return failure(
            EXIT_CONFIG_ERROR,
            "shell: compatible OVMF CODE/VARS was not found",
        );
    };
    if !ovmf.is_compatible() {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "shell: incompatible OVMF CODE/VARS: {} / {}",
                ovmf.code, ovmf.vars
            ),
        );
    }
    let requirements = match load_toolchain_requirements(root) {
        Ok(requirements) => requirements,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("shell: toolchain: {error}")),
    };
    if !ovmf_pair_is_allowed(&ovmf, &requirements) {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "shell: OVMF CODE/VARS pair is not listed in nagi.toml: {} / {}",
                ovmf.code, ovmf.vars
            ),
        );
    }
    let artifacts = match ensure_owned_directory(root, Path::new("out").join("artifacts")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("shell: {error}")),
    };
    let logs = match ensure_owned_directory(root, Path::new("out").join("logs")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("shell: {error}")),
    };
    let image_path = artifacts.join("nagi-0.1-m8-shell.img");
    let persistent_disk = artifacts.join("nagi-0.1-user-data.img");
    let vars_copy = artifacts.join("nagi-0.1-m8-shell-vars.fd");
    let first_log = logs.join("m8-first-boot.log");
    let shell_log = logs.join("m8-shell.log");
    let had_persistent_disk = match ensure_persistent_disk(&persistent_disk) {
        Ok(existing) => existing,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("shell: {error}")),
    };
    let qemu_path = Path::new(&qemu.path);
    let ovmf_code = Path::new(&ovmf.code);
    let ovmf_vars = Path::new(&ovmf.vars);
    let timeout = Duration::from_secs(45);
    if !had_persistent_disk {
        let config = QemuConfig {
            qemu: qemu_path,
            ovmf_code,
            ovmf_vars_template: ovmf_vars,
            disk_image: &image_path,
            persistent_disk: &persistent_disk,
            vars_copy: &vars_copy,
            serial_log: &first_log,
            acceptance_marker: NAGI_WRITE_MARKER,
            timeout,
        };
        if let Err(error) = run_qemu(&config) {
            return failure(EXIT_CONFIG_ERROR, format!("shell: first boot: {error}"));
        }
        let first_serial = match fs::read_to_string(&first_log) {
            Ok(serial) => serial,
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!("shell: cannot read {}: {error}", first_log.display()),
                );
            }
        };
        if !first_serial.contains(NAGI_WRITE_MARKER) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("shell: first boot did not print `{NAGI_WRITE_MARKER}`"),
            );
        }
    }
    let config = QemuConfig {
        qemu: qemu_path,
        ovmf_code,
        ovmf_vars_template: ovmf_vars,
        disk_image: &image_path,
        persistent_disk: &persistent_disk,
        vars_copy: &vars_copy,
        serial_log: &shell_log,
        acceptance_marker: "Nagi M8 acceptance PASS",
        timeout,
    };
    let status = match run_qemu_interactive(&config) {
        Ok(status) => status,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("shell: {error}")),
    };
    let serial = match fs::read_to_string(&shell_log) {
        Ok(serial) => serial,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("shell: cannot read {}: {error}", shell_log.display()),
            );
        }
    };
    if !serial.contains("Nagi M8 acceptance PASS") {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "shell: guest did not print `Nagi M8 acceptance PASS` (QEMU exit {status}; log {})",
                shell_log.display()
            ),
        );
    }
    CommandResult {
        exit_code: EXIT_SUCCESS,
        lines: vec![format!(
            "PASS shell: guest nsh printed `Nagi M8 acceptance PASS` (exit {status}; log {})",
            shell_log.display()
        )],
    }
}

fn execute_run(root: &Path, probe: &dyn HostProbe) -> CommandResult {
    let image_result = execute_image(root);
    if image_result.exit_code != EXIT_SUCCESS {
        return image_result;
    }
    let Some(qemu) = probe.command(
        &["qemu-system-x86_64", "qemu-system-x86_64.exe"],
        &["--version"],
    ) else {
        return failure(EXIT_CONFIG_ERROR, "run: QEMU was not found");
    };
    if qemu.exit_code != Some(0) {
        return failure(
            EXIT_CONFIG_ERROR,
            format!("run: QEMU probe failed: {}", qemu.output.trim()),
        );
    }
    let Some(ovmf) = probe.ovmf() else {
        return failure(
            EXIT_CONFIG_ERROR,
            "run: compatible OVMF CODE/VARS was not found",
        );
    };
    if !ovmf.is_compatible() {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "run: incompatible OVMF CODE/VARS: {} / {}",
                ovmf.code, ovmf.vars
            ),
        );
    }
    let requirements = match load_toolchain_requirements(root) {
        Ok(requirements) => requirements,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("run: toolchain: {error}")),
    };
    if !ovmf_pair_is_allowed(&ovmf, &requirements) {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "run: OVMF CODE/VARS pair is not listed in nagi.toml: {} / {}",
                ovmf.code, ovmf.vars
            ),
        );
    }

    let artifacts = match ensure_owned_directory(root, Path::new("out").join("artifacts")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("run: {error}")),
    };
    let logs = match ensure_owned_directory(root, Path::new("out").join("logs")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("run: {error}")),
    };
    let image_path = artifacts.join("nagi-0.1-m1.img");
    let persistent_disk = artifacts.join("nagi-0.1-user-data.img");
    let vars_copy = artifacts.join("nagi-0.1-m1-vars.fd");
    let first_log = logs.join("m7-first-boot.log");
    let serial_log = logs.join("m1-qemu-boot.log");
    let had_persistent_disk = match ensure_persistent_disk(&persistent_disk) {
        Ok(existing) => existing,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("run: {error}")),
    };

    let qemu_path = Path::new(&qemu.path);
    let ovmf_code = Path::new(&ovmf.code);
    let ovmf_vars = Path::new(&ovmf.vars);
    let timeout = Duration::from_secs(30);
    let run_guest = |serial_log: &Path, acceptance_marker: &str| {
        let config = QemuConfig {
            qemu: qemu_path,
            ovmf_code,
            ovmf_vars_template: ovmf_vars,
            disk_image: &image_path,
            persistent_disk: &persistent_disk,
            vars_copy: &vars_copy,
            serial_log,
            acceptance_marker,
            timeout,
        };
        run_qemu(&config)
    };
    let qemu_status;
    let serial;

    if had_persistent_disk {
        qemu_status = match run_guest(&serial_log, GUEST_ACCEPTANCE_MARKER) {
            Ok(status) => status,
            Err(error) => return failure(EXIT_CONFIG_ERROR, format!("run: {error}")),
        };
        serial = match fs::read_to_string(&serial_log) {
            Ok(serial) => serial,
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!("run: cannot read {}: {error}", serial_log.display()),
                );
            }
        };
    } else {
        let first_status = match run_guest(&first_log, NAGI_WRITE_MARKER) {
            Ok(status) => status,
            Err(error) => return failure(EXIT_CONFIG_ERROR, format!("run: {error}")),
        };
        let first_serial = match fs::read_to_string(&first_log) {
            Ok(serial) => serial,
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!("run: cannot read {}: {error}", first_log.display()),
                );
            }
        };
        if first_serial.contains(GUEST_ACCEPTANCE_MARKER) {
            if let Err(error) = fs::copy(&first_log, &serial_log) {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!(
                        "run: cannot preserve final serial log {}: {error}",
                        serial_log.display()
                    ),
                );
            }
            qemu_status = first_status;
            serial = first_serial;
        } else {
            if !first_serial.contains(NAGI_WRITE_MARKER) {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!(
                        "run: first boot did not print `{NAGI_WRITE_MARKER}` (QEMU exit {first_status}; log {})",
                        first_log.display()
                    ),
                );
            }
            qemu_status = match run_guest(&serial_log, GUEST_ACCEPTANCE_MARKER) {
                Ok(status) => status,
                Err(error) => return failure(EXIT_CONFIG_ERROR, format!("run: {error}")),
            };
            serial = match fs::read_to_string(&serial_log) {
                Ok(serial) => serial,
                Err(error) => {
                    return failure(
                        EXIT_CONFIG_ERROR,
                        format!("run: cannot read {}: {error}", serial_log.display()),
                    );
                }
            };
        }
    }
    if !serial.contains(GUEST_ACCEPTANCE_MARKER) {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "run: guest did not print `{GUEST_ACCEPTANCE_MARKER}` (QEMU exit {qemu_status}; log {})",
                serial_log.display()
            ),
        );
    }
    CommandResult {
        exit_code: EXIT_SUCCESS,
        lines: vec![format!(
            "PASS run: QEMU guest printed `{GUEST_ACCEPTANCE_MARKER}` (exit {qemu_status}; log {})",
            serial_log.display()
        )],
    }
}

fn run_cargo(root: &Path, label: &str, args: &[&str]) -> CommandResult {
    finish_cargo(
        label,
        ProcessCommand::new("cargo")
            .args(args)
            .current_dir(root)
            .output(),
    )
}

fn execute_format(root: &Path) -> CommandResult {
    for (program, args) in host_format_commands() {
        let output = ProcessCommand::new(program)
            .args(args)
            .current_dir(root)
            .output();
        let result = finish_tool(&format!("fmt ({program})"), program, output);
        if result.exit_code != EXIT_SUCCESS {
            return result;
        }
    }

    CommandResult {
        exit_code: EXIT_SUCCESS,
        lines: vec!["PASS fmt: configured repository source checks passed".into()],
    }
}

fn run_cargo_in(
    root: &Path,
    relative_directory: &Path,
    label: &str,
    args: &[&str],
) -> CommandResult {
    finish_cargo(
        label,
        ProcessCommand::new("cargo")
            .args(args)
            .current_dir(root.join(relative_directory))
            .output(),
    )
}

fn run_cargo_with_env(
    root: &Path,
    label: &str,
    args: &[&str],
    cargo_env: &[(&str, &Path)],
) -> CommandResult {
    let mut command = ProcessCommand::new("cargo");
    command.args(args).current_dir(root);
    for (key, value) in cargo_env {
        command.env(key, value);
    }
    finish_cargo(label, command.output())
}

fn run_cargo_with_rust_std_source(
    root: &Path,
    label: &str,
    args: &[&str],
    source_root: &Path,
    cargo_env: &[(&str, &Path)],
) -> CommandResult {
    let mut command = ProcessCommand::new("cargo");
    command
        .args(args)
        .env("__CARGO_TESTS_ONLY_SRC_ROOT", source_root)
        .current_dir(root);
    for (key, value) in cargo_env {
        command.env(key, value);
    }
    finish_cargo(label, command.output())
}

fn finish_cargo(label: &str, output: std::io::Result<std::process::Output>) -> CommandResult {
    finish_tool(label, "cargo", output)
}

fn finish_tool(
    label: &str,
    program: &str,
    output: std::io::Result<std::process::Output>,
) -> CommandResult {
    match output {
        Ok(output) if output.status.success() => CommandResult {
            exit_code: EXIT_SUCCESS,
            lines: vec![format!("PASS {label}: {program} completed successfully")],
        },
        Ok(output) => {
            let detail = command_output(&output).trim().to_owned();
            failure(
                output.status.code().unwrap_or(EXIT_CONFIG_ERROR),
                format!("{label}: {program} failed{}", nonempty_detail(&detail)),
            )
        }
        Err(error) => failure(
            EXIT_CONFIG_ERROR,
            format!("{label}: cannot start {program}: {error}"),
        ),
    }
}

fn prepare_nagi_rust_std_source(root: &Path) -> Result<PathBuf, String> {
    let rustc = ProcessCommand::new("rustc")
        .args(["--print", "sysroot"])
        .output()
        .map_err(|error| format!("cannot query rustc sysroot: {error}"))?;
    if !rustc.status.success() {
        return Err(format!(
            "rustc --print sysroot failed: {}",
            command_output(&rustc)
        ));
    }
    let sysroot = String::from_utf8_lossy(&rustc.stdout).trim().to_owned();
    let installed_source = PathBuf::from(sysroot)
        .join("lib")
        .join("rustlib")
        .join("src")
        .join("rust");
    if !installed_source.join("library").is_dir() {
        return Err(format!(
            "rust-src library is missing at {}",
            installed_source.display()
        ));
    }

    let generated_source = root.join("out").join("rust-src");
    if generated_source.exists() {
        fs::remove_dir_all(&generated_source).map_err(|error| {
            format!(
                "cannot replace generated Rust source {}: {error}",
                generated_source.display()
            )
        })?;
    }
    copy_directory(&installed_source, &generated_source)?;

    let patch = root
        .join("third_party")
        .join("rust-std")
        .join("patches")
        .join("0001-nagi-target-support.patch");
    if !patch.is_file() {
        return Err(format!("Rust std patch is missing at {}", patch.display()));
    }
    let applied = ProcessCommand::new("git")
        .args([
            "apply",
            "--unsafe-paths",
            "--whitespace=nowarn",
            "--directory=out/rust-src",
        ])
        .arg(
            Path::new("third_party")
                .join("rust-std")
                .join("patches")
                .join("0001-nagi-target-support.patch"),
        )
        .current_dir(root)
        .output()
        .map_err(|error| format!("cannot apply Rust std patch: {error}"))?;
    if !applied.status.success() {
        return Err(format!(
            "Rust std patch failed: {}",
            command_output(&applied)
        ));
    }

    let cargo_manifest = generated_source.join("library").join("Cargo.toml");
    let mut manifest = fs::read_to_string(&cargo_manifest)
        .map_err(|error| format!("cannot read generated Rust library manifest: {error}"))?;
    if !manifest.contains("../../../third_party/libc") {
        manifest.push_str("\nlibc = { path = \"../../../third_party/libc\" }\n");
        fs::write(&cargo_manifest, manifest)
            .map_err(|error| format!("cannot add Nagi libc patch: {error}"))?;
    }
    Ok(generated_source.join("library"))
}

fn copy_directory(source: &Path, destination: &Path) -> Result<(), String> {
    fs::create_dir_all(destination)
        .map_err(|error| format!("cannot create {}: {error}", destination.display()))?;
    for entry in fs::read_dir(source)
        .map_err(|error| format!("cannot enumerate {}: {error}", source.display()))?
    {
        let entry = entry.map_err(|error| format!("cannot inspect Rust source entry: {error}"))?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry
            .file_type()
            .map_err(|error| format!("cannot inspect {}: {error}", source_path.display()))?
            .is_dir()
        {
            copy_directory(&source_path, &destination_path)?;
        } else {
            fs::copy(&source_path, &destination_path).map_err(|error| {
                format!(
                    "cannot copy {} to {}: {error}",
                    source_path.display(),
                    destination_path.display()
                )
            })?;
        }
    }
    Ok(())
}

fn command_output(output: &std::process::Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    match (stdout.trim(), stderr.trim()) {
        ("", detail) => detail.to_owned(),
        (detail, "") => detail.to_owned(),
        (stdout, stderr) => format!("{stdout}\n{stderr}"),
    }
}

fn nonempty_detail(detail: &str) -> String {
    if detail.is_empty() {
        String::new()
    } else {
        format!(": {detail}")
    }
}

fn append_nagi_target_archive_tools<'a>(cargo_env: &mut Vec<(&'a str, &'a Path)>, host_os: &str) {
    if host_os == "macos" {
        // Apple's archiver treats freestanding Nagi ELF objects as invalid
        // Mach-O members. Keep the override target-qualified so build-script
        // host tools continue using the native macOS archiver.
        cargo_env.push(("AR_x86_64_unknown_nagi_user", Path::new("llvm-ar")));
        cargo_env.push(("RANLIB_x86_64_unknown_nagi_user", Path::new("llvm-ranlib")));
    }
}

fn execute_m18(root: &Path, probe: &dyn HostProbe) -> CommandResult {
    let cxx_headers = match resolve_m17_cxx_headers() {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m18: C++ headers: {error}")),
    };

    let fetch = execute_fetch(root);
    if fetch.exit_code != EXIT_SUCCESS {
        return fetch;
    }
    let sample = execute_m16_sample_build(root);
    if sample.exit_code != EXIT_SUCCESS {
        return sample;
    }

    let rust_std_source = match prepare_nagi_rust_std_source(root) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m18: rust std: {error}")),
    };
    let mesa_build = ProcessCommand::new("bash")
        .args(["tools/mesa/build.sh"])
        .current_dir(root)
        .output();
    match mesa_build {
        Ok(output) if output.status.success() => {}
        Ok(output) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m18: Mesa/Softpipe build failed: {}",
                    command_output(&output)
                ),
            );
        }
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m18: cannot start tools/mesa/build.sh through bash: {error}"),
            );
        }
    }

    let package_path = root.join("out").join("artifacts").join("hello-nagi.xapp");
    let mesa_build_path = root.join("out").join("m17-mesa").join("mesa-build");
    let font_cache_path = root.join("out").join("cache").join("fonts");
    let target_compiler_wrapper = root.join("tools").join("nagi-target-cc.sh");
    let init_args = [
        "build",
        "-p",
        "nagi-init",
        "--features",
        "m18-acceptance",
        "--target",
        "targets/x86_64-unknown-nagi-user.json",
        "-Zbuild-std=std,panic_abort",
        "--release",
        "--locked",
        "--offline",
    ];
    let mut init_build_env = vec![
        ("NAGI_M16_PACKAGE", package_path.as_path()),
        ("NAGI_MESA_BUILD", mesa_build_path.as_path()),
        ("NAGI_FONT_DIR", font_cache_path.as_path()),
        ("NAGI_CXX_HEADERS", cxx_headers.as_path()),
        // MozJS builds host-side configure helpers as well as Nagi
        // target objects; keep those host probes off the target wrapper.
        ("HOST_CC", Path::new("cc")),
        ("HOST_CXX", Path::new("c++")),
        (
            "CC_x86_64_unknown_nagi_user",
            target_compiler_wrapper.as_path(),
        ),
        (
            "CXX_x86_64_unknown_nagi_user",
            target_compiler_wrapper.as_path(),
        ),
    ];
    append_nagi_target_archive_tools(&mut init_build_env, std::env::consts::OS);
    let image_result = execute_image_with_init_build_env_using_writer(
        root,
        &init_args,
        Some(&rust_std_source),
        "nagi-0.1-m18-albert.img",
        &init_build_env,
        write_m17_fat12_image,
        ImageBuildFeatures {
            kernel: &["m18-browser-memory"],
            loader: &[],
        },
    );
    if image_result.exit_code != EXIT_SUCCESS {
        return image_result;
    }

    let host = match resolve_qemu_host(root, probe, "m18") {
        Ok(host) => host,
        Err(error) => return failure(EXIT_CONFIG_ERROR, error),
    };
    let artifacts = match ensure_owned_directory(root, Path::new("out").join("artifacts")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m18: {error}")),
    };
    let logs = match ensure_owned_directory(root, Path::new("out").join("logs")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m18: {error}")),
    };
    let evidence_run_id = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_nanos().to_string(),
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m18: system clock: {error}")),
    };
    let evidence_directory = match ensure_owned_directory(
        root,
        Path::new("out")
            .join("evidence")
            .join(format!("m29-browser-{evidence_run_id}")),
    ) {
        Ok(path) => path,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m18: evidence directory: {error}"),
            )
        }
    };
    let screenshot_path = evidence_directory.join("nagi-m18-browser.png");
    let image_path = artifacts.join("nagi-0.1-m18-albert.img");
    let persistent_disk = artifacts.join("nagi-0.1-m18-user-data.img");
    let vars_copy = artifacts.join("nagi-0.1-m18-vars.fd");
    let first_log = logs.join("m18-first-boot.log");
    let log_path = logs.join("m18-albert.log");
    let had_persistent_disk = match ensure_persistent_disk(&persistent_disk) {
        Ok(existing) => existing,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m18: {error}")),
    };
    let timeout = Duration::from_secs(1_200);
    if !had_persistent_disk {
        let first_config = QemuConfig {
            qemu: &host.qemu,
            ovmf_code: &host.ovmf_code,
            ovmf_vars_template: &host.ovmf_vars,
            disk_image: &image_path,
            persistent_disk: &persistent_disk,
            vars_copy: &vars_copy,
            serial_log: &first_log,
            acceptance_marker: NAGI_WRITE_MARKER,
            timeout,
        };
        if let Err(error) = run_qemu_with_read_only_boot_disk(&first_config) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m18: first boot: {error}\nserial log tail:\n{}",
                    serial_log_tail(&first_log, 64)
                ),
            );
        }
        let first_serial = match fs::read_to_string(&first_log) {
            Ok(serial) => serial,
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!("m18: cannot read {}: {error}", first_log.display()),
                );
            }
        };
        if !first_serial.contains(NAGI_WRITE_MARKER) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m18: first boot did not print `{NAGI_WRITE_MARKER}` (log {})",
                    first_log.display()
                ),
            );
        }
    }
    let config = QemuConfig {
        qemu: &host.qemu,
        ovmf_code: &host.ovmf_code,
        ovmf_vars_template: &host.ovmf_vars,
        disk_image: &image_path,
        persistent_disk: &persistent_disk,
        vars_copy: &vars_copy,
        serial_log: &log_path,
        acceptance_marker: "Nagi M18 browser scenario complete pages=3",
        timeout,
    };
    let outcome =
        match run_qemu_gui_with_read_only_boot_disk_and_staged_events_and_failure_marker_and_screenshot(
            &config,
            "Nagi M18 browser READY",
            &M18_INPUT_EVENTS,
            &[
                QmpEventStage {
                    marker: "Nagi M18 permission prompt READY",
                    events: &M18_PERMISSION_ALLOW_EVENTS,
                },
                QmpEventStage {
                    marker: "Nagi M18 clipboard page READY",
                    events: &M18_CLIPBOARD_COPY_EVENTS,
                },
                QmpEventStage {
                    marker: "Nagi M18 clipboard copy observed",
                    events: &M18_CLIPBOARD_FOCUS_EVENTS,
                },
                QmpEventStage {
                    marker: "Nagi M18 clipboard destination focused",
                    events: &M18_CLIPBOARD_PASTE_EVENTS,
                },
                QmpEventStage {
                    marker: "Nagi M18 IME page READY",
                    events: &M18_IME_EVENTS,
                },
                QmpEventStage {
                    marker: "Nagi M18 upload page READY",
                    events: &M18_UPLOAD_CLICK_EVENTS,
                },
                QmpEventStage {
                    marker: "Nagi M18 upload picker READY",
                    events: &M18_UPLOAD_CHOOSE_EVENTS,
                },
                QmpEventStage {
                    marker: "Nagi M18 download page READY",
                    events: &M18_DOWNLOAD_CLICK_EVENTS,
                },
            ],
            "Nagi M18 browser FAIL",
            &screenshot_path,
        ) {
            Ok(outcome) => outcome,
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!(
                        "m18: QEMU: {error}\nServo trace excerpt:\n{}\nserial log tail:\n{}",
                        serial_log_m17_trace_excerpt(&log_path, 256),
                        serial_log_tail(&log_path, 64),
                    ),
                );
            }
        };
    let serial = match fs::read_to_string(&log_path) {
        Ok(serial) => serial,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m18: cannot read {}: {error}", log_path.display()),
            );
        }
    };
    if let Err(error) = crate::m18_acceptance::validate_serial_log(&serial) {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "m18: guest browser acceptance failed: {error} (QEMU exit {}; log {})",
                outcome.exit_status,
                log_path.display()
            ),
        );
    }
    if !outcome.acceptance_reached {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "m18: guest reached no complete three-page browser acceptance (exit {}; log {})",
                outcome.exit_status,
                log_path.display()
            ),
        );
    }
    CommandResult {
        exit_code: EXIT_SUCCESS,
        lines: vec![format!(
            "PASS M18 Albert: three verified HTTPS pages rendered to Nagi Surface and QEMU; a user-answered site-permission prompt, gesture-bound clipboard copy/paste, Japanese IME composition, trusted-picker upload, and user-activated download passed (exit {}; log {}; screenshot {})",
            outcome.exit_status,
            log_path.display(),
            screenshot_path.display(),
        )],
    }
}
const ISOLATED_PROCESS_MARKERS: [&str; 16] = [
    "Nagi Kernel started",
    "Nagi ADR0043 isolated process spawned pid=2",
    "Nagi isolated process kernel-stamped sender PASS",
    "Nagi isolated process forged payload identity denied PASS",
    "Nagi isolated process address space and syscall isolation PASS",
    "Nagi ADR0043 isolated process exit pid=2 code=0",
    "Nagi isolated process exit cleanup PASS",
    "Nagi ADR0047 isolated process fault pid=3 vector=14",
    "Nagi ADR0047 isolated process fault pid=4 vector=6",
    "Nagi ADR0047 isolated process fault pid=5 vector=13",
    "Nagi isolated process fault containment PASS",
    "Nagi Supervisor process exit status PASS",
    "Nagi Supervisor signed package verification PASS",
    "Nagi isolated processes concurrent PASS",
    "Nagi Supervisor grant consent required PASS",
    "Nagi Supervisor grant decisions PASS",
];
const ISOLATED_PROCESS_PASS_MARKER: &str = "Nagi isolated process acceptance PASS";

/// ADR 0043 acceptance: the Supervisor (init) spawns a real second ELF into
/// its own address space and authorizes it only by kernel-stamped identity.
/// Build the isolated application ELFs (ADR 0043/0044) and return the
/// release output directory that contains them.
fn build_isolated_apps(root: &Path) -> Result<PathBuf, CommandResult> {
    let app_build = run_cargo(
        root,
        "isolated app build",
        &[
            "build",
            "-p",
            "nagi-isolated-app",
            "--target",
            "targets/x86_64-unknown-nagi-user.json",
            "-Zbuild-std=core,compiler_builtins",
            "-Zbuild-std-features=compiler-builtins-mem",
            "--release",
            "--locked",
        ],
    );
    if app_build.exit_code != EXIT_SUCCESS {
        return Err(app_build);
    }
    let release = root
        .join("target")
        .join("x86_64-unknown-nagi-user")
        .join("release");
    let packages = match ensure_owned_directory(
        root,
        Path::new("out")
            .join("artifacts")
            .join("acceptance-packages"),
    ) {
        Ok(path) => path,
        Err(error) => {
            return Err(failure(
                EXIT_CONFIG_ERROR,
                format!("acceptance packages: {error}"),
            ))
        }
    };
    for (name, application, executable) in ACCEPTANCE_PACKAGES {
        let manifest = root
            .join("user")
            .join("nagi-init")
            .join("manifests")
            .join(format!("{application}.manifest"));
        let elf = release.join(executable);
        let output = packages.join(format!("{name}.xapp"));
        let (manifest, elf, output) = (
            manifest.display().to_string(),
            elf.display().to_string(),
            output.display().to_string(),
        );
        let packaged = run_cargo(
            root,
            "acceptance package signing",
            &[
                "run",
                "--quiet",
                "--locked",
                "--manifest-path",
                "tools/nagi-pkg/Cargo.toml",
                "--",
                "build-signed",
                &manifest,
                &elf,
                &output,
            ],
        );
        if packaged.exit_code != EXIT_SUCCESS {
            return Err(packaged);
        }
    }
    Ok(packages)
}

/// Signed `.xapp` packages the Supervisor launches in acceptances
/// (ADR 0049): `(package name, manifest application ID, isolated ELF)`.
const ACCEPTANCE_PACKAGES: [(&str, &str, &str); 7] = [
    (
        "isolated-app",
        "org.nagi.acceptance.isolated-app",
        "nagi-isolated-app",
    ),
    (
        "faulting-app",
        "org.nagi.acceptance.faulting-app",
        "nagi-faulting-app",
    ),
    (
        "m19-search-search-client",
        "org.nagi.acceptance.m19-search",
        "nagi-m19-search-client",
    ),
    (
        "m19-search-action-client",
        "org.nagi.acceptance.m19-search",
        "nagi-action-client",
    ),
    (
        "foreign-search-client",
        "org.nagi.acceptance.foreign-client",
        "nagi-m19-search-client",
    ),
    (
        "foreign-action-client",
        "org.nagi.acceptance.foreign-client",
        "nagi-action-client",
    ),
    (
        "m22-files-action-client",
        "org.nagi.acceptance.m22-files",
        "nagi-action-client",
    ),
];

/// Build and sign the acceptance packages and return the environment an
/// init build with isolated applications needs.
fn isolated_client_env(root: &Path) -> Result<[(&'static str, PathBuf); 1], CommandResult> {
    Ok([("NAGI_ACCEPTANCE_PACKAGES", build_isolated_apps(root)?)])
}

/// Build an init image whose M19/M21/M22 callers are isolated client
/// processes (ADR 0044/0045). `features` must include `m21-action-ipc`.
fn execute_image_with_isolated_clients(
    root: &Path,
    features: &str,
    image_name: &str,
) -> CommandResult {
    let packages = match build_isolated_apps(root) {
        Ok(directory) => directory,
        Err(result) => return result,
    };
    let init_args = [
        "build",
        "-p",
        "nagi-init",
        "--features",
        features,
        "--target",
        "targets/x86_64-unknown-nagi-user.json",
        "-Zbuild-std=core,alloc,compiler_builtins",
        "--release",
    ];
    // Signed acceptance packages exceed the legacy 1.44 MB FAT12 image.
    execute_image_with_init_build_env_using_writer(
        root,
        &init_args,
        None,
        image_name,
        &[("NAGI_ACCEPTANCE_PACKAGES", packages.as_path())],
        write_isolated_apps_fat12_image,
        ImageBuildFeatures::default(),
    )
}

fn execute_isolated_process(root: &Path, probe: &dyn HostProbe) -> CommandResult {
    let packages = match build_isolated_apps(root) {
        Ok(directory) => directory,
        Err(result) => return result,
    };
    let init_args = [
        "build",
        "-p",
        "nagi-init",
        "--features",
        "isolated-process-acceptance",
        "--target",
        "targets/x86_64-unknown-nagi-user.json",
        "-Zbuild-std=core,alloc,compiler_builtins",
        "--release",
        "--locked",
    ];
    let image_name = "nagi-0.1-isolated-process.img";
    let image_result = execute_image_with_init_build_env_using_writer(
        root,
        &init_args,
        None,
        image_name,
        &[("NAGI_ACCEPTANCE_PACKAGES", packages.as_path())],
        write_isolated_apps_fat12_image,
        ImageBuildFeatures::default(),
    );
    if image_result.exit_code != EXIT_SUCCESS {
        return image_result;
    }
    let host = match resolve_qemu_host(root, probe, "isolated-process") {
        Ok(host) => host,
        Err(error) => return failure(EXIT_CONFIG_ERROR, error),
    };
    let artifacts = match ensure_owned_directory(root, Path::new("out").join("artifacts")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("isolated-process: {error}")),
    };
    let logs = match ensure_owned_directory(root, Path::new("out").join("logs")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("isolated-process: {error}")),
    };
    let image_path = artifacts.join(image_name);
    let persistent_disk = artifacts.join("nagi-0.1-isolated-process-user-data.img");
    let vars_copy = artifacts.join("nagi-0.1-isolated-process-vars.fd");
    let serial_log = logs.join("isolated-process.log");
    if let Err(error) = ensure_persistent_disk(&persistent_disk) {
        return failure(EXIT_CONFIG_ERROR, format!("isolated-process: {error}"));
    }
    let config = QemuConfig {
        qemu: &host.qemu,
        ovmf_code: &host.ovmf_code,
        ovmf_vars_template: &host.ovmf_vars,
        disk_image: &image_path,
        persistent_disk: &persistent_disk,
        vars_copy: &vars_copy,
        serial_log: &serial_log,
        acceptance_marker: ISOLATED_PROCESS_PASS_MARKER,
        timeout: Duration::from_secs(90),
    };
    let status = match run_qemu(&config) {
        Ok(status) => status,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("isolated-process: {error}")),
    };
    let serial = match fs::read_to_string(&serial_log) {
        Ok(serial) => serial,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "isolated-process: cannot read {}: {error}",
                    serial_log.display()
                ),
            );
        }
    };
    if let Some(missing) = missing_isolated_process_marker(&serial) {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "isolated-process: guest did not print `{missing}` (QEMU exit {status}; log {})",
                serial_log.display()
            ),
        );
    }
    CommandResult {
        exit_code: EXIT_SUCCESS,
        lines: vec![format!(
            "PASS isolated-process: Supervisor spawned an isolated ELF, authorized it by kernel-stamped PID, denied a forged payload identity, and observed exit cleanup (exit {status}; log {})",
            serial_log.display()
        )],
    }
}

fn missing_isolated_process_marker(serial: &str) -> Option<&'static str> {
    if serial.contains("Nagi isolated process acceptance FAIL")
        || serial.contains("Nagi faulting app survived its fault FAIL")
    {
        return Some(ISOLATED_PROCESS_PASS_MARKER);
    }
    ISOLATED_PROCESS_MARKERS
        .into_iter()
        .chain([ISOLATED_PROCESS_PASS_MARKER])
        .find(|marker| !serial.contains(marker))
}

fn execute_m19(root: &Path, probe: &dyn HostProbe) -> CommandResult {
    let mut fixture = match start_m13_http_fixture(root) {
        Ok(child) => child,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m19: {error}")),
    };
    let result = execute_m19_inner(root, probe);
    let _ = fixture.kill();
    let _ = fixture.wait();
    result
}

fn execute_m19_inner(root: &Path, probe: &dyn HostProbe) -> CommandResult {
    let image_result = execute_image_with_isolated_clients(
        root,
        "m21-action-ipc",
        "nagi-0.1-m19-vfs-objectid.img",
    );
    if image_result.exit_code != EXIT_SUCCESS {
        return image_result;
    }
    let host = match resolve_qemu_host(root, probe, "m19") {
        Ok(host) => host,
        Err(error) => return failure(EXIT_CONFIG_ERROR, error),
    };
    let artifacts = match ensure_owned_directory(root, Path::new("out").join("artifacts")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m19: {error}")),
    };
    let logs = match ensure_owned_directory(root, Path::new("out").join("logs")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m19: {error}")),
    };
    let image_path = artifacts.join("nagi-0.1-m19-vfs-objectid.img");
    let persistent_disk = artifacts.join("nagi-0.1-m19-vfs-objectid-user-data.img");
    let vars_copy = artifacts.join("nagi-0.1-m19-vfs-objectid-vars.fd");
    let bootstrap_log = logs.join("m19-vfs-objectid-bootstrap.log");
    let initial_log = logs.join("m19-vfs-objectid-initial.log");
    let restart_log = logs.join("m19-vfs-objectid-restart.log");
    let had_persistent_disk = match ensure_persistent_disk(&persistent_disk) {
        Ok(existing) => existing,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m19: {error}")),
    };
    // Repeated TCG integration boots have stalled in firmware past 90 seconds
    // before the guest emits serial output. Keep the same guest marker gates
    // while allowing a slow firmware start to finish.
    let timeout = Duration::from_secs(180);

    if !had_persistent_disk {
        let config = QemuConfig {
            qemu: &host.qemu,
            ovmf_code: &host.ovmf_code,
            ovmf_vars_template: &host.ovmf_vars,
            disk_image: &image_path,
            persistent_disk: &persistent_disk,
            vars_copy: &vars_copy,
            serial_log: &bootstrap_log,
            acceptance_marker: NAGI_WRITE_MARKER,
            timeout,
        };
        if let Err(error) = run_qemu(&config) {
            return failure(EXIT_CONFIG_ERROR, format!("m19: bootstrap boot: {error}"));
        }
        let bootstrap = match fs::read_to_string(&bootstrap_log) {
            Ok(serial) => serial,
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!("m19: cannot read {}: {error}", bootstrap_log.display()),
                );
            }
        };
        if !bootstrap.contains(NAGI_WRITE_MARKER) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m19: bootstrap did not print `{NAGI_WRITE_MARKER}`"),
            );
        }
    }

    let mut final_log = initial_log.as_path();
    let mut verified_restart = false;
    for boot_index in 0..2 {
        let log_path = if boot_index == 0 {
            &initial_log
        } else {
            &restart_log
        };
        let config = QemuConfig {
            qemu: &host.qemu,
            ovmf_code: &host.ovmf_code,
            ovmf_vars_template: &host.ovmf_vars,
            disk_image: &image_path,
            persistent_disk: &persistent_disk,
            vars_copy: &vars_copy,
            serial_log: log_path,
            acceptance_marker: "Nagi M13 acceptance PASS",
            timeout,
        };
        let final_status = match run_qemu(&config) {
            Ok(status) => status,
            Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m19: guest boot: {error}")),
        };
        final_log = log_path;
        let serial = match fs::read_to_string(log_path) {
            Ok(serial) => serial,
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!("m19: cannot read {}: {error}", log_path.display()),
                );
            }
        };
        for marker in [
            "Nagi Kernel started",
            "Nagi M3 CPU scheduler fairness PASS",
            "Nagi M3 acceptance PASS",
            "Nagi M7 VirtIO Block PASS",
            "Nagi M13 Rust PAL PASS",
            "Nagi M13 C POSIX PASS",
            "Nagi bootstrap Channel ABI PASS",
            "Nagi bootstrap Channel wait/wake PASS",
            "Nagi M24 semantic index ready PASS",
            "Nagi M19 Search IPC authorized isolated client PASS",
            "Nagi M19 Search IPC foreign isolated client hidden PASS",
            "Nagi M19 Search IPC authenticated caller PASS",
            "Nagi M21 foreign isolated caller denied PASS",
            "Nagi M21 file.search isolated caller PASS",
            "Nagi M19 trace inode reuse assigned a new ObjectId",
            "Nagi M19 live VFS file ObjectId rename/restart PASS",
            "Nagi M19 guest search persistence PASS",
            "Nagi M19 acceptance PASS",
            "Nagi M13 acceptance PASS",
        ] {
            if !serial.contains(marker) {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!(
                        "m19: guest did not print `{marker}` (QEMU exit {final_status}; log {})",
                        log_path.display()
                    ),
                );
            }
        }
        let metadata_restored = serial.contains("Nagi M19 previous-boot snapshot PASS");
        let semantic_restored = serial.contains("Nagi M24 semantic index persistence PASS");
        if metadata_restored && semantic_restored {
            verified_restart = true;
            break;
        }
        if boot_index == 0
            && (metadata_restored || serial.contains("Nagi M19 initial snapshot/reopen PASS"))
        {
            continue;
        }
        if !metadata_restored || !semantic_restored {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m19: guest did not verify M19 metadata and M24 semantic-index persistence after QEMU restart (QEMU exit {final_status}; log {})",
                    log_path.display()
                ),
            );
        }
    }
    if !verified_restart {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "m19: persistent snapshot was not verified after QEMU restart (log {})",
                final_log.display()
            ),
        );
    }

    CommandResult {
        exit_code: EXIT_SUCCESS,
        lines: vec![format!(
            "PASS M19 guest Search: live VFS file metadata and stable ObjectId survived rename, remount, and QEMU restart (acceptance marker reached; log {})",
            final_log.display()
        )],
    }
}

fn run_m13_qemu_with_http_fixture(root: &Path, config: &QemuConfig<'_>) -> Result<i32, String> {
    let mut fixture = start_m13_http_fixture(root)?;
    let result = run_qemu_reusing_ovmf_vars(config);
    let _ = fixture.kill();
    let _ = fixture.wait();
    result
}

fn execute_m22(root: &Path, probe: &dyn HostProbe) -> CommandResult {
    let mut fixture = match start_m13_http_fixture(root) {
        Ok(child) => child,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m22: {error}")),
    };
    let result = execute_m22_inner(root, probe);
    let _ = fixture.kill();
    let _ = fixture.wait();
    result
}

fn has_pre_guest_firmware_timeout_signature(error: &str, serial: &str) -> bool {
    error.contains("QEMU did not reach acceptance within")
        && !serial.contains("Nagi Kernel started")
        && serial.contains("QEMU timeout diagnostics:")
        && serial.contains("QMP query-status:")
        && serial.contains("\"status\": \"running\"")
        && serial.contains("QMP CPU registers:")
        && serial.contains("QMP CPU instruction window:")
}

fn path_with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut output = path.as_os_str().to_os_string();
    output.push(suffix);
    PathBuf::from(output)
}

fn run_m27_headless_with_pre_guest_retry(
    config: &QemuConfig<'_>,
    read_only_boot_disk: bool,
) -> Result<i32, String> {
    run_headless_with_pre_guest_retry_using(config, read_only_boot_disk, "M27", |config| {
        if read_only_boot_disk {
            run_qemu_reusing_ovmf_vars_with_read_only_boot_disk(config)
        } else {
            run_qemu_reusing_ovmf_vars(config)
        }
    })
}

fn qemu_writable_disk_hashes(
    config: &QemuConfig<'_>,
    read_only_boot_disk: bool,
    scope: &str,
) -> Result<Vec<(PathBuf, String)>, String> {
    let mut paths = Vec::new();
    if !read_only_boot_disk {
        paths.push(config.disk_image);
    }
    if config.persistent_disk != config.disk_image {
        paths.push(config.persistent_disk);
    }

    paths
        .into_iter()
        .map(|path| {
            m30_image_sha256(path)
                .map(|digest| (path.to_path_buf(), digest))
                .map_err(|error| {
                    format!(
                        "cannot hash writable {scope} disk {}: {error}",
                        path.display()
                    )
                })
        })
        .collect()
}

fn run_headless_with_pre_guest_retry_using(
    config: &QemuConfig<'_>,
    read_only_boot_disk: bool,
    scope: &str,
    mut run: impl FnMut(&QemuConfig<'_>) -> Result<i32, String>,
) -> Result<i32, String> {
    let first_serial = path_with_suffix(config.serial_log, ".pre-guest-timeout-1");
    let first_vars = path_with_suffix(config.serial_log, ".ovmf-vars.pre-guest-timeout-1");
    let retry_source_vars_path =
        path_with_suffix(config.serial_log, ".ovmf-vars.pre-guest-retry-source-1");
    let retry_note = path_with_suffix(config.serial_log, ".pre-guest-retry-1.txt");
    for path in [
        &first_serial,
        &first_vars,
        &retry_source_vars_path,
        &retry_note,
    ] {
        match fs::symlink_metadata(path) {
            Ok(_) => {
                return Err(format!(
                    "refusing to overwrite retry evidence {}",
                    path.display()
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "cannot inspect retry evidence {}: {error}",
                    path.display()
                ));
            }
        }
    }

    let retry_source_vars = fs::read(config.vars_copy).map_err(|read_error| {
        format!(
            "cannot snapshot OVMF variables {} before {scope} QEMU attempt: {read_error}",
            config.vars_copy.display()
        )
    })?;
    let retry_source_disk_hashes = qemu_writable_disk_hashes(config, read_only_boot_disk, scope)?;
    let first_error = match run(config) {
        Ok(status) => return Ok(status),
        Err(error) => error,
    };
    let serial = fs::read_to_string(config.serial_log).map_err(|read_error| {
        format!(
            "{first_error}; cannot inspect pre-guest timeout log {}: {read_error}",
            config.serial_log.display()
        )
    })?;
    if !has_pre_guest_firmware_timeout_signature(&first_error, &serial) {
        return Err(first_error);
    }

    fs::copy(config.serial_log, &first_serial).map_err(|error| {
        format!(
            "{first_error}; cannot preserve first-attempt log {}: {error}",
            first_serial.display()
        )
    })?;
    fs::copy(config.vars_copy, &first_vars).map_err(|error| {
        format!(
            "{first_error}; first-attempt log is preserved at {}, but OVMF variables could not be preserved at {}: {error}",
            first_serial.display(),
            first_vars.display()
        )
    })?;
    fs::write(&retry_source_vars_path, &retry_source_vars).map_err(|error| {
        format!(
            "{first_error}; failed-attempt evidence is preserved at {}, but pre-attempt OVMF variables could not be saved at {}: {error}",
            first_vars.display(),
            retry_source_vars_path.display()
        )
    })?;
    let post_attempt_disk_hashes = qemu_writable_disk_hashes(config, read_only_boot_disk, scope)
        .map_err(|error| format!("{first_error}; {error}; retry suppressed"))?;
    let changed_disk = retry_source_disk_hashes
        .iter()
        .zip(&post_attempt_disk_hashes)
        .find(|((before_path, before_hash), (after_path, after_hash))| {
            before_path == after_path && before_hash != after_hash
        });
    if let Some(((path, before_hash), (_, after_hash))) = changed_disk {
        let note = format!(
            "Retry suppressed: a writable {scope} disk changed during the pre-guest timeout. The original failed-attempt state is preserved; no second QEMU attempt was started.\nDisk: {}\nSHA-256 before attempt: {before_hash}\nSHA-256 after attempt: {after_hash}\nFirst-attempt serial log: {}\nFirst post-attempt OVMF variables: {}\nPre-attempt OVMF variables: {}\n",
            path.display(),
            first_serial.display(),
            first_vars.display(),
            retry_source_vars_path.display()
        );
        fs::write(&retry_note, note).map_err(|error| {
            format!(
                "{first_error}; writable {scope} disk {} changed, retry was suppressed, but the note {} could not be written: {error}",
                path.display(),
                retry_note.display()
            )
        })?;
        return Err(format!(
            "{first_error}; writable {scope} disk changed during the pre-guest timeout: {} (before SHA-256 {before_hash}, after {after_hash}); retry suppressed and state recorded in {}",
            path.display(),
            retry_note.display()
        ));
    }

    fs::write(config.vars_copy, &retry_source_vars).map_err(|error| {
        format!(
            "{first_error}; failed-attempt and pre-attempt OVMF variables are preserved at {} and {}, but the pre-attempt state could not be restored at {}: {error}",
            first_vars.display(),
            retry_source_vars_path.display(),
            config.vars_copy.display()
        )
    })?;

    match run(config) {
        Ok(status) => {
            fs::write(
                &retry_note,
                format!(
                    "The first {scope} QEMU attempt timed out before the guest kernel-start marker. QMP reported a running CPU and captured registers and an instruction window. Before retry, the exact pre-attempt OVMF variables were restored and all writable {scope} disks were verified unchanged by SHA-256, so the boot journal and disk state start from the same state. The retry reached its configured acceptance marker with QEMU exit status {status}.\nInitial error: {first_error}\nWritable disk SHA-256 values before the attempt: {retry_source_disk_hashes:?}\nPreserved first serial log: {}\nPreserved first post-attempt OVMF variables: {}\nPreserved pre-attempt OVMF variables: {}\n",
                    first_serial.display(),
                    first_vars.display(),
                    retry_source_vars_path.display()
                ),
            )
            .map_err(|error| {
                format!(
                    "{scope} QEMU retry reached its marker (exit {status}) but retry evidence could not be written at {}: {error}",
                    retry_note.display()
                )
            })?;
            Ok(status)
        }
        Err(retry_error) => Err(format!(
            "{first_error}; one pre-guest retry also failed ({retry_error}); first-attempt evidence is {}, {}",
            first_serial.display(),
            first_vars.display()
        )),
    }
}

fn run_m27_m13_fixture_with_pre_guest_retry(
    root: &Path,
    config: &QemuConfig<'_>,
) -> Result<i32, String> {
    run_headless_with_pre_guest_retry_using(config, false, "M27", |config| {
        run_m13_qemu_with_http_fixture(root, config)
    })
}

fn run_m22_guest_boot_with_pre_guest_retry(config: &QemuConfig<'_>) -> Result<i32, String> {
    // Each M22 process boot starts from the vars template; reuse that exact
    // state within the guarded retry so it cannot advance twice.
    initialize_ovmf_vars(config.ovmf_vars_template, config.vars_copy)?;
    run_headless_with_pre_guest_retry_using(config, false, "M22", |config| {
        run_qemu_reusing_ovmf_vars(config)
    })
}

fn execute_m22_inner(root: &Path, probe: &dyn HostProbe) -> CommandResult {
    let run_id = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_nanos().to_string(),
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m22: system clock: {error}")),
    };
    let host = match resolve_qemu_host(root, probe, "m22") {
        Ok(host) => host,
        Err(error) => return failure(EXIT_CONFIG_ERROR, error),
    };
    let artifacts = match ensure_owned_directory(root, Path::new("out").join("artifacts")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m22: {error}")),
    };
    let logs = match ensure_owned_directory(root, Path::new("out").join("logs")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m22: {error}")),
    };
    let image_name = format!("nagi-0.1-m22-history-{run_id}.img");
    let image_path = artifacts.join(&image_name);
    let persistent_disk = artifacts.join(format!("nagi-0.1-m22-history-user-data-{run_id}.img"));
    let vars_copy = artifacts.join(format!("nagi-0.1-m22-history-vars-{run_id}.fd"));
    let bootstrap_log = logs.join(format!("m22-history-bootstrap-{run_id}.log"));
    let firmware_timeout_log = path_with_suffix(&bootstrap_log, ".pre-guest-timeout-1");
    let firmware_timeout_vars = path_with_suffix(&bootstrap_log, ".ovmf-vars.pre-guest-timeout-1");
    let firmware_retry_source_vars =
        path_with_suffix(&bootstrap_log, ".ovmf-vars.pre-guest-retry-source-1");
    let firmware_retry_note = path_with_suffix(&bootstrap_log, ".pre-guest-retry-1.txt");
    let boot_logs = (1..=3)
        .map(|boot_index| logs.join(format!("m22-history-{run_id}-boot-{boot_index}.log")))
        .collect::<Vec<_>>();
    for path in [
        &image_path,
        &persistent_disk,
        &vars_copy,
        &bootstrap_log,
        &firmware_timeout_log,
        &firmware_timeout_vars,
        &firmware_retry_source_vars,
        &firmware_retry_note,
        &boot_logs[0],
        &boot_logs[1],
        &boot_logs[2],
    ] {
        match fs::symlink_metadata(path) {
            Ok(_) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!(
                        "m22: refusing to overwrite existing run artifact {}",
                        path.display()
                    ),
                );
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!("m22: cannot inspect {}: {error}", path.display()),
                );
            }
        }
    }
    let image_result =
        execute_image_with_isolated_clients(root, "m22-history,m21-action-ipc", &image_name);
    if image_result.exit_code != EXIT_SUCCESS {
        return image_result;
    }
    let had_persistent_disk = match ensure_persistent_disk(&persistent_disk) {
        Ok(existing) => existing,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m22: {error}")),
    };
    if had_persistent_disk {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "m22: unique User Data path unexpectedly existed: {}",
                persistent_disk.display()
            ),
        );
    }
    // The repeated M28 gate boots M22 several times with persistent UEFI
    // state. Preserve all guest markers while allowing a slow firmware start.
    let timeout = Duration::from_secs(180);

    let mut firmware_retry_evidence = None;
    if !had_persistent_disk {
        let config = QemuConfig {
            qemu: &host.qemu,
            ovmf_code: &host.ovmf_code,
            ovmf_vars_template: &host.ovmf_vars,
            disk_image: &image_path,
            persistent_disk: &persistent_disk,
            vars_copy: &vars_copy,
            serial_log: &bootstrap_log,
            acceptance_marker: NAGI_WRITE_MARKER,
            timeout,
        };
        if let Err(error) = run_m22_guest_boot_with_pre_guest_retry(&config) {
            return failure(EXIT_CONFIG_ERROR, format!("m22: bootstrap boot: {error}"));
        }
        if firmware_timeout_log.is_file() {
            firmware_retry_evidence = Some((firmware_timeout_log, firmware_timeout_vars));
        }
        match fs::read_to_string(&bootstrap_log) {
            Ok(serial) if serial.contains(NAGI_WRITE_MARKER) => {}
            Ok(_) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!(
                        "m22: bootstrap did not print `{NAGI_WRITE_MARKER}` (log {})",
                        bootstrap_log.display()
                    ),
                );
            }
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!("m22: cannot read {}: {error}", bootstrap_log.display()),
                );
            }
        }
    }

    let mut saw_move = false;
    let mut saw_undo = false;
    let mut saw_activity_ledger_commit = false;
    let mut saw_activity_ledger_undo = false;
    let mut saw_copy = false;
    let mut saw_copy_transaction = false;
    let mut last_log = PathBuf::new();
    for (boot_index, log_path) in boot_logs.iter().enumerate() {
        let config = QemuConfig {
            qemu: &host.qemu,
            ovmf_code: &host.ovmf_code,
            ovmf_vars_template: &host.ovmf_vars,
            disk_image: &image_path,
            persistent_disk: &persistent_disk,
            vars_copy: &vars_copy,
            serial_log: log_path,
            acceptance_marker: "Nagi M13 acceptance PASS",
            timeout,
        };
        let final_status = match run_m22_guest_boot_with_pre_guest_retry(&config) {
            Ok(status) => status,
            Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m22: guest boot: {error}")),
        };
        last_log = log_path.clone();
        let serial = match fs::read_to_string(log_path) {
            Ok(serial) => serial,
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!("m22: cannot read {}: {error}", log_path.display()),
                );
            }
        };
        for marker in [
            "Nagi Kernel started",
            "Nagi M3 CPU scheduler fairness PASS",
            "Nagi M3 acceptance PASS",
            "Nagi M7 VirtIO Block PASS",
            "Nagi M13 C POSIX PASS",
            "Nagi M24 semantic index ready PASS",
            "Nagi M19 guest search persistence PASS",
            "Nagi M22 file.search Activity Ledger PASS",
            "Nagi M13 acceptance PASS",
        ] {
            if !serial.contains(marker) {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!(
                        "m22: guest did not print `{marker}` (QEMU exit {final_status}; log {})",
                        log_path.display()
                    ),
                );
            }
        }
        if boot_index == 2 && !serial.contains("Nagi M24 semantic index persistence PASS") {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m22: final restart did not verify the durable M24 semantic index (QEMU exit {final_status}; log {})",
                    log_path.display()
                ),
            );
        }
        if boot_index == 0 && !had_persistent_disk {
            if let Some(marker) = [
                "Nagi M21 foreign isolated caller denied PASS",
                "Nagi M21 file.search isolated caller PASS",
                "Nagi M22 foreign isolated caller denied PASS",
                "Nagi M22 file.move isolated caller PASS",
                "Nagi M22 file.copy isolated caller PASS",
            ]
            .into_iter()
            .find(|marker| !serial.contains(marker))
            {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!(
                        "m22: fresh guest did not print isolated-caller marker `{marker}` (QEMU exit {final_status}; log {})",
                        log_path.display()
                    ),
                );
            }
        }
        if boot_index == 0
            && !had_persistent_disk
            && !serial.contains("Nagi M21 file.move Plan Validate Execute PASS")
        {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m22: fresh guest did not pass the M21 file.move Plan/Validate/Execute gate (QEMU exit {final_status}; log {})",
                    log_path.display()
                ),
            );
        }
        if boot_index == 0 && !serial.contains("Nagi M21 file.copy Plan Validate Execute PASS") {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m22: fresh guest did not pass the M21 file.copy Plan/Validate/Execute gate (QEMU exit {final_status}; log {})",
                    log_path.display()
                ),
            );
        }
        if boot_index == 0
            && !had_persistent_disk
            && !serial.contains("Nagi M21 plan rejection validation PASS")
        {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m22: fresh guest did not reject malformed, unsupported, out-of-context, and capability-denied plans before execution (QEMU exit {final_status}; log {})",
                    log_path.display()
                ),
            );
        }
        if boot_index == 0
            && !had_persistent_disk
            && !serial.contains("Nagi M21 partial execution failure validation PASS")
        {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m22: fresh guest did not verify Executor partial-failure reporting (QEMU exit {final_status}; log {})",
                    log_path.display()
                ),
            );
        }
        saw_activity_ledger_commit |= serial.contains("Nagi M22 AI Activity Ledger committed PASS");
        saw_activity_ledger_undo |= serial.contains("Nagi M22 AI Activity Ledger undo result PASS");
        saw_copy |= serial.contains("Nagi M21 file.copy Plan Validate Execute PASS");
        saw_copy_transaction |=
            serial.contains("Nagi M22 file.copy prepared transaction persisted PASS");
        saw_move |= serial.contains("Nagi M22 move group persisted in guest VFS PASS")
            || serial.contains("Nagi M22 recovered prepared move group PASS");
        saw_undo |= serial.contains("Nagi M22 composite undo applied and persisted PASS");
        if boot_index == 2 && !serial.contains("Nagi M22 archive restart and restored files PASS") {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m22: final restart did not verify restored files and NH16 state (QEMU exit {final_status}; log {})",
                    log_path.display()
                ),
            );
        }
        if boot_index == 2 && !serial.contains("Nagi M22 AI Activity Ledger undo result PASS") {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m22: final restart did not verify the separate AI Activity Ledger (QEMU exit {final_status}; log {})",
                    log_path.display()
                ),
            );
        }
    }
    if !saw_move && !saw_undo && had_persistent_disk {
        let final_serial = match fs::read_to_string(&last_log) {
            Ok(serial) => serial,
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!("m22: cannot read {}: {error}", last_log.display()),
                );
            }
        };
        if !final_serial.contains("Nagi M22 archive restart and restored files PASS") {
            return failure(
                EXIT_CONFIG_ERROR,
                "m22: existing guest archive neither completed the move/undo flow nor verified restored state",
            );
        }
    }
    if !had_persistent_disk && !saw_activity_ledger_commit {
        return failure(
            EXIT_CONFIG_ERROR,
            "m22: fresh guest did not persist an AI Activity Ledger commit record",
        );
    }
    if !saw_activity_ledger_undo {
        return failure(
            EXIT_CONFIG_ERROR,
            "m22: guest did not persist an AI Activity Ledger undo result",
        );
    }
    if !saw_copy || !saw_copy_transaction {
        return failure(
            EXIT_CONFIG_ERROR,
            "m22: guest did not persist the bounded file.copy action and NH16 Create transaction",
        );
    }

    let mut lines = vec![format!(
        "PASS M21/M22 guest fixture: VFS file.move and file.copy, NH16 Create/Move transactions, Activity Ledger, composite Undo, and restored state survived QEMU restarts (log {})",
        last_log.display()
    )];
    if let Some((serial_log, vars)) = firmware_retry_evidence {
        lines.push(format!(
            "INFO M22 bootstrap recovered from a pre-guest OVMF timeout after one bounded retry; failed attempt preserved at {} and {}",
            serial_log.display(),
            vars.display()
        ));
    }
    CommandResult {
        exit_code: EXIT_SUCCESS,
        lines,
    }
}

fn execute_m27(root: &Path, probe: &dyn HostProbe) -> CommandResult {
    let run_id = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_nanos().to_string(),
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m27: cannot create a unique evidence identifier: {error}"),
            );
        }
    };
    let bootstrap_image_name = format!("nagi-0.1-m27-bootstrap-{run_id}.img");
    let slots_image_name = format!("nagi-0.1-m27-ab-slots-{run_id}.img");
    let healthy_slots_image_name = format!("nagi-0.1-m27-ab-healthy-slots-{run_id}.img");
    let recovery_image_name = format!("nagi-0.1-m27-recovery-{run_id}.img");
    let bootstrap_image_result = execute_image_with_features(root, None, &bootstrap_image_name);
    if bootstrap_image_result.exit_code != EXIT_SUCCESS {
        return bootstrap_image_result;
    }
    let recovery_init_args = [
        "build",
        "-p",
        "nagi-init",
        "--features",
        "m27-recovery-undo-acceptance",
        "--target",
        "targets/x86_64-unknown-nagi-user.json",
        "-Zbuild-std=core,alloc,compiler_builtins",
        "--release",
        "--locked",
    ];
    let recovery_init_build = run_cargo(root, "M27 Recovery init", &recovery_init_args);
    if recovery_init_build.exit_code != EXIT_SUCCESS {
        return recovery_init_build;
    }
    let recovery_init_path = root
        .join("target")
        .join("x86_64-unknown-nagi-user")
        .join("release")
        .join("nagi-init");
    let recovery_init = match fs::read(&recovery_init_path) {
        Ok(bytes) => bytes,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m27: cannot read {}: {error}", recovery_init_path.display()),
            );
        }
    };
    let init_args = [
        "build",
        "-p",
        "nagi-init",
        "--features",
        "m10-desktop,m27-ro-vfs-check",
        "--target",
        "targets/x86_64-unknown-nagi-user.json",
        "-Zbuild-std=core,alloc,compiler_builtins",
        "--release",
    ];
    let image_result = execute_image_with_init_build_env_using_writer_and_recovery(
        root,
        &init_args,
        None,
        ImageBuildRequest {
            image_name: &slots_image_name,
            cargo_env: &[],
            recovery_init: Some(&recovery_init),
            image_writer: write_m27_broken_slot_image,
            external_model_store_file: None,
            build_features: ImageBuildFeatures {
                kernel: &[],
                loader: &["m27-ab-slot-acceptance"],
            },
        },
    );
    if image_result.exit_code != EXIT_SUCCESS {
        return image_result;
    }
    let healthy_image_result = execute_image_with_init_build_env_using_writer_and_recovery(
        root,
        &init_args,
        None,
        ImageBuildRequest {
            image_name: &healthy_slots_image_name,
            cargo_env: &[],
            recovery_init: Some(&recovery_init),
            image_writer: write_m27_healthy_slot_image,
            external_model_store_file: None,
            build_features: ImageBuildFeatures {
                kernel: &[],
                loader: &["m27-ab-slot-acceptance"],
            },
        },
    );
    if healthy_image_result.exit_code != EXIT_SUCCESS {
        return healthy_image_result;
    }
    let recovery_image_result = execute_image_with_init_build_env_using_writer_and_recovery(
        root,
        &init_args,
        None,
        ImageBuildRequest {
            image_name: &recovery_image_name,
            cargo_env: &[],
            recovery_init: Some(&recovery_init),
            image_writer: write_m27_recovery_image,
            external_model_store_file: None,
            build_features: ImageBuildFeatures {
                kernel: &[],
                loader: &["m27-ab-slot-acceptance"],
            },
        },
    );
    if recovery_image_result.exit_code != EXIT_SUCCESS {
        return recovery_image_result;
    }

    let host = match resolve_qemu_host(root, probe, "m27") {
        Ok(host) => host,
        Err(error) => return failure(EXIT_CONFIG_ERROR, error),
    };
    let evidence = match ensure_owned_directory(
        root,
        Path::new("out")
            .join("evidence")
            .join(format!("m27-ab-rollback-{run_id}")),
    ) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m27: {error}")),
    };
    let bootstrap_image_path = root
        .join("out")
        .join("artifacts")
        .join(&bootstrap_image_name);
    let slots_image_path = root.join("out").join("artifacts").join(&slots_image_name);
    let healthy_slots_image_path = root
        .join("out")
        .join("artifacts")
        .join(&healthy_slots_image_name);
    let recovery_image_path = root
        .join("out")
        .join("artifacts")
        .join(&recovery_image_name);
    let persistent_disk = evidence.join("user-data.img");
    let vars_copy = evidence.join("OVMF_VARS.fd");
    if let Err(error) = ensure_persistent_disk(&persistent_disk) {
        return failure(
            EXIT_CONFIG_ERROR,
            format!("m27: create user-data disk: {error}"),
        );
    }
    if let Err(error) = initialize_ovmf_vars(&host.ovmf_vars, &vars_copy) {
        return failure(
            EXIT_CONFIG_ERROR,
            format!("m27: initialize OVMF variables: {error}"),
        );
    }

    let bootstrap_log = evidence.join("bootstrap.log");
    let bootstrap_config = QemuConfig {
        qemu: &host.qemu,
        ovmf_code: &host.ovmf_code,
        ovmf_vars_template: &host.ovmf_vars,
        disk_image: &bootstrap_image_path,
        persistent_disk: &persistent_disk,
        vars_copy: &vars_copy,
        serial_log: &bootstrap_log,
        // The persistent-write marker comes before the reboot-required
        // marker. Wait for the latter so QEMU cannot stop between them.
        acceptance_marker: M27_BOOTSTRAP_COMPLETION_MARKER,
        timeout: Duration::from_secs(90),
    };
    let bootstrap_status = match run_m27_headless_with_pre_guest_retry(&bootstrap_config, false) {
        Ok(status) => status,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m27: guest storage bootstrap failed: {error}"),
            );
        }
    };
    let bootstrap_serial = match fs::read_to_string(&bootstrap_log) {
        Ok(serial) => serial,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m27: cannot read {}: {error}", bootstrap_log.display()),
            );
        }
    };
    if !m27_bootstrap_markers_present(&bootstrap_serial) {
        for marker in [NAGI_WRITE_MARKER, M27_BOOTSTRAP_COMPLETION_MARKER] {
            if bootstrap_serial.contains(marker) {
                continue;
            }
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m27: bootstrap did not print `{marker}` (QEMU exit {bootstrap_status}; log {})",
                    bootstrap_log.display()
                ),
            );
        }
    }
    if bootstrap_serial.contains("Nagi M27 UEFI variable journal persistence PASS") {
        return failure(
            EXIT_CONFIG_ERROR,
            "m27: normal bootstrap loader unexpectedly modified the boot-control journal",
        );
    }

    let recovery_vars = evidence.join("recovery-OVMF_VARS.fd");
    if let Err(error) = initialize_ovmf_vars(&host.ovmf_vars, &recovery_vars) {
        return failure(
            EXIT_CONFIG_ERROR,
            format!("m27: initialize Recovery OVMF variables: {error}"),
        );
    }

    let recovery_undo_image_name = format!("nagi-0.1-m27-recovery-undo-{run_id}.img");
    let recovery_undo_image_result = execute_image_with_isolated_clients(
        root,
        "m22-history,m21-action-ipc",
        &recovery_undo_image_name,
    );
    if recovery_undo_image_result.exit_code != EXIT_SUCCESS {
        return recovery_undo_image_result;
    }
    let recovery_undo_image_path = root
        .join("out")
        .join("artifacts")
        .join(&recovery_undo_image_name);
    let committed_log = evidence.join("recovery-committed-undo-fixture.log");
    let committed_config = QemuConfig {
        qemu: &host.qemu,
        ovmf_code: &host.ovmf_code,
        ovmf_vars_template: &host.ovmf_vars,
        disk_image: &recovery_undo_image_path,
        persistent_disk: &persistent_disk,
        vars_copy: &recovery_vars,
        serial_log: &committed_log,
        acceptance_marker: "Nagi M13 acceptance PASS",
        timeout: Duration::from_secs(90),
    };
    let committed_status = run_m27_m13_fixture_with_pre_guest_retry(root, &committed_config);
    let committed_status = match committed_status {
        Ok(status) => status,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m27: Recovery Undo guest fixture: {error}"),
            );
        }
    };
    let committed_serial = match fs::read_to_string(&committed_log) {
        Ok(serial) => serial,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m27: cannot read {}: {error}", committed_log.display()),
            );
        }
    };
    for marker in [
        "Nagi M21 file.move Plan Validate Execute PASS",
        "Nagi M22 AI Activity Ledger committed PASS",
        "Nagi M22 move group persisted in guest VFS PASS",
        "Nagi M13 acceptance PASS",
    ] {
        if !committed_serial.contains(marker) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m27: Recovery Undo fixture did not print `{marker}` (QEMU exit {committed_status}; log {})",
                    committed_log.display()
                ),
            );
        }
    }

    let recovery_log = evidence.join("recovery-boot.log");
    let recovery_config = QemuConfig {
        qemu: &host.qemu,
        ovmf_code: &host.ovmf_code,
        ovmf_vars_template: &host.ovmf_vars,
        disk_image: &recovery_image_path,
        persistent_disk: &persistent_disk,
        vars_copy: &recovery_vars,
        serial_log: &recovery_log,
        acceptance_marker: "Nagi M27 Recovery command help PASS",
        timeout: Duration::from_secs(90),
    };
    const RECOVERY_MENU_EVENTS: [&str; 2] = [
        r#"{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"r"}}}]}}"#,
        r#"{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"r"}}}]}}"#,
    ];
    const RECOVERY_COMMANDS: &[u8] =
        b"check\nlog\nfiles\nslots\nhistory\nundo-conflict-test\nundo\nhelp\n";
    let recovery_status = match run_qemu_gui_with_read_only_boot_disk_and_events_and_serial_input(
        &recovery_config,
        "Nagi M27 Recovery boot menu READY",
        &RECOVERY_MENU_EVENTS,
        "Nagi M27 Recovery console READY",
        RECOVERY_COMMANDS,
    ) {
        Ok(status) => status,
        Err(error) => {
            return failure(EXIT_CONFIG_ERROR, format!("m27: Recovery QEMU: {error}"));
        }
    };
    let recovery_serial = match fs::read_to_string(&recovery_log) {
        Ok(serial) => serial,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m27: cannot read {}: {error}", recovery_log.display()),
            );
        }
    };
    for marker in [
        "Nagi M27 manual selection: Recovery; boot journal unchanged PASS",
        "Nagi M27 Recovery Environment START",
        "Nagi M27 Recovery VFS check PASS files=",
        "Nagi M27 Recovery current-boot log PASS",
        "Nagi M27 Recovery files PASS",
        "Nagi M27 Recovery history PASS entries=",
        "Nagi M27 Recovery same-path move content conflict PASS",
        "Nagi M27 Recovery undo preflight conflict PASS",
        "Nagi M27 Recovery interrupted undo retry PASS",
        "Nagi M27 Recovery NH16 undo PASS",
        "Nagi M27 Recovery command help PASS",
    ] {
        if !recovery_serial.contains(marker) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m27: Recovery boot did not print `{marker}` (QEMU exit {recovery_status}; log {})",
                    recovery_log.display()
                ),
            );
        }
    }
    if recovery_serial.contains("Nagi M27 persistence decision:")
        || recovery_serial.contains("Nagi M27 readiness persisted")
    {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "m27: Recovery changed the A/B boot decision or reported trial readiness (log {})",
                recovery_log.display()
            ),
        );
    }

    let restored_log = evidence.join("recovery-undo-restart-verification.log");
    let restored_config = QemuConfig {
        qemu: &host.qemu,
        ovmf_code: &host.ovmf_code,
        ovmf_vars_template: &host.ovmf_vars,
        disk_image: &recovery_undo_image_path,
        persistent_disk: &persistent_disk,
        vars_copy: &recovery_vars,
        serial_log: &restored_log,
        acceptance_marker: "Nagi M13 acceptance PASS",
        timeout: Duration::from_secs(90),
    };
    let restored_status = match run_m27_m13_fixture_with_pre_guest_retry(root, &restored_config) {
        Ok(status) => status,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m27: Recovery Undo restart verification: {error}"),
            );
        }
    };
    let restored_serial = match fs::read_to_string(&restored_log) {
        Ok(serial) => serial,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m27: cannot read {}: {error}", restored_log.display()),
            );
        }
    };
    for marker in [
        "Nagi M22 file.search Activity Ledger PASS",
        "Nagi M22 archive restart and restored files PASS",
        "Nagi M22 AI Activity Ledger undo result PASS",
        "Nagi M13 acceptance PASS",
    ] {
        if !restored_serial.contains(marker) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m27: Recovery Undo restart did not print `{marker}` (QEMU exit {restored_status}; log {})",
                    restored_log.display()
                ),
            );
        }
    }

    let first_trial_log = evidence.join("boot-1.log");
    let first_trial_config = QemuConfig {
        qemu: &host.qemu,
        ovmf_code: &host.ovmf_code,
        ovmf_vars_template: &host.ovmf_vars,
        disk_image: &slots_image_path,
        persistent_disk: &persistent_disk,
        vars_copy: &vars_copy,
        serial_log: &first_trial_log,
        acceptance_marker: "Nagi Loader: invalid ELF",
        timeout: Duration::from_secs(90),
    };
    let first_trial_status = match run_m27_headless_with_pre_guest_retry(&first_trial_config, true)
    {
        Ok(status) => status,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m27: initial System B trial: {error}"),
            );
        }
    };
    let first_trial_serial = match fs::read_to_string(&first_trial_log) {
        Ok(serial) => serial,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m27: cannot read {}: {error}", first_trial_log.display()),
            );
        }
    };
    for marker in [
        "Nagi M27 persistence decision: trial attempt=1 slot=B",
        "Nagi M27 UEFI variable journal persistence PASS",
        "Nagi M27 trial payload rejected slot=B",
        "Nagi Loader: invalid ELF",
    ] {
        if !first_trial_serial.contains(marker) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m27: initial trial did not print `{marker}` (QEMU exit {first_trial_status}; log {})",
                    first_trial_log.display()
                ),
            );
        }
    }
    if !m27_trial_failure_observed(&first_trial_serial) {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "m27: initial malformed System B trial did not reach the rejection path (QEMU exit {first_trial_status}; log {})",
                first_trial_log.display()
            ),
        );
    }

    let recovery_journal_log = evidence.join("recovery-preserved-trial-journal.log");
    let recovery_journal_config = QemuConfig {
        qemu: &host.qemu,
        ovmf_code: &host.ovmf_code,
        ovmf_vars_template: &host.ovmf_vars,
        disk_image: &recovery_image_path,
        persistent_disk: &persistent_disk,
        vars_copy: &vars_copy,
        serial_log: &recovery_journal_log,
        acceptance_marker: "Nagi M27 Recovery command help PASS",
        // Recovery boots through the interactive UEFI path and runs the
        // four-vCPU M3 scheduler self-test before the console marker. Allow
        // additional TCG time while keeping the exact guest acceptance gate.
        timeout: Duration::from_secs(180),
    };
    let recovery_commands = b"check\nlog\nfiles\nslots\nhelp\n";
    let recovery_journal_status =
        match run_qemu_gui_reusing_ovmf_vars_with_read_only_boot_disk_and_events_and_serial_input(
            &recovery_journal_config,
            "Nagi M27 Recovery boot menu READY",
            &M27_RECOVERY_MENU_EVENTS,
            "Nagi M27 Recovery console READY",
            recovery_commands,
        ) {
            Ok(status) => status,
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!("m27: Recovery with pending System B trial: {error}"),
                );
            }
        };
    let recovery_journal_serial = match fs::read_to_string(&recovery_journal_log) {
        Ok(serial) => serial,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m27: cannot read {}: {error}",
                    recovery_journal_log.display()
                ),
            );
        }
    };
    for marker in [
        "Nagi M27 boot menu: confirmed=A pending=B",
        "Nagi M27 manual selection: Recovery; boot journal unchanged PASS",
        "Nagi M27 Recovery VFS check PASS files=",
        "Nagi M27 Recovery command help PASS",
    ] {
        if !recovery_journal_serial.contains(marker) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m27: Recovery with a pending trial did not print `{marker}` (QEMU exit {recovery_journal_status}; log {})",
                    recovery_journal_log.display()
                ),
            );
        }
    }
    if recovery_journal_serial.contains("Nagi M27 persistence decision:")
        || recovery_journal_serial.contains("Nagi M27 readiness persisted")
    {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "m27: Recovery changed the pending journal or reported trial readiness (log {})",
                recovery_journal_log.display()
            ),
        );
    }

    let second_trial_log = evidence.join("recovery-follow-up-trial.log");
    let second_trial_config = QemuConfig {
        serial_log: &second_trial_log,
        ..first_trial_config
    };
    let second_trial_status =
        match run_m27_headless_with_pre_guest_retry(&second_trial_config, true) {
            Ok(status) => status,
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!("m27: Recovery journal trial continuation: {error}"),
                );
            }
        };
    let second_trial_serial = match fs::read_to_string(&second_trial_log) {
        Ok(serial) => serial,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m27: cannot read {}: {error}", second_trial_log.display()),
            );
        }
    };
    for marker in [
        "Nagi M27 persistence decision: trial attempt=2 slot=B",
        "Nagi M27 UEFI variable journal persistence PASS",
        "Nagi M27 trial payload rejected slot=B",
        "Nagi Loader: invalid ELF",
    ] {
        if !second_trial_serial.contains(marker) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m27: Recovery follow-up trial did not print `{marker}` (QEMU exit {second_trial_status}; log {})",
                    second_trial_log.display()
                ),
            );
        }
    }
    if !m27_trial_failure_observed(&second_trial_serial) {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "m27: Recovery follow-up malformed System B trial did not reach rejection (QEMU exit {second_trial_status}; log {})",
                second_trial_log.display()
            ),
        );
    }

    let expected_decisions = [
        (3, "Nagi M27 persistence decision: trial attempt=3 slot=B"),
        (4, "Nagi M27 persistence decision: rollback slot=A"),
        (5, "Nagi M27 persistence decision: confirmed slot=A"),
    ];
    let mut final_log = PathBuf::new();
    for (boot_number, expected_decision) in &expected_decisions {
        let log_path = evidence.join(format!("boot-{boot_number}.log"));
        let is_trial_boot = *boot_number == 3;
        let acceptance_marker = if is_trial_boot {
            // Stop only after the loader has printed its actual invalid-ELF
            // failure. The preceding rejection marker alone is not enough:
            // wait_for_qemu kills QEMU as soon as its acceptance marker appears.
            "Nagi Loader: invalid ELF"
        } else {
            GUEST_ACCEPTANCE_MARKER
        };
        let config = QemuConfig {
            qemu: &host.qemu,
            ovmf_code: &host.ovmf_code,
            ovmf_vars_template: &host.ovmf_vars,
            disk_image: &slots_image_path,
            persistent_disk: &persistent_disk,
            vars_copy: &vars_copy,
            serial_log: &log_path,
            acceptance_marker,
            // Repeated TCG boots through the persisted journal sequence can
            // exceed the standalone boot budget. Keep guest markers as the
            // acceptance gate while allowing the sequence to finish under
            // host load.
            timeout: Duration::from_secs(180),
        };
        let status = match run_m27_headless_with_pre_guest_retry(&config, true) {
            Ok(status) => status,
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!("m27: QEMU boot {boot_number} failed: {error}"),
                );
            }
        };
        let serial = match fs::read_to_string(&log_path) {
            Ok(serial) => serial,
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!("m27: cannot read {}: {error}", log_path.display()),
                );
            }
        };
        for marker in [
            *expected_decision,
            "Nagi M27 UEFI variable journal persistence PASS",
            acceptance_marker,
        ] {
            if !serial.contains(marker) {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!(
                        "m27: boot {boot_number} did not print `{marker}` (QEMU exit {status}; log {})",
                        log_path.display()
                    ),
                );
            }
        }
        if is_trial_boot && !m27_trial_failure_observed(&serial) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m27: boot {boot_number} did not complete the broken-slot failure path (QEMU exit {status}; log {})",
                    log_path.display()
                ),
            );
        }
        if *boot_number >= 4 && !serial.contains("Nagi M7 persistent read PASS") {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m27: fallback boot {boot_number} did not verify persistent user data (QEMU exit {status}; log {})",
                    log_path.display()
                ),
            );
        }
        if *boot_number >= 4 && !serial.contains("Nagi M27 read-only VFS check PASS") {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m27: fallback boot {boot_number} did not complete the read-only VFS integrity check (QEMU exit {status}; log {})",
                    log_path.display()
                ),
            );
        }
        final_log = log_path;
    }

    let readiness_relative = Path::new("out/evidence")
        .join(format!("m27-ab-rollback-{run_id}"))
        .join("readiness-promotion");
    let readiness_evidence = match ensure_owned_directory(root, readiness_relative) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m27: {error}")),
    };
    let readiness_vars = readiness_evidence.join("OVMF_VARS.fd");
    if let Err(error) = initialize_ovmf_vars(&host.ovmf_vars, &readiness_vars) {
        return failure(
            EXIT_CONFIG_ERROR,
            format!("m27: initialize readiness OVMF variables: {error}"),
        );
    }
    let readiness_boots = [
        (
            "trial-boot.log",
            "Nagi M27 persistence decision: trial attempt=1 slot=B",
            false,
        ),
        (
            "promotion-boot.log",
            "Nagi M27 persistence decision: confirmed slot=B",
            true,
        ),
        (
            "confirmed-boot.log",
            "Nagi M27 persistence decision: confirmed slot=B",
            false,
        ),
    ];
    // Confirmation and post-promotion boots can exceed 90 seconds under
    // repeated TCG load. Keep the exact readiness and desktop markers.
    let readiness_timeout = Duration::from_secs(180);
    for (index, (log_name, expected_decision, expect_consumed_readiness)) in
        readiness_boots.iter().enumerate()
    {
        let log_path = readiness_evidence.join(log_name);
        let config = QemuConfig {
            qemu: &host.qemu,
            ovmf_code: &host.ovmf_code,
            ovmf_vars_template: &host.ovmf_vars,
            disk_image: &healthy_slots_image_path,
            persistent_disk: &persistent_disk,
            vars_copy: &readiness_vars,
            serial_log: &log_path,
            acceptance_marker: "Nagi M10 desktop READY",
            timeout: readiness_timeout,
        };
        let status = match run_m27_headless_with_pre_guest_retry(&config, true) {
            Ok(status) => status,
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!("m27: readiness QEMU boot {} failed: {error}", index + 1),
                );
            }
        };
        let serial = match fs::read_to_string(&log_path) {
            Ok(serial) => serial,
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!("m27: cannot read {}: {error}", log_path.display()),
                );
            }
        };
        for marker in [*expected_decision, "Nagi M10 desktop READY"] {
            if !serial.contains(marker) {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!(
                        "m27: readiness boot {} did not print `{marker}` (QEMU exit {status}; log {})",
                        index + 1,
                        log_path.display()
                    ),
                );
            }
        }
        if index == 0 && !m27_readiness_persisted_before_desktop(&serial) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m27: healthy trial did not persist readiness before desktop readiness (QEMU exit {status}; log {})",
                    log_path.display()
                ),
            );
        }
        if *expect_consumed_readiness && !m27_readiness_consumed_before_promotion(&serial) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m27: loader did not consume the guest readiness record (QEMU exit {status}; log {})",
                    log_path.display()
                ),
            );
        }
        if serial.contains("Nagi M27 readiness persistence FAIL") {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "m27: readiness persistence failed during promotion acceptance (log {})",
                    log_path.display()
                ),
            );
        }
    }

    if let Err(error) = execute_m27_gpt_acceptance(root, &host, &evidence, &run_id, &recovery_init)
    {
        return failure(EXIT_CONFIG_ERROR, format!("m27 GPT acceptance: {error}"));
    }

    CommandResult {
        exit_code: EXIT_SUCCESS,
        lines: vec![
            format!(
                "PASS M27 A/B and Recovery: three malformed System B trials rolled back to persistent System A; a healthy System B trial was promoted after guest readiness; Recovery boot left the journal unchanged and undid a committed M22 file.move group across restart (evidence {})",
                evidence.display()
            ),
            format!("Final serial log: {}", final_log.display()),
        ],
    }
}

fn execute_m27_gpt_acceptance(
    root: &Path,
    host: &QemuHost,
    evidence: &Path,
    run_id: &str,
    recovery_init: &[u8],
) -> Result<(), String> {
    let broken_image_name = format!("nagi-0.1-m27-gpt-broken-{run_id}.qcow2");
    let healthy_image_name = format!("nagi-0.1-m27-gpt-healthy-{run_id}.qcow2");
    let init_args = [
        "build",
        "-p",
        "nagi-init",
        "--features",
        // ADR 0063: a trial is confirmed only by a signed-in desktop.
        "desktop-login",
        "--target",
        "targets/x86_64-unknown-nagi-user.json",
        "-Zbuild-std=core,alloc,compiler_builtins",
        "--release",
        "--locked",
    ];
    let boot_features = ImageBuildFeatures {
        kernel: &[],
        loader: &["m27-ab-slot-acceptance"],
    };
    for (image_name, writer) in [
        (
            broken_image_name.as_str(),
            write_m27_gpt_broken_system_b_qcow2 as ImageWriter,
        ),
        (
            healthy_image_name.as_str(),
            write_reference_disk_qcow2 as ImageWriter,
        ),
    ] {
        let result = execute_image_with_init_build_env_using_writer_and_recovery(
            root,
            &init_args,
            None,
            ImageBuildRequest {
                image_name,
                cargo_env: &[],
                recovery_init: Some(recovery_init),
                image_writer: writer,
                external_model_store_file: None,
                build_features: boot_features,
            },
        );
        if result.exit_code != EXIT_SUCCESS {
            return Err(format!("build {image_name}: {}", result.lines.join("; ")));
        }
    }

    let artifacts = root.join("out").join("artifacts");
    let broken_image_path = artifacts.join(&broken_image_name);
    let healthy_image_path = artifacts.join(&healthy_image_name);
    let gpt_evidence_relative = evidence
        .strip_prefix(root)
        .map_err(|error| {
            format!(
                "M27 evidence path {} is outside the repository: {error}",
                evidence.display()
            )
        })?
        .join("gpt-integration");
    let gpt_evidence = ensure_owned_directory(root, gpt_evidence_relative)?;

    let broken_vars = gpt_evidence.join("broken-OVMF_VARS.fd");
    let broken_boot_log = gpt_evidence.join("broken-initialize-user-data.log");
    let broken_boot_config = QemuConfig {
        qemu: &host.qemu,
        ovmf_code: &host.ovmf_code,
        ovmf_vars_template: &host.ovmf_vars,
        disk_image: &broken_image_path,
        persistent_disk: &broken_image_path,
        vars_copy: &broken_vars,
        serial_log: &broken_boot_log,
        acceptance_marker: "Nagi M7 reboot required PASS",
        timeout: Duration::from_secs(180),
    };
    let status = run_qemu_gui_with_events(
        &broken_boot_config,
        "Nagi M27 Recovery boot menu READY",
        &M27_SYSTEM_A_MENU_EVENTS,
    )
    .map_err(|error| format!("GPT broken image User Data bootstrap: {error}"))?;
    let serial = fs::read_to_string(&broken_boot_log)
        .map_err(|error| format!("read {}: {error}", broken_boot_log.display()))?;
    require_m27_gpt_markers(
        "GPT broken image User Data bootstrap",
        status,
        &broken_boot_log,
        &serial,
        &[
            "Nagi M27 manual selection: confirmed slot=A",
            "Nagi M30 GPT partition boot: System A PASS",
            "Nagi slot manifest verified slot=A rollback-index=1 PASS",
            "Nagi M7 ext2 format PASS",
            "Nagi M7 persistent write PASS",
            "Nagi M7 reboot required PASS",
        ],
    )?;

    for attempt in 1..=3 {
        let log = gpt_evidence.join(format!("broken-system-b-trial-{attempt}.log"));
        let config = QemuConfig {
            qemu: &host.qemu,
            ovmf_code: &host.ovmf_code,
            ovmf_vars_template: &host.ovmf_vars,
            disk_image: &broken_image_path,
            persistent_disk: &broken_image_path,
            vars_copy: &broken_vars,
            serial_log: &log,
            acceptance_marker: "Nagi Loader: slot manifest rejected",
            timeout: Duration::from_secs(120),
        };
        let status = run_m27_headless_with_pre_guest_retry(&config, false)
            .map_err(|error| format!("GPT System B trial {attempt}: {error}"))?;
        let serial =
            fs::read_to_string(&log).map_err(|error| format!("read {}: {error}", log.display()))?;
        let expected_decision =
            format!("Nagi M27 persistence decision: trial attempt={attempt} slot=B");
        require_m27_gpt_markers(
            "GPT untrusted System B trial",
            status,
            &log,
            &serial,
            &[
                &expected_decision,
                "Nagi M27 UEFI variable journal persistence PASS",
                "Nagi slot manifest REJECTED slot=B reason=signature",
                "Nagi M27 trial payload rejected slot=B",
                "Nagi Loader: slot manifest rejected",
            ],
        )?;
        if !m27_trial_failure_observed(&serial) {
            return Err(format!(
                "GPT System B trial {attempt} reached the guest kernel or missed the rejection path (log {})",
                log.display()
            ));
        }
    }

    let recovery_log = gpt_evidence.join("broken-recovery.log");
    let recovery_config = QemuConfig {
        qemu: &host.qemu,
        ovmf_code: &host.ovmf_code,
        ovmf_vars_template: &host.ovmf_vars,
        disk_image: &broken_image_path,
        persistent_disk: &broken_image_path,
        vars_copy: &broken_vars,
        serial_log: &recovery_log,
        acceptance_marker: "Nagi M27 Recovery command help PASS",
        timeout: Duration::from_secs(120),
    };
    let recovery_commands = b"check\nlog\nfiles\nslots\nhelp\n";
    let status = run_qemu_gui_reusing_ovmf_vars_with_events_and_serial_input(
        &recovery_config,
        "Nagi M27 Recovery boot menu READY",
        &M27_RECOVERY_MENU_EVENTS,
        "Nagi M27 Recovery console READY",
        recovery_commands,
    )
    .map_err(|error| format!("GPT Recovery after three System B failures: {error}"))?;
    let serial = fs::read_to_string(&recovery_log)
        .map_err(|error| format!("read {}: {error}", recovery_log.display()))?;
    require_m27_gpt_markers(
        "GPT Recovery after malformed System B",
        status,
        &recovery_log,
        &serial,
        &[
            "Nagi M27 manual selection: Recovery; boot journal unchanged PASS",
            "Nagi M30 GPT partition boot: Recovery PASS",
            "Nagi slot manifest verified slot=Recovery rollback-index=1 PASS",
            "Nagi M27 Recovery VFS check PASS files=",
            "Nagi M27 Recovery current-boot log PASS",
            "Nagi M27 Recovery files PASS",
            "Nagi M27 Recovery command help PASS",
        ],
    )?;
    if serial.contains("Nagi M27 persistence decision:")
        || serial.contains("Nagi M27 readiness persisted")
    {
        return Err(format!(
            "GPT Recovery changed the boot journal or recorded trial readiness (log {})",
            recovery_log.display()
        ));
    }

    for (boot, expected_decision) in [
        (4, "Nagi M27 persistence decision: rollback slot=A"),
        (5, "Nagi M27 persistence decision: confirmed slot=A"),
    ] {
        let log = gpt_evidence.join(format!("broken-system-a-recovery-{boot}.log"));
        let config = QemuConfig {
            qemu: &host.qemu,
            ovmf_code: &host.ovmf_code,
            ovmf_vars_template: &host.ovmf_vars,
            disk_image: &broken_image_path,
            persistent_disk: &broken_image_path,
            vars_copy: &broken_vars,
            serial_log: &log,
            acceptance_marker: "Nagi M7 acceptance PASS",
            timeout: Duration::from_secs(120),
        };
        let status = run_m27_headless_with_pre_guest_retry(&config, false)
            .map_err(|error| format!("GPT rollback/confirmed boot {boot}: {error}"))?;
        let serial =
            fs::read_to_string(&log).map_err(|error| format!("read {}: {error}", log.display()))?;
        require_m27_gpt_markers(
            "GPT System A rollback after Recovery",
            status,
            &log,
            &serial,
            &[
                expected_decision,
                "Nagi M30 GPT partition boot: System A PASS",
                "Nagi M7 persistent read PASS",
                "Nagi M7 acceptance PASS",
            ],
        )?;
    }

    let healthy_vars = gpt_evidence.join("healthy-OVMF_VARS.fd");
    let healthy_boot_log = gpt_evidence.join("healthy-initialize-user-data.log");
    let healthy_boot_config = QemuConfig {
        qemu: &host.qemu,
        ovmf_code: &host.ovmf_code,
        ovmf_vars_template: &host.ovmf_vars,
        disk_image: &healthy_image_path,
        persistent_disk: &healthy_image_path,
        vars_copy: &healthy_vars,
        serial_log: &healthy_boot_log,
        acceptance_marker: "Nagi M7 reboot required PASS",
        timeout: Duration::from_secs(180),
    };
    let status = run_qemu_gui_with_events(
        &healthy_boot_config,
        "Nagi M27 Recovery boot menu READY",
        &M27_SYSTEM_A_MENU_EVENTS,
    )
    .map_err(|error| format!("GPT healthy image User Data bootstrap: {error}"))?;
    let serial = fs::read_to_string(&healthy_boot_log)
        .map_err(|error| format!("read {}: {error}", healthy_boot_log.display()))?;
    require_m27_gpt_markers(
        "GPT healthy image User Data bootstrap",
        status,
        &healthy_boot_log,
        &serial,
        &[
            "Nagi M27 manual selection: confirmed slot=A",
            "Nagi M30 GPT partition boot: System A PASS",
            "Nagi M7 reboot required PASS",
        ],
    )?;

    let trial_log = gpt_evidence.join("healthy-system-b-readiness.log");
    let trial_config = QemuConfig {
        qemu: &host.qemu,
        ovmf_code: &host.ovmf_code,
        ovmf_vars_template: &host.ovmf_vars,
        disk_image: &healthy_image_path,
        persistent_disk: &healthy_image_path,
        vars_copy: &healthy_vars,
        serial_log: &trial_log,
        acceptance_marker: "Nagi M27 readiness persisted slot=B attempt=1",
        timeout: Duration::from_secs(120),
    };
    // The trial reports readiness only after the owner signs in; create the
    // owner through the OS-owned login screen (ADR 0063).
    let sign_in = qmp_first_run_sign_in("owner", "nagi1");
    let sign_in: Vec<&str> = sign_in.iter().map(String::as_str).collect();
    let status = run_qemu_gui_reusing_ovmf_vars_with_events(
        &trial_config,
        "Nagi login READY mode=create",
        &sign_in,
    )
    .map_err(|error| format!("GPT healthy System B trial: {error}"))?;
    let serial = fs::read_to_string(&trial_log)
        .map_err(|error| format!("read {}: {error}", trial_log.display()))?;
    require_m27_gpt_markers(
        "GPT healthy System B trial",
        status,
        &trial_log,
        &serial,
        &[
            "Nagi M27 persistence decision: trial attempt=1 slot=B",
            "Nagi M27 UEFI variable journal persistence PASS",
            "Nagi M30 GPT partition boot: System B PASS",
            "Nagi slot manifest verified slot=B rollback-index=1 PASS",
            "Nagi M7 persistent read PASS",
            "Nagi M10 desktop READY",
            "Nagi login owner created PASS name=owner",
            "Nagi M27 readiness persisted slot=B attempt=1 generation=",
        ],
    )?;
    if !m27_readiness_persisted_after_sign_in(&serial) {
        return Err(format!(
            "GPT System B persisted readiness before the owner signed in (log {})",
            trial_log.display()
        ));
    }

    let healthy_recovery_log = gpt_evidence.join("healthy-recovery.log");
    let healthy_recovery_config = QemuConfig {
        qemu: &host.qemu,
        ovmf_code: &host.ovmf_code,
        ovmf_vars_template: &host.ovmf_vars,
        disk_image: &healthy_image_path,
        persistent_disk: &healthy_image_path,
        vars_copy: &healthy_vars,
        serial_log: &healthy_recovery_log,
        acceptance_marker: "Nagi M27 Recovery command help PASS",
        timeout: Duration::from_secs(120),
    };
    let status = run_qemu_gui_reusing_ovmf_vars_with_events_and_serial_input(
        &healthy_recovery_config,
        "Nagi M27 Recovery boot menu READY",
        &M27_RECOVERY_MENU_EVENTS,
        "Nagi M27 Recovery console READY",
        recovery_commands,
    )
    .map_err(|error| format!("GPT Recovery after healthy B readiness: {error}"))?;
    let serial = fs::read_to_string(&healthy_recovery_log)
        .map_err(|error| format!("read {}: {error}", healthy_recovery_log.display()))?;
    require_m27_gpt_markers(
        "GPT Recovery after healthy System B readiness",
        status,
        &healthy_recovery_log,
        &serial,
        &[
            "Nagi M27 readiness record consumed slot=B PASS",
            "Nagi M27 manual selection: Recovery; boot journal unchanged PASS",
            "Nagi M30 GPT partition boot: Recovery PASS",
            "Nagi M27 Recovery VFS check PASS files=",
            "Nagi M27 Recovery command help PASS",
        ],
    )?;

    let confirmed_log = gpt_evidence.join("healthy-system-b-confirmed.log");
    let confirmed_config = QemuConfig {
        qemu: &host.qemu,
        ovmf_code: &host.ovmf_code,
        ovmf_vars_template: &host.ovmf_vars,
        disk_image: &healthy_image_path,
        persistent_disk: &healthy_image_path,
        vars_copy: &healthy_vars,
        serial_log: &confirmed_log,
        acceptance_marker: GUEST_ACCEPTANCE_MARKER,
        timeout: Duration::from_secs(120),
    };
    let status = run_m27_headless_with_pre_guest_retry(&confirmed_config, false)
        .map_err(|error| format!("GPT confirmed System B boot: {error}"))?;
    let serial = fs::read_to_string(&confirmed_log)
        .map_err(|error| format!("read {}: {error}", confirmed_log.display()))?;
    require_m27_gpt_markers(
        "GPT confirmed System B after Recovery",
        status,
        &confirmed_log,
        &serial,
        &[
            "Nagi M27 persistence decision: confirmed slot=B",
            "Nagi M30 GPT partition boot: System B PASS",
            "Nagi M7 persistent read PASS",
            "Nagi M7 acceptance PASS",
        ],
    )?;
    Ok(())
}

fn require_m27_gpt_markers(
    phase: &str,
    qemu_status: i32,
    log_path: &Path,
    serial: &str,
    markers: &[&str],
) -> Result<(), String> {
    for marker in markers {
        if !serial.contains(marker) {
            return Err(format!(
                "{phase} did not print `{marker}` (QEMU exit {qemu_status}; log {})",
                log_path.display()
            ));
        }
    }
    Ok(())
}

fn m27_trial_failure_observed(serial: &str) -> bool {
    serial.contains("Nagi M27 trial payload rejected slot=B")
        && (serial.contains("Nagi Loader: invalid ELF")
            || serial.contains("Nagi Loader: slot manifest rejected"))
        && !serial.contains("Nagi Kernel started")
        && !serial.contains("Nagi M27 readiness persisted")
}

fn m27_bootstrap_markers_present(serial: &str) -> bool {
    serial.contains(NAGI_WRITE_MARKER) && serial.contains(M27_BOOTSTRAP_COMPLETION_MARKER)
}

/// ADR 0063: with `desktop-login`, readiness follows the owner's sign-in.
fn m27_readiness_persisted_after_sign_in(serial: &str) -> bool {
    let signed_in = serial
        .lines()
        .position(|line| line == "Nagi login unlocked PASS");
    let readiness = serial.lines().position(|line| {
        line.starts_with("Nagi M27 readiness persisted slot=B attempt=1 generation=")
            && line.ends_with(" PASS")
    });
    matches!((signed_in, readiness), (Some(signed_in), Some(record)) if signed_in < record)
}

fn m27_readiness_persisted_before_desktop(serial: &str) -> bool {
    let readiness_line = serial.lines().position(|line| {
        line.starts_with("Nagi M27 readiness persisted slot=B attempt=1 generation=")
            && line.ends_with(" PASS")
    });
    let desktop_ready_line = serial
        .lines()
        .position(|line| line == "Nagi M10 desktop READY");
    matches!((readiness_line, desktop_ready_line), (Some(record), Some(desktop)) if record < desktop)
}

fn m27_readiness_consumed_before_promotion(serial: &str) -> bool {
    let consumed_line = serial
        .lines()
        .position(|line| line == "Nagi M27 readiness record consumed slot=B PASS");
    let promotion_line = serial
        .lines()
        .position(|line| line == "Nagi M27 persistence decision: confirmed slot=B");
    matches!((consumed_line, promotion_line), (Some(consumed), Some(promotion)) if consumed < promotion)
}

fn help() -> CommandResult {
    CommandResult {
        exit_code: EXIT_SUCCESS,
        lines: vec![
            "Nagi OS developer orchestrator".into(),
            "Commands: doctor [--allow-missing], diagnostics [--json|--format text|json] [--scope SCOPE] [--output PATH], verify [--json|--format text|json] [--scope SCOPE] [--output PATH], smoke [--host-only|--vm] [--json|--format text|json] [--output PATH], fetch, build, image, run, shell, gui, desktop, security, network, posix, std, m13, m14, m15, m16, m17, m18, m19, m20-granite <artifact.gguf>, m20-granite-inference <artifact.gguf>, m20-llama-smoke, m22, m25, m25-whisper <artifact.bin>, m25-whisper-inference <artifact.bin> <mono-16k-s16le.pcm> <expected-text>, m26-qwen <artifact.gguf>, m26-gemma <artifact.gguf> --accept-gemma-terms, m27, m29, m30, m30-update, isolated-process, consent, login, dev status|resume|verify|diagnose, test, test --acceptance [options], clean, fmt, lint"
                .into(),
        ],
    }
}

fn failure(exit_code: i32, message: impl Into<String>) -> CommandResult {
    CommandResult {
        exit_code,
        lines: vec![format!("FAIL {}", message.into())],
    }
}

#[cfg(test)]
mod tests {
    use super::{
        append_nagi_target_archive_tools, has_pre_guest_firmware_timeout_signature,
        last_serial_lines, m17_trace_excerpt, m27_bootstrap_markers_present,
        m27_readiness_consumed_before_promotion, m27_readiness_persisted_after_sign_in,
        m27_readiness_persisted_before_desktop, m27_trial_failure_observed,
        m30_image_build_info_matches, parse_command, path_with_suffix, pinned_granite_manifest,
        run_headless_with_pre_guest_retry_using, scoped_artifact_name, verify_external_artifact,
        Command, QemuConfig, NAGI_WRITE_MARKER,
    };
    use sha2::{Digest, Sha256};
    use std::path::Path;

    #[test]
    fn m22_retries_only_a_running_pre_guest_firmware_timeout_with_diagnostics() {
        let firmware_timeout = concat!(
            "\u{1b}[2J\u{1b}[01;01H\u{1b}[=3h\u{1b}[2J\u{1b}[01;01H\n",
            "QEMU timeout diagnostics:\n",
            "QMP query-status: {\"return\": {\"status\": \"running\", \"running\": true}}\n",
            "QMP CPU registers: RIP=000000007eb84171\n",
            "QMP CPU instruction window: 0x7eb84171: jmp 0x7eb84150\n"
        );
        let timeout = "QEMU did not reach acceptance within 180 seconds";
        assert!(has_pre_guest_firmware_timeout_signature(
            timeout,
            firmware_timeout
        ));
        assert!(!has_pre_guest_firmware_timeout_signature(
            "QEMU exited before acceptance",
            firmware_timeout
        ));
        assert!(!has_pre_guest_firmware_timeout_signature(
            timeout,
            &firmware_timeout.replace(
                "QMP CPU instruction window:",
                "Nagi Kernel started\nQMP CPU instruction window:"
            )
        ));
        assert!(!has_pre_guest_firmware_timeout_signature(
            timeout,
            "QEMU timeout diagnostics: no CPU state captured"
        ));
    }

    #[test]
    fn m27_pre_guest_retry_restores_journal_state_and_preserves_both_snapshots() {
        use std::fs;
        use std::time::{Duration, SystemTime, UNIX_EPOCH};

        let root = std::env::temp_dir().join(format!(
            "nagi-m27-pre-guest-retry-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after Unix epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("create temporary evidence directory");
        let qemu = root.join("qemu");
        let ovmf_code = root.join("OVMF_CODE.fd");
        let ovmf_vars_template = root.join("OVMF_VARS.template.fd");
        let disk_image = root.join("boot.img");
        let persistent_disk = root.join("user-data.img");
        let vars_copy = root.join("OVMF_VARS.fd");
        let serial_log = root.join("boot-4.log");
        fs::write(&disk_image, b"boot image").expect("seed boot image");
        fs::write(&persistent_disk, b"user data").expect("seed user data");
        fs::write(&vars_copy, b"initial OVMF variables").expect("seed OVMF variables");
        let config = QemuConfig {
            qemu: &qemu,
            ovmf_code: &ovmf_code,
            ovmf_vars_template: &ovmf_vars_template,
            disk_image: &disk_image,
            persistent_disk: &persistent_disk,
            vars_copy: &vars_copy,
            serial_log: &serial_log,
            acceptance_marker: "Nagi M7 acceptance PASS",
            timeout: Duration::from_secs(180),
        };
        let diagnostics = concat!(
            "\u{1b}[2J\u{1b}[01;01H\u{1b}[=3h\u{1b}[2J\u{1b}[01;01H\n",
            "QEMU timeout diagnostics:\n",
            "QMP query-status: {\"return\": {\"status\": \"running\", \"running\": true}}\n",
            "QMP CPU registers: RIP=000000007eb84171\n",
            "QMP CPU instruction window: 0x7eb84171: jmp 0x7eb84150\n"
        );
        let mut attempts = 0;
        let result = run_headless_with_pre_guest_retry_using(&config, false, "M27", |config| {
            attempts += 1;
            if attempts == 1 {
                fs::write(config.serial_log, diagnostics).expect("write first-attempt log");
                fs::write(config.vars_copy, b"first attempt advanced boot journal")
                    .expect("mutate first-attempt variables");
                Err("QEMU did not reach acceptance within 180 seconds".to_owned())
            } else {
                assert_eq!(
                    fs::read(config.vars_copy).expect("read reused OVMF variables"),
                    b"initial OVMF variables"
                );
                assert_eq!(
                    fs::read(config.disk_image).expect("read boot disk before retry"),
                    b"boot image"
                );
                assert_eq!(
                    fs::read(config.persistent_disk).expect("read User Data before retry"),
                    b"user data"
                );
                fs::write(config.vars_copy, b"retry advanced boot journal once")
                    .expect("persist one retry journal attempt");
                fs::write(
                    config.serial_log,
                    "Nagi Kernel started\nNagi M7 acceptance PASS\n",
                )
                .expect("write retry acceptance log");
                Ok(7)
            }
        });
        assert_eq!(result, Ok(7));
        assert_eq!(attempts, 2);
        assert_eq!(
            fs::read(path_with_suffix(&serial_log, ".pre-guest-timeout-1"))
                .expect("preserved first-attempt serial log"),
            diagnostics.as_bytes()
        );
        assert_eq!(
            fs::read(path_with_suffix(
                &serial_log,
                ".ovmf-vars.pre-guest-timeout-1"
            ))
            .expect("preserved first-attempt OVMF variables"),
            b"first attempt advanced boot journal"
        );
        assert_eq!(
            fs::read(path_with_suffix(
                &serial_log,
                ".ovmf-vars.pre-guest-retry-source-1"
            ))
            .expect("preserved pre-attempt OVMF variables"),
            b"initial OVMF variables"
        );
        assert!(
            fs::read_to_string(path_with_suffix(&serial_log, ".pre-guest-retry-1.txt"))
                .expect("retry evidence note")
                .contains("pre-attempt OVMF variables were restored")
        );
        assert_eq!(
            fs::read(&vars_copy).expect("read retry OVMF variables"),
            b"retry advanced boot journal once"
        );
        fs::remove_dir_all(root).expect("remove temporary evidence directory");
    }

    #[test]
    fn m27_pre_guest_retry_refuses_to_replay_after_writable_disk_changes() {
        use std::fs;
        use std::time::{Duration, SystemTime, UNIX_EPOCH};

        let root = std::env::temp_dir().join(format!(
            "nagi-m27-pre-guest-disk-change-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after Unix epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("create temporary evidence directory");
        let disk_image = root.join("boot.img");
        let persistent_disk = root.join("user-data.img");
        fs::write(&disk_image, b"boot disk before attempt").expect("seed boot disk");
        fs::write(&persistent_disk, b"user data before attempt").expect("seed user data");
        let qemu = root.join("qemu");
        let ovmf_code = root.join("OVMF_CODE.fd");
        let ovmf_vars_template = root.join("OVMF_VARS.template.fd");
        let vars_copy = root.join("OVMF_VARS.fd");
        let serial_log = root.join("promotion-boot.log");
        fs::write(&vars_copy, b"initial OVMF variables").expect("seed OVMF variables");
        let config = QemuConfig {
            qemu: &qemu,
            ovmf_code: &ovmf_code,
            ovmf_vars_template: &ovmf_vars_template,
            disk_image: &disk_image,
            persistent_disk: &persistent_disk,
            vars_copy: &vars_copy,
            serial_log: &serial_log,
            acceptance_marker: "Nagi M10 desktop READY",
            timeout: Duration::from_secs(180),
        };
        let diagnostics = concat!(
            "QEMU timeout diagnostics:\n",
            "QMP query-status: {\"return\": {\"status\": \"running\", \"running\": true}}\n",
            "QMP CPU registers: RIP=000000007eb84171\n",
            "QMP CPU instruction window: 0x7eb84171: jmp 0x7eb84150\n"
        );
        let mut attempts = 0;
        let result = run_headless_with_pre_guest_retry_using(&config, false, "M27", |config| {
            attempts += 1;
            fs::write(config.serial_log, diagnostics).expect("write firmware timeout log");
            fs::write(
                config.persistent_disk,
                b"user data changed during firmware attempt",
            )
            .expect("simulate persistent disk mutation");
            Err("QEMU did not reach acceptance within 180 seconds".to_owned())
        });

        assert_eq!(attempts, 1, "changed disk state must suppress retry");
        let error = result.expect_err("changed disk state must fail closed");
        assert!(error.contains("writable M27 disk changed"), "{error}");
        assert_eq!(
            fs::read(&disk_image).expect("read unchanged boot disk"),
            b"boot disk before attempt"
        );
        assert_eq!(
            fs::read(&persistent_disk).expect("read preserved post-attempt User Data"),
            b"user data changed during firmware attempt"
        );
        assert!(
            fs::read_to_string(path_with_suffix(&serial_log, ".pre-guest-retry-1.txt"))
                .expect("retry suppression evidence")
                .contains("Retry suppressed")
        );
        fs::remove_dir_all(root).expect("remove temporary evidence directory");
    }

    #[test]
    fn m22_pre_guest_retry_evidence_is_scoped_to_each_boot_log() {
        use std::fs;
        use std::time::{Duration, SystemTime, UNIX_EPOCH};

        let root = std::env::temp_dir().join(format!(
            "nagi-m22-pre-guest-retry-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after Unix epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("create temporary evidence directory");
        let disk_image = root.join("boot.img");
        let persistent_disk = root.join("user-data.img");
        let vars_template = root.join("OVMF_VARS.template.fd");
        let vars_copy = root.join("OVMF_VARS.fd");
        fs::write(&disk_image, b"boot disk").expect("seed boot disk");
        fs::write(&persistent_disk, b"User Data").expect("seed User Data");
        fs::write(&vars_template, b"fresh template variables").expect("seed vars template");
        let diagnostics = concat!(
            "QEMU timeout diagnostics:\n",
            "QMP query-status: {\"return\": {\"status\": \"running\", \"running\": true}}\n",
            "QMP CPU registers: RIP=000000007eb84171\n",
            "QMP CPU instruction window: 0x7eb84171: jmp 0x7eb84150\n"
        );

        for boot_index in 1..=3 {
            fs::copy(&vars_template, &vars_copy).expect("reset OVMF vars for next boot");
            let serial_log = root.join(format!("boot-{boot_index}.log"));
            let qemu = root.join("qemu");
            let ovmf_code = root.join("OVMF_CODE.fd");
            let config = QemuConfig {
                qemu: &qemu,
                ovmf_code: &ovmf_code,
                ovmf_vars_template: &vars_template,
                disk_image: &disk_image,
                persistent_disk: &persistent_disk,
                vars_copy: &vars_copy,
                serial_log: &serial_log,
                acceptance_marker: "Nagi M13 acceptance PASS",
                timeout: Duration::from_secs(180),
            };
            let mut attempts = 0;
            let result = run_headless_with_pre_guest_retry_using(&config, false, "M22", |config| {
                attempts += 1;
                if attempts == 1 {
                    fs::write(config.serial_log, diagnostics).expect("write first boot log");
                    fs::write(config.vars_copy, b"failed attempt variables")
                        .expect("mutate first-attempt OVMF variables");
                    Err("QEMU did not reach acceptance within 180 seconds".to_owned())
                } else {
                    assert_eq!(
                        fs::read(config.vars_copy).expect("read restored OVMF variables"),
                        b"fresh template variables"
                    );
                    fs::write(
                        config.serial_log,
                        "Nagi Kernel started\nNagi M13 acceptance PASS\n",
                    )
                    .expect("write retried boot acceptance log");
                    Ok(0)
                }
            });

            assert_eq!(result, Ok(0), "M22 boot {boot_index} retry should pass");
            assert_eq!(attempts, 2, "M22 boot {boot_index} gets one retry");
            assert!(
                path_with_suffix(&serial_log, ".pre-guest-timeout-1").is_file(),
                "M22 boot {boot_index} first log should be preserved"
            );
            assert!(
                path_with_suffix(&serial_log, ".ovmf-vars.pre-guest-timeout-1").is_file(),
                "M22 boot {boot_index} failed variables should be preserved"
            );
            assert!(
                path_with_suffix(&serial_log, ".pre-guest-retry-1.txt").is_file(),
                "M22 boot {boot_index} retry note should be preserved"
            );
        }

        fs::remove_dir_all(root).expect("remove temporary evidence directory");
    }

    #[cfg(unix)]
    #[test]
    fn retry_evidence_path_suffix_preserves_non_utf8_path_bytes() {
        use std::ffi::OsString;
        use std::os::unix::ffi::{OsStrExt, OsStringExt};
        use std::path::PathBuf;

        let path = PathBuf::from(OsString::from_vec(b"/tmp/nagi-\xff".to_vec()));
        let suffixed = path_with_suffix(&path, ".retry");

        assert_eq!(suffixed.as_os_str().as_bytes(), b"/tmp/nagi-\xff.retry");
    }

    #[test]
    fn m29_command_selects_the_settings_acceptance() {
        assert_eq!(parse_command(&["m29".into()]), Ok(Command::M29));
        assert_eq!(parse_command(&["consent".into()]), Ok(Command::Consent));
        assert!(parse_command(&["consent".into(), "extra".into()]).is_err());
        assert!(parse_command(&["m29".into(), "extra".into()]).is_err());
    }

    #[test]
    fn m20_granite_command_requires_exactly_one_external_artifact_path() {
        assert_eq!(
            parse_command(&["m20-granite".into(), "model.gguf".into()]),
            Ok(Command::M20Granite)
        );
        assert!(parse_command(&["m20-granite".into()]).is_err());
        assert!(
            parse_command(&["m20-granite".into(), "model.gguf".into(), "extra".into()]).is_err()
        );
    }

    #[test]
    fn m20_granite_inference_command_requires_exactly_one_external_artifact_path() {
        assert_eq!(
            parse_command(&["m20-granite-inference".into(), "model.gguf".into()]),
            Ok(Command::M20GraniteInference)
        );
        assert!(parse_command(&["m20-granite-inference".into()]).is_err());
        assert!(parse_command(&[
            "m20-granite-inference".into(),
            "model.gguf".into(),
            "extra".into()
        ])
        .is_err());
    }

    #[test]
    fn m20_llama_smoke_command_accepts_no_arguments() {
        assert_eq!(
            parse_command(&["m20-llama-smoke".into()]),
            Ok(Command::M20LlamaSmoke)
        );
        assert!(parse_command(&["m20-llama-smoke".into(), "extra".into()]).is_err());
    }

    #[test]
    fn m25_whisper_command_requires_exactly_one_external_artifact_path() {
        assert_eq!(
            parse_command(&["m25-whisper".into(), "model.bin".into()]),
            Ok(Command::M25Whisper)
        );
        assert!(parse_command(&["m25-whisper".into()]).is_err());
        assert!(
            parse_command(&["m25-whisper".into(), "model.bin".into(), "extra".into()]).is_err()
        );
    }

    #[test]
    fn m25_whisper_inference_requires_model_pcm_and_expected_text() {
        assert_eq!(
            parse_command(&[
                "m25-whisper-inference".into(),
                "model.bin".into(),
                "voice.pcm".into(),
                "アルバートを開いて".into(),
            ]),
            Ok(Command::M25WhisperInference)
        );
        assert!(parse_command(&["m25-whisper-inference".into()]).is_err());
        assert!(parse_command(&[
            "m25-whisper-inference".into(),
            "model.bin".into(),
            "voice.pcm".into(),
        ])
        .is_err());
        assert!(parse_command(&[
            "m25-whisper-inference".into(),
            "model.bin".into(),
            "voice.pcm".into(),
            "expected".into(),
            "extra".into(),
        ])
        .is_err());
    }

    #[test]
    fn m26_model_commands_require_exact_artifact_and_gemma_terms_acknowledgement() {
        assert_eq!(
            parse_command(&["m26-qwen".into(), "qwen.gguf".into()]),
            Ok(Command::M26Qwen)
        );
        assert!(parse_command(&["m26-qwen".into()]).is_err());
        assert!(parse_command(&["m26-qwen".into(), "qwen.gguf".into(), "extra".into()]).is_err());
        assert_eq!(
            parse_command(&[
                "m26-gemma".into(),
                "gemma.gguf".into(),
                "--accept-gemma-terms".into()
            ]),
            Ok(Command::M26Gemma)
        );
        assert!(parse_command(&["m26-gemma".into(), "gemma.gguf".into()]).is_err());
        assert!(parse_command(&[
            "m26-gemma".into(),
            "gemma.gguf".into(),
            "--wrong-flag".into()
        ])
        .is_err());
    }

    #[test]
    fn m25_whisper_artifact_contract_matches_the_model_lock() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let model = crate::whisper_cpp::validate_whisper_model_artifact_lock(&root)
            .expect("pinned Whisper artifact lock");
        assert_eq!(model.model_id, "openai.whisper-small-multilingual");
        assert_eq!(model.file_name, "ggml-small.bin");
        assert_eq!(model.size_bytes, 487_601_967);
        assert_eq!(
            model.sha256,
            "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b"
        );
    }

    #[test]
    fn m20_granite_manifest_matches_the_model_lock() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let manifest = pinned_granite_manifest(&root).expect("pinned Granite manifest");
        assert_eq!(manifest.model_id.as_str(), "ibm.granite-4.2-3b");
        assert_eq!(manifest.artifact.size_bytes, Some(2_244_011_552));
    }

    #[test]
    fn m20_granite_external_artifact_is_checked_by_size_and_digest() {
        let path = std::env::temp_dir().join(format!(
            "nagi-m20-artifact-{}-{}.gguf",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after Unix epoch")
                .as_nanos()
        ));
        let bytes = b"pinned test artifact";
        std::fs::write(&path, bytes).expect("write temporary artifact");
        let digest = format!("{:x}", Sha256::digest(bytes));
        assert!(verify_external_artifact(&path, bytes.len() as u64, &digest).is_ok());
        assert!(verify_external_artifact(&path, bytes.len() as u64 + 1, &digest).is_err());
        assert!(verify_external_artifact(&path, bytes.len() as u64, &"0".repeat(64)).is_err());
        std::fs::remove_file(path).expect("remove temporary artifact");
    }

    #[test]
    fn m29_acceptance_artifacts_are_unique_and_keep_their_extensions() {
        assert_eq!(
            scoped_artifact_name("nagi-settings.img", "run-123", true),
            "nagi-settings-run-123.img"
        );
        assert_eq!(
            scoped_artifact_name("settings.log", "run-123", false),
            "settings.log"
        );
    }

    #[test]
    fn m30_image_build_info_binds_the_source_revision_and_image_digest() {
        let revision = "a".repeat(40);
        let digest = "b".repeat(64);
        let build_info =
            format!("format_version=1\nsource_revision={revision}\nimage_sha256={digest}\n");

        assert!(m30_image_build_info_matches(
            &build_info,
            &revision,
            &digest
        ));
        assert!(!m30_image_build_info_matches(
            &build_info,
            &"c".repeat(40),
            &digest
        ));
        assert!(!m30_image_build_info_matches(
            &build_info,
            &revision,
            &"d".repeat(64)
        ));
        assert!(!m30_image_build_info_matches(
            &format!("{build_info}unexpected=value\n"),
            &revision,
            &digest
        ));
    }

    #[test]
    fn serial_log_excerpt_keeps_the_last_lines_in_order() {
        assert_eq!(
            last_serial_lines("first\r\nsecond\r\nthird\r\n", 2),
            "second\nthird"
        );
    }

    #[test]
    fn m27_trial_acceptance_waits_for_loader_failure_after_rejection() {
        assert!(m27_trial_failure_observed(
            "Nagi M27 trial payload rejected slot=B\nNagi Loader: invalid ELF\n"
        ));
        assert!(m27_trial_failure_observed(
            "Nagi slot manifest REJECTED slot=B reason=signature\nNagi M27 trial payload rejected slot=B\nNagi Loader: slot manifest rejected\n"
        ));
        assert!(!m27_trial_failure_observed(
            "Nagi M27 trial payload rejected slot=B\n"
        ));
        assert!(!m27_trial_failure_observed(
            "Nagi M27 trial payload rejected slot=B\nNagi Loader: invalid ELF\nNagi Kernel started\n"
        ));
    }

    #[test]
    fn m27_bootstrap_waits_for_storage_restart_required_marker() {
        let complete = format!("{NAGI_WRITE_MARKER}\nNagi M7 reboot required PASS\n");
        assert!(m27_bootstrap_markers_present(&complete));
        assert!(!m27_bootstrap_markers_present(&format!(
            "{NAGI_WRITE_MARKER}\n"
        )));
        assert!(!m27_bootstrap_markers_present(
            "Nagi M7 reboot required PASS\n"
        ));
    }

    #[test]
    fn m27_readiness_must_follow_sign_in() {
        assert!(m27_readiness_persisted_after_sign_in(
            "Nagi M10 desktop READY\nNagi login unlocked PASS\nNagi M27 readiness persisted slot=B attempt=1 generation=4 PASS\n"
        ));
        assert!(!m27_readiness_persisted_after_sign_in(
            "Nagi M27 readiness persisted slot=B attempt=1 generation=4 PASS\nNagi login unlocked PASS\n"
        ));
        assert!(!m27_readiness_persisted_after_sign_in(
            "Nagi M27 readiness persisted slot=B attempt=1 generation=4 PASS\n"
        ));
    }

    #[test]
    fn m27_readiness_must_be_persisted_before_desktop_ready() {
        assert!(m27_readiness_persisted_before_desktop(
            "Nagi M27 readiness persisted slot=B attempt=1 generation=3 PASS\r\nNagi M10 desktop READY\r\n"
        ));
        assert!(!m27_readiness_persisted_before_desktop(
            "Nagi M10 desktop READY\r\nNagi M27 readiness persisted slot=B attempt=1 generation=3 PASS\r\n"
        ));
        assert!(!m27_readiness_persisted_before_desktop(
            "Nagi M27 readiness persisted slot=B attempt=1 generation=3 FAIL\r\nNagi M10 desktop READY\r\n"
        ));
    }

    #[test]
    fn m27_readiness_must_be_consumed_before_candidate_promotion() {
        assert!(m27_readiness_consumed_before_promotion(
            "Nagi M27 readiness record consumed slot=B PASS\r\nNagi M27 persistence decision: confirmed slot=B\r\n"
        ));
        assert!(!m27_readiness_consumed_before_promotion(
            "Nagi M27 persistence decision: confirmed slot=B\r\nNagi M27 readiness record consumed slot=B PASS\r\n"
        ));
    }

    #[test]
    fn m17_trace_excerpt_keeps_earlier_egl_events_and_skips_non_trace_lines() {
        assert_eq!(
            m17_trace_excerpt(
                "boot noise\nNagi M17 trace: TLS initialized\nother noise\nNagi M17 trace: EGL bind started\n",
                8,
            ),
            "Nagi M17 trace: TLS initialized\nNagi M17 trace: EGL bind started"
        );
    }

    #[test]
    fn cross_archive_tools_are_limited_to_macos_nagi_target_builds() {
        let mut macos_env = Vec::<(&str, &Path)>::new();
        append_nagi_target_archive_tools(&mut macos_env, "macos");
        assert_eq!(
            macos_env,
            vec![
                ("AR_x86_64_unknown_nagi_user", Path::new("llvm-ar")),
                ("RANLIB_x86_64_unknown_nagi_user", Path::new("llvm-ranlib")),
            ]
        );

        let mut linux_env = Vec::<(&str, &Path)>::new();
        append_nagi_target_archive_tools(&mut linux_env, "linux");
        assert!(linux_env.is_empty());
    }

    #[test]
    fn m17_trace_excerpt_bounds_long_logs_and_keeps_both_ends() {
        let serial = (0..8)
            .map(|index| format!("Nagi M17 trace: event {index}"))
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(
            m17_trace_excerpt(&serial, 4),
            "Nagi M17 trace: event 0\nNagi M17 trace: event 1\n[omitted 4 M17 trace lines]\nNagi M17 trace: event 6\nNagi M17 trace: event 7"
        );
    }

    #[test]
    fn m17_init_storage_bootstrap_matches_the_two_boot_acceptance_gate() {
        let init = include_str!("../../../user/nagi-init/src/main.rs");
        let runner = include_str!("image.rs");
        let start = init
            .find("pub extern \"C\" fn _start")
            .expect("nagi-init entry point");
        let init_entry = &init[start..];
        let m17_start = init_entry
            .find("#[cfg(feature = \"m17-servo\")]")
            .expect("M17 init branch");
        let m17_branch = &init_entry[m17_start..];
        let m17_end = m17_branch
            .find("#[cfg(not(any(")
            .expect("generic init branch after M17");
        let m17_branch = &m17_branch[..m17_end];
        let storage = m17_branch
            .find("run_m7_storage_acceptance(block_capability)")
            .expect("M17 first boot storage acceptance");
        let pixel = m17_branch
            .find("run_first_web_pixel(display_capability)")
            .expect("M17 first web pixel path");
        assert!(
            storage < pixel,
            "M17 must verify persistent storage before Servo"
        );
        assert!(
            m17_branch[storage..pixel].contains("libnagi::exit(exit_code)"),
            "the first persistent-write boot must stop before the pixel boot"
        );
        assert!(runner
            .contains("pub const NAGI_WRITE_MARKER: &str = \"Nagi M7 persistent write PASS\""));
        assert!(init.contains("Nagi M7 persistent write PASS"));
    }

    #[test]
    fn m17_persistence_and_pixel_boots_keep_the_esp_read_only() {
        let commands = include_str!("commands.rs");
        let start = commands.find("fn execute_m17(").expect("M17 command");
        let end = commands[start..]
            .find("fn execute_m16_sample_build(")
            .map(|offset| start + offset)
            .expect("next command helper");
        let m17_command = &commands[start..end];

        assert_eq!(
            m17_command
                .matches("run_qemu_with_read_only_boot_disk(")
                .count(),
            2,
            "both M17 QEMU boots must protect the ESP"
        );
        assert!(
            !m17_command.contains("run_qemu(&"),
            "M17 must not boot with a writable ESP"
        );
    }

    #[test]
    fn only_servo_enabled_init_builds_are_stripped_for_the_image() {
        assert!(super::servo_enabled_init(&[
            "build",
            "--features",
            "m17-servo"
        ]));
        assert!(super::servo_enabled_init(&["--features", "m18-acceptance"]));
        assert!(super::servo_enabled_init(&[
            "--features",
            "m13-posix,m17-servo"
        ]));
        assert!(!super::servo_enabled_init(&["--features", "m19-search"]));
        assert!(!super::servo_enabled_init(&["build", "-p", "nagi-init"]));
    }

    #[test]
    fn m18_browser_boot_keeps_the_esp_read_only_for_storage_selection() {
        let commands = include_str!("commands.rs");
        let m18_start = commands.find("fn execute_m18(").expect("M18 command");
        let m18_end = commands[m18_start..]
            .find("fn help() -> CommandResult")
            .map(|offset| m18_start + offset)
            .expect("next command helper");
        assert!(commands[m18_start..m18_end].contains(
            "run_qemu_gui_with_read_only_boot_disk_and_staged_events_and_failure_marker_and_screenshot("
        ));
        assert!(commands[m18_start..m18_end].contains("m29-browser-{evidence_run_id}"));

        let image = include_str!("image.rs");
        assert!(image.contains("Duration::from_millis(100)"));
        let compact_image: String = image.split_whitespace().collect();
        assert!(compact_image.contains("inter_event_delay.is_zero()"));
        assert!(compact_image.contains("mode.inter_event_delay,"));
        assert!(
            compact_image.contains("vnc_port-5900,mode.boot_disk_read_only,mode.reuse_ovmf_vars,")
        );
        assert!(compact_image.contains(
            "boot_disk_read_only:true,reuse_ovmf_vars:false,inter_event_delay:Duration::from_millis(100),"
        ));
    }

    #[test]
    fn m18_cleans_stale_servo_temp_storage_before_creating_its_profile() {
        let init = include_str!("../../../user/nagi-init/src/main.rs");
        let cleanup = init
            .find("nagi_posix::cleanup_m18_servo_temp_directories()")
            .expect("M18 stale Servo temp cleanup");
        let profile = init
            .find("nagi_posix_ensure_directory(c\"/tmp/nagi-servo-profile\".as_ptr())")
            .expect("stable Servo profile directory");
        assert!(
            cleanup < profile,
            "free VFS inodes before creating the profile"
        );

        let acceptance = include_str!("../../../user/nagi-albert/src/m18_acceptance.rs");
        assert!(acceptance.contains(
            "servo_options.config_dir = Some(std::path::PathBuf::from(SERVO_CONFIG_DIR))"
        ));
        assert!(acceptance.contains("const SERVO_CONFIG_DIR: &str = \"/tmp/nagi-servo-profile\""));
    }

    #[test]
    fn m17_rust_std_random_backend_uses_guest_rng_boundary() {
        let rust_std_patch =
            include_str!("../../../third_party/rust-std/patches/0001-nagi-target-support.patch");
        let libnagi = include_str!("../../../user/libnagi/src/lib.rs");
        let syscall = include_str!("../../../kernel/src/syscall.rs");

        assert!(rust_std_patch.contains("target_os = \"nagi\""));
        assert!(rust_std_patch.contains("mod nagi;"));
        assert!(rust_std_patch.contains("fn __nagi_std_random_fill"));
        assert!(
            libnagi.contains("pub unsafe extern \"C\" fn __nagi_std_random_fill"),
            "std entropy must cross the Nagi guest RNG ABI"
        );
        assert!(libnagi.contains("let mut result = SYS_RANDOM_GET;"));
        for diagnostic in [
            "SYS_RANDOM_GET rejected: user buffer range",
            "random_error_trace(error)",
            "RandomError::PciUnavailable",
            "RandomError::RequestTimeout",
            "RandomError::QueueCorrupt",
        ] {
            assert!(
                syscall.contains(diagnostic),
                "missing guest RNG failure diagnostic: {diagnostic}"
            );
        }
        assert!(
            !rust_std_patch
                .contains("else if #[cfg(any(target_os = \"redox\", target_os = \"nagi\"))]"),
            "Nagi must not use Rust std's Redox /scheme/rand backend"
        );
    }

    #[test]
    fn m17_posix_urandom_device_uses_guest_virtio_rng() {
        let runtime = include_str!("../../../user/nagi-posix/src/runtime.rs");
        let libnagi = include_str!("../../../user/libnagi/src/lib.rs");
        let open_start = runtime.find("pub fn open(").expect("POSIX runtime open");
        let open_end = runtime[open_start..]
            .find("\nfn allocate_descriptor(")
            .map(|offset| open_start + offset)
            .expect("descriptor allocation helper");
        let open = &runtime[open_start..open_end];
        let device = open
            .find("if name == b\"/dev/urandom\"")
            .expect("virtual urandom path");
        let vfs = open
            .find("let mut filesystem = FILESYSTEM.lock();")
            .expect("persistent VFS path");
        assert!(device < vfs, "urandom must not require a VFS mount");
        assert!(open.contains("allocate_descriptor(FdEntry::Random)"));

        assert!(runtime.contains("FdEntry::Random => {"));
        assert!(runtime.contains("libnagi::random_fill(bytes)"));
        assert!(runtime.contains("RuntimeError::EntropyUnavailable => 5"));
        assert!(runtime.contains("FdEntry::Random => Ok(requested & POLLIN)"));
        assert!(libnagi.contains("let mut result = SYS_RANDOM_GET;"));
        assert!(libnagi.contains("pub fn random_fill(bytes: &mut [u8]) -> bool"));
    }

    #[test]
    fn m17_guest_rng_scans_the_transitional_entropy_device_id() {
        let random = include_str!("../../../kernel/src/random.rs");

        assert!(random.contains("const VIRTIO_RNG_LEGACY_ID: u16 = 0x1005;"));
        assert!(random.contains("const VIRTIO_RNG_MODERN_ID: u16 = 0x1044;"));
        assert!(random.contains("if !is_rng_device(vendor, device_id)"));
        assert!(random.contains("fn is_rng_device(vendor: u16, device_id: u16) -> bool"));
    }
}
