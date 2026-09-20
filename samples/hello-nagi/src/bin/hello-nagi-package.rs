use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let mut args = env::args().skip(1);
    let output = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("out/artifacts/hello-nagi.napp"));
    if args.next().is_some() {
        panic!("usage: hello-nagi-package [output]");
    }
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent).expect("create output directory");
    }
    fs::write(&output, hello_nagi::package_executable()).expect("write NAPP executable");
    println!("PASS hello-nagi SDK artifact: {}", output.display());
}
