use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command as ProcessCommand, Stdio};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::cc_nagi::ensure_cc_nagi_checkout;
use crate::config::{load_toolchain_requirements, validate_project};
use crate::doctor::{ovmf_pair_is_allowed, run_doctor_with_requirements, DoctorPolicy, HostProbe};
use crate::image::{
    ensure_persistent_disk, initialize_ovmf_vars, run_qemu, run_qemu_gui,
    run_qemu_gui_reusing_ovmf_vars_with_events_and_serial_input,
    run_qemu_gui_reusing_ovmf_vars_with_read_only_boot_disk_and_events_and_serial_input,
    run_qemu_gui_with_events, run_qemu_gui_with_events_and_screenshot,
    run_qemu_gui_with_read_only_boot_disk_and_events_and_failure_marker,
    run_qemu_gui_with_read_only_boot_disk_and_events_and_serial_input, run_qemu_interactive,
    run_qemu_reusing_ovmf_vars, run_qemu_reusing_ovmf_vars_with_read_only_boot_disk,
    run_qemu_until_any_acceptance_marker, run_qemu_with_read_only_boot_disk,
    validate_reference_disk_qcow2, write_fat12_image, write_m17_fat12_image,
    write_m27_broken_slot_image, write_m27_gpt_broken_system_b_qcow2, write_m27_healthy_slot_image,
    write_m27_recovery_image, write_reference_disk_qcow2, ImageLayout, QemuConfig,
    GUEST_ACCEPTANCE_MARKER, NAGI_WRITE_MARKER,
};
use crate::llama_cpp::ensure_llama_cpp_checkout;
use crate::mesa::ensure_mesa_checkout;
use crate::mozjs_sys_nagi::ensure_mozjs_sys_nagi_checkout;
use crate::paths::{clean_owned_outputs, ensure_owned_directory};
use crate::servo::ensure_servo_checkout;
use crate::surfman::ensure_surfman_checkout;
use crate::tempfile_nagi::ensure_tempfile_nagi_checkout;

pub const EXIT_SUCCESS: i32 = 0;
pub const EXIT_USAGE: i32 = 2;
pub const EXIT_NOT_IMPLEMENTED: i32 = 3;
pub const EXIT_CONFIG_ERROR: i32 = 4;
pub const EXIT_DOCTOR_FAILURE: i32 = 10;

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
    build_features: ImageBuildFeatures<'a>,
}

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

const M27_RECOVERY_MENU_EVENTS: [&str; 2] = [
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":true,"key":{"type":"qcode","data":"r"}}}]}}"#,
    r#"{"execute":"input-send-event","arguments":{"events":[{"type":"key","data":{"down":false,"key":{"type":"qcode","data":"r"}}}]}}"#,
];

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
    M18,
    M19,
    M22,
    M25,
    M27,
    M30,
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
        "m18" => Command::M18,
        "m19" => Command::M19,
        "m22" => Command::M22,
        "m25" => Command::M25,
        "m27" => Command::M27,
        "m30" => Command::M30,
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
        | Command::M18
        | Command::M19
        | Command::M22
        | Command::M25
        | Command::M27
        | Command::M30
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
        Command::M18 => execute_m18(root, probe),
        Command::M19 => execute_m19(root, probe),
        Command::M22 => execute_m22(root, probe),
        Command::M25 => execute_m25(root, probe),
        Command::M27 => execute_m27(root, probe),
        Command::M30 => execute_m30(root, probe),
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
            "PASS fetch: Cargo registry sources fetched; pinned smoltcp, Surfman, tempfile, mozjs_sys, cc, Servo, Mesa/Softpipe, and llama.cpp sources validated ({}, {}, {}, {}, {}, {}, {})",
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
            mesa_relative.display()
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
    let layout = match image_writer(&image_path, &loader, &kernel, &init, recovery_init) {
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

fn execute_m30(root: &Path, probe: &dyn HostProbe) -> CommandResult {
    let host = match resolve_qemu_host(root, probe, "m30") {
        Ok(host) => host,
        Err(error) => return failure(EXIT_CONFIG_ERROR, error),
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
        let init_args = [
            "build",
            "-p",
            "nagi-init",
            "--features",
            "m10-desktop,m19-search,m22-history",
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
                cargo_env: &[],
                recovery_init: Some(&recovery_init),
                image_writer: write_reference_disk_qcow2,
                build_features: ImageBuildFeatures {
                    kernel: &[],
                    loader: &["m27-ab-slot-boot-control"],
                },
            },
        );
        if image_result.exit_code != EXIT_SUCCESS {
            return image_result;
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
        acceptance_marker: "Nagi M7 acceptance PASS",
        ..first_config
    };
    let qemu_status = match run_qemu_reusing_ovmf_vars(&restart_config) {
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
        "Nagi M27 persistence decision: confirmed slot=A",
        "Nagi M27 UEFI variable journal persistence PASS",
        "Nagi Kernel started",
        "Nagi M7 VirtIO Block PASS",
        "Nagi M7 ext2 mount PASS",
        "Nagi M7 persistent read PASS",
        "Nagi M7 acceptance PASS",
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
    for checked_image in [&image_path, &qemu_test_image] {
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
            "PASS M30 64 GiB GPT qcow2 passed System A and User Data persistence acceptance on a disposable copy (image {}; QEMU copy {}; serial log {})",
            image_path.display(),
            qemu_test_image.display(),
            serial_log.display()
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
    let screenshot_run_id = match SystemTime::now().duration_since(UNIX_EPOCH) {
        Ok(duration) => duration.as_nanos().to_string(),
        Err(error) => {
            return failure(EXIT_CONFIG_ERROR, format!("desktop: system clock: {error}"));
        }
    };
    let screenshot_directory = match ensure_owned_directory(
        root,
        Path::new("out")
            .join("evidence")
            .join(format!("m29-desktop-{screenshot_run_id}")),
    ) {
        Ok(path) => path,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("desktop: {error}")),
    };
    let screenshot_path = screenshot_directory.join("nagi-m10-desktop.png");
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
    let outcome = match run_qemu_gui_with_events_and_screenshot(
        &config,
        "Nagi M10 desktop READY",
        &M10_DESKTOP_EVENTS,
        &screenshot_path,
    ) {
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
                    "desktop: guest did not print ordered marker `{marker}` (QEMU exit {}; log {})",
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
                "desktop: guest markers are present but QEMU did not reach its acceptance marker (exit {}; log {})",
                outcome.exit_status,
                desktop_log.display()
            ),
        );
    }
    let Some(ready_after) = outcome.ready_after else {
        return failure(
            EXIT_CONFIG_ERROR,
            format!(
                "desktop: QEMU acceptance completed without observing the READY marker (log {})",
                desktop_log.display()
            ),
        );
    };
    CommandResult {
        exit_code: EXIT_SUCCESS,
        lines: vec![format!(
            "PASS desktop: QEMU guest rendered and interacted with the Nagi desktop (exit {}; guest READY after {} ms; log {}; screenshot {})",
            outcome.exit_status,
            ready_after.as_millis(),
            desktop_log.display(),
            screenshot_path.display(),
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
    let image_result =
        execute_image_with_features(root, Some("m25-voice-acceptance"), "nagi-0.1-m25-voice.img");
    if image_result.exit_code != EXIT_SUCCESS {
        return image_result;
    }
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
    let image_path = artifacts.join("nagi-0.1-m25-voice.img");
    let persistent_disk = artifacts.join("nagi-0.1-m25-voice-user-data.img");
    let vars_copy = artifacts.join("nagi-0.1-m25-voice-vars.fd");
    let bootstrap_log = logs.join("m25-voice-bootstrap.log");
    let voice_log = logs.join("m25-voice.log");
    let had_persistent_disk = match ensure_persistent_disk(&persistent_disk) {
        Ok(existing) => existing,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m25: {error}")),
    };
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
            "PASS M25 guest voice orchestration: bounded fixture capture and TTS PCM, permission/indicator ordering, provider cleanup, and no-transcript behavior passed; no real audio device, STT model, or TTS engine was used (log {})",
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
    let status = match run_qemu_gui_with_read_only_boot_disk_and_events_and_failure_marker(
        &config,
        "Nagi M18 browser READY",
        &M18_INPUT_EVENTS,
        "Nagi M18 browser FAIL",
    ) {
        Ok(status) => status,
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
                "m18: guest browser acceptance failed: {error} (QEMU exit {status}; log {})",
                log_path.display()
            ),
        );
    }
    CommandResult {
        exit_code: EXIT_SUCCESS,
        lines: vec![format!(
            "PASS M18 Albert: three verified HTTPS pages rendered to Nagi Surface and QEMU (exit {status}; log {})",
            log_path.display()
        )],
    }
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
    let image_result =
        execute_image_with_features(root, Some("m19-search"), "nagi-0.1-m19-vfs-objectid.img");
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
    let timeout = Duration::from_secs(90);

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
            "Nagi M3 acceptance PASS",
            "Nagi M7 VirtIO Block PASS",
            "Nagi M13 Rust PAL PASS",
            "Nagi M13 C POSIX PASS",
            "Nagi M24 semantic index ready PASS",
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

fn execute_m22_inner(root: &Path, probe: &dyn HostProbe) -> CommandResult {
    let image_result =
        execute_image_with_features(root, Some("m22-history"), "nagi-0.1-m22-history.img");
    if image_result.exit_code != EXIT_SUCCESS {
        return image_result;
    }
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
    let image_path = artifacts.join("nagi-0.1-m22-history.img");
    let persistent_disk = artifacts.join("nagi-0.1-m22-history-user-data.img");
    let vars_copy = artifacts.join("nagi-0.1-m22-history-vars.fd");
    let bootstrap_log = logs.join("m22-history-bootstrap.log");
    let had_persistent_disk = match ensure_persistent_disk(&persistent_disk) {
        Ok(existing) => existing,
        Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m22: {error}")),
    };
    let timeout = Duration::from_secs(90);

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
            return failure(EXIT_CONFIG_ERROR, format!("m22: bootstrap boot: {error}"));
        }
        match fs::read_to_string(&bootstrap_log) {
            Ok(serial) if serial.contains(NAGI_WRITE_MARKER) => {}
            Ok(_) => {
                return failure(
                    EXIT_CONFIG_ERROR,
                    format!("m22: bootstrap did not print `{NAGI_WRITE_MARKER}`"),
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
    let mut last_log = PathBuf::new();
    for boot_index in 0..3 {
        let log_path = logs.join(format!("m22-history-boot-{}.log", boot_index + 1));
        let config = QemuConfig {
            qemu: &host.qemu,
            ovmf_code: &host.ovmf_code,
            ovmf_vars_template: &host.ovmf_vars,
            disk_image: &image_path,
            persistent_disk: &persistent_disk,
            vars_copy: &vars_copy,
            serial_log: &log_path,
            acceptance_marker: "Nagi M13 acceptance PASS",
            timeout,
        };
        let final_status = match run_qemu(&config) {
            Ok(status) => status,
            Err(error) => return failure(EXIT_CONFIG_ERROR, format!("m22: guest boot: {error}")),
        };
        last_log = log_path.clone();
        let serial = match fs::read_to_string(&log_path) {
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
            "Nagi M3 acceptance PASS",
            "Nagi M7 VirtIO Block PASS",
            "Nagi M13 C POSIX PASS",
            "Nagi M24 semantic index ready PASS",
            "Nagi M19 guest search persistence PASS",
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

    CommandResult {
        exit_code: EXIT_SUCCESS,
        lines: vec![format!(
            "PASS M22 guest NH16 and separate AI Activity Ledger archives: grouped VFS moves, composite undo, and restored state survived QEMU restarts (log {})",
            last_log.display()
        )],
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
        "m27-recovery",
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
        acceptance_marker: NAGI_WRITE_MARKER,
        timeout: Duration::from_secs(90),
    };
    let bootstrap_status = match run_qemu_reusing_ovmf_vars(&bootstrap_config) {
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
    for marker in [NAGI_WRITE_MARKER, "Nagi M7 reboot required PASS"] {
        if !bootstrap_serial.contains(marker) {
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
    let recovery_undo_image_result =
        execute_image_with_features(root, Some("m22-history"), &recovery_undo_image_name);
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
    let committed_status = run_m13_qemu_with_http_fixture(root, &committed_config);
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
    const RECOVERY_COMMANDS: &[u8] = b"check\nlog\nfiles\nslots\nundo\nhelp\n";
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
    let restored_status = match run_m13_qemu_with_http_fixture(root, &restored_config) {
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
    let first_trial_status =
        match run_qemu_reusing_ovmf_vars_with_read_only_boot_disk(&first_trial_config) {
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
        timeout: Duration::from_secs(90),
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
        match run_qemu_reusing_ovmf_vars_with_read_only_boot_disk(&second_trial_config) {
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
            timeout: Duration::from_secs(90),
        };
        let status = match run_qemu_reusing_ovmf_vars_with_read_only_boot_disk(&config) {
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
            timeout: Duration::from_secs(90),
        };
        let status = match run_qemu_reusing_ovmf_vars_with_read_only_boot_disk(&config) {
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
        "m10-desktop",
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
            acceptance_marker: "Nagi Loader: invalid ELF",
            timeout: Duration::from_secs(120),
        };
        let status = run_qemu_reusing_ovmf_vars(&config)
            .map_err(|error| format!("GPT System B trial {attempt}: {error}"))?;
        let serial =
            fs::read_to_string(&log).map_err(|error| format!("read {}: {error}", log.display()))?;
        let expected_decision =
            format!("Nagi M27 persistence decision: trial attempt={attempt} slot=B");
        require_m27_gpt_markers(
            "GPT malformed System B trial",
            status,
            &log,
            &serial,
            &[
                &expected_decision,
                "Nagi M27 UEFI variable journal persistence PASS",
                "Nagi M30 GPT partition boot: System B PASS",
                "Nagi M27 trial payload rejected slot=B",
                "Nagi Loader: invalid ELF",
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
        let status = run_qemu_reusing_ovmf_vars(&config)
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
        acceptance_marker: "Nagi M10 desktop READY",
        timeout: Duration::from_secs(120),
    };
    let status = run_qemu_reusing_ovmf_vars(&trial_config)
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
            "Nagi M7 persistent read PASS",
            "Nagi M27 readiness persisted slot=B attempt=1 generation=",
            "Nagi M10 desktop READY",
        ],
    )?;
    if !m27_readiness_persisted_before_desktop(&serial) {
        return Err(format!(
            "GPT System B did not persist readiness before the desktop marker (log {})",
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
    let status = run_qemu_reusing_ovmf_vars(&confirmed_config)
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
        && serial.contains("Nagi Loader: invalid ELF")
        && !serial.contains("Nagi Kernel started")
        && !serial.contains("Nagi M27 readiness persisted")
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
            "Commands: doctor [--allow-missing], fetch, build, image, run, shell, gui, desktop, security, network, posix, std, m13, m14, m15, m16, m17, m18, m19, m22, m25, m27, m30, test, clean, fmt, lint"
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
        append_nagi_target_archive_tools, last_serial_lines, m17_trace_excerpt,
        m27_readiness_consumed_before_promotion, m27_readiness_persisted_before_desktop,
        m27_trial_failure_observed,
    };
    use std::path::Path;

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
        assert!(!m27_trial_failure_observed(
            "Nagi M27 trial payload rejected slot=B\n"
        ));
        assert!(!m27_trial_failure_observed(
            "Nagi M27 trial payload rejected slot=B\nNagi Loader: invalid ELF\nNagi Kernel started\n"
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
    fn m18_browser_boot_keeps_the_esp_read_only_for_storage_selection() {
        let commands = include_str!("commands.rs");
        let m18_start = commands.find("fn execute_m18(").expect("M18 command");
        let m18_end = commands[m18_start..]
            .find("fn help() -> CommandResult")
            .map(|offset| m18_start + offset)
            .expect("next command helper");
        assert!(commands[m18_start..m18_end]
            .contains("run_qemu_gui_with_read_only_boot_disk_and_events_and_failure_marker("));

        let image = include_str!("image.rs");
        assert!(image.contains("Duration::from_millis(100)"));
        let compact_image: String = image.split_whitespace().collect();
        assert!(compact_image.contains("mode.inter_event_delay.is_zero()"));
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
