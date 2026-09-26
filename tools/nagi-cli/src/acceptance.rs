use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::commands::{CommandResult, EXIT_CONFIG_ERROR, EXIT_SUCCESS, EXIT_USAGE};

const REGISTRY_PATH: &str = "tests/acceptance/registry.tsv";
const EXIT_ACCEPTANCE_FAILURE: i32 = 11;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    Host,
    Target,
}

impl Scope {
    fn as_str(self) -> &'static str {
        match self {
            Self::Host => "host",
            Self::Target => "target",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AcceptanceCase {
    id: String,
    milestone: String,
    subsystem: String,
    scope: Scope,
    script_stem: String,
    timeout_seconds: u64,
    name: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Pass,
    Fail,
    Skip,
    Blocked,
    NotRun,
}

impl Status {
    fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
            Self::Skip => "SKIP",
            Self::Blocked => "BLOCKED",
            Self::NotRun => "NOT RUN",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureStage {
    Environment,
    Dependency,
    Compile,
    Link,
    Test,
    Timeout,
    Artifact,
    Command,
}

impl FailureStage {
    fn as_str(self) -> &'static str {
        match self {
            Self::Environment => "environment",
            Self::Dependency => "dependency",
            Self::Compile => "compile",
            Self::Link => "link",
            Self::Test => "test",
            Self::Timeout => "timeout",
            Self::Artifact => "artifact",
            Self::Command => "command",
        }
    }
}

#[derive(Debug, Default)]
struct Options {
    milestone: Option<String>,
    subsystem: Option<String>,
    scope: Option<Scope>,
    ci: bool,
    verbose: bool,
    list: bool,
    json_path: Option<PathBuf>,
    compare_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
struct CaseResult {
    case: AcceptanceCase,
    status: Status,
    duration_ms: u128,
    command: String,
    exit_code: Option<i32>,
    failure_stage: Option<FailureStage>,
    diagnostic: Option<String>,
    log_path: Option<String>,
    previous_status: Option<String>,
    regressed: bool,
}

fn parse_registry(contents: &str) -> Result<Vec<AcceptanceCase>, String> {
    let mut cases = Vec::new();
    let mut ids = HashSet::new();
    for (index, line) in contents.lines().enumerate() {
        let line_number = index + 1;
        let line = line.trim_end_matches('\r');
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        let fields: Vec<_> = line.splitn(7, '\t').collect();
        if fields.len() != 7 {
            return Err(format!(
                "{REGISTRY_PATH}:{line_number}: expected 7 tab-separated fields"
            ));
        }
        let milestone = normalize_milestone(fields[1])
            .ok_or_else(|| format!("{REGISTRY_PATH}:{line_number}: invalid milestone"))?;
        let scope = match fields[3] {
            "host" => Scope::Host,
            "target" => Scope::Target,
            _ => {
                return Err(format!(
                    "{REGISTRY_PATH}:{line_number}: scope must be host or target"
                ));
            }
        };
        let timeout_seconds = fields[5]
            .parse::<u64>()
            .map_err(|_| format!("{REGISTRY_PATH}:{line_number}: invalid timeout_seconds"))?;
        if timeout_seconds == 0 {
            return Err(format!(
                "{REGISTRY_PATH}:{line_number}: timeout_seconds must be nonzero"
            ));
        }
        if fields[0].is_empty()
            || !fields[0]
                .bytes()
                .all(|byte| byte.is_ascii_uppercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(format!(
                "{REGISTRY_PATH}:{line_number}: invalid acceptance ID"
            ));
        }
        if fields[2].is_empty()
            || !fields[2]
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(format!("{REGISTRY_PATH}:{line_number}: invalid subsystem"));
        }
        if fields[4].is_empty()
            || !fields[4]
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        {
            return Err(format!(
                "{REGISTRY_PATH}:{line_number}: invalid script stem"
            ));
        }
        if fields[6].trim().is_empty() {
            return Err(format!("{REGISTRY_PATH}:{line_number}: test name is empty"));
        }
        if !ids.insert(fields[0].to_owned()) {
            return Err(format!(
                "{REGISTRY_PATH}:{line_number}: duplicate ID `{}`",
                fields[0]
            ));
        }
        cases.push(AcceptanceCase {
            id: fields[0].to_owned(),
            milestone,
            subsystem: fields[2].to_owned(),
            scope,
            script_stem: fields[4].to_owned(),
            timeout_seconds,
            name: fields[6].trim().to_owned(),
        });
    }
    if cases.is_empty() {
        return Err(format!("{REGISTRY_PATH}: registry has no acceptance cases"));
    }
    Ok(cases)
}

fn normalize_milestone(value: &str) -> Option<String> {
    let suffix = value
        .strip_prefix('M')
        .or_else(|| value.strip_prefix('m'))?;
    if suffix.is_empty() || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    suffix
        .parse::<u16>()
        .ok()
        .map(|number| format!("M{number}"))
}

fn parse_options(args: &[String]) -> Result<Options, String> {
    let mut options = Options::default();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        match flag {
            "--milestone" | "--subsystem" | "--json" | "--compare" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| format!("{flag} requires a value"))?;
                if value.starts_with("--") {
                    return Err(format!("{flag} requires a value"));
                }
                match flag {
                    "--milestone" => {
                        if options.milestone.is_some() {
                            return Err("--milestone may only be specified once".into());
                        }
                        options.milestone = Some(
                            normalize_milestone(value)
                                .ok_or_else(|| format!("invalid milestone `{value}`"))?,
                        );
                    }
                    "--subsystem" => {
                        if options.subsystem.is_some() {
                            return Err("--subsystem may only be specified once".into());
                        }
                        if value.is_empty()
                            || !value.bytes().all(|byte| {
                                byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-'
                            })
                        {
                            return Err(format!("invalid subsystem `{value}`"));
                        }
                        options.subsystem = Some(value.clone());
                    }
                    "--json" => {
                        if options.json_path.replace(PathBuf::from(value)).is_some() {
                            return Err("--json may only be specified once".into());
                        }
                    }
                    "--compare" => {
                        if options.compare_path.replace(PathBuf::from(value)).is_some() {
                            return Err("--compare may only be specified once".into());
                        }
                    }
                    _ => unreachable!(),
                }
                index += 2;
            }
            "--host-only" | "--target-only" => {
                if options.scope.is_some() {
                    return Err("--host-only and --target-only are mutually exclusive".into());
                }
                options.scope = Some(if flag == "--host-only" {
                    Scope::Host
                } else {
                    Scope::Target
                });
                index += 1;
            }
            "--ci" => {
                options.ci = true;
                index += 1;
            }
            "--verbose" => {
                options.verbose = true;
                index += 1;
            }
            "--list" => {
                options.list = true;
                index += 1;
            }
            "--help" | "-h" => return Err(acceptance_help().join("\n")),
            other => return Err(format!("unknown acceptance option `{other}`")),
        }
    }
    if options.list
        && (options.ci
            || options.verbose
            || options.json_path.is_some()
            || options.compare_path.is_some())
    {
        return Err("--list cannot be combined with execution or report options".into());
    }
    Ok(options)
}

pub fn execute(args: &[String], root: &Path) -> CommandResult {
    let options = match parse_options(args) {
        Ok(options) => options,
        Err(error) => {
            let code = if error.starts_with("Nagi acceptance usage:") {
                EXIT_SUCCESS
            } else {
                EXIT_USAGE
            };
            return CommandResult {
                exit_code: code,
                lines: error.lines().map(str::to_owned).collect(),
            };
        }
    };
    let registry = match fs::read_to_string(root.join(REGISTRY_PATH))
        .map_err(|error| format!("cannot read {REGISTRY_PATH}: {error}"))
        .and_then(|contents| parse_registry(&contents))
    {
        Ok(registry) => registry,
        Err(error) => {
            return CommandResult {
                exit_code: EXIT_CONFIG_ERROR,
                lines: vec![format!("FAIL acceptance registry: {error}")],
            }
        }
    };

    if options.list {
        return list_cases(&registry, &options);
    }

    let selected: Vec<_> = registry
        .iter()
        .filter(|case| {
            options
                .milestone
                .as_ref()
                .is_none_or(|milestone| &case.milestone == milestone)
                && options
                    .subsystem
                    .as_ref()
                    .is_none_or(|subsystem| &case.subsystem == subsystem)
                && options.scope.is_none_or(|scope| case.scope == scope)
        })
        .cloned()
        .collect();
    if selected.is_empty() {
        return CommandResult {
            exit_code: EXIT_USAGE,
            lines: vec!["FAIL acceptance selection: no registered cases match the filters".into()],
        };
    }

    let previous = match options.compare_path.as_deref() {
        Some(path) => match read_previous_results(root, path) {
            Ok(previous) => Some(previous),
            Err(error) => {
                return CommandResult {
                    exit_code: EXIT_CONFIG_ERROR,
                    lines: vec![format!("FAIL acceptance comparison: {error}")],
                }
            }
        },
        None => None,
    };
    let run_id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    let log_directory = PathBuf::from("out/logs/acceptance").join(run_id.to_string());
    let timeout_json = if options.ci && options.json_path.is_none() {
        Some(PathBuf::from(format!(
            "out/test-results/acceptance-{run_id}.json"
        )))
    } else {
        options.json_path.clone()
    };

    let mut results = Vec::with_capacity(registry.len());
    let mut selected_iter = selected.iter().peekable();
    for case in &registry {
        let was_selected = selected_iter
            .peek()
            .is_some_and(|selected_case| selected_case.id == case.id);
        if was_selected {
            let selected_case = selected_iter.next().expect("peeked selected case");
            let result = run_case(root, selected_case, &log_directory, options.verbose);
            println_result(&result);
            results.push(result);
        } else {
            results.push(CaseResult {
                case: case.clone(),
                status: Status::NotRun,
                duration_ms: 0,
                command: String::new(),
                exit_code: None,
                failure_stage: None,
                diagnostic: Some("not selected by filters".into()),
                log_path: None,
                previous_status: None,
                regressed: false,
            });
        }
    }

    if let Some(previous) = previous.as_ref() {
        for result in &mut results {
            result.previous_status = previous.get(&result.case.id).cloned();
            result.regressed = is_regression(result.status, result.previous_status.as_deref());
            if result.regressed {
                println!(
                    "REGRESSION {}: previous PASS -> current {}",
                    result.case.id,
                    result.status.as_str()
                );
            }
        }
    }

    let counts = count_statuses(&results);
    let report = json!({
        "schema_version": 1,
        "run_id": run_id.to_string(),
        "filters": {
            "milestone": options.milestone,
            "subsystem": options.subsystem,
            "scope": options.scope.map(Scope::as_str),
            "ci": options.ci,
        },
        "comparison": options.compare_path.as_ref().map(|path| path.display().to_string()),
        "summary": {
            "pass": counts.pass,
            "fail": counts.fail,
            "skip": counts.skip,
            "blocked": counts.blocked,
            "not_run": counts.not_run,
        },
        "results": results.iter().map(result_json).collect::<Vec<_>>(),
    });

    let mut lines = vec![format!(
        "Acceptance summary: PASS={} FAIL={} SKIP={} BLOCKED={} NOT RUN={}",
        counts.pass, counts.fail, counts.skip, counts.blocked, counts.not_run
    )];
    if let Some(path) = timeout_json.as_deref() {
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            root.join(path)
        };
        let write_result = absolute
            .parent()
            .map(fs::create_dir_all)
            .transpose()
            .and_then(|_| serde_json::to_vec_pretty(&report).map_err(std::io::Error::other))
            .and_then(|bytes| fs::write(&absolute, bytes));
        match write_result {
            Ok(()) => {
                let display = absolute.strip_prefix(root).unwrap_or(&absolute).display();
                lines.push(format!("PASS acceptance report: {display}"));
                append_github_summary(root, &absolute, &results, &counts);
            }
            Err(error) => {
                lines.push(format!(
                    "FAIL acceptance report {}: {error}",
                    absolute.display()
                ));
                return CommandResult {
                    exit_code: EXIT_CONFIG_ERROR,
                    lines,
                };
            }
        }
    }

    let failed = counts.fail > 0 || counts.skip > 0 || counts.blocked > 0;
    CommandResult {
        exit_code: if failed {
            EXIT_ACCEPTANCE_FAILURE
        } else {
            EXIT_SUCCESS
        },
        lines,
    }
}

fn list_cases(registry: &[AcceptanceCase], options: &Options) -> CommandResult {
    let lines: Vec<_> = registry
        .iter()
        .filter(|case| {
            options
                .milestone
                .as_ref()
                .is_none_or(|milestone| &case.milestone == milestone)
                && options
                    .subsystem
                    .as_ref()
                    .is_none_or(|subsystem| &case.subsystem == subsystem)
                && options.scope.is_none_or(|scope| case.scope == scope)
        })
        .map(|case| {
            format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}",
                case.id,
                case.milestone,
                case.subsystem,
                case.scope.as_str(),
                case.script_stem,
                case.timeout_seconds,
                case.name
            )
        })
        .collect();
    if lines.is_empty() {
        return CommandResult {
            exit_code: EXIT_USAGE,
            lines: vec!["FAIL acceptance selection: no registered cases match the filters".into()],
        };
    }
    CommandResult {
        exit_code: EXIT_SUCCESS,
        lines,
    }
}

#[derive(Debug, Default)]
struct Counts {
    pass: usize,
    fail: usize,
    skip: usize,
    blocked: usize,
    not_run: usize,
}

fn count_statuses(results: &[CaseResult]) -> Counts {
    let mut counts = Counts::default();
    for result in results {
        match result.status {
            Status::Pass => counts.pass += 1,
            Status::Fail => counts.fail += 1,
            Status::Skip => counts.skip += 1,
            Status::Blocked => counts.blocked += 1,
            Status::NotRun => counts.not_run += 1,
        }
    }
    counts
}

fn result_json(result: &CaseResult) -> Value {
    json!({
        "id": result.case.id,
        "milestone": result.case.milestone,
        "subsystem": result.case.subsystem,
        "scope": result.case.scope.as_str(),
        "test": result.case.name,
        "status": result.status.as_str(),
        "duration_ms": result.duration_ms,
        "command": result.command,
        "exit_code": result.exit_code,
        "failure_stage": result.failure_stage.map(FailureStage::as_str),
        "diagnostic": result.diagnostic,
        "log_path": result.log_path,
        "previous_status": result.previous_status,
        "regressed": result.regressed,
    })
}

fn read_previous_results(root: &Path, path: &Path) -> Result<BTreeMap<String, String>, String> {
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    let bytes =
        fs::read(&path).map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let value: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("invalid JSON in {}: {error}", path.display()))?;
    if value.get("schema_version").and_then(Value::as_u64) != Some(1) {
        return Err(format!(
            "{} has an unsupported or missing schema_version (expected 1)",
            path.display()
        ));
    }
    let results = value
        .get("results")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{} does not contain a results array", path.display()))?;
    let mut previous = BTreeMap::new();
    for result in results {
        let id = result
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{} contains a result without an ID", path.display()))?;
        let status = result
            .get("status")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{} contains a result without a status", path.display()))?;
        if !matches!(status, "PASS" | "FAIL" | "SKIP" | "BLOCKED" | "NOT RUN") {
            return Err(format!(
                "{} contains an invalid status `{status}` for `{id}`",
                path.display()
            ));
        }
        if previous.insert(id.to_owned(), status.to_owned()).is_some() {
            return Err(format!(
                "{} contains duplicate result ID `{id}`",
                path.display()
            ));
        }
    }
    Ok(previous)
}

fn run_case(root: &Path, case: &AcceptanceCase, log_directory: &Path, verbose: bool) -> CaseResult {
    let start = Instant::now();
    let script_extension = if cfg!(windows) { "ps1" } else { "sh" };
    let script_relative = PathBuf::from("tests/acceptance")
        .join(format!("{}.{}", case.script_stem, script_extension));
    let script = root.join(&script_relative);
    let log_relative = log_directory.join(format!("{}.log", case.id));
    let log_path = root.join(&log_relative);
    let powershell = if cfg!(windows) {
        find_powershell()
    } else {
        None
    };
    let command_description = command_description(&script_relative, powershell.as_deref());
    println!(
        "RUN {} [{} / {}] timeout={}s command={command_description}",
        case.id,
        case.milestone,
        case.scope.as_str(),
        case.timeout_seconds,
    );
    let _ = std::io::stdout().flush();

    if let Err(error) = log_path.parent().map(fs::create_dir_all).transpose() {
        return failed_to_start(
            case,
            start,
            command_description,
            None,
            None,
            error.to_string(),
        );
    }
    let mut log = match File::create(&log_path) {
        Ok(log) => log,
        Err(error) => {
            return failed_to_start(
                case,
                start,
                command_description,
                None,
                None,
                error.to_string(),
            );
        }
    };
    if !script.is_file() {
        let detail = format!("required wrapper is missing: {}", script_relative.display());
        let _ = writeln!(log, "BLOCKED environment: {detail}");
        return CaseResult {
            case: case.clone(),
            status: Status::Blocked,
            duration_ms: start.elapsed().as_millis(),
            command: command_description,
            exit_code: None,
            failure_stage: Some(FailureStage::Environment),
            diagnostic: Some(detail),
            log_path: Some(log_relative.display().to_string()),
            previous_status: None,
            regressed: false,
        };
    }

    let mut command = if cfg!(windows) {
        let Some(powershell) = powershell else {
            let detail = "neither pwsh nor powershell.exe is available".to_owned();
            let _ = writeln!(log, "BLOCKED environment: {detail}");
            return CaseResult {
                case: case.clone(),
                status: Status::Blocked,
                duration_ms: start.elapsed().as_millis(),
                command: command_description,
                exit_code: None,
                failure_stage: Some(FailureStage::Environment),
                diagnostic: Some(detail),
                log_path: Some(log_relative.display().to_string()),
                previous_status: None,
                regressed: false,
            };
        };
        let mut command = Command::new(powershell);
        command.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-File"]);
        command.arg(&script);
        command
    } else {
        let mut command = Command::new("sh");
        command.arg(&script);
        command
    };

    let stdout = match log.try_clone() {
        Ok(file) => file,
        Err(error) => {
            return failed_to_start(
                case,
                start,
                command_description,
                None,
                Some(log_relative.display().to_string()),
                error.to_string(),
            );
        }
    };
    let stderr = match log.try_clone() {
        Ok(file) => file,
        Err(error) => {
            return failed_to_start(
                case,
                start,
                command_description,
                None,
                Some(log_relative.display().to_string()),
                error.to_string(),
            );
        }
    };
    command
        .current_dir(root)
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            drop(command);
            let _ = writeln!(
                log,
                "BLOCKED environment: cannot start {command_description}: {error}"
            );
            return failed_to_start(
                case,
                start,
                command_description,
                None,
                Some(log_relative.display().to_string()),
                error.to_string(),
            );
        }
    };
    drop(log);
    let timeout = Duration::from_secs(case.timeout_seconds);
    let (status_code, timed_out) = loop {
        match child.try_wait() {
            Ok(Some(status)) => break (status.code(), false),
            Ok(None) if start.elapsed() >= timeout => {
                terminate_process_tree(&mut child);
                let status = child.wait().ok();
                break (status.and_then(|status| status.code()), true);
            }
            Ok(None) => thread::sleep(Duration::from_millis(100)),
            Err(error) => {
                terminate_process_tree(&mut child);
                let status = child.wait().ok();
                let code = status.and_then(|status| status.code());
                let diagnostic = format!("could not inspect child process: {error}");
                return CaseResult {
                    case: case.clone(),
                    status: Status::Fail,
                    duration_ms: start.elapsed().as_millis(),
                    command: command_description,
                    exit_code: code,
                    failure_stage: Some(FailureStage::Command),
                    diagnostic: Some(diagnostic),
                    log_path: Some(log_relative.display().to_string()),
                    previous_status: None,
                    regressed: false,
                };
            }
        }
    };
    drop(child);

    let output = fs::read(&log_path)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();
    if verbose {
        print!("--- {} output ---\n{}", case.id, output);
        if !output.ends_with('\n') {
            println!();
        }
        println!("--- end {} output ---", case.id);
    }

    let (status, failure_stage, diagnostic) = classify_result(&output, status_code, timed_out);
    CaseResult {
        case: case.clone(),
        status,
        duration_ms: start.elapsed().as_millis(),
        command: command_description,
        exit_code: status_code,
        failure_stage,
        diagnostic,
        log_path: Some(log_relative.display().to_string()),
        previous_status: None,
        regressed: false,
    }
}

fn terminate_process_tree(child: &mut std::process::Child) {
    #[cfg(unix)]
    {
        let process_group = format!("-{}", child.id());
        let term = Command::new("kill")
            .args(["-TERM", process_group.as_str()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        if term.is_ok_and(|status| status.success()) {
            thread::sleep(Duration::from_millis(200));
            let _ = Command::new("kill")
                .args(["-KILL", process_group.as_str()])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill.exe")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    let _ = child.kill();
}

fn find_powershell() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let names = ["pwsh.exe", "pwsh", "powershell.exe", "powershell"];
    for directory in std::env::split_paths(&path) {
        for name in names {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn command_description(script: &Path, powershell: Option<&Path>) -> String {
    if cfg!(windows) {
        format!(
            "{} -NoProfile -ExecutionPolicy Bypass -File {}",
            powershell.unwrap_or(Path::new("pwsh")).display(),
            script.display()
        )
    } else {
        format!("sh {}", script.display())
    }
}

fn failed_to_start(
    case: &AcceptanceCase,
    start: Instant,
    command: String,
    exit_code: Option<i32>,
    log_path: Option<String>,
    diagnostic: String,
) -> CaseResult {
    CaseResult {
        case: case.clone(),
        status: Status::Blocked,
        duration_ms: start.elapsed().as_millis(),
        command,
        exit_code,
        failure_stage: Some(FailureStage::Environment),
        diagnostic: Some(diagnostic),
        log_path,
        previous_status: None,
        regressed: false,
    }
}

fn classify_result(
    output: &str,
    exit_code: Option<i32>,
    timed_out: bool,
) -> (Status, Option<FailureStage>, Option<String>) {
    if timed_out {
        return (
            Status::Fail,
            Some(FailureStage::Timeout),
            Some("acceptance command exceeded its registry timeout".into()),
        );
    }
    if output
        .lines()
        .any(|line| line.trim_start().starts_with("BLOCKED "))
    {
        let stage = failure_stage(output);
        return (
            Status::Blocked,
            Some(if stage == FailureStage::Command {
                FailureStage::Environment
            } else {
                stage
            }),
            first_marker_line(output, "BLOCKED "),
        );
    }
    if output
        .lines()
        .any(|line| line.trim_start().starts_with("FAIL "))
    {
        return (
            Status::Fail,
            Some(failure_stage(output)),
            first_marker_line(output, "FAIL "),
        );
    }
    if output
        .lines()
        .any(|line| line.trim_start().starts_with("SKIP "))
    {
        return (Status::Skip, None, first_marker_line(output, "SKIP "));
    }
    if exit_code == Some(0) {
        return (Status::Pass, None, None);
    }
    let stage = failure_stage(output);
    let status = if stage == FailureStage::Environment
        || (stage == FailureStage::Dependency && is_external_dependency_failure(output))
    {
        Status::Blocked
    } else {
        Status::Fail
    };
    let diagnostic = first_important_line(output).or_else(|| {
        Some(format!(
            "command exited with code {} and produced no recognized diagnostic",
            exit_code.map_or_else(|| "unknown".into(), |code| code.to_string())
        ))
    });
    (status, Some(stage), diagnostic)
}

fn failure_stage(output: &str) -> FailureStage {
    let lower = output.to_ascii_lowercase();
    if is_environment_failure(&lower) {
        FailureStage::Environment
    } else if lower.contains("timed out") || lower.contains("timeout waiting") {
        FailureStage::Timeout
    } else if lower.contains("undefined symbol")
        || lower.contains("undefined reference")
        || lower.contains("linking with")
        || lower.contains("linker command failed")
        || lower.contains("rust-lld: error")
        || lower.contains("ld.lld: error")
        || lower.contains("lld-link: error")
    {
        FailureStage::Link
    } else if lower.contains("failed to download")
        || lower.contains("failed to fetch")
        || lower.contains("failed to get `")
        || lower.contains("could not resolve host")
        || lower.contains("source hash mismatch")
        || lower.contains("patch check failed")
        || lower.contains("patch application failed")
    {
        FailureStage::Dependency
    } else if lower.contains("assertion failed")
        || lower.contains("panicked at")
        || lower.contains("acceptance failed")
        || lower.contains("guest did not print")
        || lower.contains("test result: failed")
        || lower.contains("tests failed")
        || lower
            .lines()
            .any(|line| line.trim_start().starts_with("fail "))
    {
        FailureStage::Test
    } else if lower.contains("cannot read")
        || lower.contains("artifact missing")
        || lower.contains("was not created")
        || lower.contains("cannot open artifact")
    {
        FailureStage::Artifact
    } else if lower.contains("fatal error:")
        || lower.contains("error:")
        || lower.contains("could not compile")
        || lower.contains("ninja: build stopped")
        || lower.contains("cmake error")
    {
        FailureStage::Compile
    } else {
        FailureStage::Command
    }
}

fn is_environment_failure(lower: &str) -> bool {
    lower.contains("command not found")
        || lower.contains("is not recognized as the name of a cmdlet")
        || [
            "git",
            "cargo",
            "rustc",
            "rustup",
            "qemu",
            "clang",
            "lld",
            "bash",
            "powershell",
            "pwsh",
            "cmake",
            "meson",
            "ninja",
            "python",
            "ovmf",
        ]
        .iter()
        .any(|tool| {
            (lower.contains("was not found") || lower.contains("not found")) && lower.contains(tool)
        })
        || lower.contains("neither pwsh nor powershell.exe is available")
        || (lower.contains("no such file or directory")
            && [
                "cargo",
                "rustc",
                "qemu",
                "clang",
                "bash",
                "powershell",
                "pwsh",
            ]
            .iter()
            .any(|tool| lower.contains(tool)))
        || lower.contains("permission denied")
}

fn is_external_dependency_failure(output: &str) -> bool {
    let lower = output.to_ascii_lowercase();
    lower.contains("could not resolve host")
        || lower.contains("network is unreachable")
        || lower.contains("connection timed out")
        || lower.contains("failed to download")
}

fn first_marker_line(output: &str, marker: &str) -> Option<String> {
    output
        .lines()
        .find(|line| line.trim_start().starts_with(marker))
        .map(|line| limit_diagnostic(line.trim()))
}

fn first_important_line(output: &str) -> Option<String> {
    output.lines().find_map(|line| {
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        let interesting = lower.contains("undefined symbol")
            || lower.contains("undefined reference")
            || lower.contains("linking with")
            || lower.contains("fatal error:")
            || (lower.contains("error:") && !lower.contains("could not compile"))
            || lower.contains("assertion failed")
            || lower.contains("acceptance failed")
            || lower.contains("panicked at")
            || lower.starts_with("fail ")
            || lower.starts_with("blocked ")
            || lower.starts_with("skip ")
            || lower.contains("timed out")
            || lower.contains("timeout waiting")
            || lower.contains("command not found")
            || lower.contains("could not resolve host")
            || lower.contains("network is unreachable")
            || lower.contains("connection timed out")
            || lower.contains("failed to download")
            || lower.contains("failed to fetch")
            || lower.contains("failed to get `")
            || lower.contains("source hash mismatch")
            || lower.contains("patch check failed")
            || lower.contains("patch application failed")
            || lower.contains("cannot read")
            || lower.contains("was not created");
        interesting.then(|| limit_diagnostic(trimmed))
    })
}

fn limit_diagnostic(value: &str) -> String {
    const LIMIT: usize = 600;
    if value.chars().count() <= LIMIT {
        value.to_owned()
    } else {
        format!("{}…", value.chars().take(LIMIT).collect::<String>())
    }
}

fn is_regression(current: Status, previous: Option<&str>) -> bool {
    current != Status::NotRun && previous == Some("PASS") && current != Status::Pass
}

fn println_result(result: &CaseResult) {
    let elapsed = Duration::from_millis(result.duration_ms.min(u64::MAX as u128) as u64);
    let log = result
        .log_path
        .as_deref()
        .map(|path| format!("; log {path}"))
        .unwrap_or_default();
    match result.status {
        Status::Pass => println!(
            "PASS {} [{} / {}] {:.1}s{}",
            result.case.id,
            result.case.milestone,
            result.case.scope.as_str(),
            elapsed.as_secs_f64(),
            log
        ),
        Status::NotRun => println!("NOT RUN {}", result.case.id),
        status => println!(
            "{} {} [{} / {}; stage={}] {}{}",
            status.as_str(),
            result.case.id,
            result.case.milestone,
            result.case.scope.as_str(),
            result
                .failure_stage
                .map(FailureStage::as_str)
                .unwrap_or("unknown"),
            result.diagnostic.as_deref().unwrap_or("no diagnostic"),
            log
        ),
    }
}

fn append_github_summary(root: &Path, report_path: &Path, results: &[CaseResult], counts: &Counts) {
    let Some(summary_path) = std::env::var_os("GITHUB_STEP_SUMMARY").map(PathBuf::from) else {
        return;
    };
    let path = if summary_path.is_absolute() {
        summary_path
    } else {
        root.join(summary_path)
    };
    let mut text = format!(
        "\n## Acceptance results\n\nPASS: {} · FAIL: {} · BLOCKED: {} · SKIP: {} · NOT RUN: {}\n\n| ID | Milestone | Subsystem | Scope | Status | Stage | Diagnostic | Log |\n|---|---|---|---|---|---|---|---|\n",
        counts.pass, counts.fail, counts.blocked, counts.skip, counts.not_run
    );
    for result in results
        .iter()
        .filter(|result| result.status != Status::NotRun)
    {
        let stage = result
            .failure_stage
            .map(FailureStage::as_str)
            .unwrap_or("-");
        let diagnostic = result
            .diagnostic
            .as_deref()
            .unwrap_or("-")
            .replace('|', "\\|")
            .replace('\n', " ");
        let log = result.log_path.as_deref().unwrap_or("-");
        text.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} | `{}` |\n",
            result.case.id,
            result.case.milestone,
            result.case.subsystem,
            result.case.scope.as_str(),
            result.status.as_str(),
            stage,
            diagnostic,
            log
        ));
    }
    text.push_str(&format!(
        "\nMachine-readable report: `{}`\n",
        report_path.display()
    ));
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(text.as_bytes());
    }
}

fn acceptance_help() -> Vec<String> {
    vec![
        "Nagi acceptance usage: nagi test --acceptance [options]".into(),
        "  --milestone M17       run one registered milestone".into(),
        "  --subsystem servo     run one registered subsystem".into(),
        "  --host-only           run host acceptance cases".into(),
        "  --target-only         run guest/target acceptance cases".into(),
        "  --ci                  write a machine-readable result under out/test-results".into(),
        "  --json PATH           write the JSON report to PATH".into(),
        "  --compare PATH        report cases that regressed from a previous PASS".into(),
        "  --verbose             print full captured output after each case".into(),
        "  --list                list registered cases without running them".into(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_ids_are_unique_and_both_host_wrappers_exist() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("workspace root");
        let contents = fs::read_to_string(root.join(REGISTRY_PATH)).expect("acceptance registry");
        let cases = parse_registry(&contents).expect("valid registry");
        let mut ids = HashSet::new();
        for case in &cases {
            assert!(ids.insert(&case.id), "duplicate ID {}", case.id);
            for extension in ["sh", "ps1"] {
                assert!(
                    root.join("tests/acceptance")
                        .join(format!("{}.{}", case.script_stem, extension))
                        .is_file(),
                    "missing {extension} wrapper for {}",
                    case.id
                );
            }
        }
        assert!(cases.iter().any(|case| case.id == "M17-FIRST-WEB-PIXEL"));
    }

    #[test]
    fn registry_rejects_duplicate_ids_and_bad_scope() {
        let duplicate = "A\tM0\tx\thost\ta\t1\tfirst\nA\tM0\tx\thost\tb\t1\tsecond\n";
        assert!(parse_registry(duplicate)
            .unwrap_err()
            .contains("duplicate ID"));
        let bad_scope = "A\tM0\tx\tguest\ta\t1\tfirst\n";
        assert!(parse_registry(bad_scope)
            .unwrap_err()
            .contains("scope must be host or target"));
    }

    #[test]
    fn milestone_filters_normalize_and_scope_filters_are_exclusive() {
        let options = parse_options(&[
            "--milestone".into(),
            "m017".into(),
            "--target-only".into(),
            "--ci".into(),
        ])
        .unwrap();
        assert_eq!(options.milestone.as_deref(), Some("M17"));
        assert_eq!(options.scope, Some(Scope::Target));
        assert!(parse_options(&["--host-only".into(), "--target-only".into()]).is_err());
    }

    #[test]
    fn failures_keep_stage_distinction_and_block_missing_environment() {
        assert_eq!(
            failure_stage("error: could not compile due to previous error\nrust-lld: error: undefined symbol: foo"),
            FailureStage::Link
        );
        assert_eq!(
            failure_stage("src/lib.rs:2: error: type mismatch"),
            FailureStage::Compile
        );
        assert_eq!(
            failure_stage("assertion failed: marker"),
            FailureStage::Test
        );
        assert_eq!(
            failure_stage("QEMU timeout waiting for the boot marker"),
            FailureStage::Timeout
        );
        let (status, stage, _) = classify_result("cargo was not found", Some(4), false);
        assert_eq!(status, Status::Blocked);
        assert_eq!(stage, Some(FailureStage::Environment));
        let (status, stage, _) = classify_result("QEMU timed out", Some(1), true);
        assert_eq!(status, Status::Fail);
        assert_eq!(stage, Some(FailureStage::Timeout));
        assert_eq!(
            classify_result("FAIL wrapper detected a false success", Some(0), false).0,
            Status::Fail
        );
        assert_eq!(
            classify_result("SKIP optional check", Some(0), false).0,
            Status::Skip
        );
        let (status, stage, diagnostic) = classify_result(
            "BLOCKED dependency: failed to download pinned source",
            Some(9),
            false,
        );
        assert_eq!(status, Status::Blocked);
        assert_eq!(stage, Some(FailureStage::Dependency));
        assert!(diagnostic.unwrap().contains("failed to download"));
        let (status, stage, diagnostic) =
            classify_result("M17 acceptance failed at first-web-pixel", Some(1), false);
        assert_eq!(status, Status::Fail);
        assert_eq!(stage, Some(FailureStage::Test));
        assert!(diagnostic.unwrap().contains("acceptance failed"));
        assert_eq!(
            classify_result("SKIP optional check", Some(9), false).0,
            Status::Skip
        );
    }

    #[test]
    fn previous_pass_to_current_failure_is_a_regression() {
        let status = Status::Blocked;
        let previous = Some("PASS");
        let current = CaseResult {
            case: AcceptanceCase {
                id: "M0-DOCTOR".into(),
                milestone: "M0".into(),
                subsystem: "toolchain".into(),
                scope: Scope::Host,
                script_stem: "m0_doctor".into(),
                timeout_seconds: 30,
                name: "doctor".into(),
            },
            status,
            duration_ms: 0,
            command: "sh tests/acceptance/m0_doctor.sh".into(),
            exit_code: Some(4),
            failure_stage: Some(FailureStage::Environment),
            diagnostic: Some("missing tool".into()),
            log_path: None,
            previous_status: previous.map(str::to_owned),
            regressed: is_regression(status, previous),
        };
        let json = result_json(&current);
        assert_eq!(json["previous_status"], "PASS");
        assert_eq!(json["regressed"], true);
        assert!(!is_regression(Status::NotRun, Some("PASS")));
        assert!(!is_regression(Status::Pass, Some("PASS")));
    }

    #[test]
    fn previous_reports_require_supported_schema_and_status_values() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root = std::env::temp_dir().join(format!("nagi-acceptance-report-{unique}"));
        fs::create_dir_all(&root).expect("temporary report directory");
        let report = root.join("previous.json");

        fs::write(
            &report,
            r#"{"schema_version":1,"results":[{"id":"M0-DOCTOR","status":"PASS"}]}"#,
        )
        .expect("write supported report");
        assert_eq!(
            read_previous_results(&root, Path::new("previous.json")).unwrap()["M0-DOCTOR"],
            "PASS"
        );

        fs::write(
            &report,
            r#"{"schema_version":2,"results":[{"id":"M0-DOCTOR","status":"PASS"}]}"#,
        )
        .expect("write unsupported report");
        assert!(read_previous_results(&root, Path::new("previous.json"))
            .unwrap_err()
            .contains("expected 1"));

        fs::write(
            &report,
            r#"{"schema_version":1,"results":[{"id":"M0-DOCTOR","status":"MAYBE"}]}"#,
        )
        .expect("write invalid status report");
        assert!(read_previous_results(&root, Path::new("previous.json"))
            .unwrap_err()
            .contains("invalid status"));

        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn runner_captures_output_and_preserves_script_exit_state() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let root =
            std::env::temp_dir().join(format!("nagi-acceptance-{}-{unique}", std::process::id()));
        let wrappers = root.join("tests/acceptance");
        fs::create_dir_all(&wrappers).expect("temporary acceptance wrappers");
        fs::write(
            wrappers.join("fixture_success.sh"),
            "printf 'PASS fixture\\n'\nexit 0\n",
        )
        .expect("write shell success wrapper");
        fs::write(
            wrappers.join("fixture_success.ps1"),
            "Write-Output 'PASS fixture'\nexit 0\n",
        )
        .expect("write PowerShell success wrapper");
        fs::write(
            wrappers.join("fixture_failure.sh"),
            "printf 'assertion failed: expected marker\\n'\nexit 1\n",
        )
        .expect("write shell failure wrapper");
        fs::write(
            wrappers.join("fixture_failure.ps1"),
            "Write-Output 'assertion failed: expected marker'\nexit 1\n",
        )
        .expect("write PowerShell failure wrapper");
        fs::write(wrappers.join("fixture_timeout.sh"), "sleep 5\nexit 0\n")
            .expect("write shell timeout wrapper");
        fs::write(
            wrappers.join("fixture_timeout.ps1"),
            "Start-Sleep -Seconds 5\nexit 0\n",
        )
        .expect("write PowerShell timeout wrapper");

        let make_case = |id: &str, stem: &str| AcceptanceCase {
            id: id.into(),
            milestone: "M0".into(),
            subsystem: "harness".into(),
            scope: Scope::Host,
            script_stem: stem.into(),
            timeout_seconds: if stem == "fixture_timeout" { 1 } else { 5 },
            name: id.into(),
        };
        let success = run_case(
            &root,
            &make_case("FIXTURE-PASS", "fixture_success"),
            Path::new("out/logs"),
            false,
        );
        let failure = run_case(
            &root,
            &make_case("FIXTURE-FAIL", "fixture_failure"),
            Path::new("out/logs"),
            false,
        );
        assert_eq!(success.status, Status::Pass);
        assert_eq!(success.exit_code, Some(0));
        assert_eq!(failure.status, Status::Fail);
        assert_eq!(failure.failure_stage, Some(FailureStage::Test));
        assert!(failure
            .diagnostic
            .as_deref()
            .unwrap()
            .contains("assertion failed"));
        let timeout = run_case(
            &root,
            &make_case("FIXTURE-TIMEOUT", "fixture_timeout"),
            Path::new("out/logs"),
            false,
        );
        assert_eq!(timeout.status, Status::Fail);
        assert_eq!(timeout.failure_stage, Some(FailureStage::Timeout));
        assert!(
            timeout.duration_ms < 5_000,
            "timeout took {} ms",
            timeout.duration_ms
        );
        assert!(fs::read_to_string(root.join(success.log_path.unwrap()))
            .unwrap()
            .contains("PASS fixture"));
        let _ = fs::remove_dir_all(root);
    }
}
