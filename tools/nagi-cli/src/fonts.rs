//! Pinned system fonts (ADR 0055).
//!
//! `third_party/fonts.lock` pins each bundled font and license file by URL at
//! an immutable repository revision, byte size, and SHA-256. `./nagi fetch`
//! downloads missing or mismatched files into `out/cache/fonts/`, verifies
//! them, and writes `manifest.tsv` for the Servo-enabled `nagi-init` build,
//! which embeds the files and publishes them read-only at their guest paths.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use sha2::{Digest, Sha256};

pub(crate) const FONT_LOCK: &str = "third_party/fonts.lock";
pub(crate) const FONT_MANIFEST: &str = "manifest.tsv";
const GUEST_FONT_DIRECTORY: &str = "/system/fonts/";
const URL_PREFIX: &str = "https://raw.githubusercontent.com/notofonts/";
/// Guard against a lock entry that would bloat the init image.
const MAX_FONT_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FontPin {
    pub(crate) key: String,
    pub(crate) revision: String,
    pub(crate) url: String,
    pub(crate) file_name: String,
    pub(crate) size_bytes: u64,
    pub(crate) sha256: String,
    pub(crate) license: String,
    pub(crate) guest_path: String,
}

fn field(section: &str, name: &str) -> Option<String> {
    section.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        if key.trim() != name {
            return None;
        }
        let value = value.trim();
        Some(
            value
                .strip_prefix('"')
                .and_then(|value| value.strip_suffix('"'))
                .unwrap_or(value)
                .to_owned(),
        )
    })
}

fn safe_file_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('.')
        && name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_'))
}

/// Parse and validate every `[fonts.*]` entry of the lock.
pub(crate) fn parse_font_lock(contents: &str) -> Result<Vec<FontPin>, String> {
    if !contents
        .lines()
        .any(|line| line.trim() == "format_version = 1")
    {
        return Err(format!("{FONT_LOCK} must declare format_version = 1"));
    }
    let mut pins = Vec::new();
    for entry in contents.split("\n[fonts.").skip(1) {
        let (key, body) = entry
            .split_once(']')
            .ok_or_else(|| format!("{FONT_LOCK} has a malformed section header"))?;
        let body = body.split("\n[").next().unwrap_or(body);
        let require = |name: &str| {
            field(body, name).ok_or_else(|| format!("{FONT_LOCK} [fonts.{key}] is missing {name}"))
        };
        let pin = FontPin {
            key: key.to_owned(),
            revision: require("revision")?,
            url: require("url")?,
            file_name: require("file_name")?,
            size_bytes: require("size_bytes")?
                .parse()
                .map_err(|_| format!("{FONT_LOCK} [fonts.{key}] size_bytes is not a number"))?,
            sha256: require("sha256")?,
            license: require("license")?,
            guest_path: require("guest_path")?,
        };
        if pin.revision.len() != 40 || !pin.revision.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(format!(
                "{FONT_LOCK} [fonts.{key}] revision is not a full commit"
            ));
        }
        // License texts are renamed in the cache; fonts keep their name.
        let expected_tail = if pin.file_name.ends_with(".txt") {
            String::from("/LICENSE")
        } else {
            format!("/{}", pin.file_name)
        };
        if !pin.url.starts_with(URL_PREFIX)
            || !pin.url.contains(&format!("/{}/", pin.revision))
            || !pin.url.ends_with(&expected_tail)
        {
            return Err(format!(
                "{FONT_LOCK} [fonts.{key}] url must be a Noto raw URL at the pinned revision"
            ));
        }
        if !safe_file_name(&pin.file_name) {
            return Err(format!(
                "{FONT_LOCK} [fonts.{key}] file_name is not a plain file name"
            ));
        }
        if pin.size_bytes == 0 || pin.size_bytes > MAX_FONT_BYTES {
            return Err(format!(
                "{FONT_LOCK} [fonts.{key}] size_bytes is out of range"
            ));
        }
        if pin.sha256.len() != 64
            || !pin
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(format!(
                "{FONT_LOCK} [fonts.{key}] sha256 is not a lowercase digest"
            ));
        }
        if pin.license != "OFL-1.1" {
            return Err(format!("{FONT_LOCK} [fonts.{key}] license must be OFL-1.1"));
        }
        if pin.guest_path != format!("{GUEST_FONT_DIRECTORY}{}", pin.file_name) {
            return Err(format!(
                "{FONT_LOCK} [fonts.{key}] guest_path must be {GUEST_FONT_DIRECTORY}<file_name>"
            ));
        }
        if pins
            .iter()
            .any(|other: &FontPin| other.file_name == pin.file_name)
        {
            return Err(format!("{FONT_LOCK} repeats file_name {}", pin.file_name));
        }
        pins.push(pin);
    }
    if pins.is_empty() {
        return Err(format!("{FONT_LOCK} pins no fonts"));
    }
    Ok(pins)
}

fn file_sha256(path: &Path) -> Result<(u64, String), String> {
    let mut file =
        fs::File::open(path).map_err(|error| format!("cannot open {}: {error}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut length = 0_u64;
    loop {
        let count = file
            .read(&mut buffer)
            .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
        if count == 0 {
            break;
        }
        length += count as u64;
        hasher.update(&buffer[..count]);
    }
    let digest = hasher.finalize();
    let hex = digest.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok((length, hex))
}

/// Check that `path` holds exactly the pinned bytes.
pub(crate) fn verify_font_file(path: &Path, pin: &FontPin) -> Result<(), String> {
    let (length, digest) = file_sha256(path)?;
    if length != pin.size_bytes {
        return Err(format!(
            "{} is {length} bytes, expected {}",
            path.display(),
            pin.size_bytes
        ));
    }
    if digest != pin.sha256 {
        return Err(format!(
            "{} has SHA-256 {digest}, expected {}",
            path.display(),
            pin.sha256
        ));
    }
    Ok(())
}

fn download(pin: &FontPin, destination: &Path) -> Result<(), String> {
    let output = ProcessCommand::new("curl")
        .args(["--fail", "--silent", "--show-error", "--location"])
        .args(["--retry", "3", "--proto", "=https", "--output"])
        .arg(destination)
        .arg(&pin.url)
        .output()
        .map_err(|error| format!("cannot start curl for {}: {error}", pin.file_name))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "curl could not download {}: {}",
            pin.url,
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

/// Ensure every pinned font is present and verified in `out/cache/fonts/`,
/// and write the manifest the init build reads. Returns the cache directory.
pub(crate) fn ensure_font_cache(root: &Path) -> Result<PathBuf, String> {
    let lock = fs::read_to_string(root.join(FONT_LOCK))
        .map_err(|error| format!("cannot read {FONT_LOCK}: {error}"))?;
    let pins = parse_font_lock(&lock)?;
    let cache = root.join("out").join("cache").join("fonts");
    fs::create_dir_all(&cache)
        .map_err(|error| format!("cannot create font cache {}: {error}", cache.display()))?;
    let mut manifest = String::new();
    for pin in &pins {
        let path = cache.join(&pin.file_name);
        if verify_font_file(&path, pin).is_err() {
            let partial = cache.join(format!("{}.partial", pin.file_name));
            let _ = fs::remove_file(&partial);
            download(pin, &partial)?;
            if let Err(error) = verify_font_file(&partial, pin) {
                let _ = fs::remove_file(&partial);
                return Err(format!("downloaded font rejected: {error}"));
            }
            fs::rename(&partial, &path)
                .map_err(|error| format!("cannot install font {}: {error}", path.display()))?;
        }
        manifest.push_str(&format!(
            "{}\t{}\t{}\t{}\n",
            pin.guest_path, pin.file_name, pin.size_bytes, pin.sha256
        ));
    }
    fs::write(cache.join(FONT_MANIFEST), manifest)
        .map_err(|error| format!("cannot write font manifest in {}: {error}", cache.display()))?;
    Ok(cache)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lock_with(entry: &str) -> String {
        format!("format_version = 1\n\n[fonts.sample]\n{entry}")
    }

    const VALID: &str = r#"revision = "86eb2ddc3a2e97cb9747fd9069ee5d47880e3305"
url = "https://raw.githubusercontent.com/notofonts/notofonts.github.io/86eb2ddc3a2e97cb9747fd9069ee5d47880e3305/fonts/NotoSans/hinted/ttf/NotoSans-Regular.ttf"
file_name = "NotoSans-Regular.ttf"
size_bytes = 3
sha256 = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
license = "OFL-1.1"
guest_path = "/system/fonts/NotoSans-Regular.ttf"
"#;

    #[test]
    fn repository_font_lock_is_valid() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let lock = fs::read_to_string(root.join(FONT_LOCK)).expect("font lock");
        let pins = parse_font_lock(&lock).expect("valid font lock");
        let names: Vec<_> = pins.iter().map(|pin| pin.file_name.as_str()).collect();
        assert!(names.contains(&"NotoSans-Regular.ttf"));
        assert!(names.contains(&"NotoSansJP-Regular.otf"));
        assert!(names.contains(&"OFL-NotoSans.txt"));
        assert!(names.contains(&"OFL-NotoSansCJK.txt"));
    }

    #[test]
    fn rejects_unsafe_or_unpinned_entries() {
        assert!(parse_font_lock(&lock_with(VALID)).is_ok());
        for (from, to) in [
            (
                "86eb2ddc3a2e97cb9747fd9069ee5d47880e3305/fonts",
                "main/fonts",
            ),
            ("\"NotoSans-Regular.ttf\"\nsize", "\"../x.ttf\"\nsize"),
            ("ba7816bf", "BA7816BF"),
            ("OFL-1.1", "GPL-3.0"),
            (
                "/system/fonts/NotoSans-Regular.ttf",
                "/tmp/NotoSans-Regular.ttf",
            ),
            ("size_bytes = 3", "size_bytes = 0"),
            (
                "https://raw.githubusercontent.com/notofonts/",
                "http://example.com/",
            ),
        ] {
            let entry = VALID.replacen(from, to, 1);
            assert!(
                parse_font_lock(&lock_with(&entry)).is_err(),
                "{from} -> {to}"
            );
        }
    }

    #[test]
    fn verifies_size_and_digest() {
        let pins = parse_font_lock(&lock_with(VALID)).unwrap();
        let directory = std::env::temp_dir().join(format!("nagi-font-test-{}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let path = directory.join("font");
        fs::write(&path, b"abc").unwrap();
        assert_eq!(verify_font_file(&path, &pins[0]), Ok(()));
        fs::write(&path, b"abd").unwrap();
        assert!(verify_font_file(&path, &pins[0])
            .unwrap_err()
            .contains("SHA-256"));
        fs::write(&path, b"abcd").unwrap();
        assert!(verify_font_file(&path, &pins[0])
            .unwrap_err()
            .contains("bytes"));
        fs::remove_dir_all(directory).unwrap();
    }
}
