use std::fs;
use std::path::Path;

use nagi_cli::commands::{parse_command, Command, EXIT_SUCCESS, EXIT_USAGE};
use nagi_cli::config::load_toolchain_requirements;
use nagi_cli::doctor::{
    run_doctor, run_doctor_with_requirements, CommandEvidence, DoctorPolicy, HostProbe,
    OvmfEvidence, SystemProbe, ToolchainRequirements,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OvmfMode {
    Missing,
    Compatible,
    Mismatch,
}

#[derive(Debug)]
struct StaticProbe {
    commands: Vec<&'static str>,
    ovmf_mode: OvmfMode,
    bad_output: bool,
    exit_code: Option<i32>,
}

impl Default for StaticProbe {
    fn default() -> Self {
        Self {
            commands: Vec::new(),
            ovmf_mode: OvmfMode::Missing,
            bad_output: false,
            exit_code: None,
        }
    }
}

impl HostProbe for StaticProbe {
    fn command(&self, candidates: &[&str], _args: &[&str]) -> Option<CommandEvidence> {
        let candidate = candidates
            .iter()
            .find(|candidate| self.commands.contains(candidate))?;
        let output = if self.bad_output {
            "some unrelated program 0.1.0"
        } else {
            canned_version(candidate)
        };
        Some(CommandEvidence {
            path: format!("test://{candidate}"),
            exit_code: Some(self.exit_code.unwrap_or(0)),
            output: output.to_owned(),
        })
    }

    fn ovmf(&self) -> Option<OvmfEvidence> {
        match self.ovmf_mode {
            OvmfMode::Missing => None,
            OvmfMode::Compatible => Some(OvmfEvidence::new(
                "test://edk2-x86_64-code.fd",
                "test://edk2-i386-vars.fd",
            )),
            OvmfMode::Mismatch => Some(OvmfEvidence::new(
                "test://OVMF_CODE_4M.fd",
                "test://OVMF_VARS.fd",
            )),
        }
    }
}

fn canned_version(candidate: &str) -> &'static str {
    if candidate.starts_with("git") {
        "git version 2.54.0"
    } else if candidate.starts_with("rustc") {
        "rustc 1.90.0-nightly (test 2025-07-31)"
    } else if candidate.starts_with("cargo") {
        "cargo 1.90.0-nightly (test 2025-07-31)"
    } else if candidate.starts_with("rustup") {
        "rustup 1.28.2"
    } else if candidate.starts_with("clang") {
        "clang version 23.1.1"
    } else if candidate.starts_with("ld.lld") || candidate.starts_with("lld-link") {
        "LLD 23.1.1"
    } else if candidate.starts_with("qemu") {
        "QEMU emulator version 11.1.0"
    } else if candidate.starts_with("cmake") {
        "cmake version 4.4.3"
    } else if candidate.starts_with("meson") {
        "1.12.0"
    } else if candidate.starts_with("ninja") {
        "1.12.1"
    } else {
        "Python 3.12.10"
    }
}

#[test]
fn parses_the_complete_m0_command_surface() {
    let commands = [
        ("doctor", Command::Doctor),
        ("fetch", Command::Fetch),
        ("build", Command::Build),
        ("image", Command::Image),
        ("run", Command::Run),
        ("shell", Command::Shell),
        ("gui", Command::Gui),
        ("desktop", Command::Desktop),
        ("security", Command::Security),
        ("network", Command::Network),
        ("posix", Command::Posix),
        ("std", Command::Std),
        ("m13", Command::M13),
        ("m14", Command::M14),
        ("m15", Command::M15),
        ("m16", Command::M16),
        ("m17", Command::M17),
        ("test", Command::Test),
        ("clean", Command::Clean),
        ("fmt", Command::Fmt),
        ("lint", Command::Lint),
    ];

    for (name, expected) in commands {
        assert_eq!(parse_command(&[name.to_owned()]).unwrap(), expected);
    }
    assert_eq!(
        parse_command(&["dev".into(), "status".into()]).unwrap(),
        Command::Dev
    );
}

#[test]
fn rejects_unknown_commands_with_usage_exit_code() {
    let error = parse_command(&["unknown".to_owned()]).unwrap_err();

    assert_eq!(error.exit_code(), EXIT_USAGE);
}

#[test]
fn dev_requires_a_subcommand_and_rejects_unknown_dev_actions() {
    assert_eq!(
        parse_command(&["dev".to_owned()]).unwrap_err().exit_code(),
        EXIT_USAGE
    );
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let error = nagi_cli::development::execute(&["unknown".into()], root).unwrap_err();
    assert_eq!(error.exit_code(), EXIT_USAGE);
}

#[test]
fn dev_status_resume_and_verify_read_the_registered_workstream() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let status = nagi_cli::development::execute(&["status".into()], root)
        .expect("registered development workstream status");
    assert!(status
        .iter()
        .any(|line| line.contains("development-foundation")));
    assert!(status.iter().any(|line| line.starts_with("HEAD: ")));

    let resume = nagi_cli::development::execute(&["resume".into()], root).expect("resume summary");
    assert!(resume.iter().any(|line| line.starts_with("Next action: ")));

    let verify = nagi_cli::development::execute(&["verify".into()], root)
        .expect("state and registry validation");
    assert!(verify[0].starts_with("PASS development state:"));
}

#[test]
fn host_workspace_commands_exclude_kernel() {
    for command in ["build", "test", "clippy"] {
        let args = nagi_cli::commands::host_workspace_args(command);
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--exclude", "nagi-kernel"]));
        assert!(args.contains(&"--locked"));
    }
}

#[test]
fn host_clippy_command_keeps_warning_deny_boundary() {
    let args = nagi_cli::commands::host_workspace_args("clippy");
    assert!(args.windows(2).any(|pair| pair == ["--", "-D"]));
    assert_eq!(args.last(), Some(&"warnings"));
}

#[test]
fn rejects_unexpected_arguments_for_every_command() {
    for name in [
        "doctor", "fetch", "build", "image", "run", "shell", "gui", "desktop", "test", "clean",
        "fmt", "lint", "security", "network", "posix", "std", "m13", "m14", "m15", "m16", "m17",
    ] {
        let error = parse_command(&[name.to_owned(), "unexpected".to_owned()]).unwrap_err();
        assert_eq!(error.exit_code(), EXIT_USAGE, "{name}");
    }
}

#[test]
fn complete_probe_report_passes_and_reports_every_dependency() {
    let probe = StaticProbe {
        commands: vec![
            "git",
            "rustc",
            "cargo",
            "rustup",
            "clang",
            "ld.lld",
            "qemu-system-x86_64",
            "cmake",
            "meson",
            "ninja",
            "python",
        ],
        ovmf_mode: OvmfMode::Compatible,
        bad_output: false,
        exit_code: None,
    };

    let report = run_doctor(&probe, DoctorPolicy::Strict);

    assert_eq!(report.exit_code, EXIT_SUCCESS);
    assert_eq!(report.checks.len(), 12);
    assert!(report.checks.iter().all(|check| check.is_pass()));
}

#[test]
fn configured_minimums_are_enforced_by_the_doctor() {
    let probe = StaticProbe {
        commands: vec!["clang"],
        ..StaticProbe::default()
    };
    let requirements = ToolchainRequirements {
        llvm_min_version: (24, 0, 0),
        ..ToolchainRequirements::default()
    };

    let report = run_doctor_with_requirements(&probe, DoctorPolicy::Strict, &requirements);

    assert_eq!(report.exit_code, nagi_cli::commands::EXIT_DOCTOR_FAILURE);
    assert!(report
        .checks
        .iter()
        .any(|check| check.name == "LLVM/Clang" && check.is_fail()));
}

#[test]
fn project_manifest_supplies_the_pinned_host_requirements() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");

    let requirements = load_toolchain_requirements(root).expect("valid host requirements");

    assert_eq!(requirements.llvm_min_version, (19, 0, 0));
    assert_eq!(requirements.lld_min_version, (17, 0, 0));
    assert_eq!(requirements.qemu_min_version, (8, 0, 0));
    assert_eq!(requirements.python_min_version, (3, 10, 0));
    assert!(requirements
        .ovmf_pairs
        .iter()
        .any(|(code, vars)| { code == "edk2-x86_64-code.fd" && vars == "edk2-i386-vars.fd" }));
}

#[test]
fn strict_probe_report_fails_after_reporting_all_missing_dependencies() {
    let report = run_doctor(&StaticProbe::default(), DoctorPolicy::Strict);

    assert_eq!(report.exit_code, nagi_cli::commands::EXIT_DOCTOR_FAILURE);
    assert_eq!(report.checks.len(), 12);
    assert!(report.checks.iter().all(|check| check.is_fail()));
}

#[test]
fn allow_missing_probe_report_warns_without_hiding_missing_dependencies() {
    let report = run_doctor(&StaticProbe::default(), DoctorPolicy::AllowMissing);

    assert_eq!(report.exit_code, EXIT_SUCCESS);
    assert_eq!(report.checks.len(), 12);
    assert!(report.checks.iter().all(|check| check.is_warn()));
}

#[test]
fn wrong_program_output_is_not_a_pass() {
    let probe = StaticProbe {
        commands: vec!["clang"],
        bad_output: true,
        ..StaticProbe::default()
    };

    let report = run_doctor(&probe, DoctorPolicy::Strict);

    assert_eq!(report.exit_code, nagi_cli::commands::EXIT_DOCTOR_FAILURE);
    assert!(report
        .checks
        .iter()
        .any(|check| check.name == "LLVM/Clang" && check.is_fail()));
}

#[test]
fn nonzero_tool_exit_is_not_a_pass() {
    let probe = StaticProbe {
        commands: vec!["qemu-system-x86_64"],
        exit_code: Some(1),
        ..StaticProbe::default()
    };

    let report = run_doctor(&probe, DoctorPolicy::Strict);

    assert_eq!(report.exit_code, nagi_cli::commands::EXIT_DOCTOR_FAILURE);
    assert!(report
        .checks
        .iter()
        .any(|check| check.name == "QEMU" && check.is_fail()));
}

#[test]
fn an_incomplete_ovmf_pair_is_not_a_pass() {
    let probe = StaticProbe {
        commands: vec![
            "git",
            "rustc",
            "cargo",
            "rustup",
            "clang",
            "ld.lld",
            "qemu-system-x86_64",
            "cmake",
            "meson",
            "ninja",
            "python",
        ],
        ovmf_mode: OvmfMode::Missing,
        ..StaticProbe::default()
    };

    let report = run_doctor(&probe, DoctorPolicy::Strict);

    assert_eq!(report.exit_code, nagi_cli::commands::EXIT_DOCTOR_FAILURE);
    assert!(report
        .checks
        .iter()
        .any(|check| check.name == "OVMF CODE/VARS" && check.is_fail()));
}

#[test]
fn mismatched_ovmf_families_are_not_a_pass() {
    let probe = StaticProbe {
        ovmf_mode: OvmfMode::Mismatch,
        ..StaticProbe::default()
    };

    let report = run_doctor(&probe, DoctorPolicy::Strict);

    assert_eq!(report.exit_code, nagi_cli::commands::EXIT_DOCTOR_FAILURE);
    assert!(report
        .checks
        .iter()
        .any(|check| check.name == "OVMF CODE/VARS" && check.is_fail()));
}

#[test]
fn unreadable_or_non_executable_tool_is_not_a_pass() {
    let directory = std::env::temp_dir().join(format!("nagi-doctor-{}", std::process::id()));
    fs::create_dir_all(&directory).expect("temp directory");
    let fake = directory.join("qemu-system-x86_64");
    fs::write(&fake, b"not an executable").expect("fake tool");

    let probe = SystemProbe::from_path(directory.as_os_str().to_owned());
    let report = run_doctor(&probe, DoctorPolicy::Strict);
    let _ = fs::remove_dir_all(&directory);

    assert_eq!(report.exit_code, nagi_cli::commands::EXIT_DOCTOR_FAILURE);
    assert!(report
        .checks
        .iter()
        .any(|check| check.name == "QEMU" && check.is_fail()));
}

#[test]
fn repository_paths_are_not_used_by_probe_contract() {
    let report = run_doctor(&StaticProbe::default(), DoctorPolicy::AllowMissing);

    assert!(report
        .checks
        .iter()
        .all(|check| !Path::new(check.detail.as_str()).is_absolute()));
}

#[test]
fn formal_nagi_logo_asset_matches_the_boot_contract() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let asset = fs::read_to_string(root.join("assets/nagi/nagi_logo_formal.svg"))
        .expect("formal Nagi logo asset");

    assert!(asset.contains("<svg"));
    assert!(asset.contains("nagiGrad"));
    assert!(asset.contains("M36 49.5C51.5 66.4"));
}

#[test]
fn desktop_command_declares_ordered_boot_markers_before_ready() {
    let source = include_str!("../src/commands.rs");
    let markers = [
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
    ];
    let mut offset = 0;
    for marker in markers {
        let relative = source[offset..]
            .find(marker)
            .unwrap_or_else(|| panic!("missing desktop boot marker: {marker}"));
        offset += relative + marker.len();
    }
}
