use std::env;
use std::ffi::OsString;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use crate::commands::{EXIT_DOCTOR_FAILURE, EXIT_SUCCESS};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DoctorPolicy {
    Strict,
    AllowMissing,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandEvidence {
    pub path: String,
    pub exit_code: Option<i32>,
    pub output: String,
}

impl CommandEvidence {
    pub fn success(path: impl Into<String>, output: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            exit_code: Some(0),
            output: output.into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OvmfEvidence {
    pub code: String,
    pub vars: String,
}

pub type Version = (u64, u64, u64);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolchainRequirements {
    pub llvm_min_version: Version,
    pub lld_min_version: Version,
    pub qemu_min_version: Version,
    pub cmake_min_version: Version,
    pub meson_min_version: Version,
    pub ninja_min_version: Version,
    pub python_min_version: Version,
    pub ovmf_pairs: Vec<(String, String)>,
}

impl Default for ToolchainRequirements {
    fn default() -> Self {
        Self {
            llvm_min_version: (17, 0, 0),
            lld_min_version: (17, 0, 0),
            qemu_min_version: (8, 0, 0),
            cmake_min_version: (3, 25, 0),
            meson_min_version: (1, 0, 0),
            ninja_min_version: (1, 10, 0),
            python_min_version: (3, 10, 0),
            ovmf_pairs: vec![
                (
                    "edk2-x86_64-code.fd".to_owned(),
                    "edk2-i386-vars.fd".to_owned(),
                ),
                ("OVMF_CODE.fd".to_owned(), "OVMF_VARS.fd".to_owned()),
                ("OVMF_CODE_4M.fd".to_owned(), "OVMF_VARS_4M.fd".to_owned()),
                (
                    "edk2-x86_64-code-4m.fd".to_owned(),
                    "edk2-i386-vars-4m.fd".to_owned(),
                ),
            ],
        }
    }
}

impl OvmfEvidence {
    pub fn new(code: impl Into<String>, vars: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            vars: vars.into(),
        }
    }

    pub fn is_compatible(&self) -> bool {
        ovmf_family(&self.code) == ovmf_family(&self.vars)
    }
}

pub trait HostProbe {
    fn command(&self, candidates: &[&str], args: &[&str]) -> Option<CommandEvidence>;
    fn ovmf(&self) -> Option<OvmfEvidence>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckState {
    Pass,
    Warn,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckResult {
    pub name: &'static str,
    pub state: CheckState,
    pub detail: String,
}

impl CheckResult {
    pub fn is_pass(&self) -> bool {
        self.state == CheckState::Pass
    }

    pub fn is_warn(&self) -> bool {
        self.state == CheckState::Warn
    }

    pub fn is_fail(&self) -> bool {
        self.state == CheckState::Fail
    }

    pub fn render(&self) -> String {
        let state = match self.state {
            CheckState::Pass => "PASS",
            CheckState::Warn => "WARN",
            CheckState::Fail => "FAIL",
        };
        format!("{state} {}: {}", self.name, self.detail)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DoctorReport {
    pub exit_code: i32,
    pub checks: Vec<CheckResult>,
}

impl DoctorReport {
    pub fn summary(&self) -> String {
        let pass = self.checks.iter().filter(|check| check.is_pass()).count();
        let warn = self.checks.iter().filter(|check| check.is_warn()).count();
        let fail = self.checks.iter().filter(|check| check.is_fail()).count();
        let state = if self.exit_code == EXIT_SUCCESS {
            "PASS"
        } else {
            "FAIL"
        };
        format!("{state} doctor: {pass} pass, {warn} warn, {fail} fail")
    }
}

pub fn run_doctor(probe: &dyn HostProbe, policy: DoctorPolicy) -> DoctorReport {
    run_doctor_with_requirements(probe, policy, &ToolchainRequirements::default())
}

pub fn run_doctor_with_requirements(
    probe: &dyn HostProbe,
    policy: DoctorPolicy,
    requirements: &ToolchainRequirements,
) -> DoctorReport {
    let mut checks = Vec::with_capacity(12);
    check_command(
        &mut checks,
        "Git",
        &["git", "git.exe"],
        &["--version"],
        (2, 30, 0),
        "git version",
        probe,
        policy,
    );
    check_command(
        &mut checks,
        "Rust (rustc)",
        &["rustc", "rustc.exe"],
        &["--version"],
        (1, 85, 0),
        "rustc ",
        probe,
        policy,
    );
    check_command(
        &mut checks,
        "Cargo",
        &["cargo", "cargo.exe"],
        &["--version"],
        (1, 85, 0),
        "cargo ",
        probe,
        policy,
    );
    check_command(
        &mut checks,
        "Rustup",
        &["rustup", "rustup.exe"],
        &["--version"],
        (1, 20, 0),
        "rustup ",
        probe,
        policy,
    );
    check_command(
        &mut checks,
        "LLVM/Clang",
        &["clang", "clang.exe", "clang-cl.exe"],
        &["--version"],
        requirements.llvm_min_version,
        "clang version",
        probe,
        policy,
    );
    check_command(
        &mut checks,
        "LLD",
        &["ld.lld", "ld.lld.exe", "lld-link.exe"],
        &["--version"],
        requirements.lld_min_version,
        "LLD",
        probe,
        policy,
    );
    check_command(
        &mut checks,
        "QEMU",
        &["qemu-system-x86_64", "qemu-system-x86_64.exe"],
        &["--version"],
        requirements.qemu_min_version,
        "QEMU emulator version",
        probe,
        policy,
    );
    check_ovmf(&mut checks, probe.ovmf(), policy, requirements);
    check_command(
        &mut checks,
        "CMake",
        &["cmake", "cmake.exe"],
        &["--version"],
        requirements.cmake_min_version,
        "cmake version",
        probe,
        policy,
    );
    check_command(
        &mut checks,
        "Meson",
        &["meson", "meson.exe"],
        &["--version"],
        requirements.meson_min_version,
        "",
        probe,
        policy,
    );
    check_command(
        &mut checks,
        "Ninja",
        &["ninja", "ninja.exe"],
        &["--version"],
        requirements.ninja_min_version,
        "",
        probe,
        policy,
    );
    check_command(
        &mut checks,
        "Python",
        &["python", "python.exe", "py", "py.exe"],
        &["--version"],
        requirements.python_min_version,
        "Python 3.",
        probe,
        policy,
    );

    let failed = checks.iter().any(CheckResult::is_fail);
    DoctorReport {
        exit_code: if failed {
            EXIT_DOCTOR_FAILURE
        } else {
            EXIT_SUCCESS
        },
        checks,
    }
}

#[allow(clippy::too_many_arguments)]
fn check_command(
    checks: &mut Vec<CheckResult>,
    name: &'static str,
    candidates: &[&str],
    args: &[&str],
    minimum: (u64, u64, u64),
    identity: &str,
    probe: &dyn HostProbe,
    policy: DoctorPolicy,
) {
    let Some(evidence) = probe.command(candidates, args) else {
        check_missing(checks, name, policy);
        return;
    };
    let output = evidence.output.trim();
    if evidence.exit_code != Some(0) {
        check_fail(
            checks,
            name,
            format!(
                "{} failed with exit code {:?}: {}",
                evidence.path, evidence.exit_code, output
            ),
        );
    } else if !identity.is_empty() && !output.contains(identity) {
        check_fail(
            checks,
            name,
            format!(
                "{} did not identify as the expected tool: {}",
                evidence.path, output
            ),
        );
    } else if parse_version(output).is_none_or(|version| version < minimum) {
        check_fail(
            checks,
            name,
            format!(
                "{} reported unsupported or unparseable version: {} (minimum {}.{}.{})",
                evidence.path, output, minimum.0, minimum.1, minimum.2
            ),
        );
    } else {
        let first_line = output
            .lines()
            .next()
            .unwrap_or("version output unavailable");
        check_pass(checks, name, format!("{} ({first_line})", evidence.path));
    }
}

fn check_ovmf(
    checks: &mut Vec<CheckResult>,
    evidence: Option<OvmfEvidence>,
    policy: DoctorPolicy,
    requirements: &ToolchainRequirements,
) {
    match evidence {
        Some(pair) if pair.is_compatible() && ovmf_pair_is_allowed(&pair, requirements) => {
            check_pass(
                checks,
                "OVMF CODE/VARS",
                format!("code={} vars={}", pair.code, pair.vars),
            )
        }
        Some(pair) if !pair.is_compatible() => check_fail(
            checks,
            "OVMF CODE/VARS",
            format!(
                "incompatible CODE/VARS pair: code={} vars={}",
                pair.code, pair.vars
            ),
        ),
        Some(pair) => check_fail(
            checks,
            "OVMF CODE/VARS",
            format!(
                "CODE/VARS pair is not listed in nagi.toml: code={} vars={}",
                pair.code, pair.vars
            ),
        ),
        None => check_missing(checks, "OVMF CODE/VARS", policy),
    }
}

pub fn ovmf_pair_is_allowed(pair: &OvmfEvidence, requirements: &ToolchainRequirements) -> bool {
    let code_name = Path::new(&pair.code)
        .file_name()
        .and_then(|name| name.to_str());
    let vars_name = Path::new(&pair.vars)
        .file_name()
        .and_then(|name| name.to_str());
    requirements
        .ovmf_pairs
        .iter()
        .any(|(code, vars)| Some(code.as_str()) == code_name && Some(vars.as_str()) == vars_name)
}

fn check_missing(checks: &mut Vec<CheckResult>, name: &'static str, policy: DoctorPolicy) {
    if policy == DoctorPolicy::Strict {
        check_fail(checks, name, "not found".to_owned());
    } else {
        checks.push(CheckResult {
            name,
            state: CheckState::Warn,
            detail: "not found (allowed by --allow-missing)".to_owned(),
        });
    }
}

fn check_pass(checks: &mut Vec<CheckResult>, name: &'static str, detail: String) {
    checks.push(CheckResult {
        name,
        state: CheckState::Pass,
        detail,
    });
}

fn check_fail(checks: &mut Vec<CheckResult>, name: &'static str, detail: String) {
    checks.push(CheckResult {
        name,
        state: CheckState::Fail,
        detail,
    });
}

fn parse_version(output: &str) -> Option<(u64, u64, u64)> {
    output.split_whitespace().find_map(|token| {
        let token =
            token.trim_matches(|character: char| !character.is_ascii_digit() && character != '.');
        let mut parts = token.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next().unwrap_or("0").parse().ok()?;
        let patch = parts.next().unwrap_or("0").parse().ok()?;
        Some((major, minor, patch))
    })
}

fn ovmf_family(path: &str) -> OvmfFamily {
    if path.to_ascii_lowercase().contains("4m") {
        OvmfFamily::FourMiB
    } else {
        OvmfFamily::Standard
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OvmfFamily {
    Standard,
    FourMiB,
}

#[derive(Debug, Default)]
pub struct SystemProbe {
    path: Option<OsString>,
}

impl SystemProbe {
    pub fn from_path(path: impl Into<OsString>) -> Self {
        Self {
            path: Some(path.into()),
        }
    }
}

impl HostProbe for SystemProbe {
    fn command(&self, candidates: &[&str], args: &[&str]) -> Option<CommandEvidence> {
        let mut directories = Vec::new();
        if let Some(path) = self.path.clone().or_else(|| env::var_os("PATH")) {
            directories.extend(env::split_paths(&path));
        }
        for directory in standard_tool_directories() {
            if !directories.contains(&directory) {
                directories.push(directory);
            }
        }
        for directory in directories {
            for candidate in candidates {
                for candidate_path in candidate_paths(&directory, candidate) {
                    if is_readable_file(&candidate_path) {
                        return Some(run_candidate(&candidate_path, args));
                    }
                }
            }
        }
        None
    }

    fn ovmf(&self) -> Option<OvmfEvidence> {
        let pairs = [
            ("OVMF_CODE", "OVMF_VARS"),
            ("OVMF_CODE_PATH", "OVMF_VARS_PATH"),
        ];
        for (code_key, vars_key) in pairs {
            let Some(code) = env::var_os(code_key).map(PathBuf::from) else {
                continue;
            };
            let Some(vars) = env::var_os(vars_key).map(PathBuf::from) else {
                continue;
            };
            if is_readable_file(&code) && is_readable_file(&vars) {
                return Some(OvmfEvidence::new(
                    code.display().to_string(),
                    vars.display().to_string(),
                ));
            }
        }

        const OVMF_FILE_PAIRS: [(&str, &str); 4] = [
            ("OVMF_CODE.fd", "OVMF_VARS.fd"),
            ("OVMF_CODE_4M.fd", "OVMF_VARS_4M.fd"),
            ("edk2-x86_64-code.fd", "edk2-i386-vars.fd"),
            ("edk2-x86_64-code-4m.fd", "edk2-i386-vars-4m.fd"),
        ];
        for directory in ovmf_directories() {
            for (code_name, vars_name) in OVMF_FILE_PAIRS {
                let code = directory.join(code_name);
                let vars = directory.join(vars_name);
                if is_readable_file(&code) && is_readable_file(&vars) {
                    return Some(OvmfEvidence::new(
                        code.display().to_string(),
                        vars.display().to_string(),
                    ));
                }
            }
        }
        None
    }
}

fn run_candidate(path: &Path, args: &[&str]) -> CommandEvidence {
    let command_result = if path.extension().is_some_and(|extension| {
        extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
    }) {
        ProcessCommand::new("cmd.exe")
            .arg("/d")
            .arg("/c")
            .arg(path)
            .args(args)
            .output()
    } else {
        ProcessCommand::new(path).args(args).output()
    };

    match command_result {
        Ok(output) => {
            let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.trim().is_empty() {
                if !text.is_empty() {
                    text.push('\n');
                }
                text.push_str(&stderr);
            }
            CommandEvidence {
                path: path.display().to_string(),
                exit_code: output.status.code(),
                output: text,
            }
        }
        Err(error) => CommandEvidence {
            path: path.display().to_string(),
            exit_code: None,
            output: format!("cannot execute: {error}"),
        },
    }
}

fn candidate_paths(directory: &Path, candidate: &str) -> Vec<PathBuf> {
    let direct = directory.join(candidate);
    if Path::new(candidate).extension().is_some() {
        return vec![direct];
    }
    vec![
        direct.clone(),
        directory.join(format!("{candidate}.exe")),
        directory.join(format!("{candidate}.cmd")),
        directory.join(format!("{candidate}.bat")),
    ]
}

fn is_readable_file(path: &Path) -> bool {
    fs::metadata(path)
        .map(|metadata| metadata.is_file())
        .unwrap_or(false)
        && File::open(path).is_ok()
}

fn ovmf_directories() -> Vec<PathBuf> {
    let mut directories = Vec::new();
    for key in ["OVMF_HOME", "OVMF_PATH"] {
        if let Some(path) = env::var_os(key) {
            directories.push(PathBuf::from(path));
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
    directories
}

fn standard_tool_directories() -> Vec<PathBuf> {
    let mut directories = Vec::new();
    if let Some(user_profile) = env::var_os("USERPROFILE") {
        directories.push(PathBuf::from(user_profile).join(".cargo").join("bin"));
    }
    if let Some(program_files) = env::var_os("ProgramFiles") {
        let program_files = PathBuf::from(program_files);
        directories.extend([
            program_files.join("LLVM").join("bin"),
            program_files.join("qemu"),
            program_files.join("QEMU"),
            program_files.join("CMake").join("bin"),
            program_files.join("Meson"),
            program_files.join("Git").join("cmd"),
        ]);
    }
    directories
}
