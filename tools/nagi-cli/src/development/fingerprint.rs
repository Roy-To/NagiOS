use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::commands::{CommandResult, EXIT_CONFIG_ERROR, EXIT_SUCCESS, EXIT_USAGE};

const FINGERPRINT_SCHEMA_VERSION: u32 = 1;
const PRODUCER_NAME: &str = "nagi dev fingerprint";
const MAX_INPUT_PATHS: usize = 128;
const MAX_DIRECTORY_FILES: usize = 20_000;
const ENVIRONMENT_ALLOWLIST: &[&str] = &[
    "AR",
    "CC",
    "CARGO_ENCODED_RUSTFLAGS",
    "CARGO_BUILD_RUSTFLAGS",
    "CARGO_INCREMENTAL",
    "CFLAGS",
    "CPPFLAGS",
    "CXX",
    "CXXFLAGS",
    "LD",
    "LDFLAGS",
    "NAGI_CXX_HEADERS",
    "NAGI_M16_PACKAGE",
    "NAGI_MESA_BUILD",
    "NAGI_QEMU",
    "NAGI_RELIBC_HEADERS",
    "NAGI_TARGET",
    "NAGI_TARGET_CLANG",
    "OVMF_CODE",
    "OVMF_CODE_PATH",
    "OVMF_HOME",
    "OVMF_PATH",
    "OVMF_VARS",
    "OVMF_VARS_PATH",
    "RUSTC_WRAPPER",
    "RUSTFLAGS",
    "CC_x86_64_unknown_nagi_user",
    "CXX_x86_64_unknown_nagi_user",
    "__CARGO_TESTS_ONLY_SRC_ROOT",
];
const ENVIRONMENT_PATHS_CAPTURED_BY_CONTENT: &[&str] = &[
    "NAGI_CXX_HEADERS",
    "NAGI_M16_PACKAGE",
    "NAGI_MESA_BUILD",
    "NAGI_QEMU",
    "NAGI_RELIBC_HEADERS",
    "OVMF_CODE",
    "OVMF_CODE_PATH",
    "OVMF_HOME",
    "OVMF_PATH",
    "OVMF_VARS",
    "OVMF_VARS_PATH",
    "__CARGO_TESTS_ONLY_SRC_ROOT",
];
const CONFIGURATION_FILES: &[&str] = &[
    "Cargo.toml",
    "Cargo.lock",
    ".cargo/config.toml",
    "nagi.toml",
    "rust-toolchain.toml",
    "third_party/sources.lock",
];

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct FingerprintDocument {
    schema_version: u32,
    compatibility: CompatibilityFingerprint,
    artifacts: Vec<ArtifactRecord>,
    provenance: Provenance,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct CompatibilityFingerprint {
    digest_algorithm: String,
    digest: String,
    inputs: CompatibilityInputs,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct CompatibilityInputs {
    repository_id: String,
    source: SourceIdentity,
    toolchain: ToolchainIdentity,
    target_triple: Option<String>,
    enabled_features: Vec<String>,
    build_profile: Option<String>,
    build_command_digest: Option<String>,
    build_flags: Vec<String>,
    environment_inputs: BTreeMap<String, String>,
    third_party_revisions: BTreeMap<String, String>,
    submodule_revisions: BTreeMap<String, String>,
    nagi_patch_digest: String,
    build_configuration_digest: String,
    generated_inputs: Vec<FileIdentity>,
    runtime_harness: RuntimeHarness,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SourceIdentity {
    commit_sha: String,
    dirty: bool,
    dirty_tree_digest: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ToolchainIdentity {
    rustc: BTreeMap<String, String>,
    cargo: Option<String>,
    host_triple: Option<String>,
    native_tools: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct FileIdentity {
    kind: String,
    path: String,
    byte_length: u64,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct FilePayload {
    byte_length: u64,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ArtifactRecord {
    role: String,
    path: String,
    byte_length: u64,
    sha256: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct RuntimeHarness {
    qemu_version: Option<String>,
    firmware_inputs: Vec<FileIdentity>,
}

struct ThirdPartyInputs {
    revisions: BTreeMap<String, String>,
    patch_inventory: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct Provenance {
    created_at_unix_seconds: u64,
    producer: String,
    producer_version: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct Mismatch {
    class: String,
    field: String,
    left: Value,
    right: Value,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ComparisonReport {
    schema_version: u32,
    left_compatibility_digest: String,
    right_compatibility_digest: String,
    inputs_compatible: bool,
    artifact_inventories_match: bool,
    #[serde(rename = "match")]
    match_: bool,
    mismatches: Vec<Mismatch>,
}

#[derive(Default)]
struct FingerprintOptions {
    output: Option<PathBuf>,
    artifacts: Vec<(Option<String>, String)>,
    generated_inputs: Vec<String>,
    firmware_inputs: Vec<String>,
    target: Option<String>,
    features: Vec<String>,
    profile: Option<String>,
    flags: Vec<String>,
    build_command: Option<String>,
    qemu: Option<String>,
}

pub(super) fn execute(args: &[String], root: &Path) -> CommandResult {
    match args.first().map(String::as_str) {
        Some("--help" | "-h" | "help") if args.len() == 1 => success(vec![
            "usage: nagi dev fingerprint [--output PATH] [--artifact [ROLE=]PATH]... [--input FILE_OR_DIR]... [--firmware PATH]... [--target TRIPLE] [--feature NAME]... [--profile NAME] [--build-command VALUE] [--flag VALUE]... [--qemu PATH]".into(),
            "       nagi dev fingerprint compare <left.json> <right.json> [--json]".into(),
        ]),
        Some("compare") => compare_command(&args[1..], root),
        Some("" | "status" | "resume" | "verify" | "diagnose") => failure(
            EXIT_USAGE,
            "fingerprint accepts only generation options or the compare subcommand",
        ),
        _ => generate_command(args, root),
    }
}

fn generate_command(args: &[String], root: &Path) -> CommandResult {
    let options = match parse_options(args) {
        Ok(options) => options,
        Err(error) => return failure(error.exit_code(), error.to_string()),
    };
    match create_fingerprint(root, &options) {
        Ok(document) => {
            let bytes = match serialize_document(&document) {
                Ok(bytes) => bytes,
                Err(error) => return failure(error.exit_code(), error.to_string()),
            };
            if let Some(path) = options.output {
                if let Err(error) = write_new_file(&path, &bytes) {
                    return failure(error.exit_code(), error.to_string());
                }
                success(vec![format!(
                    "PASS fingerprint {} recorded at {}",
                    document.compatibility.digest,
                    display_output_path(&path)
                )])
            } else {
                success(vec![String::from_utf8_lossy(&bytes).into_owned()])
            }
        }
        Err(error) => failure(error.exit_code(), error.to_string()),
    }
}

fn compare_command(args: &[String], root: &Path) -> CommandResult {
    let (paths, json_output) = match parse_compare_options(args) {
        Ok(options) => options,
        Err(error) => return failure(error.exit_code(), error.to_string()),
    };
    let left = match read_fingerprint(root, &paths[0]) {
        Ok(document) => document,
        Err(error) => return failure(error.exit_code(), error.to_string()),
    };
    let right = match read_fingerprint(root, &paths[1]) {
        Ok(document) => document,
        Err(error) => return failure(error.exit_code(), error.to_string()),
    };
    let report = compare_fingerprints(&left, &right);
    let output = if json_output {
        match serde_json::to_string_pretty(&report) {
            Ok(output) => output,
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!("cannot serialize comparison report: {error}"),
                )
            }
        }
    } else {
        format_comparison(&report)
    };
    CommandResult {
        exit_code: if report.match_ {
            EXIT_SUCCESS
        } else {
            EXIT_CONFIG_ERROR
        },
        lines: vec![output],
    }
}

fn parse_options(args: &[String]) -> Result<FingerprintOptions, crate::commands::CliError> {
    let mut options = FingerprintOptions::default();
    let mut seen_singletons = BTreeSet::new();
    let mut index = 0;
    while index < args.len() {
        let flag = args[index].as_str();
        let value = args
            .get(index + 1)
            .ok_or_else(|| usage_error(format!("{flag} requires a value")))?;
        if !matches!(
            flag,
            "--feature" | "--flag" | "--artifact" | "--input" | "--firmware"
        ) && !seen_singletons.insert(flag)
        {
            return Err(usage_error(format!("{flag} may only be supplied once")));
        }
        match flag {
            "--output" => options.output = Some(PathBuf::from(value)),
            "--artifact" => {
                let (role, path) = parse_artifact_argument(value)?;
                options.artifacts.push((role, path));
            }
            "--input" => options.generated_inputs.push(value.clone()),
            "--firmware" => options.firmware_inputs.push(value.clone()),
            "--target" => options.target = Some(validated_target(value)?),
            "--feature" => options
                .features
                .push(validated_identifier(value, "feature")?),
            "--profile" => options.profile = Some(validated_identifier(value, "build profile")?),
            "--flag" => {
                reject_secret_material(value, "build flag")?;
                options.flags.push(value.clone());
            }
            "--build-command" => {
                reject_secret_material(value, "build command")?;
                options.build_command = Some(value.clone());
            }
            "--qemu" => options.qemu = Some(value.clone()),
            _ => return Err(usage_error(format!("unsupported option `{flag}`"))),
        }
        index += 2;
        if options.artifacts.len() + options.generated_inputs.len() + options.firmware_inputs.len()
            > MAX_INPUT_PATHS
        {
            return Err(usage_error(format!(
                "at most {MAX_INPUT_PATHS} artifact/input paths are allowed"
            )));
        }
    }
    options.features.sort();
    options.features.dedup();
    Ok(options)
}

fn parse_compare_options(
    args: &[String],
) -> Result<([String; 2], bool), crate::commands::CliError> {
    let mut paths = Vec::new();
    let mut json_output = false;
    for arg in args {
        if arg == "--json" {
            if json_output {
                return Err(usage_error("compare accepts --json only once".into()));
            }
            json_output = true;
        } else if arg.starts_with('-') {
            return Err(usage_error(format!("unsupported compare option `{arg}`")));
        } else {
            paths.push(arg.clone());
        }
    }
    if paths.len() != 2 {
        return Err(usage_error(
            "compare requires exactly two fingerprint files".into(),
        ));
    }
    Ok(([paths.remove(0), paths.remove(0)], json_output))
}

fn parse_artifact_argument(
    value: &str,
) -> Result<(Option<String>, String), crate::commands::CliError> {
    if let Some((role, path)) = value.split_once('=') {
        if !role.is_empty() && !path.is_empty() && !role.contains('/') && !role.contains('\\') {
            let role = validated_logical_name(role, "artifact role")?;
            return Ok((Some(role), path.to_owned()));
        }
    }
    if value.is_empty() {
        return Err(usage_error("artifact path cannot be empty".into()));
    }
    Ok((None, value.to_owned()))
}

fn validated_identifier(value: &str, label: &str) -> Result<String, crate::commands::CliError> {
    if value.is_empty()
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'+'))
    {
        return Err(usage_error(format!(
            "{label} must contain only letters, digits, `_`, `-`, `.`, or `+`"
        )));
    }
    Ok(value.to_owned())
}

fn validated_logical_name(value: &str, label: &str) -> Result<String, crate::commands::CliError> {
    if value.is_empty()
        || value.starts_with('/')
        || value.contains("..")
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b'/'))
    {
        return Err(usage_error(format!("{label} is not a safe logical name")));
    }
    Ok(value.to_owned())
}

fn validated_target(value: &str) -> Result<String, crate::commands::CliError> {
    if value.contains('/') || value.contains('\\') {
        validated_logical_name(value, "target")
    } else {
        validated_identifier(value, "target triple")
    }
}

fn target_triple_from_reference(
    root: &Path,
    reference: &str,
) -> Result<String, crate::commands::CliError> {
    let reference = validated_target(reference)?;
    let path = Path::new(&reference);
    if path.extension().is_none_or(|extension| extension != "json") {
        return Ok(reference);
    }
    let path = resolve_input_path(root, &reference);
    if !path.is_file() {
        return Err(config_error(format!(
            "custom target specification `{reference}` is unavailable"
        )));
    }
    let name = path
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or_else(|| config_error("custom target specification has no UTF-8 filename".into()))?;
    validated_identifier(name, "custom target triple")
}

fn create_fingerprint(
    root: &Path,
    options: &FingerprintOptions,
) -> Result<FingerprintDocument, crate::commands::CliError> {
    let artifacts = collect_artifacts(root, &options.artifacts)?;
    let mut generated_inputs = collect_file_identities(root, &options.generated_inputs)?;
    let firmware_inputs = collect_firmware_inputs(root, &options.firmware_inputs)?;
    let repo = repo_identity(root)?;
    let (dirty, dirty_tree_digest) = dirty_source_identity(root)?;
    let source = SourceIdentity {
        commit_sha: git_output(root, &["rev-parse", "HEAD"])?,
        dirty,
        dirty_tree_digest,
    };
    let target_reference = options
        .target
        .clone()
        .or_else(|| env::var("CARGO_BUILD_TARGET").ok())
        .or_else(|| cargo_config_target(root));
    let target_triple = target_reference
        .as_deref()
        .map(|target| target_triple_from_reference(root, target))
        .transpose()?;
    let toolchain = toolchain_identity(root)?;
    let target_triple = target_triple.or_else(|| toolchain.host_triple.clone());
    let environment_inputs = capture_environment_inputs(root, target_triple.as_deref())?;
    generated_inputs.extend(environment_generated_inputs(
        root,
        target_triple.as_deref(),
    )?);
    generated_inputs.sort_by(file_identity_order);
    generated_inputs.dedup();
    let third_party = third_party_inputs(root)?;
    let submodule_revisions = submodule_revisions(root)?;
    let mut configuration_files = configuration_files(root, target_reference.as_deref())?;
    configuration_files.sort_by(|left, right| left.path.cmp(&right.path));
    let build_configuration_digest = digest_serializable(&configuration_files)?;
    let nagi_patch_digest = digest_serializable(&third_party.patch_inventory)?;
    let qemu_path = options.qemu.clone().or_else(|| env::var("NAGI_QEMU").ok());

    let mut inputs = CompatibilityInputs {
        repository_id: repo,
        source,
        toolchain,
        target_triple,
        enabled_features: options.features.clone(),
        build_profile: options.profile.clone(),
        build_command_digest: options
            .build_command
            .as_deref()
            .map(|command| digest_normalized_text(root, command)),
        build_flags: options
            .flags
            .iter()
            .map(|flag| digest_normalized_text(root, flag))
            .collect(),
        environment_inputs,
        third_party_revisions: third_party.revisions,
        submodule_revisions,
        nagi_patch_digest,
        build_configuration_digest,
        generated_inputs,
        runtime_harness: RuntimeHarness {
            qemu_version: qemu_version(root, qemu_path.as_deref())?,
            firmware_inputs,
        },
    };
    inputs.enabled_features.sort();
    inputs.enabled_features.dedup();
    let digest = digest_serializable(&inputs)?;
    let created_at_unix_seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| config_error(format!("system clock is before Unix epoch: {error}")))?
        .as_secs();
    let document = FingerprintDocument {
        schema_version: FINGERPRINT_SCHEMA_VERSION,
        compatibility: CompatibilityFingerprint {
            digest_algorithm: "sha256".into(),
            digest,
            inputs,
        },
        artifacts,
        provenance: Provenance {
            created_at_unix_seconds,
            producer: PRODUCER_NAME.into(),
            producer_version: env!("CARGO_PKG_VERSION").into(),
        },
    };
    validate_fingerprint(&document)?;
    Ok(document)
}

fn toolchain_identity(root: &Path) -> Result<ToolchainIdentity, crate::commands::CliError> {
    let rustc_path = env::var_os("RUSTC")
        .map(PathBuf::from)
        .or_else(|| pinned_tool_path(root, "rustc"))
        .unwrap_or_else(|| PathBuf::from("rustc"));
    let rustc_output = required_command_output(
        &rustc_path,
        &["--version", "--verbose"],
        root,
        "Rust compiler",
    )?;
    let mut rustc = BTreeMap::new();
    for line in String::from_utf8_lossy(&rustc_output.stdout).lines() {
        if let Some((key, value)) = line.split_once(':') {
            if matches!(
                key.trim(),
                "release" | "commit-hash" | "host" | "llvm-version"
            ) {
                rustc.insert(key.trim().to_owned(), value.trim().to_owned());
            }
        } else if line.starts_with("rustc ") {
            rustc.insert("version".into(), line.trim().to_owned());
        }
    }
    let cargo_path = env::var_os("CARGO")
        .map(PathBuf::from)
        .or_else(|| pinned_tool_path(root, "cargo"))
        .unwrap_or_else(|| PathBuf::from("cargo"));
    let cargo = required_command_output(&cargo_path, &["--version"], root, "Cargo")?;
    let cargo = String::from_utf8_lossy(&cargo.stdout).trim().to_owned();
    if cargo.is_empty() {
        return Err(config_error(
            "Cargo returned an empty version string".into(),
        ));
    }
    let host_triple = rustc
        .get("host")
        .cloned()
        .or_else(|| Some(format!("{}-{}", env::consts::ARCH, env::consts::OS)));
    let native_tools = native_tool_versions(root);
    Ok(ToolchainIdentity {
        rustc,
        cargo: Some(cargo),
        host_triple,
        native_tools,
    })
}

fn native_tool_versions(root: &Path) -> BTreeMap<String, String> {
    let mut commands = BTreeMap::new();
    commands.insert(
        "nagi_target_clang".to_owned(),
        env::var("NAGI_TARGET_CLANG").unwrap_or_else(|_| "clang".into()),
    );
    for key in ["CC", "CXX", "AR", "LD"] {
        if let Ok(command) = env::var(key) {
            commands.insert(key.to_ascii_lowercase(), command);
        }
    }
    let mut versions = BTreeMap::new();
    for (name, command) in commands {
        let program = command.split_whitespace().next().unwrap_or("");
        if program.is_empty() {
            continue;
        }
        if let Some(output) = command_output(Path::new(program), &["--version"], root) {
            if output.status.success() {
                let version = String::from_utf8_lossy(&output.stdout)
                    .lines()
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_owned();
                if !version.is_empty() && !has_secret_assignment(&version) {
                    versions.insert(name, version);
                }
            }
        }
    }
    versions
}

fn pinned_tool_path(root: &Path, tool: &str) -> Option<PathBuf> {
    let toolchain = fs::read_to_string(root.join("rust-toolchain.toml"))
        .ok()?
        .lines()
        .find_map(|line| {
            let (key, value) = line.trim().split_once('=')?;
            (key.trim() == "channel").then(|| unquote_toml_string(value.trim()))
        })?;
    let output = Command::new("rustup")
        .args(["which", tool, "--toolchain", &toolchain])
        .current_dir(root)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8(output.stdout).ok()?;
    let path = PathBuf::from(path.trim());
    path.is_file().then_some(path)
}

fn required_command_output(
    program: &Path,
    args: &[&str],
    root: &Path,
    label: &str,
) -> Result<std::process::Output, crate::commands::CliError> {
    let output = Command::new(program)
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|error| {
            config_error(format!(
                "cannot query {label} version using {}: {error}",
                program.display()
            ))
        })?;
    if !output.status.success() {
        return Err(config_error(format!(
            "cannot query {label} version: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output)
}

fn repo_identity(root: &Path) -> Result<String, crate::commands::CliError> {
    let output = Command::new("git")
        .args(["remote", "get-url", "origin"])
        .current_dir(root)
        .output()
        .map_err(|error| config_error(format!("cannot identify repository remote: {error}")))?;
    if !output.status.success() {
        return Ok("nagi-os".into());
    }
    let remote = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    Ok(sanitize_repository_identifier(&remote))
}

fn sanitize_repository_identifier(remote: &str) -> String {
    let mut value = remote.trim();
    if value.starts_with("file:") || value.starts_with('/') || value.starts_with("\\\\") {
        return "nagi-os".into();
    }
    if let Some(rest) = value
        .strip_prefix("https://")
        .or_else(|| value.strip_prefix("http://"))
    {
        value = rest;
    } else if let Some(rest) = value.strip_prefix("ssh://") {
        value = rest;
    }
    if let Some((host, path)) = value.split_once(':') {
        if host.contains('@') && !host.contains('/') {
            let host = host.rsplit('@').next().unwrap_or(host);
            let value = format!("{host}/{path}");
            return value
                .trim_end_matches('/')
                .trim_end_matches(".git")
                .to_owned();
        }
    }
    let without_query = value.split(['?', '#']).next().unwrap_or(value);
    let without_user = if let Some((before_path, after_path)) = without_query.split_once('/') {
        let authority = before_path.rsplit('@').next().unwrap_or(before_path);
        format!("{authority}/{after_path}")
    } else if let Some((host, path)) = without_query.split_once(':') {
        if host.contains('@') {
            format!("{}/{path}", host.rsplit('@').next().unwrap_or(host))
        } else {
            without_query.to_owned()
        }
    } else {
        without_query.to_owned()
    };
    let value = without_user.trim_end_matches('/').trim_end_matches(".git");
    if value.is_empty() || value.starts_with('/') || value.contains('\\') {
        "nagi-os".into()
    } else {
        value.to_owned()
    }
}

fn dirty_source_identity(root: &Path) -> Result<(bool, Option<String>), crate::commands::CliError> {
    let mut paths = BTreeSet::new();
    for args in [
        &["diff", "--name-only", "-z", "--no-renames", "HEAD"][..],
        &["ls-files", "--others", "--exclude-standard", "-z"][..],
    ] {
        let bytes = git_bytes(root, args)?;
        for raw in bytes
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
        {
            let path = std::str::from_utf8(raw).map_err(|error| {
                config_error(format!("Git returned a non-UTF-8 changed path: {error}"))
            })?;
            paths.insert(path.to_owned());
        }
    }
    if paths.is_empty() {
        return Ok((false, None));
    }
    let mut inventory = BTreeMap::new();
    for path in paths {
        let full_path = root.join(&path);
        let metadata = match fs::symlink_metadata(&full_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                inventory.insert(path, sha256_hex(b"deleted"));
                continue;
            }
            Err(error) => {
                return Err(config_error(format!(
                    "cannot inspect changed source {}: {error}",
                    path
                )))
            }
        };
        let digest = if metadata.file_type().is_symlink() {
            let target = fs::read_link(&full_path).map_err(|error| {
                config_error(format!("cannot read changed symlink {path}: {error}"))
            })?;
            sha256_hex(target.to_string_lossy().as_bytes())
        } else if metadata.is_file() {
            hash_file(&full_path)?.0
        } else if metadata.is_dir() {
            let revision = git_output(&full_path, &["rev-parse", "HEAD"]).unwrap_or_default();
            sha256_hex(revision.as_bytes())
        } else {
            sha256_hex(b"non-regular-source")
        };
        inventory.insert(path, digest);
    }
    Ok((true, Some(digest_serializable(&inventory)?)))
}

fn submodule_revisions(root: &Path) -> Result<BTreeMap<String, String>, crate::commands::CliError> {
    let output = git_output(root, &["submodule", "status", "--recursive"])?;
    let mut revisions = BTreeMap::new();
    for line in output.lines().filter(|line| !line.trim().is_empty()) {
        if line.len() < 42 {
            return Err(config_error(
                "git submodule status returned malformed output".into(),
            ));
        }
        let marker = &line[..1];
        let remainder = line[1..].trim_start();
        let (revision, path) = remainder.split_once(' ').ok_or_else(|| {
            config_error("git submodule status returned a line without a path".into())
        })?;
        let path = path.trim();
        let status = if marker == " " { "clean" } else { marker };
        let checkout = root.join(path);
        let nested_dirty = if is_separate_git_checkout(root, &checkout) {
            dirty_source_identity(&checkout).ok()
        } else {
            None
        };
        let value = if let Some((true, digest)) = nested_dirty {
            format!("{revision}:{status}:dirty:{}", digest.unwrap_or_default())
        } else {
            format!("{revision}:{status}:clean")
        };
        revisions.insert(path.to_owned(), value);
    }
    Ok(revisions)
}

fn third_party_inputs(root: &Path) -> Result<ThirdPartyInputs, crate::commands::CliError> {
    let lock_path = root.join("third_party/sources.lock");
    let lock = fs::read_to_string(&lock_path)
        .map_err(|error| config_error(format!("cannot read third-party source lock: {error}")))?;
    let mut current = None::<String>;
    let mut revisions = BTreeMap::new();
    let mut patch_paths = BTreeSet::new();
    for line in lock.lines() {
        let line = line.trim();
        if line.starts_with("[sources.") && line.ends_with(']') {
            current = Some(line[9..line.len() - 1].to_owned());
            continue;
        }
        let Some(component) = current.as_deref() else {
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = unquote_toml_string(value.trim());
        if matches!(key, "revision" | "source_hash" | "version" | "toolchain") {
            revisions.insert(format!("{component}.{key}"), value.clone());
        }
        if matches!(key, "nagi_patch" | "patch" | "nagi_adapter") && value != "none" {
            patch_paths.insert(value.clone());
        }
        if key == "vendored_path" {
            let checkout = root.join(&value);
            if is_separate_git_checkout(root, &checkout) {
                if let Ok(revision) = git_output(&checkout, &["rev-parse", "HEAD"]) {
                    revisions.insert(format!("{component}.checkout_revision"), revision);
                }
                if let Ok((dirty, digest)) = dirty_source_identity(&checkout) {
                    revisions.insert(
                        format!("{component}.checkout_dirty"),
                        if dirty {
                            format!("true:{}", digest.unwrap_or_default())
                        } else {
                            "false".into()
                        },
                    );
                }
            }
        }
    }

    let mut inventory = BTreeMap::new();
    let canonical_root = fs::canonicalize(root)
        .map_err(|error| config_error(format!("cannot resolve repository root: {error}")))?;
    for patch_path in patch_paths {
        let full_path = root.join(&patch_path);
        let metadata = fs::symlink_metadata(&full_path).map_err(|error| {
            config_error(format!(
                "third-party lock references unavailable Nagi patch or adapter `{patch_path}`: {error}"
            ))
        })?;
        if metadata.file_type().is_symlink() {
            return Err(config_error(format!(
                "third-party patch input must not be a symlink: `{patch_path}`"
            )));
        }
        let canonical_path = fs::canonicalize(&full_path).map_err(|error| {
            config_error(format!(
                "cannot resolve third-party patch input `{patch_path}`: {error}"
            ))
        })?;
        if !canonical_path.starts_with(&canonical_root) {
            return Err(config_error(format!(
                "third-party lock path escapes the repository: `{patch_path}`"
            )));
        }
        if metadata.is_file() {
            let logical_path = relative_path(&canonical_root, &canonical_path)?;
            inventory.insert(logical_path, hash_file(&canonical_path)?.0);
        } else if metadata.is_dir() {
            collect_tree_file_hashes(&canonical_root, &canonical_path, &mut inventory)?;
        } else {
            return Err(config_error(format!(
                "third-party patch input is not a regular file or directory: `{patch_path}`"
            )));
        }
    }
    Ok(ThirdPartyInputs {
        revisions,
        patch_inventory: inventory,
    })
}

fn is_separate_git_checkout(repository_root: &Path, candidate: &Path) -> bool {
    if !candidate.is_dir() {
        return false;
    }
    let Some(candidate_root) = git_output(candidate, &["rev-parse", "--show-toplevel"]).ok() else {
        return false;
    };
    let candidate_root = PathBuf::from(candidate_root);
    let candidate_root = fs::canonicalize(&candidate_root).unwrap_or(candidate_root);
    let repository_root =
        fs::canonicalize(repository_root).unwrap_or_else(|_| repository_root.to_path_buf());
    candidate_root != repository_root
}

fn collect_tree_file_hashes(
    root: &Path,
    directory: &Path,
    output: &mut BTreeMap<String, String>,
) -> Result<(), crate::commands::CliError> {
    let entries = fs::read_dir(directory).map_err(|error| {
        config_error(format!(
            "cannot read patch directory {}: {error}",
            directory.display()
        ))
    })?;
    let mut entries = entries
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| config_error(format!("cannot read patch directory entry: {error}")))?;
    entries.sort();
    for path in entries {
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            config_error(format!(
                "cannot inspect patch input {}: {error}",
                path.display()
            ))
        })?;
        if metadata.file_type().is_symlink() {
            return Err(config_error(format!(
                "third-party patch inventory contains a symlink: {}",
                path.display()
            )));
        }
        if metadata.is_dir() {
            collect_tree_file_hashes(root, &path, output)?;
        } else if metadata.is_file() {
            let logical = relative_path(root, &path)?;
            output.insert(logical, hash_file(&path)?.0);
        }
    }
    Ok(())
}

fn configuration_files(
    root: &Path,
    target_reference: Option<&str>,
) -> Result<Vec<FileIdentity>, crate::commands::CliError> {
    let mut paths = CONFIGURATION_FILES
        .iter()
        .map(|path| (*path).to_owned())
        .collect::<BTreeSet<_>>();
    if let Some(target) = target_reference {
        let target_path = Path::new(target);
        if target_path
            .extension()
            .is_some_and(|extension| extension == "json")
        {
            paths.insert(target.to_owned());
        } else {
            paths.insert(format!("targets/{target}.json"));
        }
    }
    let mut records = Vec::new();
    for path in paths {
        let full_path = root.join(&path);
        if full_path.is_file() {
            records.push(file_identity(root, &path)?);
        }
    }
    Ok(records)
}

fn collect_file_identities(
    root: &Path,
    paths: &[String],
) -> Result<Vec<FileIdentity>, crate::commands::CliError> {
    let mut records = paths
        .iter()
        .map(|path| file_identity(root, path))
        .collect::<Result<Vec<_>, _>>()?;
    records.sort_by(file_identity_order);
    records.dedup();
    Ok(records)
}

fn file_identity(root: &Path, input: &str) -> Result<FileIdentity, crate::commands::CliError> {
    let requested_path = resolve_input_path(root, input);
    let requested_metadata = fs::symlink_metadata(&requested_path).map_err(|error| {
        config_error(format!(
            "requested input {} is unavailable: {error}",
            requested_path.display()
        ))
    })?;
    let path = if requested_metadata.file_type().is_symlink() {
        fs::canonicalize(&requested_path).map_err(|error| {
            config_error(format!(
                "cannot resolve requested input link {}: {error}",
                requested_path.display()
            ))
        })?
    } else {
        requested_path
    };
    let metadata = fs::metadata(&path).map_err(|error| {
        config_error(format!(
            "cannot inspect requested input {}: {error}",
            path.display()
        ))
    })?;
    let (kind, sha256, byte_length) = if metadata.is_file() {
        let (sha256, byte_length) = hash_file(&path)?;
        ("file", sha256, byte_length)
    } else if metadata.is_dir() {
        let (sha256, byte_length) = hash_directory(&path)?;
        ("directory", sha256, byte_length)
    } else {
        return Err(config_error(format!(
            "requested input is not a regular file or directory: {}",
            path.display()
        )));
    };
    Ok(FileIdentity {
        kind: kind.into(),
        path: logical_path(root, &path)?,
        byte_length,
        sha256,
    })
}

fn hash_directory(path: &Path) -> Result<(String, u64), crate::commands::CliError> {
    let mut inventory = BTreeMap::new();
    collect_directory_inventory(path, Path::new(""), &mut inventory)?;
    let byte_length = inventory
        .values()
        .try_fold(0u64, |total, entry: &FilePayload| {
            total.checked_add(entry.byte_length)
        })
        .ok_or_else(|| config_error("generated input directory size overflowed".into()))?;
    Ok((digest_serializable(&inventory)?, byte_length))
}

fn collect_directory_inventory(
    root: &Path,
    relative: &Path,
    inventory: &mut BTreeMap<String, FilePayload>,
) -> Result<(), crate::commands::CliError> {
    let current = root.join(relative);
    let entries = fs::read_dir(&current).map_err(|error| {
        config_error(format!(
            "cannot read generated input directory {}: {error}",
            current.display()
        ))
    })?;
    let mut entries = entries
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| config_error(format!("cannot read generated input entry: {error}")))?;
    entries.sort();
    for entry in entries {
        let metadata = fs::symlink_metadata(&entry).map_err(|error| {
            config_error(format!(
                "cannot inspect generated input {}: {error}",
                entry.display()
            ))
        })?;
        let linked = metadata.file_type().is_symlink();
        let resolved_entry = if linked {
            fs::canonicalize(&entry).map_err(|error| {
                config_error(format!(
                    "cannot resolve generated input link {}: {error}",
                    entry.display()
                ))
            })?
        } else {
            entry.clone()
        };
        let resolved_metadata = fs::metadata(&resolved_entry).map_err(|error| {
            config_error(format!(
                "cannot inspect generated input {}: {error}",
                entry.display()
            ))
        })?;
        let name = entry
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                config_error("generated input directory contains a non-UTF-8 path".into())
            })?;
        let child_relative = relative.join(name);
        if resolved_metadata.is_dir() {
            if linked {
                return Err(config_error(format!(
                    "generated input directory contains a directory symlink: {}",
                    entry.display()
                )));
            }
            collect_directory_inventory(root, &child_relative, inventory)?;
        } else if resolved_metadata.is_file() {
            if inventory.len() >= MAX_DIRECTORY_FILES {
                return Err(config_error(format!(
                    "generated input directory exceeds the {MAX_DIRECTORY_FILES}-file limit"
                )));
            }
            let relative_name = child_relative
                .to_str()
                .ok_or_else(|| config_error("generated input path is not valid UTF-8".into()))?
                .replace('\\', "/");
            let (sha256, byte_length) = hash_file(&resolved_entry)?;
            inventory.insert(
                relative_name,
                FilePayload {
                    byte_length,
                    sha256,
                },
            );
        }
    }
    Ok(())
}

fn collect_firmware_inputs(
    root: &Path,
    requested: &[String],
) -> Result<Vec<FileIdentity>, crate::commands::CliError> {
    let mut inputs = collect_file_identities(root, requested)?;
    if inputs.iter().any(|input| input.kind != "file") {
        return Err(config_error("firmware inputs must be regular files".into()));
    }

    for (code_key, vars_key) in [
        ("OVMF_CODE", "OVMF_VARS"),
        ("OVMF_CODE_PATH", "OVMF_VARS_PATH"),
    ] {
        let (Some(code), Some(vars)) = (env::var_os(code_key), env::var_os(vars_key)) else {
            continue;
        };
        let code = PathBuf::from(code);
        let vars = PathBuf::from(vars);
        if code.is_file() && vars.is_file() {
            inputs.push(file_identity(root, &code.to_string_lossy())?);
            inputs.push(file_identity(root, &vars.to_string_lossy())?);
            break;
        }
    }

    if inputs.is_empty() {
        if let Some((code, vars)) = discover_firmware_pair() {
            inputs.push(file_identity(root, &code.to_string_lossy())?);
            inputs.push(file_identity(root, &vars.to_string_lossy())?);
        }
    }
    inputs.sort_by(file_identity_order);
    inputs.dedup();
    Ok(inputs)
}

fn discover_firmware_pair() -> Option<(PathBuf, PathBuf)> {
    const FILE_PAIRS: [(&str, &str); 4] = [
        ("OVMF_CODE.fd", "OVMF_VARS.fd"),
        ("OVMF_CODE_4M.fd", "OVMF_VARS_4M.fd"),
        ("edk2-x86_64-code.fd", "edk2-i386-vars.fd"),
        ("edk2-x86_64-code-4m.fd", "edk2-i386-vars-4m.fd"),
    ];
    let mut directories = Vec::new();
    for key in ["OVMF_HOME", "OVMF_PATH"] {
        if let Some(value) = env::var_os(key) {
            directories.push(PathBuf::from(value));
        }
    }
    if let Some(program_files) = env::var_os("ProgramFiles") {
        directories.push(PathBuf::from(&program_files).join("qemu").join("share"));
        directories.push(PathBuf::from(program_files).join("OVMF"));
    }
    directories.extend([
        PathBuf::from("/usr/share/OVMF"),
        PathBuf::from("/usr/share/edk2/ovmf"),
        PathBuf::from("/usr/share/qemu"),
    ]);
    for directory in directories {
        for (code_name, vars_name) in FILE_PAIRS {
            let code = directory.join(code_name);
            let vars = directory.join(vars_name);
            if code.is_file() && vars.is_file() {
                return Some((code, vars));
            }
        }
    }
    None
}

fn file_identity_order(left: &FileIdentity, right: &FileIdentity) -> std::cmp::Ordering {
    (&left.path, &left.kind, left.byte_length, &left.sha256).cmp(&(
        &right.path,
        &right.kind,
        right.byte_length,
        &right.sha256,
    ))
}

fn collect_artifacts(
    root: &Path,
    paths: &[(Option<String>, String)],
) -> Result<Vec<ArtifactRecord>, crate::commands::CliError> {
    let mut artifacts = paths
        .iter()
        .map(|(role, input)| {
            let path = resolve_input_path(root, input);
            let (sha256, byte_length) = hash_regular_file(&path, "artifact")?;
            let path_name = logical_path(root, &path)?;
            let inferred_role = Path::new(&path_name)
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| {
                    config_error(format!("artifact has no UTF-8 filename: {path_name}"))
                })?;
            Ok(ArtifactRecord {
                role: role.clone().unwrap_or_else(|| inferred_role.to_owned()),
                path: path_name,
                byte_length,
                sha256,
            })
        })
        .collect::<Result<Vec<_>, crate::commands::CliError>>()?;
    artifacts.sort_by(|left, right| (&left.role, &left.path).cmp(&(&right.role, &right.path)));
    Ok(artifacts)
}

fn hash_regular_file(path: &Path, label: &str) -> Result<(String, u64), crate::commands::CliError> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        config_error(format!(
            "requested {label} {} is unavailable: {error}",
            path.display()
        ))
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(config_error(format!(
            "requested {label} is not a regular file: {}",
            path.display()
        )));
    }
    hash_file(path)
}

fn hash_file(path: &Path) -> Result<(String, u64), crate::commands::CliError> {
    let mut file = File::open(path)
        .map_err(|error| config_error(format!("cannot open {}: {error}", path.display())))?;
    let mut hasher = Sha256::new();
    let mut length = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| config_error(format!("cannot hash {}: {error}", path.display())))?;
        if count == 0 {
            break;
        }
        hasher.update(&buffer[..count]);
        length += count as u64;
    }
    let digest = hasher.finalize();
    Ok((
        digest.iter().map(|byte| format!("{byte:02x}")).collect(),
        length,
    ))
}

fn capture_environment_inputs(
    root: &Path,
    target_triple: Option<&str>,
) -> Result<BTreeMap<String, String>, crate::commands::CliError> {
    let mut captured = BTreeMap::new();
    let home = env::var("HOME")
        .ok()
        .or_else(|| env::var("USERPROFILE").ok());
    let temporary = env::var("TMPDIR")
        .ok()
        .or_else(|| env::var("TEMP").ok())
        .or_else(|| env::var("TMP").ok());
    let mut variables = BTreeMap::new();
    for (name, value) in env::vars_os() {
        let Some(name) = name.to_str() else {
            continue;
        };
        if !is_relevant_environment_name(name, target_triple) {
            continue;
        }
        let value = value.to_str().ok_or_else(|| {
            config_error(format!(
                "build environment input `{name}` is not valid UTF-8"
            ))
        })?;
        variables.insert(name.to_owned(), value.to_owned());
    }
    for (name, value) in variables {
        if ENVIRONMENT_PATHS_CAPTURED_BY_CONTENT.contains(&name.as_str())
            || environment_name_is_sensitive(&name)
            || has_secret_assignment(&value)
        {
            continue;
        }
        let normalized = normalize_host_paths(root, &value, home.as_deref(), temporary.as_deref());
        captured.insert(name, sha256_hex(normalized.as_bytes()));
    }
    Ok(captured)
}

fn is_relevant_environment_name(name: &str, target_triple: Option<&str>) -> bool {
    if ENVIRONMENT_ALLOWLIST.contains(&name) {
        return true;
    }
    if name.starts_with("CARGO_PROFILE_") {
        return true;
    }
    let Some(target_triple) = target_triple else {
        return false;
    };
    let target = target_triple
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    let prefix = format!("CARGO_TARGET_{target}_");
    let Some(suffix) = name.strip_prefix(&prefix) else {
        return false;
    };
    matches!(suffix, "LINKER" | "RUNNER" | "RUSTFLAGS" | "RUSTDOCFLAGS")
}

fn environment_name_is_sensitive(name: &str) -> bool {
    let upper = name.to_ascii_uppercase();
    [
        "TOKEN",
        "API_KEY",
        "KEY",
        "PASSWORD",
        "PASSWD",
        "SECRET",
        "CREDENTIAL",
        "PRIVATE_KEY",
        "COOKIE",
        "AUTHORIZATION",
    ]
    .iter()
    .any(|marker| upper.contains(marker))
}

#[cfg(test)]
fn capture_environment_inputs_from(
    variables: &[(&str, &str)],
    repository_root: &str,
    home: Option<&str>,
    temporary: Option<&str>,
    target_triple: Option<&str>,
) -> BTreeMap<String, String> {
    let mut captured = BTreeMap::new();
    for (name, value) in variables {
        if !is_relevant_environment_name(name, target_triple)
            || ENVIRONMENT_PATHS_CAPTURED_BY_CONTENT.contains(name)
            || environment_name_is_sensitive(name)
            || has_secret_assignment(value)
        {
            continue;
        }
        let normalized = normalize_text_paths(value, repository_root, home, temporary);
        captured.insert((*name).to_owned(), sha256_hex(normalized.as_bytes()));
    }
    captured
}

fn environment_generated_inputs(
    root: &Path,
    target_triple: Option<&str>,
) -> Result<Vec<FileIdentity>, crate::commands::CliError> {
    let mut inputs = Vec::new();
    let mut commands = BTreeMap::new();
    for name in [
        "NAGI_CXX_HEADERS",
        "NAGI_RELIBC_HEADERS",
        "NAGI_M16_PACKAGE",
        "__CARGO_TESTS_ONLY_SRC_ROOT",
    ] {
        if let Some(path) = env::var_os(name) {
            let path = PathBuf::from(path);
            inputs.push(file_identity(root, &path.to_string_lossy())?);
        }
    }
    if let Some(target_triple) = target_triple {
        let target = target_triple
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() {
                    character.to_ascii_uppercase()
                } else {
                    '_'
                }
            })
            .collect::<String>();
        let prefix = format!("CARGO_TARGET_{target}_");
        for suffix in ["LINKER", "RUNNER"] {
            let name = format!("{prefix}{suffix}");
            if let Ok(value) = env::var(&name) {
                commands.insert(format!("cargo_{}", name.to_ascii_lowercase()), value);
            }
        }
    }

    if env::var_os("NAGI_RELIBC_HEADERS").is_none() {
        if let Some(mesa_build) = env::var_os("NAGI_MESA_BUILD") {
            let mesa_build = PathBuf::from(mesa_build);
            let target =
                env::var("NAGI_TARGET").unwrap_or_else(|_| "x86_64-unknown-nagi-user".into());
            let mesa_root = if mesa_build
                .file_name()
                .is_some_and(|name| name == "mesa-build")
            {
                mesa_build.parent().unwrap_or(&mesa_build).to_path_buf()
            } else {
                mesa_build
            };
            let headers = mesa_root
                .join("relibc-target")
                .join(validated_target(&target)?)
                .join("include");
            inputs.push(file_identity(root, &headers.to_string_lossy())?);
        }
    }

    commands.insert(
        "nagi_target_clang".to_owned(),
        env::var("NAGI_TARGET_CLANG").unwrap_or_else(|_| "clang".into()),
    );
    commands.insert(
        "nagi_qemu".to_owned(),
        env::var("NAGI_QEMU").unwrap_or_else(|_| "qemu-system-x86_64".into()),
    );
    for name in [
        "AR",
        "CC",
        "CXX",
        "LD",
        "RUSTC_WRAPPER",
        "CC_x86_64_unknown_nagi_user",
        "CXX_x86_64_unknown_nagi_user",
    ] {
        if let Ok(command) = env::var(name) {
            commands.insert(name.to_ascii_lowercase(), command);
        }
    }
    for (label, command) in commands {
        if let Some(path) = resolve_executable(root, &command) {
            let canonical = fs::canonicalize(&path).unwrap_or(path);
            let (sha256, byte_length) = hash_file(&canonical)?;
            inputs.push(FileIdentity {
                kind: "tool".into(),
                path: format!("toolchain/{label}"),
                byte_length,
                sha256,
            });
        }
    }
    inputs.sort_by(file_identity_order);
    inputs.dedup();
    Ok(inputs)
}

fn resolve_executable(root: &Path, command: &str) -> Option<PathBuf> {
    let program = command.split_whitespace().next()?;
    let candidate = Path::new(program);
    if candidate.is_absolute() || program.contains('/') || program.contains('\\') {
        let candidate = if candidate.is_absolute() {
            candidate.to_path_buf()
        } else {
            root.join(candidate)
        };
        return candidate.is_file().then_some(candidate);
    }
    let search_path = env::var_os("PATH")?;
    for directory in env::split_paths(&search_path) {
        let candidate = directory.join(program);
        if candidate.is_file() {
            return Some(candidate);
        }
        #[cfg(windows)]
        for extension in ["exe", "cmd", "bat"] {
            let candidate = directory.join(format!("{program}.{extension}"));
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

fn has_secret_assignment(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    [
        "api_key=",
        "api-key=",
        "token=",
        "password=",
        "passwd=",
        "secret=",
        "credential=",
        "authorization=",
        "cookie=",
        "private_key=",
        "private-key=",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

fn reject_secret_material(value: &str, label: &str) -> Result<(), crate::commands::CliError> {
    if has_secret_assignment(value) {
        Err(usage_error(format!(
            "{label} contains a secret-like assignment"
        )))
    } else {
        Ok(())
    }
}

fn normalize_host_paths(
    root: &Path,
    value: &str,
    home: Option<&str>,
    temporary: Option<&str>,
) -> String {
    let root = root.to_string_lossy();
    normalize_text_paths(value, &root, home, temporary)
}

fn normalize_text_paths(
    value: &str,
    repository_root: &str,
    home: Option<&str>,
    temporary: Option<&str>,
) -> String {
    let mut normalized = value.to_owned();
    for (path, replacement) in [
        (Some(repository_root), "${REPO}"),
        (home, "${HOME}"),
        (temporary, "${TEMP}"),
    ] {
        if let Some(path) = path.filter(|path| !path.is_empty()) {
            normalized = normalized.replace(path, replacement);
        }
    }
    normalized
}

fn cargo_config_target(root: &Path) -> Option<String> {
    let config = fs::read_to_string(root.join(".cargo/config.toml")).ok()?;
    let mut in_build = false;
    for line in config.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.starts_with('[') && line.ends_with(']') {
            in_build = line == "[build]";
            continue;
        }
        if in_build {
            if let Some((key, value)) = line.split_once('=') {
                if key.trim() == "target" {
                    return Some(unquote_toml_string(value.trim()));
                }
            }
        }
    }
    None
}

fn qemu_version(
    root: &Path,
    requested_path: Option<&str>,
) -> Result<Option<String>, crate::commands::CliError> {
    let program = requested_path.unwrap_or("qemu-system-x86_64");
    match command_output(Path::new(program), &["--version"], root) {
        Some(output) if output.status.success() => {
            let line = String::from_utf8_lossy(&output.stdout)
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_owned();
            Ok((!line.is_empty()).then_some(line))
        }
        Some(output) if requested_path.is_some() => Err(config_error(format!(
            "requested QEMU command failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))),
        Some(_) | None if requested_path.is_none() => Ok(None),
        Some(output) => Err(config_error(format!(
            "requested QEMU command failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))),
        None => Err(config_error(format!(
            "cannot run requested QEMU command `{program}`"
        ))),
    }
}

fn command_output(program: &Path, args: &[&str], root: &Path) -> Option<std::process::Output> {
    Command::new(program)
        .args(args)
        .current_dir(root)
        .output()
        .ok()
}

fn read_fingerprint(
    root: &Path,
    input: &str,
) -> Result<FingerprintDocument, crate::commands::CliError> {
    let path = resolve_input_path(root, input);
    let bytes = fs::read(&path).map_err(|error| {
        config_error(format!(
            "cannot read fingerprint {}: {error}",
            path.display()
        ))
    })?;
    let document: FingerprintDocument = serde_json::from_slice(&bytes).map_err(|error| {
        config_error(format!("invalid fingerprint {}: {error}", path.display()))
    })?;
    validate_fingerprint(&document)?;
    Ok(document)
}

fn validate_fingerprint(document: &FingerprintDocument) -> Result<(), crate::commands::CliError> {
    if document.schema_version != FINGERPRINT_SCHEMA_VERSION {
        return Err(config_error(format!(
            "unsupported fingerprint schema version {}; expected {FINGERPRINT_SCHEMA_VERSION}",
            document.schema_version
        )));
    }
    if document.compatibility.digest_algorithm != "sha256" {
        return Err(config_error(format!(
            "unsupported compatibility digest algorithm `{}`",
            document.compatibility.digest_algorithm
        )));
    }
    let inputs = &document.compatibility.inputs;
    if inputs.repository_id.trim().is_empty()
        || inputs.source.commit_sha.len() != 40
        || !inputs
            .source
            .commit_sha
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        || inputs.source.dirty != inputs.source.dirty_tree_digest.is_some()
    {
        return Err(config_error(
            "fingerprint contains an invalid repository/source identity".into(),
        ));
    }
    if inputs.toolchain.cargo.as_deref().is_none_or(str::is_empty)
        || inputs
            .toolchain
            .rustc
            .get("version")
            .is_none_or(String::is_empty)
        || inputs
            .toolchain
            .host_triple
            .as_deref()
            .is_none_or(str::is_empty)
    {
        return Err(config_error(
            "fingerprint is missing Rust/Cargo toolchain identity".into(),
        ));
    }
    if !is_sha256(&inputs.nagi_patch_digest)
        || !is_sha256(&inputs.build_configuration_digest)
        || inputs
            .build_command_digest
            .as_deref()
            .is_some_and(|digest| !is_sha256(digest))
        || inputs
            .source
            .dirty_tree_digest
            .as_deref()
            .is_some_and(|digest| !is_sha256(digest))
        || inputs
            .environment_inputs
            .values()
            .any(|digest| !is_sha256(digest))
        || inputs.build_flags.iter().any(|digest| !is_sha256(digest))
    {
        return Err(config_error(
            "fingerprint contains an invalid compatibility digest".into(),
        ));
    }
    if !is_sorted_by(&inputs.enabled_features, |left, right| left.cmp(right))
        || !is_sorted_by(&inputs.generated_inputs, file_identity_order)
        || !is_sorted_by(&inputs.runtime_harness.firmware_inputs, file_identity_order)
        || !is_sorted_by(&document.artifacts, |left, right| {
            (&left.role, &left.path).cmp(&(&right.role, &right.path))
        })
    {
        return Err(config_error(
            "fingerprint arrays are not in canonical order".into(),
        ));
    }
    let actual = digest_serializable(&document.compatibility.inputs)?;
    if document.compatibility.digest != actual {
        return Err(config_error(
            "fingerprint compatibility digest does not match its normalized inputs".into(),
        ));
    }
    for artifact in &document.artifacts {
        if !is_sha256(&artifact.sha256) {
            return Err(config_error(format!(
                "artifact `{}` has an invalid SHA-256 digest",
                artifact.role
            )));
        }
    }
    for input in inputs
        .generated_inputs
        .iter()
        .chain(inputs.runtime_harness.firmware_inputs.iter())
    {
        if !matches!(input.kind.as_str(), "file" | "directory" | "tool")
            || !is_sha256(&input.sha256)
        {
            return Err(config_error(format!(
                "input `{}` has an invalid kind or SHA-256 digest",
                input.path
            )));
        }
    }
    if inputs
        .runtime_harness
        .firmware_inputs
        .iter()
        .any(|input| input.kind != "file")
    {
        return Err(config_error(
            "firmware inventory contains a non-file input".into(),
        ));
    }
    Ok(())
}

fn is_sorted_by<T, F>(values: &[T], mut compare: F) -> bool
where
    F: FnMut(&T, &T) -> std::cmp::Ordering,
{
    values
        .windows(2)
        .all(|pair| compare(&pair[0], &pair[1]).is_le())
}

fn compare_fingerprints(
    left: &FingerprintDocument,
    right: &FingerprintDocument,
) -> ComparisonReport {
    let mut mismatches = Vec::new();
    let left_inputs = &left.compatibility.inputs;
    let right_inputs = &right.compatibility.inputs;
    add_mismatch(
        &mut mismatches,
        "source_revision",
        "repository_id",
        &left_inputs.repository_id,
        &right_inputs.repository_id,
    );
    add_mismatch(
        &mut mismatches,
        "source_revision",
        "source.commit_sha",
        &left_inputs.source.commit_sha,
        &right_inputs.source.commit_sha,
    );
    add_mismatch(
        &mut mismatches,
        "dirty_source_state",
        "source.dirty",
        &left_inputs.source.dirty,
        &right_inputs.source.dirty,
    );
    add_mismatch(
        &mut mismatches,
        "dirty_source_state",
        "source.dirty_tree_digest",
        &left_inputs.source.dirty_tree_digest,
        &right_inputs.source.dirty_tree_digest,
    );
    add_mismatch(
        &mut mismatches,
        "toolchain",
        "toolchain",
        &left_inputs.toolchain,
        &right_inputs.toolchain,
    );
    add_mismatch(
        &mut mismatches,
        "target",
        "target_triple",
        &left_inputs.target_triple,
        &right_inputs.target_triple,
    );
    add_mismatch(
        &mut mismatches,
        "features",
        "enabled_features",
        &left_inputs.enabled_features,
        &right_inputs.enabled_features,
    );
    add_mismatch(
        &mut mismatches,
        "build_profile",
        "build_profile",
        &left_inputs.build_profile,
        &right_inputs.build_profile,
    );
    add_mismatch(
        &mut mismatches,
        "build_command",
        "build_command_digest",
        &left_inputs.build_command_digest,
        &right_inputs.build_command_digest,
    );
    add_mismatch(
        &mut mismatches,
        "build_flags",
        "build_flags",
        &left_inputs.build_flags,
        &right_inputs.build_flags,
    );
    add_mismatch(
        &mut mismatches,
        "environment",
        "environment_inputs",
        &left_inputs.environment_inputs,
        &right_inputs.environment_inputs,
    );
    add_mismatch(
        &mut mismatches,
        "third_party_source_or_patch",
        "third_party_revisions",
        &left_inputs.third_party_revisions,
        &right_inputs.third_party_revisions,
    );
    add_mismatch(
        &mut mismatches,
        "third_party_source_or_patch",
        "submodule_revisions",
        &left_inputs.submodule_revisions,
        &right_inputs.submodule_revisions,
    );
    add_mismatch(
        &mut mismatches,
        "third_party_source_or_patch",
        "nagi_patch_digest",
        &left_inputs.nagi_patch_digest,
        &right_inputs.nagi_patch_digest,
    );
    add_mismatch(
        &mut mismatches,
        "build_configuration",
        "build_configuration_digest",
        &left_inputs.build_configuration_digest,
        &right_inputs.build_configuration_digest,
    );
    add_mismatch(
        &mut mismatches,
        "generated_inputs",
        "generated_inputs",
        &left_inputs.generated_inputs,
        &right_inputs.generated_inputs,
    );
    add_mismatch(
        &mut mismatches,
        "firmware_runtime_harness",
        "runtime_harness",
        &left_inputs.runtime_harness,
        &right_inputs.runtime_harness,
    );
    let artifact_inventories_match = left.artifacts == right.artifacts;
    if !artifact_inventories_match {
        mismatches.push(Mismatch {
            class: "artifact_digest".into(),
            field: "artifacts".into(),
            left: json!(left.artifacts),
            right: json!(right.artifacts),
        });
    }
    let inputs_compatible = left.compatibility.digest == right.compatibility.digest;
    ComparisonReport {
        schema_version: FINGERPRINT_SCHEMA_VERSION,
        left_compatibility_digest: left.compatibility.digest.clone(),
        right_compatibility_digest: right.compatibility.digest.clone(),
        inputs_compatible,
        artifact_inventories_match,
        match_: inputs_compatible && artifact_inventories_match,
        mismatches,
    }
}

fn add_mismatch<T: Serialize + PartialEq>(
    mismatches: &mut Vec<Mismatch>,
    class: &str,
    field: &str,
    left: &T,
    right: &T,
) {
    if left != right {
        mismatches.push(Mismatch {
            class: class.into(),
            field: field.into(),
            left: serde_json::to_value(left).unwrap_or(Value::Null),
            right: serde_json::to_value(right).unwrap_or(Value::Null),
        });
    }
}

fn format_comparison(report: &ComparisonReport) -> String {
    let status = if report.match_ { "MATCH" } else { "MISMATCH" };
    let mut lines = vec![
        format!("Fingerprint comparison: {status}"),
        format!(
            "Input compatibility: {}",
            if report.inputs_compatible {
                "MATCH"
            } else {
                "MISMATCH"
            }
        ),
        format!(
            "Artifact inventory: {}",
            if report.artifact_inventories_match {
                "MATCH"
            } else {
                "MISMATCH"
            }
        ),
    ];
    lines.extend(report.mismatches.iter().map(|mismatch| {
        format!(
            "Mismatch [{}] {}: {} != {}",
            mismatch.class, mismatch.field, mismatch.left, mismatch.right
        )
    }));
    lines.join("\n")
}

fn serialize_document(
    document: &FingerprintDocument,
) -> Result<Vec<u8>, crate::commands::CliError> {
    let mut bytes = serde_json::to_vec_pretty(document)
        .map_err(|error| config_error(format!("cannot serialize fingerprint: {error}")))?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<(), crate::commands::CliError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            config_error(format!(
                "cannot create fingerprint {}: {error}",
                path.display()
            ))
        })?;
    if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        drop(file);
        let cleanup = fs::remove_file(path).err();
        let detail = cleanup
            .map(|error| format!("; cannot remove partial output: {error}"))
            .unwrap_or_default();
        return Err(config_error(format!(
            "cannot write fingerprint {}: {error}{detail}",
            path.display()
        )));
    }
    Ok(())
}

fn display_output_path(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn resolve_input_path(root: &Path, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

fn logical_path(root: &Path, path: &Path) -> Result<String, crate::commands::CliError> {
    let canonical_root = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let canonical_path = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if let Ok(relative) = canonical_path.strip_prefix(&canonical_root) {
        let value = relative
            .to_str()
            .ok_or_else(|| config_error("input path is not valid UTF-8".into()))?;
        return Ok(value.replace('\\', "/"));
    }
    let name = canonical_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| config_error("external input path must have a UTF-8 filename".into()))?;
    Ok(format!("external/{name}"))
}

fn relative_path(root: &Path, path: &Path) -> Result<String, crate::commands::CliError> {
    let relative = path.strip_prefix(root).map_err(|_| {
        config_error(format!(
            "patch path escaped the repository: {}",
            path.display()
        ))
    })?;
    relative
        .to_str()
        .map(|value| value.replace('\\', "/"))
        .ok_or_else(|| config_error("patch path is not valid UTF-8".into()))
}

fn git_output(root: &Path, args: &[&str]) -> Result<String, crate::commands::CliError> {
    let bytes = git_bytes(root, args)?;
    String::from_utf8(bytes)
        .map(|output| output.trim().to_owned())
        .map_err(|error| {
            config_error(format!(
                "git {} returned non-UTF-8 output: {error}",
                args.join(" ")
            ))
        })
}

fn git_bytes(root: &Path, args: &[&str]) -> Result<Vec<u8>, crate::commands::CliError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|error| config_error(format!("cannot run git {}: {error}", args.join(" "))))?;
    if !output.status.success() {
        return Err(config_error(format!(
            "git {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(output.stdout)
}

fn unquote_toml_string(value: &str) -> String {
    let value = value.split('#').next().unwrap_or("").trim();
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(value)
        .to_owned()
}

fn digest_serializable<T: Serialize>(value: &T) -> Result<String, crate::commands::CliError> {
    let bytes = serde_json::to_vec(value).map_err(|error| {
        config_error(format!("cannot canonicalize fingerprint inputs: {error}"))
    })?;
    Ok(sha256_hex(&bytes))
}

fn digest_normalized_text(root: &Path, value: &str) -> String {
    let home = env::var("HOME")
        .ok()
        .or_else(|| env::var("USERPROFILE").ok());
    let temporary = env::var("TMPDIR").ok().or_else(|| env::var("TEMP").ok());
    let normalized = normalize_host_paths(root, value, home.as_deref(), temporary.as_deref());
    sha256_hex(normalized.as_bytes())
}

fn digest_bytes(bytes: impl AsRef<[u8]>) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn sha256_hex(bytes: &[u8]) -> String {
    digest_bytes(bytes)
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn usage_error(message: String) -> crate::commands::CliError {
    crate::commands::CliError::new(
        format!("{message}\nusage: nagi dev fingerprint [generation options] | compare <left.json> <right.json> [--json]"),
        EXIT_USAGE,
    )
}

fn config_error(message: String) -> crate::commands::CliError {
    crate::commands::CliError::new(message, EXIT_CONFIG_ERROR)
}

fn success(lines: Vec<String>) -> CommandResult {
    CommandResult {
        exit_code: EXIT_SUCCESS,
        lines,
    }
}

fn failure(code: i32, message: impl Into<String>) -> CommandResult {
    CommandResult {
        exit_code: code,
        lines: vec![format!("FAIL {}", message.into())],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory(PathBuf);

    impl TestDirectory {
        fn new(label: &str) -> Self {
            loop {
                let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
                let path = env::temp_dir().join(format!(
                    "nagi-fingerprint-{label}-{}-{sequence}",
                    std::process::id()
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("create test directory {}: {error}", path.display()),
                }
            }
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn repository_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn sample_inputs() -> CompatibilityInputs {
        let mut rustc = BTreeMap::new();
        rustc.insert("version".into(), "rustc 1.90.0-nightly".into());
        rustc.insert("release".into(), "1.90.0-nightly".into());
        rustc.insert("host".into(), "aarch64-apple-darwin".into());
        CompatibilityInputs {
            repository_id: "github.com/example/nagi".into(),
            source: SourceIdentity {
                commit_sha: "a".repeat(40),
                dirty: false,
                dirty_tree_digest: None,
            },
            toolchain: ToolchainIdentity {
                rustc,
                cargo: Some("cargo 1.90.0-nightly".into()),
                host_triple: Some("aarch64-apple-darwin".into()),
                native_tools: BTreeMap::from([("nagi_target_clang".into(), "clang 19.0.0".into())]),
            },
            target_triple: Some("x86_64-unknown-nagi".into()),
            enabled_features: vec!["alloc".into(), "servo".into()],
            build_profile: Some("release".into()),
            build_command_digest: Some(sha256_hex(b"cargo build --release")),
            build_flags: vec![sha256_hex(b"-C opt-level=3")],
            environment_inputs: BTreeMap::from([("RUSTFLAGS".into(), sha256_hex(b"-C lto"))]),
            third_party_revisions: BTreeMap::from([("servo.revision".into(), "b".repeat(40))]),
            submodule_revisions: BTreeMap::new(),
            nagi_patch_digest: sha256_hex(b"patch inventory"),
            build_configuration_digest: sha256_hex(b"configuration inventory"),
            generated_inputs: vec![FileIdentity {
                kind: "file".into(),
                path: "out/generated/target.json".into(),
                byte_length: 12,
                sha256: sha256_hex(b"target input"),
            }],
            runtime_harness: RuntimeHarness {
                qemu_version: Some("QEMU emulator version 9.0.0".into()),
                firmware_inputs: vec![FileIdentity {
                    kind: "file".into(),
                    path: "out/firmware/OVMF_CODE.fd".into(),
                    byte_length: 10,
                    sha256: sha256_hex(b"firmware"),
                }],
            },
        }
    }

    fn seal(inputs: CompatibilityInputs, timestamp: u64) -> FingerprintDocument {
        FingerprintDocument {
            schema_version: FINGERPRINT_SCHEMA_VERSION,
            compatibility: CompatibilityFingerprint {
                digest_algorithm: "sha256".into(),
                digest: digest_serializable(&inputs).expect("digest inputs"),
                inputs,
            },
            artifacts: vec![ArtifactRecord {
                role: "nagi-image".into(),
                path: "out/nagi.img".into(),
                byte_length: 17,
                sha256: sha256_hex(b"target artifact"),
            }],
            provenance: Provenance {
                created_at_unix_seconds: timestamp,
                producer: PRODUCER_NAME.into(),
                producer_version: "0.1.0".into(),
            },
        }
    }

    fn sample_document() -> FingerprintDocument {
        seal(sample_inputs(), 1_790_000_000)
    }

    fn mismatch_classes(report: &ComparisonReport) -> Vec<&str> {
        report
            .mismatches
            .iter()
            .map(|mismatch| mismatch.class.as_str())
            .collect()
    }

    #[test]
    fn canonical_inputs_produce_a_deterministic_sha256_digest() {
        let first = sample_inputs();
        let second = sample_inputs();
        assert_eq!(
            digest_serializable(&first).unwrap(),
            digest_serializable(&second).unwrap()
        );
        assert!(is_sha256(&digest_serializable(&first).unwrap()));
    }

    #[test]
    fn creation_timestamp_is_provenance_and_does_not_change_compatibility() {
        let left = seal(sample_inputs(), 1_700_000_000);
        let right = seal(sample_inputs(), 1_800_000_000);
        assert_eq!(left.compatibility.digest, right.compatibility.digest);
        assert!(compare_fingerprints(&left, &right).match_);
    }

    #[test]
    fn repository_absolute_paths_normalize_to_the_same_logical_path() {
        let first_root = Path::new("/tmp/worktree-one/NagiOS");
        let second_root = Path::new("/Users/example/worktree-two/NagiOS");
        let first = logical_path(first_root, &first_root.join("out/nagi.img")).unwrap();
        let second = logical_path(second_root, &second_root.join("out/nagi.img")).unwrap();
        assert_eq!(first, "out/nagi.img");
        assert_eq!(first, second);
    }

    #[test]
    fn source_revision_change_is_classified() {
        let left = sample_document();
        let mut inputs = sample_inputs();
        inputs.source.commit_sha = "c".repeat(40);
        let right = seal(inputs, 1_790_000_001);
        let report = compare_fingerprints(&left, &right);
        assert!(!report.inputs_compatible);
        assert_eq!(mismatch_classes(&report), vec!["source_revision"]);
    }

    #[test]
    fn feature_target_and_toolchain_changes_have_specific_classes() {
        let left = sample_document();

        let mut inputs = sample_inputs();
        inputs.enabled_features.push("network".into());
        inputs.enabled_features.sort();
        let features = compare_fingerprints(&left, &seal(inputs, 2));
        assert_eq!(mismatch_classes(&features), vec!["features"]);

        let mut inputs = sample_inputs();
        inputs.target_triple = Some("aarch64-unknown-nagi".into());
        let target = compare_fingerprints(&left, &seal(inputs, 2));
        assert_eq!(mismatch_classes(&target), vec!["target"]);

        let mut inputs = sample_inputs();
        inputs.toolchain.cargo = Some("cargo 1.91.0-nightly".into());
        let toolchain = compare_fingerprints(&left, &seal(inputs, 2));
        assert_eq!(mismatch_classes(&toolchain), vec!["toolchain"]);
    }

    #[test]
    fn build_command_change_is_part_of_compatibility() {
        let left = sample_document();
        let mut inputs = sample_inputs();
        inputs.build_command_digest = Some(sha256_hex(b"cargo build --features other"));
        let report = compare_fingerprints(&left, &seal(inputs, 2));
        assert_eq!(mismatch_classes(&report), vec!["build_command"]);
    }

    #[test]
    fn patch_and_generated_input_changes_are_reported_separately() {
        let left = sample_document();
        let mut inputs = sample_inputs();
        inputs.nagi_patch_digest = sha256_hex(b"different patches");
        inputs.generated_inputs[0].sha256 = sha256_hex(b"different generated target");
        let report = compare_fingerprints(&left, &seal(inputs, 2));
        assert_eq!(
            mismatch_classes(&report),
            vec!["third_party_source_or_patch", "generated_inputs"]
        );
    }

    #[test]
    fn artifact_content_change_is_a_distinct_mismatch() {
        let left = sample_document();
        let mut right = sample_document();
        right.artifacts[0].sha256 = sha256_hex(b"changed artifact");
        let report = compare_fingerprints(&left, &right);
        assert!(report.inputs_compatible);
        assert!(!report.artifact_inventories_match);
        assert_eq!(mismatch_classes(&report), vec!["artifact_digest"]);
    }

    #[test]
    fn mismatch_classes_and_order_are_stable() {
        let left = sample_document();
        let mut inputs = sample_inputs();
        inputs.source.dirty = true;
        inputs.source.dirty_tree_digest = Some(sha256_hex(b"dirty"));
        inputs.target_triple = Some("aarch64-unknown-nagi".into());
        inputs.enabled_features = vec!["alloc".into()];
        let right = seal(inputs, 2);
        let first = compare_fingerprints(&left, &right);
        let second = compare_fingerprints(&left, &right);
        assert_eq!(first, second);
        assert_eq!(
            mismatch_classes(&first),
            vec![
                "dirty_source_state",
                "dirty_source_state",
                "target",
                "features"
            ]
        );
    }

    #[test]
    fn secret_like_environment_values_are_never_emitted() {
        let captured = capture_environment_inputs_from(
            &[
                ("RUSTFLAGS", "-C opt-level=2"),
                ("CFLAGS", "-DAPI_KEY=very-private-value"),
                ("NAGI_API_TOKEN", "also-private-value"),
                ("CPPFLAGS", "-I /worktree-a/repo/include -I /tmp/build-a"),
            ],
            "/worktree-a/repo",
            Some("/Users/alice"),
            Some("/tmp/build-a"),
            None,
        );
        let serialized = serde_json::to_string(&captured).unwrap();
        assert!(captured.contains_key("RUSTFLAGS"));
        assert!(captured.contains_key("CPPFLAGS"));
        assert!(!captured.contains_key("CFLAGS"));
        assert!(!captured.contains_key("NAGI_API_TOKEN"));
        assert!(!captured.contains_key("NAGI_CXX_HEADERS"));
        assert!(!serialized.contains("very-private-value"));
        assert!(!serialized.contains("also-private-value"));
        assert!(!serialized.contains("/worktree-a"));
        assert!(!serialized.contains("/tmp/build-a"));
    }

    #[test]
    fn normalized_environment_flags_ignore_repository_and_temp_roots() {
        let first = capture_environment_inputs_from(
            &[(
                "RUSTFLAGS",
                "-L /worktree-one/repo/out/lib -I /tmp/one/include",
            )],
            "/worktree-one/repo",
            None,
            Some("/tmp/one"),
            None,
        );
        let second = capture_environment_inputs_from(
            &[(
                "RUSTFLAGS",
                "-L /Users/alice/repo/out/lib -I /tmp/two/include",
            )],
            "/Users/alice/repo",
            None,
            Some("/tmp/two"),
            None,
        );
        assert_eq!(first, second);
    }

    #[test]
    fn cargo_profile_and_selected_target_environment_are_captured_selectively() {
        let first = capture_environment_inputs_from(
            &[
                ("CARGO_PROFILE_RELEASE_LTO", "thin"),
                (
                    "CARGO_TARGET_X86_64_UNKNOWN_NAGI_USER_LINKER",
                    "/worktree-one/repo/tools/nagi-linker --target x86_64",
                ),
                (
                    "CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER",
                    "different-unselected-linker",
                ),
                ("CARGO_PROFILE_RELEASE_SIGNING_KEY", "must-not-capture"),
            ],
            "/worktree-one/repo",
            None,
            None,
            Some("x86_64-unknown-nagi-user"),
        );
        let second = capture_environment_inputs_from(
            &[
                ("CARGO_PROFILE_RELEASE_LTO", "thin"),
                (
                    "CARGO_TARGET_X86_64_UNKNOWN_NAGI_USER_LINKER",
                    "/Users/alice/repo/tools/nagi-linker --target x86_64",
                ),
            ],
            "/Users/alice/repo",
            Some("/Users/alice"),
            None,
            Some("x86_64-unknown-nagi-user"),
        );
        assert_eq!(first, second);
        assert!(first.contains_key("CARGO_PROFILE_RELEASE_LTO"));
        assert!(first.contains_key("CARGO_TARGET_X86_64_UNKNOWN_NAGI_USER_LINKER"));
        assert!(!first.contains_key("CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER"));
        assert!(!first.contains_key("CARGO_PROFILE_RELEASE_SIGNING_KEY"));
    }

    #[test]
    fn custom_target_json_reference_is_recorded_as_a_stable_target_name() {
        let root = repository_root();
        let target =
            target_triple_from_reference(&root, "targets/x86_64-unknown-nagi-user.json").unwrap();
        assert_eq!(target, "x86_64-unknown-nagi-user");
        let files =
            configuration_files(&root, Some("targets/x86_64-unknown-nagi-user.json")).unwrap();
        assert!(files
            .iter()
            .any(|file| file.path == "targets/x86_64-unknown-nagi-user.json"));
    }

    #[test]
    fn fingerprint_schema_version_and_digest_are_validated() {
        let mut document = sample_document();
        validate_fingerprint(&document).unwrap();
        document.schema_version = 2;
        assert!(validate_fingerprint(&document)
            .unwrap_err()
            .to_string()
            .contains("unsupported fingerprint schema version"));

        let mut document = sample_document();
        document.compatibility.digest = "0".repeat(64);
        assert!(validate_fingerprint(&document)
            .unwrap_err()
            .to_string()
            .contains("does not match"));
    }

    #[test]
    fn requested_missing_artifact_fails_closed() {
        let missing = env::temp_dir().join(format!(
            "nagi-fingerprint-missing-{}-{}.img",
            std::process::id(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        ));
        let error = hash_regular_file(&missing, "artifact").unwrap_err();
        assert!(error.to_string().contains("requested artifact"));
    }

    #[test]
    fn artifact_manifest_records_size_and_sha256() {
        let temp = TestDirectory::new("artifact-hash");
        let path = temp.path().join("nagi.img");
        fs::write(&path, b"image-bytes").unwrap();
        let (digest, size) = hash_regular_file(&path, "artifact").unwrap();
        assert_eq!(size, 11);
        assert_eq!(digest, sha256_hex(b"image-bytes"));
    }

    #[test]
    fn generated_directory_digest_tracks_file_names_and_contents() {
        let first_root = TestDirectory::new("input-tree-one");
        let second_root = TestDirectory::new("input-tree-two");
        let first = first_root.path().join("generated");
        let second = second_root.path().join("generated");
        fs::create_dir(&first).unwrap();
        fs::create_dir(&second).unwrap();
        fs::write(first.join("manifest.json"), b"{}\n").unwrap();
        fs::write(second.join("manifest.json"), b"{}\n").unwrap();
        let repo = repository_root();
        let first_identity = file_identity(&repo, &first.to_string_lossy()).unwrap();
        let second_identity = file_identity(&repo, &second.to_string_lossy()).unwrap();
        assert_eq!(first_identity, second_identity);
        assert_eq!(first_identity.kind, "directory");

        fs::write(second.join("manifest.json"), b"{\"changed\":true}\n").unwrap();
        let changed = file_identity(&repo, &second.to_string_lossy()).unwrap();
        assert_ne!(first_identity.sha256, changed.sha256);
    }

    #[test]
    fn repository_identifier_strips_remote_credentials_and_host_paths() {
        assert_eq!(
            sanitize_repository_identifier(
                "https://user:private@github.com/org/nagi.git?token=secret"
            ),
            "github.com/org/nagi"
        );
        assert_eq!(
            sanitize_repository_identifier("git@github.com:org/nagi.git"),
            "github.com/org/nagi"
        );
        assert_eq!(
            sanitize_repository_identifier("file:///Users/alice/NagiOS"),
            "nagi-os"
        );
    }

    #[test]
    fn fingerprint_compare_json_reports_mismatch_and_returns_nonzero() {
        let temp = TestDirectory::new("compare");
        let left = sample_document();
        let mut right_inputs = sample_inputs();
        right_inputs.source.commit_sha = "d".repeat(40);
        let right = seal(right_inputs, 2);
        let left_path = temp.path().join("left.json");
        let right_path = temp.path().join("right.json");
        fs::write(&left_path, serialize_document(&left).unwrap()).unwrap();
        fs::write(&right_path, serialize_document(&right).unwrap()).unwrap();
        let result = execute(
            &[
                "compare".into(),
                left_path.to_string_lossy().into_owned(),
                right_path.to_string_lossy().into_owned(),
                "--json".into(),
            ],
            &repository_root(),
        );
        assert_eq!(result.exit_code, EXIT_CONFIG_ERROR);
        let report: Value = serde_json::from_str(&result.lines[0]).unwrap();
        assert_eq!(report["match"], false);
        assert_eq!(report["mismatches"][0]["class"], "source_revision");
    }

    #[test]
    fn fingerprint_output_does_not_overwrite_existing_evidence() {
        let temp = TestDirectory::new("immutable-output");
        let output = temp.path().join("fingerprint.json");
        fs::write(&output, b"existing evidence").unwrap();
        let error = write_new_file(&output, b"replacement").unwrap_err();
        assert!(error.to_string().contains("cannot create fingerprint"));
        assert_eq!(fs::read(&output).unwrap(), b"existing evidence");
    }

    #[test]
    fn command_parser_accepts_repeated_inputs_and_deduplicates_features() {
        let args = [
            "--feature",
            "servo",
            "--feature",
            "alloc",
            "--feature",
            "servo",
            "--artifact",
            "kernel=out/kernel.elf",
            "--input",
            "out/generated.json",
        ]
        .map(str::to_owned);
        let options = parse_options(&args).unwrap();
        assert_eq!(options.features, vec!["alloc", "servo"]);
        assert_eq!(options.artifacts[0].0.as_deref(), Some("kernel"));
        assert_eq!(options.generated_inputs, vec!["out/generated.json"]);
    }
}
