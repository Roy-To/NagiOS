use std::fs;
use std::path::Path;

fn run_fixture_git(
    root: &Path,
    args: &[&str],
    injected_environment: &[(&str, std::ffi::OsString)],
) -> String {
    let empty_global_config = root
        .parent()
        .unwrap_or(root)
        .join(".nagi-empty-global-gitconfig");
    if !empty_global_config.exists() {
        fs::write(&empty_global_config, b"").expect("create empty fixture git config");
    }
    let mut command = std::process::Command::new("git");
    command.args([
        "-c",
        "user.name=Nagi Test",
        "-c",
        "user.email=nagi@example.test",
        "-c",
        "commit.gpgsign=false",
        "-c",
        "core.hooksPath=.git/hooks",
    ]);
    command.current_dir(root);
    // Apply these first to model inherited contamination; the sanitizer below
    // must remove them without changing the test process environment.
    for (key, value) in injected_environment {
        command.env(key, value);
    }
    let inherited_git_keys = std::env::vars_os()
        .map(|(key, _)| key)
        .filter(|key| {
            key.to_string_lossy()
                .get(..4)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("GIT_"))
        })
        .collect::<Vec<_>>();
    for key in inherited_git_keys {
        command.env_remove(key);
    }
    for (key, _) in injected_environment {
        command.env_remove(key);
    }
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_ATTR_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", &empty_global_config)
        .args(args);
    let output = command.output().expect("run fixture git");
    let succeeded = output.status.success();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    let stdout = String::from_utf8(output.stdout).expect("fixture git output is UTF-8");
    let _ = fs::remove_file(&empty_global_config);
    assert!(succeeded, "git {args:?}: {stderr}");
    stdout.trim().to_owned()
}

use nagi_cli::commands::{
    execute, parse_command, Command, EXIT_CONFIG_ERROR, EXIT_SUCCESS, EXIT_USAGE,
};
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
        ("diagnostics", Command::Diagnostics),
        ("verify", Command::Verify),
        ("smoke", Command::Smoke),
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
        ("m18", Command::M18),
        ("m19", Command::M19),
        ("m22", Command::M22),
        ("m25", Command::M25),
        ("m27", Command::M27),
        ("m30", Command::M30),
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

fn host_cli_output(args: &[&str]) -> std::process::Output {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("repository root");
    let binary = option_env!("CARGO_BIN_EXE_nagi-bootstrap")
        .or(option_env!("CARGO_BIN_EXE_nagi"))
        .expect("host CLI binary");
    std::process::Command::new(binary)
        .args(args)
        .current_dir(root)
        .env("SOURCE_DATE_EPOCH", "1780272000")
        .output()
        .expect("run host CLI")
}

#[test]
fn legal_delegation_preserves_json_and_error_exit_status() {
    assert_eq!(parse_command(&["legal".into()]).unwrap(), Command::Legal);
    let help = host_cli_output(&["legal", "--help"]);
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("nagi-legal"));
    let unknown = host_cli_output(&["legal", "unsupported"]);
    assert_eq!(unknown.status.code(), Some(EXIT_USAGE));
    assert!(String::from_utf8_lossy(&unknown.stderr).contains("unsupported command"));
    let invalid = host_cli_output(&["legal", "scan", "--format", "invalid"]);
    assert_eq!(invalid.status.code(), Some(EXIT_USAGE));
    let missing = host_cli_output(&[
        "legal",
        "diff",
        "--before",
        "absent.json",
        "--after",
        "absent.json",
    ]);
    assert!(!missing.status.success());
    let scan = host_cli_output(&["legal", "scan", "--json"]);
    assert!(
        scan.status.success(),
        "{}",
        String::from_utf8_lossy(&scan.stderr)
    );
    let inventory: serde_json::Value = serde_json::from_slice(&scan.stdout)
        .expect("delegated stdout is JSON without a PASS prefix");
    assert!(inventory["components"].is_array());
}

#[test]
fn delegated_legal_outputs_are_deterministic_and_notice_remains_a_candidate() {
    let output_dir = std::env::temp_dir().join(format!("nagi-legal-cli-{}", std::process::id()));
    fs::create_dir_all(&output_dir).expect("create output directory");
    let inventory = output_dir.join("inventory.json");
    let sbom_a = output_dir.join("a.spdx.json");
    let sbom_b = output_dir.join("b.spdx.json");
    let notice = output_dir.join("NOTICE.candidate.md");
    for args in [
        vec!["legal", "scan", "--output", inventory.to_str().unwrap()],
        vec!["legal", "check"],
        vec!["legal", "sbom", "--output", sbom_a.to_str().unwrap()],
        vec!["legal", "sbom", "--output", sbom_b.to_str().unwrap()],
        vec!["legal", "notice", "--output", notice.to_str().unwrap()],
        vec![
            "legal",
            "diff",
            "--before",
            inventory.to_str().unwrap(),
            "--after",
            inventory.to_str().unwrap(),
        ],
    ] {
        let output = host_cli_output(&args);
        assert!(
            output.status.success(),
            "{args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert_eq!(fs::read(&sbom_a).unwrap(), fs::read(&sbom_b).unwrap());
    let spdx: serde_json::Value = serde_json::from_slice(&fs::read(&sbom_a).unwrap()).unwrap();
    assert_eq!(spdx["spdxVersion"], "SPDX-2.3");
    assert!(fs::read_to_string(&notice)
        .unwrap()
        .to_lowercase()
        .contains("candidate"));
    for file in [&sbom_a, &notice] {
        let content = fs::read_to_string(file).unwrap();
        assert!(
            !content.contains(output_dir.to_str().unwrap()),
            "private output path leaked"
        );
    }
    fs::remove_dir_all(output_dir).expect("remove output fixture");
}

#[test]
fn fingerprint_binary_and_library_dispatch_fail_closed_for_missing_artifacts() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap();
    let args = [
        "dev",
        "fingerprint",
        "--artifact",
        "missing=out/host-test-does-not-exist.bin",
    ];
    let library = execute(&args.map(str::to_owned), root, &StaticProbe::default());
    assert_ne!(library.exit_code, EXIT_SUCCESS);
    assert_eq!(
        host_cli_output(&args).status.code(),
        Some(library.exit_code)
    );
}

#[test]
fn acceptance_runner_is_nested_under_the_existing_test_command() {
    assert_eq!(
        parse_command(&["test".into(), "--acceptance".into()]).unwrap(),
        Command::Acceptance
    );
    assert_eq!(parse_command(&["test".into()]).unwrap(), Command::Test);
    assert!(parse_command(&["test".into(), "--unknown".into()]).is_err());
}

#[test]
fn acceptance_list_uses_the_registry_and_rejects_unregistered_milestones() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let args = ["test", "--acceptance", "--list", "--milestone", "M17"].map(str::to_owned);
    let result = nagi_cli::commands::execute(&args, root, &SystemProbe::default());
    assert_eq!(result.exit_code, EXIT_SUCCESS);
    assert_eq!(result.lines.len(), 1);
    assert!(result.lines[0].starts_with("M17-FIRST-WEB-PIXEL\tM17\tservo\ttarget\t"));

    let args = ["test", "--acceptance", "--list", "--milestone", "M18"].map(str::to_owned);
    let result = nagi_cli::commands::execute(&args, root, &SystemProbe::default());
    assert_eq!(result.exit_code, EXIT_USAGE);
    assert!(result.lines[0].contains("no registered cases match"));
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
    // Exercise every branch case regardless of the caller's checkout or CI merge ref.
    struct Fixture(std::path::PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let fixture = Fixture(std::env::temp_dir().join(format!(
        "nagi-dev-cli-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("fixture timestamp")
            .as_nanos()
    )));
    let root = &fixture.0;
    fs::create_dir(root).expect("create isolated repository");
    let git = |args: &[&str]| run_fixture_git(root, args, &[]);
    git(&[
        "init",
        "--initial-branch=codex/integration-next-phase",
        "--template=",
    ]);
    let registry_path = root.join(".dev/workstreams.json");
    let state_path = root.join(".dev/workstreams/integration-next-phase/state.json");
    fs::create_dir_all(state_path.parent().unwrap()).expect("create fixture workstream");
    fs::write(
        &registry_path,
        include_bytes!("../../../.dev/workstreams.json"),
    )
    .expect("write fixture registry");
    fs::write(
        &state_path,
        include_bytes!("../../../.dev/workstreams/integration-next-phase/state.json"),
    )
    .expect("write fixture state");
    git(&["add", ".dev"]);
    git(&["commit", "-m", "test fixture"]);
    let head = git(&["rev-parse", "HEAD"]);

    let status = nagi_cli::development::execute(&["status".into()], root)
        .expect("registered workstream status");
    assert!(status
        .iter()
        .any(|line| line.starts_with("Workstream: integration-next-phase (")));
    assert!(status
        .iter()
        .any(|line| line == "Branch: codex/integration-next-phase"));
    assert!(status.iter().any(|line| line == &format!("HEAD: {head}")));
    let resume = nagi_cli::development::execute(&["resume".into()], root).expect("resume summary");
    assert!(resume.iter().any(|line| line.starts_with("Next action: ")));
    let verify = nagi_cli::development::execute(&["verify".into()], root)
        .expect("state and registry validation");
    assert!(verify[0].starts_with("PASS development state:"));
    assert!(verify[0].contains("1 state files validated"));

    for args in [
        ["checkout", "-b", "codex/unregistered-fixture"],
        ["checkout", "--detach", "HEAD"],
    ] {
        git(&args);
        for action in ["status", "resume"] {
            let error = nagi_cli::development::execute(&[action.into()], root)
                .expect_err("unregistered branches cannot guess their workstream");
            assert_eq!(error.exit_code(), nagi_cli::commands::EXIT_CONFIG_ERROR);
            assert!(error.to_string().contains("is not registered"));
        }
        let verify = nagi_cli::development::execute(&["verify".into()], root)
            .expect("verify validates registry and states independently of branch registration");
        assert!(verify[0].starts_with("PASS development state:"));
    }

    git(&["checkout", "codex/integration-next-phase"]);
    for path in [&state_path, &registry_path] {
        let valid = fs::read(path).expect("read valid fixture");
        fs::write(path, b"{").expect("corrupt fixture JSON");
        for action in ["status", "resume", "verify"] {
            let error = nagi_cli::development::execute(&[action.into()], root)
                .expect_err("malformed state and registry must be rejected");
            assert_eq!(error.exit_code(), nagi_cli::commands::EXIT_CONFIG_ERROR);
            assert!(error.to_string().contains("invalid JSON"));
        }
        fs::write(path, valid).expect("restore valid fixture");
    }
}

#[test]
fn fixture_git_ignores_inherited_repository_environment() {
    struct Fixture(std::path::PathBuf);
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let fixture = Fixture(std::env::temp_dir().join(format!(
        "nagi-dev-cli-git-env-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    )));
    fs::create_dir_all(&fixture.0).unwrap();
    let outer = fixture.0.join("outer");
    let scratch = fixture.0.join("scratch");
    fs::create_dir(&outer).unwrap();
    fs::create_dir(&scratch).unwrap();
    run_fixture_git(
        &outer,
        &["init", "--initial-branch=main", "--template="],
        &[],
    );
    fs::write(outer.join("sentinel"), b"outer repository").unwrap();
    run_fixture_git(&outer, &["add", "sentinel"], &[]);
    run_fixture_git(&outer, &["commit", "-m", "outer sentinel"], &[]);
    let outer_head = run_fixture_git(&outer, &["rev-parse", "HEAD"], &[]);
    let outer_index = fs::read(outer.join(".git/index")).unwrap();
    let outer_config = fs::read(outer.join(".git/config")).unwrap();

    run_fixture_git(
        &scratch,
        &[
            "init",
            "--initial-branch=codex/integration-next-phase",
            "--template=",
        ],
        &[],
    );
    fs::write(outer.join("candidate.txt"), b"outer-only candidate").unwrap();
    fs::write(scratch.join("candidate.txt"), b"scratch-only candidate").unwrap();
    let git_environment_before = std::env::vars_os()
        .filter(|(key, _)| {
            key.to_string_lossy()
                .get(..4)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("GIT_"))
        })
        .collect::<Vec<_>>();
    let injected = [
        ("GIT_DIR", outer.join(".git").into_os_string()),
        ("GIT_WORK_TREE", outer.clone().into_os_string()),
        ("GIT_INDEX_FILE", outer.join(".git/index").into_os_string()),
        (
            "GIT_OBJECT_DIRECTORY",
            outer.join(".git/objects").into_os_string(),
        ),
        (
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            outer.join(".git/objects").into_os_string(),
        ),
        ("GIT_CONFIG_COUNT", "1".into()),
        ("GIT_CONFIG_KEY_0", "core.hooksPath".into()),
        (
            "GIT_CONFIG_VALUE_0",
            outer.join("unsafe-hooks").into_os_string(),
        ),
    ];
    run_fixture_git(&scratch, &["add", "candidate.txt"], &injected);
    assert_eq!(
        run_fixture_git(&outer, &["rev-parse", "HEAD"], &[]),
        outer_head
    );
    assert_eq!(fs::read(outer.join(".git/index")).unwrap(), outer_index);
    assert_eq!(fs::read(outer.join(".git/config")).unwrap(), outer_config);
    assert_eq!(
        run_fixture_git(&outer, &["status", "--porcelain"], &[]),
        "?? candidate.txt"
    );
    assert_eq!(
        run_fixture_git(&scratch, &["status", "--porcelain"], &[]),
        "A  candidate.txt"
    );
    assert_eq!(
        std::env::vars_os()
            .filter(|(key, _)| {
                key.to_string_lossy()
                    .get(..4)
                    .is_some_and(|prefix| prefix.eq_ignore_ascii_case("GIT_"))
            })
            .collect::<Vec<_>>(),
        git_environment_before,
        "fixture setup must not mutate process-global Git environment"
    );

    let registry_path = scratch.join(".dev/workstreams.json");
    let state_path = scratch.join(".dev/workstreams/integration-next-phase/state.json");
    fs::create_dir_all(state_path.parent().unwrap()).unwrap();
    fs::write(
        &registry_path,
        include_bytes!("../../../.dev/workstreams.json"),
    )
    .unwrap();
    fs::write(
        &state_path,
        include_bytes!("../../../.dev/workstreams/integration-next-phase/state.json"),
    )
    .unwrap();
    run_fixture_git(&scratch, &["add", ".dev"], &[]);
    run_fixture_git(&scratch, &["commit", "-m", "fixture workstream"], &[]);
    let status = nagi_cli::development::execute(&["status".into()], &scratch)
        .expect("production status path reads the isolated fixture repository");
    assert!(status
        .iter()
        .any(|line| line == "Branch: codex/integration-next-phase"));
}

#[test]
fn host_workspace_commands_exclude_kernel() {
    for command in ["build", "test", "clippy"] {
        let args = nagi_cli::commands::host_workspace_args_for_arch(command, "x86_64");
        assert!(args
            .windows(2)
            .any(|pair| pair == ["--exclude", "nagi-kernel"]));
        assert!(args.contains(&"--locked"));
    }
}

#[test]
fn host_clippy_command_keeps_warning_deny_boundary() {
    let args = nagi_cli::commands::host_workspace_args_for_arch("clippy", "x86_64");
    assert!(args.windows(2).any(|pair| pair == ["--", "-D"]));
    assert_eq!(args.last(), Some(&"warnings"));
}

#[test]
fn format_commands_match_ci_source_scope_without_vendored_servo() {
    let commands = nagi_cli::commands::host_format_commands();
    assert_eq!(commands.len(), 4);

    let (cargo, workspace_args) = &commands[0];
    assert_eq!(*cargo, "cargo");
    assert_eq!(workspace_args.first(), Some(&"fmt"));
    assert!(!workspace_args.contains(&"--all"));
    for package in [
        "nagi-cli",
        "nagi-idl",
        "nagi-bootinfo",
        "nagi-abi",
        "nagi-model",
        "nagi-kernel",
        "libnagi",
        "nagi-net",
        "nagi-pal",
        "nagi-posix",
        "nagi-audio",
        "nagi-history",
        "nagi-model-manager",
        "nagi-search",
        "nagi-ai",
        "nagi-servo-adapter",
        "nagi-init",
        "nagi-package",
        "nagi-sdk",
    ] {
        assert!(workspace_args
            .windows(2)
            .any(|pair| pair == ["--package", package]));
    }
    assert!(workspace_args.ends_with(&["--", "--check"]));

    assert_eq!(
        commands[1],
        (
            "cargo",
            vec![
                "fmt",
                "--manifest-path",
                "tools/nagi-pkg/Cargo.toml",
                "--",
                "--check"
            ]
        )
    );
    assert_eq!(
        commands[2],
        (
            "cargo",
            vec![
                "fmt",
                "--manifest-path",
                "loader/Cargo.toml",
                "--",
                "--check"
            ]
        )
    );
    assert_eq!(
        commands[3],
        ("rustfmt", vec!["--check", "user/nagi-albert/src/lib.rs"])
    );
    assert!(commands
        .iter()
        .flat_map(|(_, args)| args)
        .all(|arg| !arg.contains("third_party/servo")));
}

struct PortingFixture(std::path::PathBuf);

impl PortingFixture {
    fn new(label: &str) -> Self {
        let path = std::env::temp_dir().join(format!(
            "nagi-port-readiness-{label}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("fixture timestamp")
                .as_nanos()
        ));
        fs::create_dir_all(&path).expect("create porting fixture");
        Self(path)
    }

    fn write(&self, relative: &str, contents: &str) {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().expect("fixture parent"))
            .expect("create fixture subdirectory");
        fs::write(path, contents).expect("write fixture source");
    }

    fn write_bytes(&self, relative: &str, contents: &[u8]) {
        let path = self.0.join(relative);
        fs::create_dir_all(path.parent().expect("fixture parent"))
            .expect("create fixture subdirectory");
        fs::write(path, contents).expect("write fixture bytes");
    }

    fn readiness(&self) -> std::process::Output {
        host_cli_output(&[
            "port",
            "readiness",
            self.0.to_str().expect("UTF-8 fixture path"),
            "--format",
            "json",
        ])
    }

    fn readiness_text(&self) -> std::process::Output {
        host_cli_output(&[
            "port",
            "readiness",
            self.0.to_str().expect("UTF-8 fixture path"),
            "--format",
            "text",
        ])
    }

    fn readiness_from_external_cwd(&self) -> std::process::Output {
        let binary = option_env!("CARGO_BIN_EXE_nagi-bootstrap")
            .or(option_env!("CARGO_BIN_EXE_nagi"))
            .expect("host CLI binary");
        std::process::Command::new(binary)
            .args(["port", "readiness", ".", "--format", "json"])
            .current_dir(&self.0)
            .env_remove("NAGI_RUN_RECEIPT_DIR")
            .env_remove("NAGI_RUN_ID")
            .env_remove("NAGI_RUN_STAGE")
            .output()
            .expect("run readiness command outside a Nagi checkout")
    }

    fn readiness_with_receipt(&self, receipt_dir: &Path) -> std::process::Output {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("repository root");
        let binary = option_env!("CARGO_BIN_EXE_nagi-bootstrap")
            .or(option_env!("CARGO_BIN_EXE_nagi"))
            .expect("host CLI binary");
        std::process::Command::new(binary)
            .args([
                "port",
                "readiness",
                self.0.to_str().expect("UTF-8 fixture path"),
                "--format",
                "json",
            ])
            .current_dir(repo_root)
            .env("NAGI_RUN_RECEIPT_DIR", receipt_dir)
            .env("NAGI_RUN_ID", "port-readiness-run")
            .env("NAGI_RUN_STAGE", "port-readiness-test")
            .output()
            .expect("run readiness with receipts from a Nagi checkout")
    }

    fn readiness_from_external_cwd_with_receipt(&self, receipt_dir: &Path) -> std::process::Output {
        let binary = option_env!("CARGO_BIN_EXE_nagi-bootstrap")
            .or(option_env!("CARGO_BIN_EXE_nagi"))
            .expect("host CLI binary");
        std::process::Command::new(binary)
            .args(["port", "readiness", ".", "--format", "json"])
            .current_dir(&self.0)
            .env("NAGI_RUN_RECEIPT_DIR", receipt_dir)
            .env("NAGI_RUN_ID", "external-port-readiness-run")
            .env("NAGI_RUN_STAGE", "port-readiness-test")
            .output()
            .expect("run readiness with receipts outside a Nagi checkout")
    }
}

impl Drop for PortingFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn port_readiness_reports_a_portable_rust_project_without_running_build_scripts() {
    let fixture = PortingFixture::new("portable-rust");
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"portable\"\nversion = \"0.1.0\"\n",
    );
    fixture.write("src/lib.rs", "pub fn answer() -> u8 { 42 }\n");
    fixture.write(
        "build.rs",
        "fn main() { println!(\"cargo:warning=build-script-ran\"); }\n",
    );

    let output = fixture.readiness();
    assert!(
        output.status.success(),
        "readiness command failed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON report");
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["detected_languages"], serde_json::json!(["rust"]));
    assert_eq!(
        report["analysis"]["conclusion"],
        "no-known-boundary-detected"
    );
    assert!(report["findings"].as_array().unwrap().is_empty());
    assert!(!String::from_utf8_lossy(&output.stdout).contains("build-script-ran"));
}

#[test]
fn port_readiness_reports_rust_filesystem_process_and_unverified_dependencies() {
    let fixture = PortingFixture::new("rust-boundaries");
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"boundaries\"\nversion = \"0.1.0\"\n[dependencies]\nserde = \"1\"\n",
    );
    fixture.write(
        "src/main.rs",
        "fn main() { let _ = std::fs::read(\"data\"); let _ = std::process::Command::new(\"tool\"); }\n",
    );

    let output = fixture.readiness();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON report");
    let findings = report["findings"].as_array().unwrap();
    assert!(findings.iter().any(|item| item["category"] == "filesystem"));
    assert!(findings.iter().any(|item| item["category"] == "process"));
    assert_eq!(report["dependencies"][0]["name"], "serde");
    assert_eq!(
        report["dependencies"][0]["review_status"],
        "requires-review"
    );
    assert_eq!(report["analysis"]["conclusion"], "requires-review");
}

#[test]
fn port_readiness_reports_c_process_and_thread_boundaries() {
    let fixture = PortingFixture::new("c-boundaries");
    fixture.write("Makefile", "all:\n\tcc main.c -lpthread\n");
    fixture.write(
        "main.c",
        "#include <pthread.h>\n#include <unistd.h>\nvoid *work(void *p) { return p; }\nint main(void) { pthread_create(0, 0, work, 0); fork(); }\n",
    );

    let output = fixture.readiness();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON report");
    assert_eq!(report["detected_languages"], serde_json::json!(["c"]));
    let findings = report["findings"].as_array().unwrap();
    assert!(findings.iter().any(|item| item["category"] == "process"));
    assert!(findings.iter().any(|item| item["category"] == "threading"));
}

#[test]
fn port_readiness_reports_cpp_gui_framework_evidence_for_manual_review() {
    let fixture = PortingFixture::new("cpp-gui");
    fixture.write(
        "CMakeLists.txt",
        "find_package(Qt6 REQUIRED COMPONENTS Widgets)\npkg_check_modules(PCRE2 REQUIRED IMPORTED_TARGET libpcre2-8)\n",
    );
    fixture.write(
        "src/main.cpp",
        "#include <QApplication>\nint main(int argc, char **argv) { QApplication app(argc, argv); }\n",
    );

    let output = fixture.readiness();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON report");
    assert_eq!(report["detected_languages"], serde_json::json!(["c++"]));
    assert!(report["findings"].as_array().unwrap().iter().any(|item| {
        item["category"] == "gui-framework" && item["review_status"] == "requires-review"
    }));
    assert!(report["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| { item["name"] == "Qt6" && item["review_status"] == "requires-review" }));
    assert!(report["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| {
            item["name"] == "libpcre2-8" && item["review_status"] == "requires-review"
        }));
    assert!(report["limitations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| { item.as_str().unwrap().contains("compatibility") }));
}

#[test]
fn port_readiness_rejects_a_file_as_project_root() {
    let fixture = PortingFixture::new("malformed-root");
    fixture.write("not-a-project", "plain file\n");
    let file = fixture.0.join("not-a-project");
    let output = host_cli_output(&[
        "port",
        "readiness",
        file.to_str().expect("UTF-8 fixture path"),
    ]);
    assert_eq!(output.status.code(), Some(4));
}

#[test]
fn port_readiness_json_is_stable_across_repeated_runs() {
    let fixture = PortingFixture::new("stable-json");
    fixture.write("main.c", "int main(void) { return 0; }\n");
    let first = fixture.readiness();
    let second = fixture.readiness();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stdout)
    );
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stdout)
    );
    assert_eq!(first.stdout, second.stdout);
}

#[test]
fn port_readiness_bounds_directory_only_trees() {
    let fixture = PortingFixture::new("deep-empty-tree");
    let mut path = fixture.0.clone();
    for _ in 0..70 {
        path.push("d");
        fs::create_dir(&path).expect("create nested empty directory");
    }

    let output = fixture.readiness();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON report");
    assert_eq!(report["analysis"]["truncated"], true);
    assert_eq!(report["analysis"]["files_scanned"], 0);
}

#[test]
fn port_readiness_counts_non_utf8_bytes_toward_the_total_read_limit() {
    let fixture = PortingFixture::new("non-utf8-total-limit");
    let invalid_source = vec![0xff; 512 * 1024];
    for index in 0..41 {
        fixture.write_bytes(&format!("src/file-{index:02}.rs"), &invalid_source);
    }

    let output = fixture.readiness();
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON report");
    assert_eq!(report["analysis"]["bytes_read"], 20 * 1024 * 1024);
    assert_eq!(report["analysis"]["truncated"], true);
    assert!(report["limitations"]
        .as_array()
        .unwrap()
        .iter()
        .any(|item| { item.as_str().unwrap().contains("file or total-size limit") }));
}

#[test]
fn port_readiness_does_not_echo_source_literals_or_terminal_controls() {
    let fixture = PortingFixture::new("untrusted-source-text");
    fixture.write(
        "main.c",
        "int main(void) { fopen(\"C:\\\\Users\\\\alice\\\\private.txt\", \"r\"); /*\u{1b}[31m*/ }\n",
    );

    let json = fixture.readiness();
    assert!(
        json.status.success(),
        "{}",
        String::from_utf8_lossy(&json.stdout)
    );
    let json = String::from_utf8_lossy(&json.stdout);
    assert!(!json.contains("alice"));
    assert!(!json
        .chars()
        .any(|character| character != '\n' && char::is_control(character)));

    let text = fixture.readiness_text();
    assert!(
        text.status.success(),
        "{}",
        String::from_utf8_lossy(&text.stdout)
    );
    let text = String::from_utf8_lossy(&text.stdout);
    assert!(!text.contains("alice"));
    assert!(!text
        .chars()
        .any(|character| character != '\n' && char::is_control(character)));
}

#[test]
fn port_readiness_shows_dependencies_and_scan_limits_in_text_output() {
    let fixture = PortingFixture::new("text-summary");
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"text-summary\"\nversion = \"0.1.0\"\n[dependencies]\nserde = \"1\"\n",
    );
    fixture.write(
        "src/lib.rs",
        "pub fn read() { let _ = std::fs::read(\"x\"); }\n",
    );
    let mut path = fixture.0.join("deep");
    for _ in 0..70 {
        path.push("d");
        fs::create_dir_all(&path).expect("create nested directory");
    }

    let output = fixture.readiness_text();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(text.contains("serde"));
    assert!(text.contains("Cargo.toml"));
    assert!(text.contains("truncated: yes"));
    assert!(text.contains("directory-depth limit"));
    assert!(text.contains("does not establish Nagi"));
}

#[test]
fn port_readiness_can_run_from_an_external_project_directory() {
    let fixture = PortingFixture::new("external-cwd");
    fixture.write("src/lib.rs", "pub fn portable() -> bool { true }\n");

    let output = fixture.readiness_from_external_cwd();
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).expect("JSON report");
    assert_eq!(report["detected_languages"], serde_json::json!(["rust"]));
}

#[test]
fn port_readiness_preserves_explicit_run_receipts_in_a_nagi_checkout() {
    let fixture = PortingFixture::new("receipt-in-checkout");
    fixture.write("src/lib.rs", "pub fn portable() -> bool { true }\n");
    let receipt_dir = fixture.0.join("receipts");

    let output = fixture.readiness_with_receipt(&receipt_dir);
    assert!(
        output.status.success(),
        "stdout: {}\nstderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(receipt_dir.join("started.json").is_file());
    let terminal = fs::read(receipt_dir.join("terminal.json")).expect("terminal receipt");
    let terminal: serde_json::Value = serde_json::from_slice(&terminal).expect("receipt JSON");
    assert_eq!(terminal["command_result"], 0);
}

#[test]
fn port_readiness_rejects_explicit_run_receipts_outside_a_nagi_checkout() {
    let fixture = PortingFixture::new("receipt-outside-checkout");
    fixture.write("src/lib.rs", "pub fn portable() -> bool { true }\n");
    let receipt_dir = fixture.0.join("receipts");

    let output = fixture.readiness_from_external_cwd_with_receipt(&receipt_dir);
    assert_eq!(output.status.code(), Some(EXIT_CONFIG_ERROR));
    assert!(String::from_utf8_lossy(&output.stderr)
        .contains("run receipts require a Nagi repository when enabled"));
    assert!(!receipt_dir.exists());
}

#[test]
fn arm64_host_commands_select_only_host_compatible_packages() {
    let host_packages = [
        "nagi-cli",
        "nagi-idl",
        "nagi-bootinfo",
        "nagi-abi",
        "nagi-model",
        "nagi-audio",
        "nagi-history",
        "nagi-package",
        "nagi-model-manager",
        "nagi-search",
        "nagi-ai",
        "nagi-servo-adapter",
    ];

    for command in ["build", "test", "clippy"] {
        let args = nagi_cli::commands::host_workspace_args_for_arch(command, "aarch64");
        assert_eq!(args.first(), Some(&command));
        assert!(!args.contains(&"--workspace"));
        assert!(args.contains(&"--locked"));
        for package in host_packages {
            assert!(args.windows(2).any(|pair| pair == ["--package", package]));
        }
        for target_only_package in ["libnagi", "nagi-net", "nagi-pal", "nagi-posix", "nagi-init"] {
            assert!(!args
                .windows(2)
                .any(|pair| pair == ["--package", target_only_package]));
        }
    }

    let clippy = nagi_cli::commands::host_workspace_args_for_arch("clippy", "aarch64");
    assert!(clippy.contains(&"--all-targets"));
    assert!(clippy.windows(2).any(|pair| pair == ["--", "-D"]));
    assert_eq!(clippy.last(), Some(&"warnings"));
}

#[test]
fn rejects_unexpected_arguments_for_every_command() {
    for name in [
        "doctor",
        "diagnostics",
        "verify",
        "smoke",
        "fetch",
        "build",
        "image",
        "run",
        "shell",
        "gui",
        "desktop",
        "test",
        "clean",
        "fmt",
        "lint",
        "security",
        "network",
        "posix",
        "std",
        "m13",
        "m14",
        "m15",
        "m16",
        "m17",
        "m18",
        "m19",
        "m22",
        "m25",
        "m27",
        "m30",
    ] {
        let error = parse_command(&[name.to_owned(), "unexpected".to_owned()]).unwrap_err();
        assert_eq!(error.exit_code(), EXIT_USAGE, "{name}");
    }
}

#[test]
fn diagnostics_commands_validate_modes_and_options() {
    assert_eq!(
        parse_command(&[
            "verify".into(),
            "--scope".into(),
            "diagnostics".into(),
            "--format".into(),
            "json".into(),
        ])
        .unwrap(),
        Command::Verify
    );
    assert_eq!(
        parse_command(&["diagnostics".into(), "--json".into()]).unwrap(),
        Command::Diagnostics
    );
    assert_eq!(
        parse_command(&[
            "smoke".into(),
            "--vm".into(),
            "--format".into(),
            "json".into()
        ])
        .unwrap(),
        Command::Smoke
    );
    for args in [
        vec!["verify".into(), "--format".into(), "xml".into()],
        vec!["verify".into(), "--scope".into(), "".into()],
        vec!["smoke".into(), "--vm".into(), "--host-only".into()],
        vec!["diagnostics".into(), "--unknown".into()],
        vec![
            "verify".into(),
            "--json".into(),
            "--format".into(),
            "json".into(),
        ],
    ] {
        assert_eq!(parse_command(&args).unwrap_err().exit_code(), EXIT_USAGE);
    }
}

#[test]
fn focused_diagnostics_verification_emits_a_machine_report() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let result = execute(
        &[
            "verify".into(),
            "--scope".into(),
            "diagnostics".into(),
            "--format".into(),
            "json".into(),
        ],
        root,
        &StaticProbe::default(),
    );
    assert_eq!(
        result.exit_code,
        EXIT_SUCCESS,
        "{}",
        result.lines.join("\n")
    );
    let report: serde_json::Value = serde_json::from_str(&result.lines.join("\n")).unwrap();
    assert_eq!(report["outcome"], "PASS");
    assert_eq!(report["requested_scope"], "diagnostics");
    assert_eq!(report["checks"][0]["evidence_kind"], "HOST");
}

#[test]
fn registered_workstream_validator_reports_verified_state() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let result = execute(
        &[
            "verify".into(),
            "--scope".into(),
            "workstreams".into(),
            "--format".into(),
            "json".into(),
        ],
        root,
        &StaticProbe::default(),
    );
    assert_eq!(
        result.exit_code,
        EXIT_SUCCESS,
        "{}",
        result.lines.join("\n")
    );
    let report: serde_json::Value = serde_json::from_str(&result.lines.join("\n")).unwrap();
    assert_eq!(report["outcome"], "PASS");
    assert_eq!(report["checks"][0]["status"], "PASS");
    assert_eq!(report["checks"][0]["evidence_kind"], "HOST");
    assert_eq!(report["checks"][0]["id"], "workstream-state-registry");
    assert!(report["checks"][0]["summary"]
        .as_str()
        .unwrap()
        .contains("state files validated"));
}

#[test]
fn diagnostics_bundle_is_local_and_versioned() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("workspace root");
    let result = execute(
        &[
            "diagnostics".into(),
            "--scope".into(),
            "diagnostics".into(),
            "--format".into(),
            "json".into(),
        ],
        root,
        &StaticProbe::default(),
    );
    assert_eq!(
        result.exit_code,
        EXIT_SUCCESS,
        "{}",
        result.lines.join("\n")
    );
    let report: serde_json::Value = serde_json::from_str(&result.lines.join("\n")).unwrap();
    assert_eq!(report["schema_version"], 1);
    assert_eq!(report["verification"]["outcome"], "PASS");
    assert_eq!(
        report["events"][0]["event_code"],
        "DIAGNOSTICS.REPORT_CREATED"
    );
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
fn doctor_recognizes_python3_without_a_python_alias() {
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
            "python3",
        ],
        ovmf_mode: OvmfMode::Compatible,
        bad_output: false,
        exit_code: None,
    };

    let report = run_doctor(&probe, DoctorPolicy::Strict);

    assert_eq!(report.exit_code, EXIT_SUCCESS);
    assert!(report.checks.iter().any(|check| {
        check.name == "Python" && check.is_pass() && check.detail.contains("python3")
    }));
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
