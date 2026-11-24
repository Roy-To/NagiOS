use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use crate::config::{load_toolchain_requirements, validate_project};
use crate::diagnostics::{
    ClosureHealthCheck, DiagnosticEvent, DiagnosticsBundle, EvidenceKind, FailureClass,
    HealthCheckRegistry, OverallStatus, PrivacyClass, Severity, VerificationCheckResult,
    VerificationReport, DIAGNOSTIC_BUNDLE_SCHEMA_VERSION,
};
use crate::doctor::{run_doctor_with_requirements, CheckState, DoctorPolicy, HostProbe};
use crate::paths::clean_owned_outputs;

pub const EXIT_SUCCESS: i32 = 0;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_NOT_IMPLEMENTED: i32 = 3;
pub const EXIT_CONFIG_ERROR: i32 = 4;
pub const EXIT_DOCTOR_FAILURE: i32 = 10;
pub const EXIT_VERIFY_FAILURE: i32 = 11;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Help,
    Doctor,
    Diagnostics,
    Verify,
    Smoke,
    Fetch,
    Build,
    Run,
    Test,
    Acceptance,
    Clean,
    Fmt,
    Lint,
    Dev,
    Legal,
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
            "--host-only" if allow_smoke_mode => {
                if smoke_mode_seen {
                    return Err(CliError::new(
                        "smoke: --host-only specified more than once",
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
        "run" => Command::Run,
        "test" if args.get(1).is_some_and(|arg| arg == "--acceptance") => Command::Acceptance,
        "test" => Command::Test,
        "clean" => Command::Clean,
        "fmt" => Command::Fmt,
        "lint" => Command::Lint,
        "dev" => Command::Dev,
        "legal" => Command::Legal,
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
        Command::Help
        | Command::Fetch
        | Command::Build
        | Command::Run
        | Command::Clean
        | Command::Fmt
        | Command::Lint => args.len() == 1,
        Command::Diagnostics | Command::Verify | Command::Smoke | Command::Legal => true,
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
        ] {
            args.extend(["--package", package]);
        }
    }

    if command == "clippy" {
        args.push("--all-targets");
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
        "nagi-abi",
        "nagi-model",
        "nagi-i18n",
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
        Command::Run => execute_run(root, probe),
        Command::Dev => execute_dev(&args[1..], root),
        Command::Legal => execute_legal(&args[1..], root),
    }
}

fn execute_dev(args: &[String], root: &Path) -> CommandResult {
    if args.first().is_some_and(|arg| arg == "fingerprint") {
        return crate::development::execute_fingerprint_cli(&args[1..], root);
    }
    match crate::development::execute(args, root) {
        Ok(lines) => CommandResult {
            exit_code: EXIT_SUCCESS,
            lines,
        },
        Err(error) => failure(error.exit_code(), error.to_string()),
    }
}

fn execute_legal(args: &[String], root: &Path) -> CommandResult {
    // Keep the existing standalone tool and its locked dependency graph intact.
    // Inherit its stdout/stderr and exact exit status rather than adding a PASS
    // banner to JSON output or masking an inventory/policy failure.
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    match ProcessCommand::new(cargo)
        .args(["run", "--quiet", "--locked", "--offline", "--manifest-path"])
        .arg(root.join("tools/legal/Cargo.toml"))
        .arg("--")
        .args(args)
        .current_dir(root)
        .status()
    {
        Ok(status) => CommandResult {
            exit_code: status.code().unwrap_or(EXIT_CONFIG_ERROR),
            lines: Vec::new(),
        },
        Err(error) => failure(
            EXIT_CONFIG_ERROR,
            format!("legal: cannot start cargo: {error}"),
        ),
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
    let report = run_verification_report(root, probe, None, read_source_commit(root));
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
    if let Err(error) =
        crate::fetch_progress::phase("fonts", || crate::fonts::ensure_font_cache(root))
    {
        return failure(EXIT_CONFIG_ERROR, format!("fetch: fonts: {error}"));
    }
    CommandResult {
        exit_code: EXIT_SUCCESS,
        lines: vec!["PASS fetch: verified pinned smoltcp metadata and prepared fonts".into()],
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

fn execute_run(root: &Path, _probe: &dyn HostProbe) -> CommandResult {
    // DEC-123: launch a prepared Linux development image.
    let script = root
        .join("tools")
        .join("linux-boot")
        .join("run_dev_image.py");
    if !script.is_file() {
        return failure(
            EXIT_CONFIG_ERROR,
            format!("run: {} is missing", script.display()),
        );
    }

    let python = if cfg!(windows) { "python" } else { "python3" };
    let status = std::process::Command::new(python)
        .arg("-B")
        .arg(&script)
        .arg(root.join("out").join("linux-dev"))
        .current_dir(root)
        .status();
    match status {
        Ok(status) if status.success() => CommandResult {
            exit_code: EXIT_SUCCESS,
            lines: vec!["run: QEMU exited normally".to_string()],
        },
        Ok(status) => failure(
            EXIT_CONFIG_ERROR,
            format!("run: development image exited with {status}"),
        ),
        Err(error) => failure(
            EXIT_CONFIG_ERROR,
            format!("run: cannot start {python}: {error}"),
        ),
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

fn help() -> CommandResult {
    CommandResult {
        exit_code: EXIT_SUCCESS,
        lines: vec![
            "Nagi OS developer orchestrator".into(),
            "Commands: doctor [--allow-missing], diagnostics [--json|--format text|json] [--scope SCOPE] [--output PATH], verify [--json|--format text|json] [--scope SCOPE] [--output PATH], smoke [--host-only] [--json|--format text|json] [--output PATH], fetch, build, run, dev status|resume|verify|diagnose|fingerprint, legal scan|check|sbom|notice|diff, test, test --acceptance [options], clean, fmt, lint"
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
    #[test]
    fn run_reports_missing_linux_boot_script() {
        let root = std::env::temp_dir().join(format!("nagi-run-missing-{}", std::process::id()));
        let result = super::execute_run(&root, &crate::doctor::SystemProbe::default());
        assert_eq!(result.exit_code, super::EXIT_CONFIG_ERROR);
        assert!(result
            .lines
            .join("\n")
            .contains("run_dev_image.py is missing"));
    }
}
