use crate::diff::compare_inventories;
use crate::model::{Inventory, InventoryDiff};
use crate::report::{render_diff, render_notice_candidate, render_scan_report};
use crate::sbom::build_spdx_now;
use crate::scan::scan_repository;
use std::env;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

const USAGE: &str = "Usage: nagi-legal <scan|sbom|notice|check|diff> [--root PATH] [--json] [--output PATH]\n       nagi-legal diff --before INVENTORY.json --after INVENTORY.json";
const MAX_DIFF_INVENTORY_BYTES: u64 = 64 * 1024 * 1024;

struct Options {
    root: PathBuf,
    output: Option<PathBuf>,
    before: Option<PathBuf>,
    after: Option<PathBuf>,
    json: bool,
}

pub fn run(args: &[String]) -> i32 {
    let current_dir = match env::current_dir() {
        Ok(directory) => directory,
        Err(_) => {
            eprintln!("nagi-legal: cannot determine current directory");
            return 2;
        }
    };
    run_from(args, &current_dir)
}

pub fn run_from(args: &[String], current_dir: &Path) -> i32 {
    let Some(command) = args.first().map(String::as_str) else {
        println!("{USAGE}");
        return 0;
    };
    if matches!(command, "help" | "--help" | "-h") {
        println!("{USAGE}");
        return 0;
    }
    if !matches!(command, "scan" | "sbom" | "notice" | "check" | "diff") {
        eprintln!("nagi-legal: unsupported command: {command}\n{USAGE}");
        return 2;
    }
    let options = match parse_options(&args[1..], current_dir) {
        Ok(options) => options,
        Err(error) => {
            eprintln!("nagi-legal: {error}\n{USAGE}");
            return 2;
        }
    };

    if command == "diff" {
        return run_diff(&options);
    }
    let inventory = match scan_repository(&options.root) {
        Ok(inventory) => inventory,
        Err(error) => {
            eprintln!("nagi-legal: scan failed: {error}");
            return 1;
        }
    };
    match command {
        "scan" => {
            if let Some(output) = &options.output {
                let content = match serde_json::to_string_pretty(&inventory) {
                    Ok(content) => format!("{content}\n"),
                    Err(_) => {
                        eprintln!("nagi-legal: cannot serialize inventory");
                        return 1;
                    }
                };
                write_output(output, &content)
            } else if options.json {
                match serde_json::to_string_pretty(&inventory) {
                    Ok(content) => {
                        println!("{content}");
                        0
                    }
                    Err(_) => {
                        eprintln!("nagi-legal: cannot serialize inventory");
                        1
                    }
                }
            } else {
                print!("{}", render_scan_report(&inventory));
                0
            }
        }
        "sbom" => {
            let Some(output) = &options.output else {
                eprintln!("nagi-legal: sbom requires --output PATH");
                return 2;
            };
            let document = match build_spdx_now(&inventory) {
                Ok(document) => document,
                Err(errors) => {
                    eprintln!(
                        "nagi-legal: generated SPDX profile failed: {}",
                        errors.join("; ")
                    );
                    return 1;
                }
            };
            match serde_json::to_string_pretty(&document) {
                Ok(content) => write_output(output, &format!("{content}\n")),
                Err(_) => {
                    eprintln!("nagi-legal: cannot serialize SPDX document");
                    1
                }
            }
        }
        "notice" => {
            let Some(output) = &options.output else {
                eprintln!("nagi-legal: notice requires --output PATH");
                return 2;
            };
            write_output(output, &render_notice_candidate(&inventory))
        }
        "check" => {
            print!("{}", render_scan_report(&inventory));
            let mut structural_errors = inventory
                .findings
                .iter()
                .filter(|finding| finding.severity == "error")
                .count();
            if let Err(errors) = build_spdx_now(&inventory) {
                structural_errors += errors.len();
                for error in errors {
                    println!("FAIL SPDX profile: {error}");
                }
            }
            if structural_errors == 0 {
                println!("PASS: no structural inventory or SPDX profile errors");
                0
            } else {
                println!("FAIL: {structural_errors} structural inventory error(s)");
                1
            }
        }
        _ => unreachable!("supported commands were validated before scanning"),
    }
}

fn parse_options(args: &[String], current_dir: &Path) -> Result<Options, String> {
    let mut root = current_dir.to_path_buf();
    let mut output = None;
    let mut before = None;
    let mut after = None;
    let mut json = false;
    let mut format_seen = false;
    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--root" => {
                let value = next_value(args, index, "--root")?;
                root = resolve_from(current_dir, value);
                index += 2;
            }
            "--output" => {
                let value = next_value(args, index, "--output")?;
                output = Some(resolve_output(current_dir, value));
                index += 2;
            }
            "--before" => {
                let value = next_value(args, index, "--before")?;
                before = Some(resolve_from(current_dir, value));
                index += 2;
            }
            "--after" => {
                let value = next_value(args, index, "--after")?;
                after = Some(resolve_from(current_dir, value));
                index += 2;
            }
            "--json" | "--format=json" => {
                if format_seen {
                    return Err("choose one JSON output option".to_owned());
                }
                json = true;
                format_seen = true;
                index += 1;
            }
            "--format" => {
                if format_seen {
                    return Err("choose one output format".to_owned());
                }
                let value = next_value(args, index, "--format")?;
                match value.as_str() {
                    "json" => json = true,
                    "text" => {}
                    _ => return Err("--format must be text or json".to_owned()),
                }
                format_seen = true;
                index += 2;
            }
            value => return Err(format!("unsupported option {value}")),
        }
    }
    Ok(Options {
        root,
        output,
        before,
        after,
        json,
    })
}

fn next_value(args: &[String], index: usize, option: &str) -> Result<String, String> {
    args.get(index + 1)
        .filter(|value| !value.is_empty() && !value.starts_with("--"))
        .cloned()
        .ok_or_else(|| format!("{option} requires a value"))
}

fn resolve_from(base: &Path, value: String) -> PathBuf {
    let path = PathBuf::from(value);
    if path.is_absolute() {
        path
    } else {
        base.join(path)
    }
}

fn resolve_output(base: &Path, value: String) -> PathBuf {
    if value == "-" {
        PathBuf::from("-")
    } else {
        resolve_from(base, value)
    }
}

fn write_output(path: &Path, content: &str) -> i32 {
    if path == Path::new("-") {
        print!("{content}");
        return 0;
    }
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        if let Err(error) = fs::create_dir_all(parent) {
            eprintln!("nagi-legal: cannot prepare the selected output directory: {error}");
            return 1;
        }
    }
    match fs::write(path, content) {
        Ok(()) => {
            println!("Wrote selected output file");
            0
        }
        Err(error) => {
            eprintln!("nagi-legal: cannot write selected output file: {error}");
            1
        }
    }
}

fn run_diff(options: &Options) -> i32 {
    let (Some(before_path), Some(after_path)) = (&options.before, &options.after) else {
        eprintln!("nagi-legal: diff requires --before and --after inventory files");
        return 2;
    };
    let before = match read_inventory(before_path) {
        Ok(inventory) => inventory,
        Err(error) => {
            eprintln!("nagi-legal: cannot read baseline inventory: {error}");
            return 1;
        }
    };
    let after = match read_inventory(after_path) {
        Ok(inventory) => inventory,
        Err(error) => {
            eprintln!("nagi-legal: cannot read current inventory: {error}");
            return 1;
        }
    };
    let diff: InventoryDiff = compare_inventories(&before, &after);
    let content = if options.json {
        match serde_json::to_string_pretty(&diff) {
            Ok(content) => format!("{content}\n"),
            Err(_) => {
                eprintln!("nagi-legal: cannot serialize inventory comparison");
                return 1;
            }
        }
    } else {
        render_diff(&diff)
    };
    if let Some(output) = &options.output {
        write_output(output, &content)
    } else {
        print!("{content}");
        0
    }
}

fn read_inventory(path: &Path) -> Result<Inventory, String> {
    let metadata = fs::metadata(path).map_err(|_| "input file could not be read".to_owned())?;
    if metadata.len() > MAX_DIFF_INVENTORY_BYTES {
        return Err("input inventory exceeds the 64 MiB size bound".to_owned());
    }
    let file = File::open(path).map_err(|_| "input file could not be read".to_owned())?;
    let mut contents = Vec::new();
    file.take(MAX_DIFF_INVENTORY_BYTES + 1)
        .read_to_end(&mut contents)
        .map_err(|_| "input file could not be read".to_owned())?;
    if contents.len() as u64 > MAX_DIFF_INVENTORY_BYTES {
        return Err("input inventory exceeds the 64 MiB size bound".to_owned());
    }
    serde_json::from_slice(&contents)
        .map_err(|_| "input is not a valid inventory JSON document".to_owned())
}

#[cfg(test)]
mod tests {
    use super::{parse_options, run_from};
    use std::path::Path;

    #[test]
    fn rejects_duplicate_or_missing_output_arguments() {
        let args = vec!["--output".to_owned()];
        assert!(parse_options(&args, Path::new("/repo")).is_err());
        let args = vec![
            "--json".to_owned(),
            "--format".to_owned(),
            "json".to_owned(),
        ];
        assert!(parse_options(&args, Path::new("/repo")).is_err());
        let args = vec![
            "--format".to_owned(),
            "text".to_owned(),
            "--format".to_owned(),
            "json".to_owned(),
        ];
        assert!(parse_options(&args, Path::new("/repo")).is_err());
    }

    #[test]
    fn resolves_relative_paths_against_the_selected_cwd() {
        let args = vec!["--output".to_owned(), "out/result.json".to_owned()];
        let options = parse_options(&args, Path::new("/repo")).expect("command options parse");
        assert_eq!(
            options.output.as_deref(),
            Some(Path::new("/repo/out/result.json"))
        );
        let args = vec!["--output".to_owned(), "-".to_owned()];
        let options = parse_options(&args, Path::new("/repo")).expect("stdout option parses");
        assert_eq!(options.output.as_deref(), Some(Path::new("-")));
    }

    #[test]
    fn rejects_unknown_command_before_trying_to_scan_a_root() {
        assert_eq!(
            run_from(
                &["unknown-command".to_owned(), "--root".to_owned()],
                Path::new("/root/does-not-exist")
            ),
            2
        );
    }
}
