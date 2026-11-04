//! Bounded, read-only source inspection for third-party application porting.
//!
//! This command never builds, executes, installs, or resolves dependencies for
//! the inspected project. Its findings are heuristic evidence for a human
//! review, not a compatibility verdict.

use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

const MAX_FILES: usize = 20_000;
const MAX_ENTRIES: usize = 50_000;
const MAX_DIRECTORIES: usize = 10_000;
const MAX_DIRECTORY_DEPTH: usize = 64;
const MAX_FILE_BYTES: u64 = 512 * 1024;
const MAX_TOTAL_BYTES: u64 = 20 * 1024 * 1024;
const MAX_FINDINGS: usize = 10_000;

#[derive(Debug)]
struct Finding {
    category: &'static str,
    path: String,
    line: usize,
    evidence: String,
}

#[derive(Debug, Default)]
struct Scan {
    files: Vec<PathBuf>,
    files_scanned: usize,
    directories_visited: usize,
    entries_visited: usize,
    manifests: BTreeSet<String>,
    dependencies: BTreeSet<(String, String)>,
    findings: Vec<Finding>,
    languages: BTreeSet<&'static str>,
    limitations: BTreeSet<&'static str>,
    bytes_read: u64,
    truncated: bool,
}

/// Analyze a project tree and return a stable schema-v1 JSON report.
pub fn analyze(project: &Path) -> Result<Value, String> {
    let metadata =
        fs::metadata(project).map_err(|error| format!("cannot inspect project root: {error}"))?;
    if !metadata.is_dir() {
        return Err("project root must be a directory".into());
    }

    let root = project
        .canonicalize()
        .map_err(|error| format!("cannot resolve project root: {error}"))?;
    let mut scan = Scan::default();
    collect_files(&root, &mut scan)?;
    scan.files.sort();

    for relative in scan.files.clone() {
        let path = root.join(&relative);
        let name = relative
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        let relative_text = normalized_path(&relative);
        if is_manifest(name) {
            scan.manifests.insert(relative_text.clone());
        }
        detect_language(&relative, &mut scan.languages);
        if !is_source_file(&relative) && !is_manifest(name) {
            continue;
        }

        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => {
                scan.truncated = true;
                scan.limitations
                    .insert("Some files could not be inspected.");
                continue;
            }
        };
        if metadata.len() > MAX_FILE_BYTES
            || scan.bytes_read.saturating_add(metadata.len()) > MAX_TOTAL_BYTES
        {
            scan.truncated = true;
            scan.limitations
                .insert("The scan stopped at a file or total-size limit.");
            continue;
        }
        let entry_metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => {
                scan.limitations
                    .insert("Some files could not be inspected.");
                continue;
            }
        };
        if entry_metadata.file_type().is_symlink() {
            scan.limitations.insert("Symbolic links were skipped.");
            continue;
        }
        if !entry_metadata.is_file() {
            continue;
        }
        let remaining_bytes = MAX_TOTAL_BYTES.saturating_sub(scan.bytes_read);
        let read_limit = MAX_FILE_BYTES.min(remaining_bytes);
        let mut file = match File::open(&path) {
            Ok(file) => file,
            Err(_) => {
                scan.limitations
                    .insert("Some files could not be inspected.");
                continue;
            }
        };
        let read = read_text_bounded(&mut file, read_limit);
        scan.bytes_read = scan.bytes_read.saturating_add(read.bytes_read);
        let current_file_size = file.metadata().map(|metadata| metadata.len()).unwrap_or(0);
        if current_file_size > read.bytes_read {
            scan.truncated = true;
            scan.limitations
                .insert("The scan stopped at a file or total-size limit.");
            continue;
        }
        let contents = match read.contents {
            Ok(contents) => contents,
            Err(ReadTextError::InvalidUtf8 | ReadTextError::Io) => {
                scan.limitations
                    .insert("Binary, non-UTF-8, or unreadable files were skipped.");
                continue;
            }
        };

        if name == "Cargo.toml" {
            parse_cargo_dependencies(&contents, &relative_text, &mut scan.dependencies);
        } else if name == "package.json" || name == "vcpkg.json" {
            parse_json_dependencies(&contents, &relative_text, &mut scan.dependencies);
        } else if name == "CMakeLists.txt" {
            parse_cmake_dependencies(&contents, &relative_text, &mut scan.dependencies);
        } else if name == "Makefile" {
            parse_make_dependencies(&contents, &relative_text, &mut scan.dependencies);
        }

        if is_source_file(&relative) {
            scan_source(&contents, &relative_text, &mut scan);
        }
        scan.files_scanned += 1;
    }

    scan.findings.sort_by(|left, right| {
        (&left.path, left.line, left.category, &left.evidence).cmp(&(
            &right.path,
            right.line,
            right.category,
            &right.evidence,
        ))
    });
    scan.findings.dedup_by(|left, right| {
        left.path == right.path
            && left.line == right.line
            && left.category == right.category
            && left.evidence == right.evidence
    });
    if scan.findings.len() > MAX_FINDINGS {
        scan.findings.truncate(MAX_FINDINGS);
        scan.truncated = true;
        scan.limitations.insert("The finding limit was reached.");
    }

    let dependencies: Vec<Value> = scan
        .dependencies
        .iter()
        .map(|(name, source)| {
            json!({
                "name": name,
                "source": source,
                "review_status": "requires-review"
            })
        })
        .collect();
    let findings: Vec<Value> = scan
        .findings
        .iter()
        .map(|finding| {
            json!({
                "category": finding.category,
                "path": finding.path,
                "line": finding.line,
                "evidence": finding.evidence,
                "review_status": "requires-review"
            })
        })
        .collect();
    let languages: Vec<&str> = scan.languages.into_iter().collect();
    let limitations: Vec<&str> = scan.limitations.into_iter().chain([
        "This heuristic scan does not establish Nagi compatibility or POSIX/ABI compatibility.",
        "Dependency versions, licenses, transitive dependencies, and runtime behavior require separate review.",
        "A host build or package validation does not prove that an application installs or runs on Nagi OS.",
    ]).collect();
    let needs_review = scan.truncated || !findings.is_empty() || !dependencies.is_empty();

    Ok(json!({
        "schema_version": 1,
        "detected_languages": languages,
        "manifests": scan.manifests.into_iter().collect::<Vec<_>>(),
        "dependencies": dependencies,
        "analysis": {
            "files_scanned": scan.files_scanned,
            "directories_visited": scan.directories_visited,
            "entries_visited": scan.entries_visited,
            "bytes_read": scan.bytes_read,
            "truncated": scan.truncated,
            "conclusion": if needs_review { "requires-review" } else { "no-known-boundary-detected" }
        },
        "findings": findings,
        "limitations": limitations
    }))
}

#[derive(Debug, PartialEq, Eq)]
enum ReadTextError {
    Io,
    InvalidUtf8,
}

#[derive(Debug)]
struct BoundedTextRead {
    contents: Result<String, ReadTextError>,
    bytes_read: u64,
}

fn read_text_bounded<R: Read>(reader: &mut R, limit: u64) -> BoundedTextRead {
    let mut bytes = Vec::new();
    let result = (&mut *reader).take(limit).read_to_end(&mut bytes);
    let bytes_read = bytes.len() as u64;
    let contents = if result.is_err() {
        Err(ReadTextError::Io)
    } else {
        String::from_utf8(bytes).map_err(|_| ReadTextError::InvalidUtf8)
    };
    BoundedTextRead {
        contents,
        bytes_read,
    }
}

fn collect_files(root: &Path, scan: &mut Scan) -> Result<(), String> {
    let mut pending = vec![(root.to_path_buf(), 0usize)];
    'walk: while let Some((path, depth)) = pending.pop() {
        let file_type = fs::symlink_metadata(&path)
            .map_err(|error| format!("cannot inspect project entry type: {error}"))?
            .file_type();
        if file_type.is_symlink() {
            scan.limitations.insert("Symbolic links were skipped.");
            continue;
        }
        if file_type.is_dir() {
            if path != root
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(ignored_directory)
            {
                continue;
            }
            if depth > MAX_DIRECTORY_DEPTH {
                scan.truncated = true;
                scan.limitations
                    .insert("The scan stopped at a directory-depth limit.");
                continue;
            }
            scan.directories_visited += 1;
            if scan.directories_visited > MAX_DIRECTORIES {
                scan.truncated = true;
                scan.limitations
                    .insert("The scan stopped at a directory-count limit.");
                break;
            }
            let entries = fs::read_dir(&path)
                .map_err(|error| format!("cannot read project directory: {error}"))?;
            let mut children = Vec::new();
            for entry in entries {
                if scan.entries_visited >= MAX_ENTRIES {
                    scan.truncated = true;
                    scan.limitations
                        .insert("The scan stopped at a directory-entry limit.");
                    break 'walk;
                }
                let entry =
                    entry.map_err(|error| format!("cannot inspect project entry: {error}"))?;
                scan.entries_visited += 1;
                children.push(entry.path());
            }
            children.sort();
            for child in children.into_iter().rev() {
                pending.push((child, depth + 1));
            }
        } else if file_type.is_file() {
            if let Ok(relative) = path.strip_prefix(root) {
                let name = relative
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("");
                if (is_source_file(relative) || is_manifest(name)) && scan.files.len() >= MAX_FILES
                {
                    scan.truncated = true;
                    scan.limitations
                        .insert("The scan stopped at a file-count limit.");
                    break;
                }
                if is_source_file(relative) || is_manifest(name) {
                    scan.files.push(relative.to_path_buf());
                }
            }
        }
    }
    Ok(())
}

fn ignored_directory(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | ".hg"
            | ".svn"
            | "target"
            | "build"
            | "dist"
            | "vendor"
            | "third_party"
            | "node_modules"
    )
}

fn normalized_path(path: &Path) -> String {
    path.components()
        .map(|component| component.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

fn is_manifest(name: &str) -> bool {
    matches!(
        name,
        "Cargo.toml"
            | "CMakeLists.txt"
            | "meson.build"
            | "Makefile"
            | "package.json"
            | "vcpkg.json"
            | "conanfile.py"
            | "conanfile.txt"
            | "go.mod"
            | "pom.xml"
            | "build.gradle"
            | "build.gradle.kts"
    )
}

fn is_source_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            matches!(
                extension.to_ascii_lowercase().as_str(),
                "rs" | "c" | "h" | "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" | "c++" | "h++"
            )
        })
}

fn detect_language(path: &Path, languages: &mut BTreeSet<&'static str>) {
    let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
        return;
    };
    match extension.to_ascii_lowercase().as_str() {
        "rs" => {
            languages.insert("rust");
        }
        "c" => {
            languages.insert("c");
        }
        "cc" | "cpp" | "cxx" | "hpp" | "hh" | "hxx" | "c++" | "h++" => {
            languages.insert("c++");
        }
        _ => {}
    }
}

fn parse_cargo_dependencies(contents: &str, path: &str, output: &mut BTreeSet<(String, String)>) {
    let mut in_dependency_section = false;
    for raw_line in contents.lines() {
        let line = raw_line.split('#').next().unwrap_or("").trim();
        if line.starts_with('[') && line.ends_with(']') {
            let section = &line[1..line.len() - 1];
            in_dependency_section = section == "dependencies"
                || section == "dev-dependencies"
                || section == "build-dependencies"
                || (section.starts_with("target.") && section.ends_with(".dependencies"));
            if in_dependency_section {
                continue;
            }
            if section.starts_with("dependencies.")
                || section.starts_with("dev-dependencies.")
                || section.starts_with("build-dependencies.")
            {
                let name = section.rsplit('.').next().unwrap_or("").trim_matches('"');
                if !name.is_empty() {
                    output.insert((name.to_owned(), path.to_owned()));
                }
            }
            continue;
        }
        if !in_dependency_section {
            continue;
        }
        if let Some((name, _)) = line.split_once('=') {
            let name = name.trim().trim_matches('"');
            if !name.is_empty()
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
            {
                output.insert((name.to_owned(), path.to_owned()));
            }
        }
    }
}

fn parse_json_dependencies(contents: &str, path: &str, output: &mut BTreeSet<(String, String)>) {
    let Ok(document) = serde_json::from_str::<Value>(contents) else {
        return;
    };
    for section in [
        "dependencies",
        "devDependencies",
        "peerDependencies",
        "overrides",
    ] {
        if let Some(dependencies) = document.get(section).and_then(Value::as_object) {
            for name in dependencies.keys() {
                output.insert((name.clone(), path.to_owned()));
            }
        }
    }
    if let Some(dependencies) = document.get("dependencies").and_then(Value::as_array) {
        for dependency in dependencies.iter().filter_map(Value::as_str) {
            output.insert((dependency.to_owned(), path.to_owned()));
        }
    }
}

fn parse_cmake_dependencies(contents: &str, path: &str, output: &mut BTreeSet<(String, String)>) {
    for raw_line in contents.lines() {
        let line = raw_line.trim();
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("find_package(") || lower.starts_with("find_package (") {
            if let Some(name) = line
                .split_once('(')
                .and_then(|(_, args)| {
                    args.trim_start()
                        .split(|character: char| character.is_whitespace() || character == ')')
                        .next()
                })
                .filter(|name| !name.is_empty())
            {
                output.insert((name.trim_matches('"').to_owned(), path.to_owned()));
            }
        } else if lower.starts_with("pkg_check_modules(")
            || lower.starts_with("pkg_check_modules (")
        {
            let args = line
                .split_once('(')
                .map(|(_, args)| args.trim())
                .unwrap_or("");
            let mut words = args.split(|character: char| {
                character.is_whitespace() || matches!(character, ')' | ';')
            });
            let _prefix = words.next();
            for name in words.filter(|word| {
                !word.is_empty()
                    && !matches!(
                        *word,
                        "REQUIRED"
                            | "QUIET"
                            | "IMPORTED_TARGET"
                            | "NO_CMAKE_PATH"
                            | "NO_CMAKE_ENVIRONMENT_PATH"
                            | "NO_CMAKE_SYSTEM_PATH"
                            | "NO_CMAKE_PACKAGE_REGISTRY"
                            | "NO_CMAKE_BUILDS_PATH"
                            | "NO_CMAKE_SYSTEM_PACKAGE_REGISTRY"
                    )
            }) {
                output.insert((name.trim_matches('"').to_owned(), path.to_owned()));
            }
        }
    }
}

fn parse_make_dependencies(contents: &str, path: &str, output: &mut BTreeSet<(String, String)>) {
    for token in contents.split_whitespace() {
        if let Some(name) = token.strip_prefix("-l") {
            let name = name.trim_matches(|character: char| {
                !character.is_ascii_alphanumeric() && character != '_' && character != '-'
            });
            if !name.is_empty() {
                output.insert((name.to_owned(), path.to_owned()));
            }
        }
    }
}

fn scan_source(contents: &str, path: &str, scan: &mut Scan) {
    const PATTERNS: &[(&str, &[&str])] = &[
        (
            "filesystem",
            &[
                "std::fs",
                "File::open",
                "File::create",
                "fopen(",
                "open(",
                "opendir(",
            ],
        ),
        (
            "process",
            &[
                "std::process",
                "Command::new",
                "fork(",
                "execve(",
                "system(",
                "CreateProcess",
            ],
        ),
        (
            "threading",
            &["std::thread", "pthread_", "CreateThread", "std::sync::mpsc"],
        ),
        (
            "networking",
            &[
                "TcpStream",
                "TcpListener",
                "UdpSocket",
                "socket(",
                "connect(",
                "accept(",
                "WSAStartup",
            ],
        ),
        (
            "dynamic-loading",
            &["libloading", "dlopen(", "LoadLibrary(", "dlsym("],
        ),
        (
            "gui-framework",
            &[
                "QApplication",
                "QWidget",
                "gtk_",
                "GtkApplication",
                "SDL_Init",
                "electron",
                "winit::",
            ],
        ),
        (
            "os-contract",
            &[
                "libc::",
                "unistd.h",
                "sys/",
                "cfg(target_os",
                "_WIN32",
                "std::env::var",
            ],
        ),
    ];

    for (index, raw_line) in contents.lines().enumerate() {
        let lower = raw_line.to_ascii_lowercase();
        for (category, patterns) in PATTERNS {
            let matched_tokens: Vec<&str> = patterns
                .iter()
                .filter(|pattern| lower.contains(&pattern.to_ascii_lowercase()))
                .copied()
                .collect();
            if !matched_tokens.is_empty() {
                if scan.findings.len() >= MAX_FINDINGS {
                    scan.truncated = true;
                    scan.limitations.insert("The finding limit was reached.");
                    return;
                }
                scan.findings.push(Finding {
                    category,
                    path: path.to_owned(),
                    line: index + 1,
                    evidence: format!("matched tokens: {}", matched_tokens.join(", ")),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::{read_text_bounded, ReadTextError};

    #[test]
    fn bounded_reader_counts_valid_and_invalid_input_without_exceeding_limit() {
        let mut exact = Cursor::new(b"abc");
        let exact = read_text_bounded(&mut exact, 3);
        assert_eq!(exact.contents, Ok("abc".into()));
        assert_eq!(exact.bytes_read, 3);

        let mut over_limit = Cursor::new(b"abcd");
        let over_limit = read_text_bounded(&mut over_limit, 3);
        assert_eq!(over_limit.contents, Ok("abc".into()));
        assert_eq!(over_limit.bytes_read, 3);

        let mut invalid = Cursor::new(b"\xffx");
        let invalid = read_text_bounded(&mut invalid, 2);
        assert_eq!(invalid.contents, Err(ReadTextError::InvalidUtf8));
        assert_eq!(invalid.bytes_read, 2);

        let mut empty = Cursor::new(b"");
        let empty = read_text_bounded(&mut empty, 0);
        assert_eq!(empty.contents, Ok(String::new()));
        assert_eq!(empty.bytes_read, 0);
    }
}
