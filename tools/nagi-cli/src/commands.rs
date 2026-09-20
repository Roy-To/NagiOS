use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as ProcessCommand};
use std::thread;
use std::time::Duration;

use crate::config::{load_toolchain_requirements, validate_project};
use crate::doctor::{ovmf_pair_is_allowed, run_doctor_with_requirements, DoctorPolicy, HostProbe};
use crate::image::{
    ensure_persistent_disk, run_qemu, run_qemu_gui, run_qemu_gui_with_events, run_qemu_interactive,
    write_fat12_image, QemuConfig, GUEST_ACCEPTANCE_MARKER, NAGI_WRITE_MARKER,
};
use crate::mesa::ensure_mesa_checkout;
use crate::paths::{clean_owned_outputs, ensure_owned_directory};
use crate::servo::ensure_servo_checkout;
use crate::surfman::ensure_surfman_checkout;

pub const EXIT_SUCCESS: i32 = 0;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_NOT_IMPLEMENTED: i32 = 3;
pub const EXIT_CONFIG_ERROR: i32 = 4;
pub const EXIT_DOCTOR_FAILURE: i32 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    Help,
    Doctor,
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
    Test,
    Clean,
    Fmt,
    Lint,
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

pub fn parse_command(args: &[String]) -> Result<Command, CliError> {
    let Some(name) = args.first().map(String::as_str) else {
        return Ok(Command::Help);
    };

    let command = match name {
        "help" | "--help" | "-h" => Command::Help,
        "doctor" => Command::Doctor,
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
        "test" => Command::Test,
        "clean" => Command::Clean,
        "fmt" => Command::Fmt,
        "lint" => Command::Lint,
        other => {
            return Err(CliError::new(
                format!("unknown command `{other}`"),
                EXIT_USAGE,
            ));
        }
    };

    let valid_arity = match command {
        Command::Doctor => {
            args.len() == 1 || args.get(1).is_some_and(|arg| arg == "--allow-missing")
        }
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
        | Command::Test
        | Command::Clean
        | Command::Fmt
        | Command::Lint => args.len() == 1,
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
    match command {
        "build" => vec![
            "build",
            "--workspace",
            "--exclude",
            "nagi-kernel",
            "--locked",
        ],
        "test" => vec![
            "test",
            "--workspace",
            "--exclude",
            "nagi-kernel",
            "--locked",
        ],
        "clippy" => vec![
            "clippy",
            "--workspace",
            "--all-targets",
            "--exclude",
            "nagi-kernel",
            "--locked",
            "--",
            "-D",
            "warnings",
        ],
        _ => panic!("unsupported host workspace command: {command}"),
    }
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
        Command::Fetch => execute_fetch(root),
        Command::Build => run_cargo(root, "build", &host_workspace_args("build")),
        Command::Test => run_cargo(root, "test", &host_workspace_args("test")),
        Command::Fmt => run_cargo(root, "fmt", &["fmt", "--all", "--", "--check"]),
        Command::Lint => run_cargo(root, "lint", &host_workspace_args("clippy")),
        Command::Clean => execute_clean(root),
        Command::Image => execute_image(root),
        Command::Run => execute_run(root, probe),
        Command::Shell => execute_shell(root, probe),
        Command::Gui => execute_gui(root, probe),
        Command::Desktop => execute_desktop(root, probe),
        Command::Security => execute_security(root, probe),
        Command::Network => execute_network(root, probe),
        Command::Posix => execute_posix(root, probe),
        Command::Std => execute_std(root, probe),
        Command::M13 => execute_m13(root, probe),
        Command::M14 => execute_m14(root, probe),
        Command::M15 => execute_m15(root, probe),
        Command::M16 => execute_m16(root, probe),
        Command::M17 => execute_m17(root, probe),
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
            )
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
    let surfman = match ensure_surfman_checkout(root) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("fetch: {error}")),
    };
    let servo = match ensure_servo_checkout(root) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("fetch: {error}")),
    };
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
        lines: vec![
            format!(
                "PASS fetch: Cargo registry sources fetched; pinned smoltcp, Surfman, Servo, and Mesa/Softpipe sources validated ({}, {}, {})",
                surfman.strip_prefix(root).unwrap_or(Path::new("third_party/surfman")).display(),
                servo_relative.display(),
                mesa_relative.display()
            ),
        ],
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
        Some(feature) => vec![
            "build",
            "-p",
            "nagi-init",
            "--features",
            feature,
            "--target",
            "targets/x86_64-unknown-nagi-user.json",
            "-Zbuild-std=core,compiler_builtins",
            "--release",
        ],
        None => vec![
            "build",
            "-p",
            "nagi-init",
            "--target",
            "targets/x86_64-unknown-nagi-user.json",
            "-Zbuild-std=core,compiler_builtins",
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
    let init_build = match rust_std_source {
        Some(source) => {
            run_cargo_with_rust_std_source(root, "user init", init_args, source, cargo_env)
        }
        None => run_cargo_with_env(root, "user init", init_args, cargo_env),
    };
    if init_build.exit_code != EXIT_SUCCESS {
        return init_build;
    }
    let kernel_build = run_cargo(
        root,
        "kernel",
        &[
            "build",
            "-p",
            "nagi-kernel",
            "--target",
            "targets/x86_64-unknown-nagi.json",
            "-Zbuild-std=core,compiler_builtins",
            "--release",
        ],
    );
    if kernel_build.exit_code != EXIT_SUCCESS {
        return kernel_build;
    }
    let loader_build = run_cargo(
        root,
        "loader",
        &[
            "build",
            "--manifest-path",
            "loader/Cargo.toml",
            "--target",
            "x86_64-unknown-uefi",
            "--release",
        ],
    );
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
    let layout = match write_fat12_image(&image_path, &loader, &kernel, &init) {
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
    let image_result =
        execute_image_with_features(root, Some("m10-desktop"), "nagi-0.1-m10-desktop.img");
    if image_result.exit_code != EXIT_SUCCESS {
        return image_result;
    }
    let host = match resolve_qemu_host(root, probe, "desktop") {
        Ok(host) => host,
        Err(error) => return failure(EXIT_CONFIG_ERROR, error),
    };
    let artifacts = match ensure_owned_directory(root, Path::new("out").join("artifacts")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("desktop: {error}")),
    };
    let logs = match ensure_owned_directory(root, Path::new("out").join("logs")) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("desktop: {error}")),
    };
    let image_path = artifacts.join("nagi-0.1-m10-desktop.img");
    let persistent_disk = artifacts.join("nagi-0.1-user-data.img");
    let vars_copy = artifacts.join("nagi-0.1-m10-desktop-vars.fd");
    let first_log = logs.join("m10-first-boot.log");
    let desktop_log = logs.join("m10-desktop.log");
    let had_persistent_disk = match ensure_persistent_disk(&persistent_disk) {
        Ok(existing) => existing,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("desktop: {error}")),
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
            return failure(EXIT_CONFIG_ERROR, format!("desktop: first boot: {error}"));
        }
        let first_serial = match fs::read_to_string(&first_log) {
            Ok(serial) => serial,
            Err(error) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!("desktop: cannot read {}: {error}", first_log.display()),
                );
            }
        };
        if !first_serial.contains(NAGI_WRITE_MARKER) {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("desktop: first boot did not print `{NAGI_WRITE_MARKER}`"),
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
        acceptance_marker: "Nagi M10 acceptance PASS",
        timeout,
    };
    let status =
        match run_qemu_gui_with_events(&config, "Nagi M10 desktop READY", &M10_DESKTOP_EVENTS) {
            Ok(status) => status,
            Err(error) => return failure(EXIT_CONFIG_ERROR, format!("desktop: {error}")),
        };
    let serial = match fs::read_to_string(&desktop_log) {
        Ok(serial) => serial,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("desktop: cannot read {}: {error}", desktop_log.display()),
            );
        }
    };
    let mut last_marker_end = 0;
    for marker in [
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
        "Nagi M10 acceptance PASS",
    ] {
        let Some(relative) = serial[last_marker_end..].find(marker) else {
            return failure(
                EXIT_CONFIG_ERROR,
                format!(
                    "desktop: guest did not print ordered marker `{marker}` (QEMU exit {status}; log {})",
                    desktop_log.display()
                ),
            );
        };
        last_marker_end += relative + marker.len();
    }
    CommandResult {
        exit_code: EXIT_SUCCESS,
        lines: vec![format!(
            "PASS desktop: QEMU guest rendered and interacted with the Nagi desktop (exit {status}; log {})",
            desktop_log.display()
        )],
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
                )
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
            )
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
                )
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
            )
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
            )
        }
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m17: cannot start tools/mesa/build.sh through bash: {error}"),
            )
        }
    }

    let package_path = root.join("out").join("artifacts").join("hello-nagi.xapp");
    let mesa_build_path = root.join("out").join("m17-mesa").join("mesa-build");
    let target_c_compiler = root.join("tools").join("nagi-target-cc.sh");
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
    let image_result = execute_image_with_init_build_env(
        root,
        &init_args,
        Some(&rust_std_source),
        "nagi-0.1-m17-servo.img",
        &[
            ("NAGI_M16_PACKAGE", package_path.as_path()),
            ("NAGI_MESA_BUILD", mesa_build_path.as_path()),
            ("CC_x86_64_unknown_nagi_user", target_c_compiler.as_path()),
        ],
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
    let log_path = logs.join("m17-servo.log");
    let config = QemuConfig {
        qemu: &host.qemu,
        ovmf_code: &host.ovmf_code,
        ovmf_vars_template: &host.ovmf_vars,
        disk_image: &image_path,
        persistent_disk: &persistent_disk,
        vars_copy: &vars_copy,
        serial_log: &log_path,
        acceptance_marker: "Nagi M17 first web pixel PASS",
        timeout: Duration::from_secs(120),
    };
    let status = match run_qemu(&config) {
        Ok(status) => status,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m17: QEMU: {error}")),
    };
    let serial = match fs::read_to_string(&log_path) {
        Ok(serial) => serial,
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m17: cannot read {}: {error}", log_path.display()),
            )
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
            )
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
            )
        }
        Err(error) => {
            return failure(
                EXIT_CONFIG_ERROR,
                format!("m16: sample SDK artifact: {error}"),
            )
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
            )
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
            )
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
        "-Zbuild-std=core,compiler_builtins",
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
                )
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
            )
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
    match output {
        Ok(output) if output.status.success() => CommandResult {
            exit_code: EXIT_SUCCESS,
            lines: vec![format!("PASS {label}: cargo completed successfully")],
        },
        Ok(output) => {
            let detail = command_output(&output).trim().to_owned();
            failure(
                output.status.code().unwrap_or(EXIT_CONFIG_ERROR),
                format!("{label}: cargo failed{}", nonempty_detail(&detail)),
            )
        }
        Err(error) => failure(
            EXIT_CONFIG_ERROR,
            format!("{label}: cannot start cargo: {error}"),
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

fn help() -> CommandResult {
    CommandResult {
        exit_code: EXIT_SUCCESS,
        lines: vec![
            "Nagi OS developer orchestrator".into(),
            "Commands: doctor [--allow-missing], fetch, build, image, run, shell, gui, desktop, security, network, posix, std, m13, m14, m15, m16, m17, test, clean, fmt, lint"
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
