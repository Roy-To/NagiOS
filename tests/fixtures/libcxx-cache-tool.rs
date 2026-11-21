//! Native fixture-only tar/curl shim, compiled with the installed host rustc.
//! All paths and controls belong to one temporary fixture; no network is used.
use std::{env, fs, path::PathBuf, process::Command, thread, time::Duration};

fn main() {
    let root = PathBuf::from(env::var_os("FIXTURE_ROOT").expect("fixture root"));
    let executable = env::current_exe().expect("fixture executable");
    if executable.file_stem().unwrap() == "curl" {
        eprintln!("fixture-curl-blocked");
        std::process::exit(91);
    }
    if root.join("fail-tar").exists() {
        eprintln!("fixture-tar-failed");
        std::process::exit(7);
    }
    if root.join("block-tar").exists() && fs::create_dir(root.join("first-tar")).is_ok() {
        fs::write(root.join("tar-entered"), b"entered").unwrap();
        for _ in 0..500 {
            if root.join("release-tar").exists() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        if !root.join("release-tar").exists() {
            std::process::exit(8);
        }
    }
    let status = Command::new(env::var_os("FIXTURE_REAL_TAR").expect("real tar path"))
        .args(env::args_os().skip(1))
        .status()
        .expect("execute real tar");
    let worker = env::var("FIXTURE_WORKER").expect("fixture worker");
    fs::write(root.join(format!("tar-finished-{worker}")), b"finished").unwrap();
    std::process::exit(status.code().unwrap_or(1));
}
