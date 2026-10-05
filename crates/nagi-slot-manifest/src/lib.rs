//! Authenticated system-slot manifests (ADR 0054).
//!
//! Every System A, System B and Recovery volume carries `SLOT.MAN`: a small
//! text manifest followed by a 64-byte Ed25519 signature. The manifest pins
//! the SHA-256 digest and size of the slot's `KERNEL.ELF` and `INIT.ELF`, a
//! version label, and a rollback index. The loader verifies the signature
//! against the pinned Developer Preview signer before it trusts any payload
//! byte, then checks each payload against its digest. A trial slot must also
//! not lower the confirmed slot's rollback index.
//!
//! ```text
//! nagi-slot-manifest 1
//! version=0.1.0
//! rollback-index=1
//! kernel-sha256=<64 lowercase hex digits>
//! kernel-size=<decimal bytes>
//! init-sha256=<64 lowercase hex digits>
//! init-size=<decimal bytes>
//! ```
//!
//! The signature covers `SIGNATURE_DOMAIN || text`, so a slot manifest
//! signature can never be replayed as an application package signature or
//! the reverse, even though both use the same Developer Preview key.
//!
//! The crate is `no_std`, allocation-free, and host-testable.

#![no_std]

use ed25519_dalek::{Signature, VerifyingKey};
use sha2::{Digest, Sha256};

/// File name of the manifest at the root of each slot volume.
pub const SLOT_MANIFEST_FILE: &str = "SLOT.MAN";
pub const SIGNATURE_BYTES: usize = 64;
/// Upper bound of the manifest text.
pub const MAX_MANIFEST_TEXT: usize = 512;
pub const MAX_SLOT_MANIFEST_BYTES: usize = MAX_MANIFEST_TEXT + SIGNATURE_BYTES;
pub const MAX_VERSION_BYTES: usize = 32;
/// Domain separator prepended to the signed text.
pub const SIGNATURE_DOMAIN: &[u8] = b"nagi-slot-manifest-v1\0";
const HEADER: &[u8] = b"nagi-slot-manifest 1";

/// RFC 8032 test-vector public key: the pinned Developer Preview signer,
/// the same key that signs M16 packages (`nagi-package`). Production trust
/// provisioning is later work.
pub const TRUSTED_SLOT_SIGNING_PUBLIC_KEY: [u8; 32] = [
    0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64, 0x07, 0x3a,
    0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0x1a, 0x68, 0xf7, 0x07, 0x51, 0x1a,
];

/// RFC 8032 test-vector secret for `TRUSTED_SLOT_SIGNING_PUBLIC_KEY`. It is
/// public test data, used only so Developer Preview images are reproducible.
#[cfg(feature = "sign")]
pub const DEVELOPER_PREVIEW_SIGNING_SECRET: [u8; 32] = [
    0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c, 0xc4,
    0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae, 0x7f, 0x60,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManifestError {
    /// Shorter than a signature plus the header, or longer than the bound.
    InvalidLength,
    /// The signature does not verify against the trusted key.
    Signature,
    /// The signed text does not follow the format exactly.
    Malformed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PayloadKind {
    Kernel,
    Init,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PayloadError {
    Size,
    Digest,
}

/// A payload's pinned size and SHA-256 digest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PayloadDigest {
    pub size: u64,
    pub sha256: [u8; 32],
}

impl PayloadDigest {
    pub fn of(bytes: &[u8]) -> Self {
        Self {
            size: bytes.len() as u64,
            sha256: Sha256::digest(bytes).into(),
        }
    }

    /// Check `bytes` against this digest.
    pub fn check(&self, bytes: &[u8]) -> Result<(), PayloadError> {
        if bytes.len() as u64 != self.size {
            return Err(PayloadError::Size);
        }
        if Self::of(bytes).sha256 != self.sha256 {
            return Err(PayloadError::Digest);
        }
        Ok(())
    }
}

/// A verified slot manifest.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SlotManifest {
    version: [u8; MAX_VERSION_BYTES],
    version_length: u8,
    pub rollback_index: u64,
    pub kernel: PayloadDigest,
    pub init: PayloadDigest,
}

impl SlotManifest {
    pub fn new(
        version: &[u8],
        rollback_index: u64,
        kernel: PayloadDigest,
        init: PayloadDigest,
    ) -> Result<Self, ManifestError> {
        if !valid_version(version) {
            return Err(ManifestError::Malformed);
        }
        let mut stored = [0; MAX_VERSION_BYTES];
        stored[..version.len()].copy_from_slice(version);
        Ok(Self {
            version: stored,
            version_length: version.len() as u8,
            rollback_index,
            kernel,
            init,
        })
    }

    pub fn version(&self) -> &[u8] {
        &self.version[..self.version_length as usize]
    }

    pub const fn payload(&self, kind: PayloadKind) -> PayloadDigest {
        match kind {
            PayloadKind::Kernel => self.kernel,
            PayloadKind::Init => self.init,
        }
    }

    /// Verify `file` (text followed by its signature) against `public_key`
    /// and parse the signed text. Nothing in the text is trusted before the
    /// signature verifies.
    pub fn verify(file: &[u8], public_key: &[u8; 32]) -> Result<Self, ManifestError> {
        if file.len() <= SIGNATURE_BYTES || file.len() > MAX_SLOT_MANIFEST_BYTES {
            return Err(ManifestError::InvalidLength);
        }
        let (text, signature) = file.split_at(file.len() - SIGNATURE_BYTES);
        let key = VerifyingKey::from_bytes(public_key).map_err(|_| ManifestError::Signature)?;
        let signature = Signature::from_slice(signature).map_err(|_| ManifestError::Signature)?;
        let mut message = [0; SIGNATURE_DOMAIN.len() + MAX_MANIFEST_TEXT];
        message[..SIGNATURE_DOMAIN.len()].copy_from_slice(SIGNATURE_DOMAIN);
        message[SIGNATURE_DOMAIN.len()..SIGNATURE_DOMAIN.len() + text.len()].copy_from_slice(text);
        key.verify_strict(&message[..SIGNATURE_DOMAIN.len() + text.len()], &signature)
            .map_err(|_| ManifestError::Signature)?;
        Self::parse_text(text)
    }

    fn parse_text(text: &[u8]) -> Result<Self, ManifestError> {
        let malformed = ManifestError::Malformed;
        let text = text.strip_suffix(b"\n").ok_or(malformed)?;
        let mut lines = text.split(|byte| *byte == b'\n');
        if lines.next() != Some(HEADER) {
            return Err(malformed);
        }
        let mut field = |key: &[u8]| -> Result<&[u8], ManifestError> {
            let line = lines.next().ok_or(malformed)?;
            line.strip_prefix(key)
                .and_then(|rest| rest.strip_prefix(b"="))
                .ok_or(malformed)
        };
        let version = field(b"version")?;
        let rollback_index = parse_decimal(field(b"rollback-index")?).ok_or(malformed)?;
        let kernel_sha256 = parse_digest(field(b"kernel-sha256")?).ok_or(malformed)?;
        let kernel_size = parse_decimal(field(b"kernel-size")?).ok_or(malformed)?;
        let init_sha256 = parse_digest(field(b"init-sha256")?).ok_or(malformed)?;
        let init_size = parse_decimal(field(b"init-size")?).ok_or(malformed)?;
        if lines.next().is_some() || kernel_size == 0 || init_size == 0 {
            return Err(malformed);
        }
        Self::new(
            version,
            rollback_index,
            PayloadDigest {
                size: kernel_size,
                sha256: kernel_sha256,
            },
            PayloadDigest {
                size: init_size,
                sha256: init_sha256,
            },
        )
    }

    /// Write the manifest text into `output` and return its length.
    pub fn encode_text(&self, output: &mut [u8; MAX_MANIFEST_TEXT]) -> usize {
        let mut writer = Writer { output, length: 0 };
        writer.put(HEADER);
        writer.put(b"\nversion=");
        writer.put(self.version());
        writer.put(b"\nrollback-index=");
        writer.decimal(self.rollback_index);
        writer.put(b"\nkernel-sha256=");
        writer.hex(&self.kernel.sha256);
        writer.put(b"\nkernel-size=");
        writer.decimal(self.kernel.size);
        writer.put(b"\ninit-sha256=");
        writer.hex(&self.init.sha256);
        writer.put(b"\ninit-size=");
        writer.decimal(self.init.size);
        writer.put(b"\n");
        writer.length
    }

    /// Encode and sign the manifest with `secret` into `output`, returning
    /// the file length.
    #[cfg(feature = "sign")]
    pub fn encode_signed(
        &self,
        secret: &[u8; 32],
        output: &mut [u8; MAX_SLOT_MANIFEST_BYTES],
    ) -> usize {
        use ed25519_dalek::{Signer, SigningKey};
        let mut text = [0; MAX_MANIFEST_TEXT];
        let length = self.encode_text(&mut text);
        let mut message = [0; SIGNATURE_DOMAIN.len() + MAX_MANIFEST_TEXT];
        message[..SIGNATURE_DOMAIN.len()].copy_from_slice(SIGNATURE_DOMAIN);
        message[SIGNATURE_DOMAIN.len()..SIGNATURE_DOMAIN.len() + length]
            .copy_from_slice(&text[..length]);
        let signature = SigningKey::from_bytes(secret)
            .sign(&message[..SIGNATURE_DOMAIN.len() + length])
            .to_bytes();
        output[..length].copy_from_slice(&text[..length]);
        output[length..length + SIGNATURE_BYTES].copy_from_slice(&signature);
        length + SIGNATURE_BYTES
    }
}

/// Whether a trial slot may replace the confirmed slot: its rollback index
/// must not be lower than the confirmed one.
pub const fn permits_trial(confirmed: &SlotManifest, candidate: &SlotManifest) -> bool {
    candidate.rollback_index >= confirmed.rollback_index
}

fn valid_version(version: &[u8]) -> bool {
    !version.is_empty()
        && version.len() <= MAX_VERSION_BYTES
        && version
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'))
}

fn parse_decimal(digits: &[u8]) -> Option<u64> {
    if digits.is_empty() || digits.len() > 20 || (digits.len() > 1 && digits[0] == b'0') {
        return None;
    }
    digits.iter().try_fold(0_u64, |value, digit| {
        if !digit.is_ascii_digit() {
            return None;
        }
        value.checked_mul(10)?.checked_add(u64::from(digit - b'0'))
    })
}

fn parse_digest(digits: &[u8]) -> Option<[u8; 32]> {
    if digits.len() != 64 {
        return None;
    }
    let mut digest = [0; 32];
    for (index, pair) in digits.chunks_exact(2).enumerate() {
        digest[index] = nibble(pair[0])? << 4 | nibble(pair[1])?;
    }
    Some(digest)
}

const fn nibble(digit: u8) -> Option<u8> {
    match digit {
        b'0'..=b'9' => Some(digit - b'0'),
        b'a'..=b'f' => Some(digit - b'a' + 10),
        _ => None,
    }
}

struct Writer<'a> {
    output: &'a mut [u8; MAX_MANIFEST_TEXT],
    length: usize,
}

impl Writer<'_> {
    fn put(&mut self, bytes: &[u8]) {
        self.output[self.length..self.length + bytes.len()].copy_from_slice(bytes);
        self.length += bytes.len();
    }

    fn hex(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.put(&[
                b"0123456789abcdef"[usize::from(byte >> 4)],
                b"0123456789abcdef"[usize::from(byte & 0xf)],
            ]);
        }
    }

    fn decimal(&mut self, mut value: u64) {
        let mut digits = [0; 20];
        let mut count = 0;
        loop {
            digits[count] = b'0' + (value % 10) as u8;
            count += 1;
            value /= 10;
            if value == 0 {
                break;
            }
        }
        digits[..count].reverse();
        self.put(&digits[..count]);
    }
}

#[cfg(all(test, feature = "sign"))]
mod tests {
    use super::*;

    const OTHER_SECRET: [u8; 32] = [7; 32];

    fn manifest(rollback_index: u64) -> SlotManifest {
        SlotManifest::new(
            b"0.1.0",
            rollback_index,
            PayloadDigest::of(b"kernel bytes"),
            PayloadDigest::of(b"init bytes"),
        )
        .expect("manifest")
    }

    fn signed(
        manifest: &SlotManifest,
        secret: &[u8; 32],
    ) -> ([u8; MAX_SLOT_MANIFEST_BYTES], usize) {
        let mut file = [0; MAX_SLOT_MANIFEST_BYTES];
        let length = manifest.encode_signed(secret, &mut file);
        (file, length)
    }

    #[test]
    fn signed_manifests_round_trip_and_pin_payloads() {
        let original = manifest(3);
        let (file, length) = signed(&original, &DEVELOPER_PREVIEW_SIGNING_SECRET);
        let verified = SlotManifest::verify(&file[..length], &TRUSTED_SLOT_SIGNING_PUBLIC_KEY)
            .expect("verify");
        assert_eq!(verified, original);
        assert_eq!(verified.version(), b"0.1.0");
        assert!(verified.kernel.check(b"kernel bytes").is_ok());
        assert_eq!(
            verified.kernel.check(b"kernel bytez"),
            Err(PayloadError::Digest)
        );
        assert_eq!(verified.init.check(b"init"), Err(PayloadError::Size));
        let text = core::str::from_utf8(&file[..length - SIGNATURE_BYTES]).expect("utf8");
        assert!(text.starts_with("nagi-slot-manifest 1\nversion=0.1.0\nrollback-index=3\n"));
    }

    #[test]
    fn untrusted_or_altered_manifests_are_rejected() {
        let (file, length) = signed(&manifest(1), &OTHER_SECRET);
        assert_eq!(
            SlotManifest::verify(&file[..length], &TRUSTED_SLOT_SIGNING_PUBLIC_KEY),
            Err(ManifestError::Signature)
        );
        let (mut file, length) = signed(&manifest(1), &DEVELOPER_PREVIEW_SIGNING_SECRET);
        // Raise the rollback index in the text without re-signing.
        let position = file[..length]
            .windows(16)
            .position(|window| window == b"rollback-index=1")
            .expect("field");
        file[position + 15] = b'9';
        assert_eq!(
            SlotManifest::verify(&file[..length], &TRUSTED_SLOT_SIGNING_PUBLIC_KEY),
            Err(ManifestError::Signature)
        );
        for bytes in [
            &[][..],
            &file[..SIGNATURE_BYTES],
            &[0; MAX_SLOT_MANIFEST_BYTES + 1],
        ] {
            assert_eq!(
                SlotManifest::verify(bytes, &TRUSTED_SLOT_SIGNING_PUBLIC_KEY),
                Err(ManifestError::InvalidLength)
            );
        }
    }

    #[test]
    fn signatures_are_domain_separated() {
        // A signature over the bare text (as a package signature would be)
        // does not verify as a slot manifest.
        use ed25519_dalek::{Signer, SigningKey};
        let mut text = [0; MAX_MANIFEST_TEXT];
        let length = manifest(1).encode_text(&mut text);
        let signature = SigningKey::from_bytes(&DEVELOPER_PREVIEW_SIGNING_SECRET)
            .sign(&text[..length])
            .to_bytes();
        let mut file = [0; MAX_SLOT_MANIFEST_BYTES];
        file[..length].copy_from_slice(&text[..length]);
        file[length..length + SIGNATURE_BYTES].copy_from_slice(&signature);
        assert_eq!(
            SlotManifest::verify(
                &file[..length + SIGNATURE_BYTES],
                &TRUSTED_SLOT_SIGNING_PUBLIC_KEY
            ),
            Err(ManifestError::Signature)
        );
    }

    #[test]
    fn strict_text_parsing() {
        let good = manifest(1);
        let mut text = [0; MAX_MANIFEST_TEXT];
        let length = good.encode_text(&mut text);
        assert_eq!(SlotManifest::parse_text(&text[..length]), Ok(good));
        assert_eq!(
            SlotManifest::parse_text(&text[..length - 1]),
            Err(ManifestError::Malformed)
        );
        let mut extra = [0; MAX_MANIFEST_TEXT];
        extra[..length].copy_from_slice(&text[..length]);
        extra[length..length + 4].copy_from_slice(b"x=1\n");
        assert_eq!(
            SlotManifest::parse_text(&extra[..length + 4]),
            Err(ManifestError::Malformed)
        );
        for bad in [&b"01"[..], b"", b"1a", b"99999999999999999999"] {
            assert_eq!(parse_decimal(bad), None);
        }
        assert_eq!(parse_decimal(b"0"), Some(0));
        assert!(SlotManifest::new(b"bad version", 1, good.kernel, good.init).is_err());
        assert!(parse_digest(&[b'A'; 64]).is_none());
    }

    #[test]
    fn trials_may_not_lower_the_rollback_index() {
        assert!(permits_trial(&manifest(2), &manifest(2)));
        assert!(permits_trial(&manifest(2), &manifest(3)));
        assert!(!permits_trial(&manifest(2), &manifest(1)));
    }
}
