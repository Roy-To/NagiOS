use std::env;
use std::path::Path;

use nagi_cli::commands::execute;
use nagi_cli::doctor::SystemProbe;
use nagi_cli::paths::discover_repo_root;
use nagi_cli::run_receipt::RunReceiptConfig;

fn main() {
    let root = match discover_repo_root(Path::new(".")) {
        Ok(root) => root,
        Err(error) => {
            eprintln!("FAIL repository: {error}");
            std::process::exit(nagi_cli::commands::EXIT_CONFIG_ERROR);
        }
    };
    let receipt_config = match RunReceiptConfig::from_env() {
        Ok(config) => config,
        Err(error) => {
            eprintln!("FAIL run receipt configuration: {error}");
            std::process::exit(nagi_cli::commands::EXIT_CONFIG_ERROR);
        }
    };
    let started_run = match receipt_config {
        Some(config) => match config.start(&root) {
            Ok(started) => Some(started),
            Err(error) => {
                eprintln!("FAIL run receipt start: {error}");
                std::process::exit(nagi_cli::commands::EXIT_CONFIG_ERROR);
            }
        },
        None => None,
    };
    let args: Vec<String> = env::args().skip(1).collect();
    let probe = SystemProbe::default();
    let result = execute(&args, &root, &probe);
    let exit_code = result.exit_code;

    for line in result.lines {
        println!("{line}");
    }

    // Keep the durable result write as the final action before process::exit.
    // An abrupt kill or power loss can still make the OS wait status unavailable.
    if let Some(started) = started_run {
        if let Err(error) = started.finish(exit_code) {
            eprintln!("WARN run receipt terminal result was not saved: {error}");
        }
    }

    std::process::exit(exit_code);
}
