//! Compiler discovery without building packages or executing dependency scripts.
use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::{
        fs,
        sync::atomic::{AtomicUsize, Ordering},
    };
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    #[test]
    #[ignore = "isolated selector fixture child"]
    fn selector_child() {
        let root = PathBuf::from(env::var_os("SELECTOR_ROOT").unwrap());
        let selected = selected_rustc(&root).unwrap();
        assert_eq!(
            selected,
            PathBuf::from(env::var_os("SELECTOR_EXPECTED").unwrap())
        );
    }
    #[test]
    fn windows_suffix_rules_match_pinned_rust() {
        for (input, explicit, expected) in [
            ("rustc", true, vec!["rustc.exe", "rustc"]),
            (
                "rustc.custom",
                true,
                vec!["rustc.custom.exe", "rustc.custom"],
            ),
            ("rustc.EXE", true, vec!["rustc.EXE"]),
            ("rustc", false, vec!["rustc.exe"]),
            ("rustc.custom", false, vec!["rustc.custom"]),
        ] {
            assert_eq!(
                windows_program_candidates(PathBuf::from(input), explicit),
                expected.into_iter().map(PathBuf::from).collect::<Vec<_>>()
            );
        }
    }
    #[test]
    fn nonexecutable_path_entry_is_skipped_and_spaces_are_preserved() {
        let root = env::temp_dir().join(format!(
            "compiler-access-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("blocked")).unwrap();
        fs::create_dir_all(root.join("compiler dir")).unwrap();
        fs::write(root.join("blocked/rustc"), b"not executable").unwrap();
        let compiler = root.join("compiler dir/rustc");
        fs::copy("/bin/true", &compiler).unwrap();
        for selector in [
            std::ffi::OsString::from("rustc"),
            compiler.as_os_str().to_owned(),
        ] {
            let output = Command::new(env::current_exe().unwrap())
                .args([
                    "--exact",
                    "compiler_selector::tests::selector_child",
                    "--ignored",
                ])
                .env(
                    "PATH",
                    env::join_paths([root.join("blocked"), root.join("compiler dir")]).unwrap(),
                )
                .env("RUSTC", selector)
                .env("SELECTOR_ROOT", &root)
                .env("SELECTOR_EXPECTED", &compiler)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        assert!(Command::new(&compiler)
            .arg("--version")
            .status()
            .unwrap()
            .success());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn cargo_selector_precedence_and_origins() {
        let root = env::temp_dir().join(format!(
            "compiler-select-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir_all(root.join("repo/.cargo")).unwrap();
        fs::create_dir_all(root.join("repo/nested")).unwrap();
        fs::create_dir_all(root.join("home")).unwrap();
        fs::write(
            root.join("repo/.cargo/config.toml"),
            "[build]\nrustc=\"./repo-compiler\"\n",
        )
        .unwrap();
        fs::write(
            root.join("home/config.toml"),
            "[build]\nrustc=\"./home-compiler\"\n",
        )
        .unwrap();
        for path in [
            "repo/repo-compiler",
            "repo/env-compiler",
            "repo/rustc-explicit",
            "home-compiler",
            "home/rustc",
        ] {
            fs::copy("/bin/true", root.join(path)).unwrap();
        }
        let cases = [
            ("repo", None, None, root.join("repo/./repo-compiler")),
            ("repo/nested", None, None, root.join("repo/./repo-compiler")),
            (
                "repo",
                None,
                Some("./env-compiler"),
                root.join("repo/./env-compiler"),
            ),
            (
                "repo",
                Some("./rustc-explicit"),
                Some("./env-compiler"),
                root.join("repo/./rustc-explicit"),
            ),
            ("home", None, None, root.join("./home-compiler")),
        ];
        for (cwd, rustc, build_rustc, expected) in cases {
            let mut cmd = Command::new(env::current_exe().unwrap());
            cmd.args([
                "--exact",
                "compiler_selector::tests::selector_child",
                "--ignored",
            ])
            .current_dir(root.join("repo/nested"))
            .env_remove("RUSTC")
            .env_remove("CARGO_BUILD_RUSTC")
            .env("CARGO_HOME", root.join("home"))
            .env("SELECTOR_ROOT", root.join(cwd))
            .env("SELECTOR_EXPECTED", expected);
            if let Some(v) = rustc {
                cmd.env("RUSTC", v);
            }
            if let Some(v) = build_rustc {
                cmd.env("CARGO_BUILD_RUSTC", v);
            }
            let output = cmd.output().unwrap();
            assert!(
                output.status.success(),
                "{}{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        }
        fs::remove_file(root.join("home/config.toml")).unwrap();
        let output = Command::new(env::current_exe().unwrap())
            .args([
                "--exact",
                "compiler_selector::tests::selector_child",
                "--ignored",
            ])
            .env_remove("RUSTC")
            .env_remove("CARGO_BUILD_RUSTC")
            .env("CARGO_HOME", root.join("home"))
            .env("SELECTOR_ROOT", root.join("home"))
            .env("PATH", root.join("home"))
            .env("SELECTOR_EXPECTED", root.join("home/rustc"))
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        fs::remove_dir_all(root).unwrap();
    }
}

pub(crate) fn selected_rustc(root: &Path) -> Result<PathBuf, String> {
    for name in ["RUSTC", "CARGO_BUILD_RUSTC"] {
        if let Some(value) = env::var_os(name) {
            return resolve_program(root, Path::new(&value));
        }
    }
    let cargo = env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let query = |format: &str, origin: bool| {
        let mut command = Command::new(&cargo);
        command.current_dir(root).args([
            "-Z",
            "unstable-options",
            "config",
            "get",
            "build.rustc",
            "--offline",
            "--format",
            format,
        ]);
        if origin {
            command.arg("--show-origin");
        }
        command
            .output()
            .map_err(|_| "cannot query Cargo compiler configuration".to_string())
    };
    // The project pins nightly Cargo. Fail closed if its config query is not
    // supported; never interpret a malformed configuration as PATH fallback.
    let output = query("json", false)?;
    if !output.status.success() {
        if String::from_utf8_lossy(&output.stderr).contains("config value `build.rustc` is not set")
        {
            return resolve_program(root, Path::new("rustc"));
        }
        return Err("cannot resolve Cargo build.rustc configuration".into());
    }
    let document: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|_| "invalid Cargo compiler configuration response")?;
    let value = document
        .get("build")
        .and_then(|v| v.get("rustc"))
        .and_then(|v| v.as_str())
        .ok_or("Cargo compiler configuration is not a string")?;
    let path = PathBuf::from(value);
    if path.is_absolute() || !value.contains(['/', '\\']) {
        return resolve_program(root, &path);
    }
    let output = query("toml", true)?;
    if !output.status.success() {
        return Err("cannot resolve Cargo compiler configuration origin".into());
    }
    let text =
        std::str::from_utf8(&output.stdout).map_err(|_| "invalid Cargo configuration origin")?;
    if text.lines().count() != 1 {
        return Err("ambiguous Cargo compiler configuration origin".into());
    }
    let (_, origin) = text
        .trim_end()
        .rsplit_once(" # ")
        .ok_or("missing Cargo compiler configuration origin")?;
    let origin = Path::new(origin);
    if !origin.is_absolute() {
        return Err("Cargo compiler configuration origin is not absolute".into());
    }
    let base = origin
        .parent()
        .and_then(Path::parent)
        .ok_or("missing Cargo configuration parent")?;
    resolve_program(root, &base.join(path))
}

// Execute and hash exactly this concrete executable; this is not provenance attestation.
fn resolve_program(root: &Path, program: &Path) -> Result<PathBuf, String> {
    let explicit = program.is_absolute() || program.components().count() > 1;
    let mut directories = Vec::new();
    #[cfg(windows)]
    if !explicit {
        if let Ok(exe) = env::current_exe() {
            if let Some(parent) = exe.parent() {
                directories.push(parent.to_path_buf());
            }
        }
        if let Some(system) = env::var_os("SystemRoot") {
            directories.push(PathBuf::from(&system).join("System32"));
            directories.push(PathBuf::from(system));
        }
    }
    directories.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
    let candidates: Vec<_> = if explicit {
        vec![if program.is_absolute() {
            program.to_path_buf()
        } else {
            root.join(program)
        }]
    } else {
        directories
            .into_iter()
            .map(|dir| {
                let dir = if dir.is_absolute() {
                    dir
                } else {
                    root.join(dir)
                };
                dir.join(program)
            })
            .collect()
    };
    #[cfg(windows)]
    let candidates: Vec<_> = candidates
        .into_iter()
        .flat_map(|path| windows_program_candidates(path, explicit))
        .collect();
    for path in candidates {
        if !path.is_file() {
            continue;
        }
        #[cfg(unix)]
        {
            let status = Command::new("/bin/test")
                .arg("-x")
                .arg(&path)
                .status()
                .map_err(|_| "cannot inspect compiler executable access")?;
            if !status.success() {
                continue;
            }
        }
        return Ok(path);
    }
    Err("selected Cargo compiler executable is unavailable".into())
}

#[cfg(any(windows, test))]
fn windows_program_candidates(path: PathBuf, explicit: bool) -> Vec<PathBuf> {
    let exe_suffix = path
        .extension()
        .is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case("exe"));
    if explicit && !exe_suffix {
        let mut extended = path.clone().into_os_string();
        extended.push(".exe");
        vec![PathBuf::from(extended), path]
    } else if !explicit
        && !path
            .file_name()
            .unwrap_or_default()
            .as_encoded_bytes()
            .contains(&b'.')
    {
        vec![path.with_extension("exe")]
    } else {
        vec![path]
    }
}
