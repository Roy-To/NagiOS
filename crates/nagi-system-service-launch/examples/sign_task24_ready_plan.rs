//! Author a Task24 development READY plan with the boot catalog signer.

use ed25519_dalek::SigningKey;
use nagi_system_service_launch::{
    BootReadyRoute, VerifiedBootCatalog, MAX_SIGNED_BOOT_CATALOG_BYTES,
};
use std::env;
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;

fn read_bounded(path: &Path, limit: usize) -> io::Result<Vec<u8>> {
    let file = File::open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "expected a regular file",
        ));
    }
    let mut bytes = Vec::new();
    file.take((limit + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > limit {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "input exceeds its size limit",
        ));
    }
    Ok(bytes)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = env::args_os().skip(1).collect();
    let [catalog_file, public_key_file, route_file, output_file] = args.as_slice() else {
        return Err(concat!(
            "usage: sign_task24_ready_plan <signed-catalog> <pinned-public-key> ",
            "<identity-route.json> <new-output> < signer-seed"
        )
        .into());
    };
    let public_key: [u8; 32] = read_bounded(Path::new(public_key_file), 32)?
        .try_into()
        .map_err(|_| "pinned catalog public key must contain exactly 32 raw bytes")?;
    let catalog = VerifiedBootCatalog::verify(
        &read_bounded(Path::new(catalog_file), MAX_SIGNED_BOOT_CATALOG_BYTES)?,
        &public_key,
    )
    .map_err(|error| format!("catalog verification failed ({error:?})"))?;
    let route: BootReadyRoute =
        serde_json::from_slice(&read_bounded(Path::new(route_file), 1024)?)?;
    let mut seed = [0_u8; 32];
    let mut stdin = io::stdin().lock();
    stdin.read_exact(&mut seed)?;
    if stdin.read(&mut [0_u8; 1])? != 0 {
        return Err("catalog signer seed must contain exactly 32 raw bytes".into());
    }
    let signer = SigningKey::from_bytes(&seed);
    seed.fill(0);
    let plan = catalog
        .sign_development_identity_ready_plan(&signer, route)
        .map_err(|error| format!("READY plan signing failed ({error:?})"))?;
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .mode(0o600)
        .open(output_file)?;
    output.write_all(&plan)?;
    output.sync_all()?;
    Ok(())
}
