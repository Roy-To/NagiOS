use std::collections::{BTreeSet, HashMap, HashSet};
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::commands::{CliError, EXIT_CONFIG_ERROR, EXIT_USAGE};

const REGISTRY_PATH: &str = ".dev/workstreams.json";
const FAILURE_CLASSES: &[&str] = &[
    "SOURCE",
    "BUILD",
    "LINK",
    "ABI",
    "RUNTIME",
    "BOOT",
    "DEVICE",
    "STORAGE",
    "GRAPHICS",
    "NETWORK",
    "MODEL",
    "PERMISSION",
    "ACCEPTANCE",
    "CI_INFRA",
    "HOST_ENV",
    "UNKNOWN",
];
const WORKSTREAM_FIELDS: &[&str] = &[
    "id",
    "owner",
    "owner_branch",
    "recommended_worktree",
    "state_file",
    "dependencies",
    "allowed_paths",
    "forbidden_paths",
    "activation_gate",
    "merge_boundary",
];
const STATE_FIELDS: &[&str] = &[
    "schema_ref",
    "schema_version",
    "workstream_id",
    "current_milestone",
    "current_checkpoint",
    "status",
    "last_verified",
    "release_line_gate",
    "blocker",
    "next_action",
    "acceptance_criteria",
    "deferred",
    "cross_workstream_dependencies",
];

#[derive(Debug, Clone)]
struct Workstream {
    id: String,
    owner: String,
    branch: String,
    state_file: String,
    dependencies: Vec<String>,
}

#[derive(Debug, Clone)]
struct RepoState {
    branch: String,
    head: String,
    dirty: bool,
}

pub fn execute(args: &[String], root: &Path) -> Result<Vec<String>, CliError> {
    let Some(command) = args.first().map(String::as_str) else {
        return Err(usage_error());
    };
    match command {
        "status" if args.len() == 1 => status(root, false),
        "resume" if args.len() == 1 => status(root, true),
        "verify" if args.len() == 1 => verify(root),
        "diagnose" => diagnose(&args[1..], root),
        "help" | "--help" | "-h" if args.len() == 1 => Ok(vec![
            "Nagi development workflow".into(),
            "Commands: dev status, dev resume, dev verify, dev diagnose --stage ID --exit-code N --log PATH [--artifact PATH] --output PATH".into(),
        ]),
        _ => Err(usage_error()),
    }
}

fn usage_error() -> CliError {
    CliError::new(
        "usage: nagi dev status|resume|verify | nagi dev diagnose --stage ID --exit-code N --log PATH [--log PATH] [--artifact PATH] [--failure-class CLASS] [--output PATH]",
        EXIT_USAGE,
    )
}

fn status(root: &Path, resume: bool) -> Result<Vec<String>, CliError> {
    let registry = load_registry(root)?;
    let repo = repo_state(root)?;
    let workstream = registry
        .iter()
        .find(|entry| entry.branch == repo.branch)
        .ok_or_else(|| {
            CliError::new(
                format!(
                    "branch `{}` is not registered in {REGISTRY_PATH}; create/assign a workstream before using dev status",
                    repo.branch
                ),
                EXIT_CONFIG_ERROR,
            )
        })?;
    let state = load_state(root, workstream, &registry)?;
    let verified = &state["last_verified"];
    let blocker = &state["blocker"];
    let mut lines = vec![
        format!("Workstream: {} ({})", workstream.id, workstream.owner),
        format!("Branch: {}", repo.branch),
        format!("HEAD: {}", repo.head),
        format!("Worktree: {}", if repo.dirty { "DIRTY" } else { "clean" }),
        format!(
            "Milestone/checkpoint: {} / {}",
            state["current_milestone"].as_str().unwrap_or(""),
            state["current_checkpoint"].as_str().unwrap_or("")
        ),
        format!("Status: {}", state["status"].as_str().unwrap_or("")),
        format!(
            "Release-line gate: {} ({})",
            state["release_line_gate"]["activation_requirement"]
                .as_str()
                .unwrap_or(""),
            state["release_line_gate"]["source"].as_str().unwrap_or("")
        ),
        format!(
            "Last verified commit: {} ({})",
            verified["commit_sha"].as_str().unwrap_or(""),
            verified["verified_at"].as_str().unwrap_or("")
        ),
        format!(
            "Blocker: {}",
            if blocker["active"].as_bool().unwrap_or(false) {
                format!(
                    "{} — {}",
                    blocker["class"].as_str().unwrap_or("UNKNOWN"),
                    blocker["summary"].as_str().unwrap_or("")
                )
            } else {
                "none".into()
            }
        ),
    ];
    if let Some(ci_runs) = verified["ci_runs"].as_array() {
        for run in ci_runs.iter().take(2) {
            lines.push(format!(
                "CI: run {} {} ({}) — {}",
                run["run_id"].as_u64().unwrap_or_default(),
                run["status"].as_str().unwrap_or("UNKNOWN"),
                run["conclusion"].as_str().unwrap_or("no conclusion"),
                run["url"].as_str().unwrap_or("")
            ));
        }
    }
    lines.push(format!(
        "Next action: {}",
        state["next_action"].as_str().unwrap_or("")
    ));
    if resume {
        lines.push("Resume: read AGENTS.md, docs/0.2/DEVELOPMENT_ARCHITECTURE.md, then this workstream state and linked evidence.".into());
        lines.push(format!(
            "Dependencies: {}",
            workstream.dependencies.join(", ").if_empty("none")
        ));
        if let Some(do_not_try) = blocker["do_not_try"].as_array() {
            lines.extend(
                do_not_try
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|item| format!("Do not: {item}")),
            );
        }
    }
    Ok(lines)
}

fn verify(root: &Path) -> Result<Vec<String>, CliError> {
    let registry = load_registry(root)?;
    let mut verified_states = 0usize;
    for workstream in &registry {
        let state_path = root.join(&workstream.state_file);
        if state_path.exists() {
            load_state(root, workstream, &registry)?;
            verified_states += 1;
        }
    }
    let repo = repo_state(root)?;
    let current = registry.iter().find(|entry| entry.branch == repo.branch);
    if let Some(workstream) = current {
        if !root.join(&workstream.state_file).is_file() {
            return Err(CliError::new(
                format!(
                    "registered current workstream `{}` has no state file",
                    workstream.id
                ),
                EXIT_CONFIG_ERROR,
            ));
        }
    }
    Ok(vec![format!(
        "PASS development state: {} registered workstreams, {verified_states} state files validated; branch {} at {}",
        registry.len(), repo.branch, repo.head
    )])
}

fn load_registry(root: &Path) -> Result<Vec<Workstream>, CliError> {
    let path = root.join(REGISTRY_PATH);
    let bytes = fs::read(&path).map_err(|error| {
        CliError::new(
            format!("cannot read {}: {error}", path.display()),
            EXIT_CONFIG_ERROR,
        )
    })?;
    let document: Value = serde_json::from_slice(&bytes).map_err(|error| {
        CliError::new(
            format!("invalid JSON in {}: {error}", path.display()),
            EXIT_CONFIG_ERROR,
        )
    })?;
    validate_keys(
        &document,
        &["schema_ref", "schema_version", "workstreams"],
        REGISTRY_PATH,
    )?;
    if document["schema_version"].as_u64() != Some(1) {
        return Err(config_error(format!(
            "{REGISTRY_PATH}: schema_version must be 1"
        )));
    }
    let entries = document["workstreams"]
        .as_array()
        .ok_or_else(|| config_error(format!("{REGISTRY_PATH}: workstreams must be an array")))?;
    if entries.is_empty() {
        return Err(config_error(format!(
            "{REGISTRY_PATH}: workstreams cannot be empty"
        )));
    }

    let mut streams = Vec::with_capacity(entries.len());
    let mut ids = HashSet::new();
    let mut branches = HashSet::new();
    let mut state_files = HashSet::new();
    for (index, entry) in entries.iter().enumerate() {
        let context = format!("{REGISTRY_PATH}: workstreams[{index}]");
        validate_keys(entry, WORKSTREAM_FIELDS, &context)?;
        let id = required_string(entry, "id", &context)?.to_owned();
        let branch = required_string(entry, "owner_branch", &context)?.to_owned();
        let state_file = required_string(entry, "state_file", &context)?.to_owned();
        let state_path = Path::new(&state_file);
        if state_path.is_absolute()
            || state_path
                .components()
                .any(|part| matches!(part, Component::ParentDir))
            || !state_file.starts_with(".dev/workstreams/")
            || !state_file.ends_with("/state.json")
            || state_file.contains('\\')
        {
            return Err(config_error(format!(
                "{context}: unsafe state_file `{state_file}`"
            )));
        }
        if id.is_empty()
            || id.split('-').any(|part| {
                part.is_empty()
                    || !part
                        .chars()
                        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit())
            })
        {
            return Err(config_error(format!(
                "{context}: invalid workstream id `{id}`"
            )));
        }
        let branch_suffix = branch.strip_prefix("codex/").unwrap_or("");
        if branch_suffix.is_empty()
            || !branch_suffix
                .chars()
                .next()
                .is_some_and(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit())
            || !branch_suffix.chars().all(|ch| {
                ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '.' | '/' | '-')
            })
        {
            return Err(config_error(format!(
                "{context}: invalid owner_branch `{branch}`"
            )));
        }
        if state_file != format!(".dev/workstreams/{id}/state.json") {
            return Err(config_error(format!(
                "{context}: state_file must be .dev/workstreams/{id}/state.json"
            )));
        }
        if !ids.insert(id.clone()) {
            return Err(config_error(format!(
                "{REGISTRY_PATH}: duplicate workstream id `{id}`"
            )));
        }
        if !branches.insert(branch.clone()) {
            return Err(config_error(format!(
                "{REGISTRY_PATH}: duplicate owner branch `{branch}`"
            )));
        }
        if !state_files.insert(state_file.clone()) {
            return Err(config_error(format!(
                "{REGISTRY_PATH}: duplicate state file `{state_file}`"
            )));
        }
        let dependencies = string_array(entry, "dependencies", &context)?;
        let unique_dependencies: HashSet<&str> = dependencies.iter().map(String::as_str).collect();
        if unique_dependencies.len() != dependencies.len() {
            return Err(config_error(format!(
                "{context}: dependencies contains a duplicate workstream"
            )));
        }
        if string_array(entry, "allowed_paths", &context)?.is_empty() {
            return Err(config_error(format!(
                "{context}: allowed_paths cannot be empty"
            )));
        }
        string_array(entry, "forbidden_paths", &context)?;
        for field in [
            "owner",
            "recommended_worktree",
            "activation_gate",
            "merge_boundary",
        ] {
            required_string(entry, field, &context)?;
        }
        streams.push(Workstream {
            id,
            owner: required_string(entry, "owner", &context)?.to_owned(),
            branch,
            state_file,
            dependencies: string_array(entry, "dependencies", &context)?,
        });
    }
    let known: HashSet<&str> = streams.iter().map(|stream| stream.id.as_str()).collect();
    for stream in &streams {
        for dependency in &stream.dependencies {
            if !known.contains(dependency.as_str()) {
                return Err(config_error(format!(
                    "{REGISTRY_PATH}: workstream `{}` has missing dependency `{dependency}`",
                    stream.id
                )));
            }
            if dependency == &stream.id {
                return Err(config_error(format!(
                    "{REGISTRY_PATH}: workstream `{}` cannot depend on itself",
                    stream.id
                )));
            }
        }
    }
    detect_dependency_cycles(&streams)?;
    Ok(streams)
}

fn detect_dependency_cycles(streams: &[Workstream]) -> Result<(), CliError> {
    let graph: HashMap<&str, &Workstream> = streams
        .iter()
        .map(|stream| (stream.id.as_str(), stream))
        .collect();
    for stream in streams {
        let mut seen = HashSet::new();
        let mut pending = stream.dependencies.clone();
        while let Some(next) = pending.pop() {
            if next == stream.id {
                return Err(config_error(format!(
                    "{REGISTRY_PATH}: dependency cycle reaches `{}`",
                    stream.id
                )));
            }
            if seen.insert(next.clone()) {
                if let Some(dependency) = graph.get(next.as_str()) {
                    pending.extend(dependency.dependencies.iter().cloned());
                }
            }
        }
    }
    Ok(())
}

fn load_state(
    root: &Path,
    workstream: &Workstream,
    registry: &[Workstream],
) -> Result<Value, CliError> {
    let path = root.join(&workstream.state_file);
    let bytes = fs::read(&path).map_err(|error| {
        CliError::new(
            format!("cannot read {}: {error}", path.display()),
            EXIT_CONFIG_ERROR,
        )
    })?;
    let state: Value = serde_json::from_slice(&bytes).map_err(|error| {
        CliError::new(
            format!("invalid JSON in {}: {error}", path.display()),
            EXIT_CONFIG_ERROR,
        )
    })?;
    validate_state(&state, workstream, registry)?;
    Ok(state)
}

fn validate_state(
    state: &Value,
    workstream: &Workstream,
    registry: &[Workstream],
) -> Result<(), CliError> {
    let context = workstream.state_file.as_str();
    validate_keys(state, STATE_FIELDS, context)?;
    if state["schema_version"].as_u64() != Some(1) {
        return Err(config_error(format!("{context}: schema_version must be 1")));
    }
    if required_string(state, "workstream_id", context)? != workstream.id {
        return Err(config_error(format!(
            "{context}: workstream_id does not match registry"
        )));
    }
    for field in ["current_milestone", "current_checkpoint", "next_action"] {
        required_string(state, field, context)?;
    }
    let status = required_string(state, "status", context)?;
    if ![
        "NOT_STARTED",
        "IN_PROGRESS",
        "PASS",
        "PARTIAL",
        "BLOCKED",
        "DEFERRED",
    ]
    .contains(&status)
    {
        return Err(config_error(format!(
            "{context}: invalid status `{status}`"
        )));
    }
    validate_last_verified(&state["last_verified"], context)?;
    validate_release_line_gate(&state["release_line_gate"], context)?;
    validate_blocker(&state["blocker"], context)?;
    string_array(state, "acceptance_criteria", context)?;
    string_array(state, "deferred", context)?;
    let dependencies = state["cross_workstream_dependencies"]
        .as_array()
        .ok_or_else(|| {
            config_error(format!(
                "{context}: cross_workstream_dependencies must be an array"
            ))
        })?;
    let known: HashSet<&str> = registry.iter().map(|entry| entry.id.as_str()).collect();
    let mut seen = HashSet::new();
    for (index, dependency) in dependencies.iter().enumerate() {
        let dep_context = format!("{context}: cross_workstream_dependencies[{index}]");
        validate_keys(
            dependency,
            &["workstream_id", "contract", "version", "status"],
            &dep_context,
        )?;
        let id = required_string(dependency, "workstream_id", &dep_context)?;
        if !known.contains(id) {
            return Err(config_error(format!(
                "{dep_context}: missing registered workstream `{id}`"
            )));
        }
        if !seen.insert(id.to_owned()) {
            return Err(config_error(format!(
                "{dep_context}: duplicate dependency `{id}`"
            )));
        }
        for field in ["contract", "version", "status"] {
            required_string(dependency, field, &dep_context)?;
        }
    }
    Ok(())
}

fn validate_last_verified(value: &Value, context: &str) -> Result<(), CliError> {
    let section = format!("{context}: last_verified");
    validate_keys(
        value,
        &[
            "commit_sha",
            "verified_at",
            "normal_state",
            "commands",
            "tests",
            "ci_runs",
        ],
        &section,
    )?;
    let sha = required_string(value, "commit_sha", &section)?;
    if !is_sha(sha) {
        return Err(config_error(format!(
            "{section}: commit_sha must be 40 lowercase hexadecimal characters"
        )));
    }
    let verified_at = required_string(value, "verified_at", &section)?;
    if !is_rfc3339_datetime(verified_at) {
        return Err(config_error(format!(
            "{section}: verified_at must be an RFC 3339 date-time"
        )));
    }
    required_string(value, "normal_state", &section)?;
    string_array(value, "commands", &section)?;
    let tests = value["tests"]
        .as_array()
        .ok_or_else(|| config_error(format!("{section}: tests must be an array")))?;
    for (index, test) in tests.iter().enumerate() {
        let test_context = format!("{section}: tests[{index}]");
        validate_keys(
            test,
            &["name", "command", "outcome", "exit_code", "evidence"],
            &test_context,
        )?;
        for field in ["name", "command", "evidence"] {
            required_string(test, field, &test_context)?;
        }
        let outcome = required_string(test, "outcome", &test_context)?;
        if !["PASS", "FAIL", "IN_PROGRESS", "NOT_RUN"].contains(&outcome) {
            return Err(config_error(format!(
                "{test_context}: invalid outcome `{outcome}`"
            )));
        }
        if !test["exit_code"].is_null() && !test["exit_code"].is_i64() {
            return Err(config_error(format!(
                "{test_context}: exit_code must be an integer or null"
            )));
        }
    }
    let runs = value["ci_runs"]
        .as_array()
        .ok_or_else(|| config_error(format!("{section}: ci_runs must be an array")))?;
    for (index, run) in runs.iter().enumerate() {
        let run_context = format!("{section}: ci_runs[{index}]");
        validate_keys(
            run,
            &[
                "run_id",
                "url",
                "head_sha",
                "status",
                "conclusion",
                "summary",
            ],
            &run_context,
        )?;
        if run["run_id"].as_u64().is_none_or(|id| id == 0) {
            return Err(config_error(format!(
                "{run_context}: run_id must be positive"
            )));
        }
        let sha = required_string(run, "head_sha", &run_context)?;
        if !is_sha(sha) {
            return Err(config_error(format!("{run_context}: invalid head_sha")));
        }
        for field in ["url", "summary"] {
            required_string(run, field, &run_context)?;
        }
        if !is_absolute_uri(run["url"].as_str().unwrap_or_default()) {
            return Err(config_error(format!(
                "{run_context}: url must be an absolute URI"
            )));
        }
        let status = required_string(run, "status", &run_context)?;
        if !["QUEUED", "IN_PROGRESS", "COMPLETED"].contains(&status) {
            return Err(config_error(format!(
                "{run_context}: invalid status `{status}`"
            )));
        }
        if !run["conclusion"].is_null() && !run["conclusion"].is_string() {
            return Err(config_error(format!(
                "{run_context}: conclusion must be a string or null"
            )));
        }
    }
    Ok(())
}

fn validate_release_line_gate(value: &Value, context: &str) -> Result<(), CliError> {
    let section = format!("{context}: release_line_gate");
    validate_keys(
        value,
        &["source", "last_checked_at", "activation_requirement"],
        &section,
    )?;
    required_string(value, "source", &section)?;
    let last_checked_at = required_string(value, "last_checked_at", &section)?;
    if !is_rfc3339_datetime(last_checked_at) {
        return Err(config_error(format!(
            "{section}: last_checked_at must be an RFC 3339 date-time"
        )));
    }
    required_string(value, "activation_requirement", &section)?;
    Ok(())
}

fn validate_blocker(value: &Value, context: &str) -> Result<(), CliError> {
    let section = format!("{context}: blocker");
    validate_keys(
        value,
        &[
            "active",
            "class",
            "summary",
            "reproduction_commands",
            "failure_log_summary",
            "attempted_fixes",
            "do_not_try",
        ],
        &section,
    )?;
    let active = value["active"]
        .as_bool()
        .ok_or_else(|| config_error(format!("{section}: active must be boolean")))?;
    let class = &value["class"];
    let summary = &value["summary"];
    let class = if class.is_null() {
        None
    } else {
        Some(class.as_str().ok_or_else(|| {
            config_error(format!(
                "{section}: class must be a known failure class or null"
            ))
        })?)
    };
    if let Some(class) = class {
        if !FAILURE_CLASSES.contains(&class) {
            return Err(config_error(format!(
                "{section}: unknown failure class `{class}`"
            )));
        }
    }
    if active {
        if class.is_none() || summary.as_str().is_none_or(str::is_empty) {
            return Err(config_error(format!(
                "{section}: active blocker needs a known class and summary"
            )));
        }
        if value["reproduction_commands"]
            .as_array()
            .is_none_or(Vec::is_empty)
        {
            return Err(config_error(format!(
                "{section}: active blocker needs a reproduction command"
            )));
        }
    }
    for field in ["summary", "failure_log_summary"] {
        if !value[field].is_null() && !value[field].is_string() {
            return Err(config_error(format!(
                "{section}: {field} must be a string or null"
            )));
        }
    }
    for field in ["reproduction_commands", "attempted_fixes", "do_not_try"] {
        string_array(value, field, &section)?;
    }
    Ok(())
}

fn diagnose(args: &[String], root: &Path) -> Result<Vec<String>, CliError> {
    let mut stage: Option<String> = None;
    let mut exit_code: Option<i32> = None;
    let mut declared_class: Option<String> = None;
    let mut logs = Vec::new();
    let mut artifacts = Vec::new();
    let mut output = None;
    let mut index = 0;
    while index < args.len() {
        let option = args[index].as_str();
        index += 1;
        let value = args.get(index).ok_or_else(usage_error)?;
        index += 1;
        match option {
            "--stage" if stage.is_none() => stage = Some(value.clone()),
            "--exit-code" if exit_code.is_none() => {
                exit_code = Some(value.parse().map_err(|_| usage_error())?)
            }
            "--failure-class" if declared_class.is_none() => {
                let parsed = value.to_ascii_uppercase();
                if !FAILURE_CLASSES.contains(&parsed.as_str()) {
                    return Err(usage_error());
                }
                declared_class = Some(parsed);
            }
            "--log" => logs.push(value.clone()),
            "--artifact" => artifacts.push(value.clone()),
            "--output" if output.is_none() => output = Some(value.clone()),
            _ => return Err(usage_error()),
        }
    }
    let stage = stage
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(usage_error)?;
    let exit_code = exit_code.ok_or_else(usage_error)?;
    if logs.is_empty() {
        return Err(usage_error());
    }
    let report = build_diagnostic_report(
        root,
        &stage,
        exit_code,
        declared_class.as_deref(),
        &logs,
        &artifacts,
    )?;
    let bytes = serde_json::to_vec_pretty(&report)
        .map_err(|error| config_error(format!("cannot encode diagnostic report: {error}")))?;
    if let Some(output) = output {
        let path = resolve_path(root, &output);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| {
                config_error(format!("cannot create {}: {error}", parent.display()))
            })?;
        }
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| {
                config_error(format!(
                    "cannot create diagnostic report {} without overwriting: {error}",
                    path.display()
                ))
            })?;
        file.write_all(&bytes)
            .map_err(|error| config_error(format!("cannot write {}: {error}", path.display())))?;
        Ok(vec![
            format!(
                "DIAGNOSTICS recorded: stage {stage}; process exit code {exit_code}; report {}",
                path.display()
            ),
            format!(
                "Suggested failure class: {}",
                report["suggested_failure_class"]
                    .as_str()
                    .unwrap_or("UNKNOWN")
            ),
            format!(
                "Logs: {}; undefined symbols: {}; errors: {}; artifacts: {}",
                logs.len(),
                report["undefined_symbols"].as_array().map_or(0, Vec::len),
                report["errors"].as_array().map_or(0, Vec::len),
                artifacts.len()
            ),
        ])
    } else {
        Ok(vec![String::from_utf8_lossy(&bytes).into_owned()])
    }
}

fn build_diagnostic_report(
    root: &Path,
    stage: &str,
    exit_code: i32,
    declared_class: Option<&str>,
    log_paths: &[String],
    artifact_paths: &[String],
) -> Result<Value, CliError> {
    let mut log_reports = Vec::new();
    let mut errors = BTreeSet::new();
    let mut symbols = BTreeSet::new();
    let mut combined = String::new();
    for input in log_paths {
        let path = resolve_path(root, input);
        match fs::read(&path) {
            Ok(bytes) => {
                let content = String::from_utf8_lossy(&bytes);
                let inventory = inventory_log(&content);
                errors.extend(inventory.errors.iter().cloned());
                symbols.extend(inventory.undefined_symbols.iter().cloned());
                combined.push_str(&content);
                combined.push('\n');
                log_reports.push(json!({
                    "path": display_input_path(root, &path),
                    "exists": true,
                    "byte_length": bytes.len(),
                    "sha256": sha256_hex(&bytes),
                    "errors": inventory.errors,
                    "undefined_symbols": inventory.undefined_symbols,
                    "tail": inventory.tail,
                }));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                log_reports.push(json!({
                    "path": display_input_path(root, &path),
                    "exists": false,
                    "error": error.to_string(),
                }));
            }
            Err(error) => {
                return Err(config_error(format!(
                    "cannot read log {}: {error}",
                    path.display()
                )))
            }
        }
    }
    let mut artifact_reports = Vec::new();
    for input in artifact_paths {
        let path = resolve_path(root, input);
        match fs::metadata(&path) {
            Ok(metadata) => {
                if !metadata.is_file() {
                    return Err(config_error(format!(
                        "artifact path is not a regular file: {}",
                        path.display()
                    )));
                }
                artifact_reports.push(json!({
                    "path": display_input_path(root, &path),
                    "exists": true,
                    "byte_length": metadata.len(),
                    "sha256": sha256_file(&path)?,
                    "format_hint": path.extension().and_then(|ext| ext.to_str()).unwrap_or("unknown"),
                }));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                artifact_reports.push(json!({
                    "path": display_input_path(root, &path),
                    "exists": false,
                    "error": error.to_string(),
                }))
            }
            Err(error) => {
                return Err(config_error(format!(
                    "cannot read artifact {}: {error}",
                    path.display()
                )))
            }
        }
    }
    let suggested_class = suggest_failure_class(&combined);
    let source_commit = git_output(root, &["rev-parse", "HEAD"]).ok();
    let source_worktree_dirty = git_output(root, &["status", "--porcelain"])
        .ok()
        .map(|status| !status.trim().is_empty());
    Ok(json!({
        "schema_version": 1,
        "source_commit": source_commit,
        "source_worktree_dirty": source_worktree_dirty,
        "stage": stage,
        "process_exit_code": exit_code,
        "termination": termination_reason(exit_code, &combined),
        "declared_failure_class": declared_class,
        "suggested_failure_class": suggested_class,
        "logs": log_reports,
        "errors": errors,
        "undefined_symbols": symbols,
        "artifacts": artifact_reports,
        "summary": format!("stage {stage} exited with code {exit_code}; {} unique diagnostic lines and {} unique undefined symbols", errors.len(), symbols.len()),
    }))
}

#[derive(Debug, Default)]
struct LogInventory {
    errors: BTreeSet<String>,
    undefined_symbols: BTreeSet<String>,
    tail: Vec<String>,
}

fn inventory_log(log: &str) -> LogInventory {
    const TAIL_LINES: usize = 80;
    let lines: Vec<&str> = log.lines().collect();
    let mut inventory = LogInventory {
        tail: lines
            .iter()
            .rev()
            .take(TAIL_LINES)
            .rev()
            .map(|line| (*line).to_owned())
            .collect(),
        ..LogInventory::default()
    };
    for line in &lines {
        let trimmed = line.trim();
        let lower = trimmed.to_ascii_lowercase();
        if lower.contains("error:")
            || lower.starts_with("fail ")
            || lower.contains("timed out")
            || lower.contains("timeout")
            || lower.contains("did not exit within")
            || lower.contains("panic")
            || lower.contains("failed to")
        {
            inventory.errors.insert(trimmed.to_owned());
        }
        if lower.contains("undefined symbol")
            || lower.contains("undefined reference")
            || lower.contains("unresolved external symbol")
        {
            if let Some(symbol) = extract_symbol(trimmed) {
                inventory.undefined_symbols.insert(symbol);
            } else {
                inventory.errors.insert(trimmed.to_owned());
            }
        }
    }
    inventory
}

fn extract_symbol(line: &str) -> Option<String> {
    let lower = line.to_ascii_lowercase();
    let (index, marker_len) = [
        (lower.find("undefined symbol"), "undefined symbol".len()),
        (
            lower.find("undefined reference to"),
            "undefined reference to".len(),
        ),
        (
            lower.find("unresolved external symbol"),
            "unresolved external symbol".len(),
        ),
    ]
    .into_iter()
    .filter_map(|(index, length)| index.map(|index| (index, length)))
    .min_by_key(|(index, _)| *index)?;
    let rest = line[index + marker_len..]
        .trim()
        .trim_start_matches(':')
        .trim();
    let rest = rest
        .trim_start_matches('`')
        .trim_start_matches('\'')
        .trim_start_matches('"');
    let symbol = rest
        .split(|ch: char| ch.is_whitespace() || matches!(ch, '\'' | '"' | '`' | ',' | ';'))
        .next()
        .unwrap_or("")
        .trim();
    (!symbol.is_empty()).then(|| symbol.to_owned())
}

fn suggest_failure_class(log: &str) -> &'static str {
    let lower = log.to_ascii_lowercase();
    let timed_out = lower.contains("timeout")
        || lower.contains("timed out")
        || lower.contains("did not exit within");
    if timed_out
        && ["egl", "gl context", "softpipe", "surfman", "mesa", "servo"]
            .iter()
            .any(|term| lower.contains(term))
    {
        "GRAPHICS"
    } else if lower.contains("undefined symbol")
        || lower.contains("undefined reference")
        || lower.contains("unresolved external symbol")
    {
        "LINK"
    } else if [
        "connection reset",
        "network is unreachable",
        "temporary failure in name resolution",
        "could not resolve host",
        "failed to download",
    ]
    .iter()
    .any(|term| lower.contains(term))
    {
        "NETWORK"
    } else if [
        "runner lost",
        "runner communication",
        "job was cancelled",
        "job was canceled",
        "out of memory: killed process",
    ]
    .iter()
    .any(|term| lower.contains(term))
    {
        "CI_INFRA"
    } else if ["kernel panic", "page fault", "triple fault", "boot failed"]
        .iter()
        .any(|term| lower.contains(term))
    {
        "BOOT"
    } else if lower.contains("permission denied") || lower.contains("permission check failed") {
        "PERMISSION"
    } else if lower.contains("acceptance")
        || lower.contains("did not print")
        || lower.contains("checksum mismatch")
    {
        "ACCEPTANCE"
    } else if lower.contains("could not compile")
        || lower.contains("fatal error:")
        || lower.contains("syntax error")
    {
        "SOURCE"
    } else if lower.contains("command not found") || lower.contains("no such file or directory") {
        "HOST_ENV"
    } else {
        "UNKNOWN"
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sha256_file(path: &Path) -> Result<String, CliError> {
    let mut file = File::open(path).map_err(|error| {
        config_error(format!("cannot open artifact {}: {error}", path.display()))
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file.read(&mut buffer).map_err(|error| {
            config_error(format!("cannot hash artifact {}: {error}", path.display()))
        })?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}

fn termination_reason(exit_code: i32, logs: &str) -> &'static str {
    let lower = logs.to_ascii_lowercase();
    if exit_code == 0 {
        "exited"
    } else if lower.contains("timed out")
        || lower.contains("timeout")
        || lower.contains("did not exit within")
    {
        "timeout"
    } else if exit_code == 124 {
        "timeout_exit_124"
    } else if exit_code >= 128 {
        "signal_or_wrapper_exit"
    } else {
        "nonzero_exit"
    }
}

fn repo_state(root: &Path) -> Result<RepoState, CliError> {
    let branch = git_output(root, &["rev-parse", "--abbrev-ref", "HEAD"])?;
    let head = git_output(root, &["rev-parse", "HEAD"])?;
    let status = git_output(root, &["status", "--porcelain"])?;
    Ok(RepoState {
        branch,
        head,
        dirty: !status.trim().is_empty(),
    })
}

fn git_output(root: &Path, args: &[&str]) -> Result<String, CliError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|error| {
            CliError::new(
                format!("cannot run git {}: {error}", args.join(" ")),
                EXIT_CONFIG_ERROR,
            )
        })?;
    if !output.status.success() {
        return Err(CliError::new(
            format!(
                "git {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            ),
            EXIT_CONFIG_ERROR,
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn validate_keys(value: &Value, expected: &[&str], context: &str) -> Result<(), CliError> {
    let object = value
        .as_object()
        .ok_or_else(|| config_error(format!("{context}: expected an object")))?;
    for key in expected {
        if !object.contains_key(*key) {
            return Err(config_error(format!(
                "{context}: missing required field `{key}`"
            )));
        }
    }
    if let Some(unexpected) = object.keys().find(|key| !expected.contains(&key.as_str())) {
        return Err(config_error(format!(
            "{context}: unknown field `{unexpected}`"
        )));
    }
    Ok(())
}

fn required_string<'a>(value: &'a Value, field: &str, context: &str) -> Result<&'a str, CliError> {
    value[field]
        .as_str()
        .filter(|item| !item.trim().is_empty())
        .ok_or_else(|| config_error(format!("{context}: `{field}` must be a non-empty string")))
}

fn string_array(value: &Value, field: &str, context: &str) -> Result<Vec<String>, CliError> {
    let array = value[field]
        .as_array()
        .ok_or_else(|| config_error(format!("{context}: `{field}` must be an array")))?;
    array
        .iter()
        .map(|entry| {
            entry
                .as_str()
                .map(ToOwned::to_owned)
                .filter(|item| !item.trim().is_empty())
                .ok_or_else(|| {
                    config_error(format!(
                        "{context}: `{field}` entries must be non-empty strings"
                    ))
                })
        })
        .collect()
}

fn is_sha(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn is_absolute_uri(value: &str) -> bool {
    let Some((scheme, remainder)) = value.split_once(':') else {
        return false;
    };
    let mut chars = scheme.chars();
    chars
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '.' | '-'))
        && !remainder.is_empty()
        && !value.chars().any(char::is_whitespace)
        && !value.chars().any(char::is_control)
}

fn is_rfc3339_datetime(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() < 20
        || bytes[4] != b'-'
        || bytes[7] != b'-'
        || !matches!(bytes[10], b'T' | b't')
        || bytes[13] != b':'
        || bytes[16] != b':'
    {
        return false;
    }
    for range in [0..4, 5..7, 8..10, 11..13, 14..16, 17..19] {
        if !bytes[range].iter().all(u8::is_ascii_digit) {
            return false;
        }
    }

    let year = decimal(&bytes[0..4]);
    let month = decimal(&bytes[5..7]);
    let day = decimal(&bytes[8..10]);
    let hour = decimal(&bytes[11..13]);
    let minute = decimal(&bytes[14..16]);
    let second = decimal(&bytes[17..19]);
    if !(1..=12).contains(&month) || !(0..=23).contains(&hour) || minute > 59 || second > 60 {
        return false;
    }
    let leap_year =
        year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400));
    let days_in_month = match month {
        2 if leap_year => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if !(1..=days_in_month).contains(&day) {
        return false;
    }

    let zone_index = if matches!(bytes.last(), Some(b'Z' | b'z')) {
        bytes.len() - 1
    } else if bytes.len() >= 25
        && matches!(bytes[bytes.len() - 6], b'+' | b'-')
        && bytes[bytes.len() - 3] == b':'
        && bytes[bytes.len() - 5..bytes.len() - 3]
            .iter()
            .all(u8::is_ascii_digit)
        && bytes[bytes.len() - 2..].iter().all(u8::is_ascii_digit)
    {
        let offset_hour = decimal(&bytes[bytes.len() - 5..bytes.len() - 3]);
        let offset_minute = decimal(&bytes[bytes.len() - 2..]);
        if offset_hour > 23 || offset_minute > 59 {
            return false;
        }
        bytes.len() - 6
    } else {
        return false;
    };

    if zone_index == 19 {
        return true;
    }
    bytes[19] == b'.' && zone_index > 20 && bytes[20..zone_index].iter().all(u8::is_ascii_digit)
}

fn decimal(digits: &[u8]) -> u32 {
    digits
        .iter()
        .fold(0, |value, digit| value * 10 + u32::from(digit - b'0'))
}

fn resolve_path(root: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn display_input_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .map(|relative| relative.display().to_string())
        .unwrap_or_else(|_| path.display().to_string())
}

fn config_error(message: String) -> CliError {
    CliError::new(message, EXIT_CONFIG_ERROR)
}

trait EmptyFallback {
    fn if_empty(self, fallback: &str) -> String;
}

impl EmptyFallback for String {
    fn if_empty(self, fallback: &str) -> String {
        if self.is_empty() {
            fallback.into()
        } else {
            self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        build_diagnostic_report, diagnose, inventory_log, is_rfc3339_datetime, load_registry,
        load_state, sha256_file, sha256_hex, suggest_failure_class, validate_state, REGISTRY_PATH,
    };
    use serde_json::json;
    use std::path::Path;

    fn repo_root() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    #[test]
    fn registry_and_active_workstream_state_validate_from_repository() {
        let root = repo_root();
        let registry = load_registry(&root).expect("valid workstream registry");
        let stream = registry
            .iter()
            .find(|entry| entry.id == "development-foundation")
            .expect("foundation stream");
        let state = load_state(&root, stream, &registry).expect("valid workstream state");
        assert!(matches!(
            state["status"].as_str(),
            Some("NOT_STARTED" | "IN_PROGRESS" | "PASS" | "PARTIAL" | "BLOCKED" | "DEFERRED")
        ));
        assert_eq!(
            state["release_line_gate"]["source"],
            "docs/implementation_status.md"
        );
    }

    #[test]
    fn registry_validation_rejects_duplicate_ids_branches_dependencies_and_unsafe_paths() {
        let row = |id: &str, branch: &str, state_file: &str, dependencies: Vec<&str>| {
            json!({
                "id": id,
                "owner": "test owner",
                "owner_branch": branch,
                "recommended_worktree": "../NagiOS-test",
                "state_file": state_file,
                "dependencies": dependencies,
                "allowed_paths": ["docs/**"],
                "forbidden_paths": [],
                "activation_gate": "test gate",
                "merge_boundary": "test boundary"
            })
        };
        let alpha = row(
            "alpha",
            "codex/alpha",
            ".dev/workstreams/alpha/state.json",
            vec![],
        );
        let beta = row(
            "beta",
            "codex/beta",
            ".dev/workstreams/beta/state.json",
            vec![],
        );
        let invalid = [
            (
                "duplicate-id",
                vec![
                    alpha.clone(),
                    row(
                        "alpha",
                        "codex/other",
                        ".dev/workstreams/alpha/state.json",
                        vec![],
                    ),
                ],
                "duplicate workstream id",
            ),
            (
                "duplicate-branch",
                vec![
                    alpha.clone(),
                    row(
                        "beta",
                        "codex/alpha",
                        ".dev/workstreams/beta/state.json",
                        vec![],
                    ),
                ],
                "duplicate owner branch",
            ),
            (
                "missing-dependency",
                vec![row(
                    "alpha",
                    "codex/alpha",
                    ".dev/workstreams/alpha/state.json",
                    vec!["missing"],
                )],
                "missing dependency",
            ),
            (
                "unsafe-state-path",
                vec![row(
                    "alpha",
                    "codex/alpha",
                    ".dev/workstreams/alpha/../state.json",
                    vec![],
                )],
                "unsafe state_file",
            ),
            (
                "duplicate-dependency",
                vec![
                    row(
                        "alpha",
                        "codex/alpha",
                        ".dev/workstreams/alpha/state.json",
                        vec!["beta", "beta"],
                    ),
                    beta.clone(),
                ],
                "dependencies contains a duplicate",
            ),
        ];

        for (label, workstreams, expected_error) in invalid {
            let root =
                std::env::temp_dir().join(format!("nagi-registry-{label}-{}", std::process::id()));
            std::fs::create_dir_all(root.join(".dev")).expect("create test registry directory");
            let document = json!({
                "schema_ref": "schemas/workstreams.schema.json",
                "schema_version": 1,
                "workstreams": workstreams
            });
            std::fs::write(
                root.join(REGISTRY_PATH),
                serde_json::to_vec(&document).expect("encode registry"),
            )
            .expect("write test registry");
            let error = load_registry(&root).expect_err("invalid registry must be rejected");
            assert!(error.to_string().contains(expected_error), "{error}");
            std::fs::remove_dir_all(root).expect("remove test registry");
        }
    }

    #[test]
    fn diagnostic_inventory_keeps_every_link_symbol_and_error() {
        let inventory = inventory_log(
            "rust-lld: error: undefined symbol: alpha\nerror: undefined reference to `beta`\nerror: undefined symbol: gamma\n",
        );
        assert_eq!(
            inventory.undefined_symbols.into_iter().collect::<Vec<_>>(),
            ["alpha", "beta", "gamma"]
        );
        assert_eq!(inventory.errors.len(), 3);
        assert_eq!(
            suggest_failure_class("rust-lld: error: undefined symbol: alpha"),
            "LINK"
        );
    }

    #[test]
    fn diagnostic_report_hashes_logs_and_records_artifact_metadata() {
        let temp =
            std::env::temp_dir().join(format!("nagi-diagnostic-test-{}", std::process::id()));
        std::fs::create_dir_all(&temp).expect("temp directory");
        let log = temp.join("build.log");
        let artifact = temp.join("sample.img");
        std::fs::write(&log, "error: timed out in Servo GL context\n").expect("write log");
        std::fs::write(&artifact, b"artifact").expect("write artifact");
        let report = build_diagnostic_report(
            &temp,
            "qemu-acceptance",
            124,
            Some("GRAPHICS"),
            &["build.log".into()],
            &["sample.img".into()],
        )
        .expect("diagnostic report");
        assert_eq!(report["suggested_failure_class"], "GRAPHICS");
        assert_eq!(
            report["logs"][0]["sha256"],
            sha256_hex(b"error: timed out in Servo GL context\n")
        );
        assert_eq!(report["artifacts"][0]["byte_length"], 8);
        assert_eq!(report["artifacts"][0]["sha256"], sha256_hex(b"artifact"));
        assert_eq!(report["process_exit_code"], 124);
        assert_eq!(report["termination"], "timeout");
        assert!(report["source_commit"].is_null());
        assert!(report["source_worktree_dirty"].is_null());
        assert_eq!(
            sha256_file(&artifact).expect("stream artifact hash"),
            sha256_hex(b"artifact")
        );
        std::fs::remove_dir_all(temp).expect("cleanup temp directory");
    }

    #[test]
    fn diagnostic_output_preserves_source_logs_and_never_overwrites_reports() {
        let root = std::env::temp_dir().join(format!(
            "nagi-diagnostic-output-test-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&root).expect("create diagnostic test directory");
        let log = root.join("runtime.log");
        let report = root.join("diagnostics.json");
        let source_log = b"GL context setup timed out\n";
        std::fs::write(&log, source_log).expect("write source log");
        let args = vec![
            "--stage".into(),
            "runtime".into(),
            "--exit-code".into(),
            "4".into(),
            "--log".into(),
            log.display().to_string(),
            "--output".into(),
            report.display().to_string(),
        ];

        diagnose(&args, &root).expect("create diagnostic report");
        let first_report = std::fs::read(&report).expect("read diagnostic report");
        let report_json: serde_json::Value =
            serde_json::from_slice(&first_report).expect("valid diagnostic JSON");
        assert_eq!(report_json["process_exit_code"], 4);
        assert_eq!(report_json["source_commit"], serde_json::Value::Null);
        assert_eq!(
            std::fs::read(&log).expect("read unchanged source log"),
            source_log
        );
        assert!(
            diagnose(&args, &root).is_err(),
            "existing report is not overwritten"
        );
        assert_eq!(
            std::fs::read(&report).expect("report remains unchanged"),
            first_report
        );
        std::fs::remove_dir_all(root).expect("remove diagnostic test directory");
    }

    #[test]
    fn diagnostic_class_suggests_infrastructure_for_runner_loss() {
        assert_eq!(
            suggest_failure_class("GitHub runner communication was lost"),
            "CI_INFRA"
        );
    }

    #[test]
    fn later_graphics_timeout_takes_precedence_over_earlier_linker_warnings() {
        let log = "rust-lld: warning: undefined symbol: optional_symbol\nNagi M17 trace: GL context creation started\nQEMU did not exit within 120 seconds\n";
        assert_eq!(suggest_failure_class(log), "GRAPHICS");
    }

    #[test]
    fn datetime_validation_accepts_rfc3339_and_rejects_invalid_calendar_values() {
        assert!(is_rfc3339_datetime("2026-09-25T08:08:00Z"));
        assert!(is_rfc3339_datetime("2026-09-25T08:08:00.125+09:00"));
        assert!(!is_rfc3339_datetime("2026-13-25T08:08:00Z"));
        assert!(!is_rfc3339_datetime("2026-02-30T08:08:00Z"));
        assert!(!is_rfc3339_datetime("2026-09-25"));
    }

    #[test]
    fn sha256_matches_standard_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn validator_rejects_unknown_status_and_missing_next_action() {
        let root = repo_root();
        let registry = load_registry(&root).expect("registry");
        let stream = registry
            .iter()
            .find(|entry| entry.id == "development-foundation")
            .expect("foundation stream");
        let mut state = load_state(&root, stream, &registry).expect("valid state");
        state["status"] = json!("SUCCESS");
        assert!(validate_state(&state, stream, &registry).is_err());
        state["status"] = json!("IN_PROGRESS");
        state["blocker"]["class"] = json!("MADE_UP");
        assert!(validate_state(&state, stream, &registry).is_err());
        state["blocker"]["class"] = serde_json::Value::Null;
        state
            .as_object_mut()
            .expect("state object")
            .remove("next_action");
        assert!(validate_state(&state, stream, &registry).is_err());
    }
}
