use std::env;
use std::io::{self, Write};
use std::path::Path;

use nagi_cli::commands::execute;
use nagi_cli::doctor::SystemProbe;
use nagi_cli::paths::discover_repo_root;
use nagi_cli::run_receipt::RunReceiptConfig;

fn main() {
    let receipt_config = match RunReceiptConfig::take_from_env_at_startup() {
        Ok(config) => config,
        Err(error) => {
            report_failure("run receipt start", &error);
            std::process::exit(nagi_cli::commands::EXIT_CONFIG_ERROR);
        }
    };

    let root = match discover_repo_root(Path::new(".")) {
        Ok(root) => root,
        Err(error) => {
            report_failure("repository", &error);
            std::process::exit(nagi_cli::commands::EXIT_CONFIG_ERROR);
        }
    };
    let started_receipt = match receipt_config {
        Some(config) => match config.start(&root) {
            Ok(started) => Some(started),
            Err(error) => {
                report_failure("run receipt start", &error);
                std::process::exit(nagi_cli::commands::EXIT_CONFIG_ERROR);
            }
        },
        None => None,
    };

    let args: Vec<String> = env::args().skip(1).collect();
    let probe = SystemProbe::default();
    let result = execute(&args, &root, &probe);

    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    for line in result.lines {
        let _ = writeln!(stdout, "{line}");
    }
    drop(stdout);

    if let Some(started) = started_receipt {
        if let Err(error) = started.finish(result.exit_code) {
            report_failure("run receipt finish", &error);
        }
    }
    std::process::exit(result.exit_code);
}

fn report_failure(area: &str, error: &impl std::fmt::Display) {
    let stderr = io::stderr();
    let _ = writeln!(stderr.lock(), "FAIL {area}: {error}");
}
