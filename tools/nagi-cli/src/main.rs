use std::env;
use std::path::Path;

use nagi_cli::commands::execute;
use nagi_cli::doctor::SystemProbe;
use nagi_cli::paths::discover_repo_root;

fn main() {
    let root = match discover_repo_root(Path::new(".")) {
        Ok(root) => root,
        Err(error) => {
            eprintln!("FAIL repository: {error}");
            std::process::exit(nagi_cli::commands::EXIT_CONFIG_ERROR);
        }
    };
    let args: Vec<String> = env::args().skip(1).collect();
    let result = if args.first().is_some_and(|arg| arg == "dev")
        && args.get(1).is_some_and(|arg| arg == "fingerprint")
    {
        nagi_cli::development::execute_fingerprint_cli(&args[2..], &root)
    } else {
        let probe = SystemProbe::default();
        execute(&args, &root, &probe)
    };

    for line in result.lines {
        println!("{line}");
    }
    std::process::exit(result.exit_code);
}
