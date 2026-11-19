use std::env;
use std::fmt;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use nagi_cli::commands::execute;
use nagi_cli::doctor::SystemProbe;
use nagi_cli::paths::discover_repo_root;
use nagi_cli::run_receipt::RunReceiptConfig;

fn main() {
    let receipt_config = match RunReceiptConfig::take_from_env_at_startup() {
        Ok(config) => config,
        Err(error) => {
            report(format_args!("FAIL run receipt configuration: {error}"));
            std::process::exit(nagi_cli::commands::EXIT_CONFIG_ERROR);
        }
    };
    let args: Vec<String> = env::args().skip(1).collect();
    let is_port_readiness = args.first().is_some_and(|arg| arg == "port")
        && args.get(1).is_some_and(|arg| arg == "readiness");
    let (root, started_run) = if is_port_readiness {
        // This read-only analysis targets the caller's project and can run
        // from any working directory, outside a Nagi checkout. If the caller
        // explicitly opted into run receipts, retain them in a checkout or
        // fail clearly when no repository identity is available.
        let started_run = match receipt_config {
            Some(config) => {
                let receipt_root = match discover_repo_root(Path::new(".")) {
                    Ok(root) => root,
                    Err(error) => {
                        report(format_args!(
                            "FAIL run receipts require a Nagi repository when enabled: {error}"
                        ));
                        std::process::exit(nagi_cli::commands::EXIT_CONFIG_ERROR);
                    }
                };
                match config.start(&receipt_root) {
                    Ok(started) => Some(started),
                    Err(error) => {
                        report(format_args!("FAIL run receipt start: {error}"));
                        std::process::exit(nagi_cli::commands::EXIT_CONFIG_ERROR);
                    }
                }
            }
            None => None,
        };
        (PathBuf::from("."), started_run)
    } else {
        let root = match discover_repo_root(Path::new(".")) {
            Ok(root) => root,
            Err(error) => {
                report(format_args!("FAIL repository: {error}"));
                std::process::exit(nagi_cli::commands::EXIT_CONFIG_ERROR);
            }
        };
        let started_run = match receipt_config {
            Some(config) => match config.start(&root) {
                Ok(started) => Some(started),
                Err(error) => {
                    report(format_args!("FAIL run receipt start: {error}"));
                    std::process::exit(nagi_cli::commands::EXIT_CONFIG_ERROR);
                }
            },
            None => None,
        };
        (root, started_run)
    };
    let probe = SystemProbe::default();
    let result = execute(&args, &root, &probe);
    let command_result = result.exit_code;

    // Persist the already known command result before any fallible output. A
    // disconnected terminal or closed pipe must not prevent the receipt or
    // replace command_result. This is still not an OS wait status.
    let receipt_error = started_run
        .as_ref()
        .and_then(|started| started.finish(command_result).err());

    let mut stdout = io::stdout().lock();
    for line in &result.lines {
        if writeln!(stdout, "{line}").is_err() {
            break;
        }
    }
    let _ = stdout.flush();
    drop(stdout);

    if let Some(error) = receipt_error {
        report(format_args!("WARN run receipt terminal outcome: {error}"));
    }

    std::process::exit(command_result);
}

/// Write a diagnostic to stderr without panicking when stderr is closed.
fn report(message: fmt::Arguments<'_>) {
    let _ = writeln!(io::stderr().lock(), "{message}");
}
