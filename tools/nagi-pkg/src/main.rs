use std::env;
use std::fs;
use std::path::PathBuf;

use ed25519_dalek::{Signer, SigningKey};
use nagi_package::{build_xapp, PackageView, MAX_PACKAGE_BYTES, SIGNATURE_BYTES};

fn main() {
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_else(|| usage("missing command"));
    match command.as_str() {
        "build-hello" => build_hello(&mut args, false),
        "build-signed-hello" => build_hello(&mut args, true),
        "info" => {
            let path = PathBuf::from(args.next().unwrap_or_else(|| usage("missing package path")));
            let bytes = fs::read(&path).unwrap_or_else(|error| panic!("read package: {error}"));
            let package = PackageView::parse(&bytes)
                .unwrap_or_else(|error| panic!("parse package: {error:?}"));
            println!(
                "{} {} {}",
                String::from_utf8_lossy(package.manifest().id()),
                String::from_utf8_lossy(package.manifest().version()),
                package.executable().len()
            );
        }
        _ => usage("unknown command"),
    }
}

fn usage(message: &str) -> ! {
    eprintln!("{message}\nusage: nagi-pkg build-hello <napp> [output] | build-signed-hello <napp> [output] | info <package>");
    std::process::exit(2)
}

fn build_hello(args: &mut impl Iterator<Item = String>, signed: bool) {
    let executable_path = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| usage("missing NAPP executable path"));
    let output = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("out/artifacts/hello-nagi.xapp"));
    if args.next().is_some() {
        usage("too many arguments");
    }
    let executable =
        fs::read(&executable_path).unwrap_or_else(|error| panic!("read NAPP executable: {error}"));
    if executable.len() < 8 || &executable[..4] != b"NAPP" {
        usage("sample executable is not a NAPP artifact");
    }
    let mut package = [0; MAX_PACKAGE_BYTES];
    let empty_signature = [0; SIGNATURE_BYTES];
    let length = build_xapp(
        nagi_sdk::HELLO_MANIFEST,
        &executable,
        b"",
        b"",
        b"MIT\n",
        if signed { &empty_signature } else { &[] },
        &mut package,
    )
    .unwrap_or_else(|error| usage(&format!("cannot build package: {error:?}")));
    if signed {
        let signing_key = SigningKey::from_bytes(&[
            0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec,
            0x2c, 0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03,
            0x1c, 0xae, 0x7f, 0x60,
        ]);
        let signed_region = length - SIGNATURE_BYTES;
        let signature = signing_key.sign(&package[..signed_region]);
        package[signed_region..length].copy_from_slice(&signature.to_bytes());
    }
    if let Some(parent) = output.parent() {
        fs::create_dir_all(parent)
            .unwrap_or_else(|error| panic!("create output directory: {error}"));
    }
    fs::write(&output, &package[..length]).unwrap_or_else(|error| panic!("write package: {error}"));
    let parsed = PackageView::parse(&package[..length]).expect("generated package parses");
    if parsed.manifest().app_id() != nagi_sdk::HELLO_APP_ID || (signed && !parsed.is_signed()) {
        usage("SDK/package identity or signature verification failed");
    }
    println!(
        "PASS nagi-pkg build{}: {} AppId={}",
        if signed { " signed" } else { "" },
        output.display(),
        parsed.manifest().app_id().0
    );
}
