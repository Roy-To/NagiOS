//! Persisted local account credentials (ADR 0063).
//!
//! A password is never stored. The owner account record keeps a random
//! 16-byte salt, an iteration count and PBKDF2-HMAC-SHA256 output, and
//! verification compares in constant time. The record is versioned and
//! carries a SHA-256 of its body so a corrupted file is refused instead of
//! being read as a different account.
//!
//! The module is allocation-free and host-testable.

use sha2::{Digest, Sha256};

pub const SALT_BYTES: usize = 16;
pub const HASH_BYTES: usize = 32;
/// Iterations for new credentials (ADR 0063 amendment 2026-11-22: OWASP's
/// current minimum for PBKDF2-HMAC-SHA256). Records are accepted within
/// MIN..=MAX: the lower bound keeps older 20,000-iteration records readable
/// until they are re-derived, and the upper bound stops a corrupt record from
/// stalling boot.
pub const DEFAULT_ITERATIONS: u32 = 600_000;
pub const MIN_ITERATIONS: u32 = 10_000;
pub const MAX_ITERATIONS: u32 = 1_000_000;
pub const MAX_ACCOUNT_NAME_BYTES: usize = 32;
pub const MIN_PASSWORD_BYTES: usize = 4;
pub const MAX_PASSWORD_BYTES: usize = 64;
const RECORD_MAGIC: &[u8; 8] = b"NAGIACCT";
const RECORD_VERSION: u8 = 1;
/// magic, version, role, name length, reserved, iterations, name, salt,
/// hash, then the body checksum.
pub const ACCOUNT_RECORD_BYTES: usize =
    8 + 1 + 1 + 1 + 1 + 4 + MAX_ACCOUNT_NAME_BYTES + SALT_BYTES + HASH_BYTES + 32;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Credential {
    salt: [u8; SALT_BYTES],
    iterations: u32,
    hash: [u8; HASH_BYTES],
}

impl Credential {
    /// Derive a credential for `password` with a caller-supplied random salt.
    pub fn derive(password: &[u8], salt: [u8; SALT_BYTES], iterations: u32) -> Option<Self> {
        if !(MIN_PASSWORD_BYTES..=MAX_PASSWORD_BYTES).contains(&password.len())
            || !(MIN_ITERATIONS..=MAX_ITERATIONS).contains(&iterations)
        {
            return None;
        }
        Some(Self {
            salt,
            iterations,
            hash: pbkdf2_hmac_sha256(password, &salt, iterations),
        })
    }

    /// Constant-time check of `password` against this credential.
    pub fn verify(&self, password: &[u8]) -> bool {
        if password.len() > MAX_PASSWORD_BYTES {
            return false;
        }
        let candidate = pbkdf2_hmac_sha256(password, &self.salt, self.iterations);
        candidate
            .iter()
            .zip(self.hash.iter())
            .fold(0_u8, |difference, (left, right)| {
                difference | (left ^ right)
            })
            == 0
    }
}

/// The persisted owner account: a display/login name and its credential.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccountRecord {
    name: [u8; MAX_ACCOUNT_NAME_BYTES],
    name_length: u8,
    pub credential: Credential,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountRecordError {
    InvalidName,
    Malformed,
    Checksum,
}

impl AccountRecord {
    pub fn new(name: &[u8], credential: Credential) -> Result<Self, AccountRecordError> {
        if !valid_name(name) {
            return Err(AccountRecordError::InvalidName);
        }
        let mut stored = [0; MAX_ACCOUNT_NAME_BYTES];
        stored[..name.len()].copy_from_slice(name);
        Ok(Self {
            name: stored,
            name_length: name.len() as u8,
            credential,
        })
    }

    pub fn name(&self) -> &[u8] {
        &self.name[..self.name_length as usize]
    }

    pub fn encode(&self) -> [u8; ACCOUNT_RECORD_BYTES] {
        let mut bytes = [0; ACCOUNT_RECORD_BYTES];
        bytes[..8].copy_from_slice(RECORD_MAGIC);
        bytes[8] = RECORD_VERSION;
        // Role 1: Owner. Nagi 0.1 persists only the owner account.
        bytes[9] = 1;
        bytes[10] = self.name_length;
        bytes[12..16].copy_from_slice(&self.credential.iterations.to_le_bytes());
        let mut offset = 16;
        bytes[offset..offset + MAX_ACCOUNT_NAME_BYTES].copy_from_slice(&self.name);
        offset += MAX_ACCOUNT_NAME_BYTES;
        bytes[offset..offset + SALT_BYTES].copy_from_slice(&self.credential.salt);
        offset += SALT_BYTES;
        bytes[offset..offset + HASH_BYTES].copy_from_slice(&self.credential.hash);
        offset += HASH_BYTES;
        let checksum: [u8; 32] = Sha256::digest(&bytes[..offset]).into();
        bytes[offset..].copy_from_slice(&checksum);
        bytes
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, AccountRecordError> {
        if bytes.len() != ACCOUNT_RECORD_BYTES
            || bytes[..8] != *RECORD_MAGIC
            || bytes[8] != RECORD_VERSION
            || bytes[9] != 1
            || bytes[11] != 0
        {
            return Err(AccountRecordError::Malformed);
        }
        let body = ACCOUNT_RECORD_BYTES - 32;
        let checksum: [u8; 32] = Sha256::digest(&bytes[..body]).into();
        if checksum[..] != bytes[body..] {
            return Err(AccountRecordError::Checksum);
        }
        let name_length = bytes[10] as usize;
        let iterations = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
        if !(MIN_ITERATIONS..=MAX_ITERATIONS).contains(&iterations) {
            return Err(AccountRecordError::Malformed);
        }
        let mut offset = 16;
        let name = &bytes[offset..offset + MAX_ACCOUNT_NAME_BYTES];
        if name_length > MAX_ACCOUNT_NAME_BYTES || name[name_length..].iter().any(|byte| *byte != 0)
        {
            return Err(AccountRecordError::Malformed);
        }
        offset += MAX_ACCOUNT_NAME_BYTES;
        let mut salt = [0; SALT_BYTES];
        salt.copy_from_slice(&bytes[offset..offset + SALT_BYTES]);
        offset += SALT_BYTES;
        let mut hash = [0; HASH_BYTES];
        hash.copy_from_slice(&bytes[offset..offset + HASH_BYTES]);
        Self::new(
            &name[..name_length],
            Credential {
                salt,
                iterations,
                hash,
            },
        )
        .map_err(|_| AccountRecordError::Malformed)
    }
}

/// Lowercase ASCII letters and digits, starting with a letter.
pub fn valid_name(name: &[u8]) -> bool {
    !name.is_empty()
        && name.len() <= MAX_ACCOUNT_NAME_BYTES
        && name[0].is_ascii_lowercase()
        && name
            .iter()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
}

struct HmacSha256 {
    inner: Sha256,
    outer: Sha256,
}

impl HmacSha256 {
    fn new(key: &[u8]) -> Self {
        let mut block = [0_u8; 64];
        if key.len() > block.len() {
            let digest: [u8; 32] = Sha256::digest(key).into();
            block[..digest.len()].copy_from_slice(&digest);
        } else {
            block[..key.len()].copy_from_slice(key);
        }

        let mut inner_pad = [0x36_u8; 64];
        let mut outer_pad = [0x5c_u8; 64];
        for index in 0..block.len() {
            inner_pad[index] ^= block[index];
            outer_pad[index] ^= block[index];
        }

        let mut inner = Sha256::new();
        inner.update(inner_pad);
        let mut outer = Sha256::new();
        outer.update(outer_pad);
        Self { inner, outer }
    }

    fn digest(&self, parts: &[&[u8]]) -> [u8; 32] {
        let mut inner = self.inner.clone();
        for part in parts {
            inner.update(part);
        }
        let mut outer = self.outer.clone();
        outer.update(inner.finalize());
        outer.finalize().into()
    }
}

/// PBKDF2-HMAC-SHA256 with a single 32-byte output block (RFC 8018).
fn pbkdf2_hmac_sha256(password: &[u8], salt: &[u8], iterations: u32) -> [u8; 32] {
    // Reuse the keyed states for all U values within this derivation.
    let hmac = HmacSha256::new(password);
    let mut block = hmac.digest(&[salt, &1_u32.to_be_bytes()]);
    let mut output = block;
    for _ in 1..iterations {
        block = hmac.digest(&[&block]);
        for (byte, next) in output.iter_mut().zip(block.iter()) {
            *byte ^= next;
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(bytes: &[u8]) -> [u8; 64] {
        let mut out = [0; 64];
        for (index, byte) in bytes.iter().enumerate() {
            out[index * 2] = b"0123456789abcdef"[usize::from(byte >> 4)];
            out[index * 2 + 1] = b"0123456789abcdef"[usize::from(byte & 0xf)];
        }
        out
    }

    #[test]
    fn pbkdf2_matches_published_vectors() {
        assert_eq!(
            &hex(&pbkdf2_hmac_sha256(b"password", b"salt", 1)),
            b"120fb6cffcf8b32c43e7225256c4f837a86548c92ccc35480805987cb70be17b"
        );
        assert_eq!(
            &hex(&pbkdf2_hmac_sha256(b"password", b"salt", 2)),
            b"ae4d0c95af6b46d32d0adff928f06dd02a303f8ef3c251dfd6e2d85a95474c43"
        );
        assert_eq!(
            &hex(&pbkdf2_hmac_sha256(b"password", b"salt", 4096)),
            b"c5e478d59288c841aa530db6845c4c8d962893a001ce4e11a4963873aa98134a"
        );
    }

    #[test]
    fn pbkdf2_hashes_keys_longer_than_the_sha256_block() {
        let password: [u8; 65] = core::array::from_fn(|index| index as u8);
        assert_eq!(
            &hex(&pbkdf2_hmac_sha256(&password, b"salt", 2)),
            b"7ab0e8cf8cb6173aa06f140ba94b9573ba5f27fc96499f7070acc7783b542b67"
        );
    }

    #[test]
    fn version_one_credential_keeps_its_bytes_and_verification() {
        // Fixed v1 fixture generated independently with Python hashlib.
        // This uses a synthetic 64-byte password and the existing 20,000 rounds.
        let bytes: [u8; ACCOUNT_RECORD_BYTES] = [
            0x4e, 0x41, 0x47, 0x49, 0x41, 0x43, 0x43, 0x54, 0x01, 0x01, 0x06, 0x00, 0x20, 0x4e,
            0x00, 0x00, 0x6f, 0x77, 0x6e, 0x65, 0x72, 0x31, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07,
            0x08, 0x09, 0x0a, 0x0b, 0x0c, 0x0d, 0x0e, 0x0f, 0x48, 0x18, 0xcd, 0xeb, 0x1b, 0x59,
            0x68, 0x93, 0xd6, 0x17, 0xe2, 0xb4, 0x1e, 0x72, 0x77, 0xbd, 0x70, 0xdc, 0x50, 0x3a,
            0xca, 0xb3, 0x7f, 0xf8, 0xa8, 0xff, 0x61, 0x75, 0xb8, 0xd1, 0x5f, 0xc2, 0x49, 0xd2,
            0x7a, 0x0b, 0x8b, 0x07, 0xf2, 0x5e, 0x0d, 0xf1, 0xa3, 0xaf, 0xed, 0x22, 0x8c, 0xc1,
            0xa9, 0x41, 0xfb, 0x72, 0x1f, 0xe5, 0x4f, 0x28, 0xde, 0xbb, 0x6c, 0xd1, 0x09, 0x09,
            0x1a, 0xe3,
        ];
        let password: [u8; 64] = core::array::from_fn(|index| index as u8);
        let salt: [u8; SALT_BYTES] = core::array::from_fn(|index| index as u8);
        let record = AccountRecord::decode(&bytes).expect("existing v1 record");
        assert_eq!(record.name(), b"owner1");
        assert!(record.credential.verify(&password));
        let mut wrong = password;
        wrong[63] ^= 1;
        assert!(!record.credential.verify(&wrong));
        assert_eq!(record.encode(), bytes);
        let derived = Credential::derive(&password, salt, 20_000).expect("derive");
        assert_eq!(
            AccountRecord::new(b"owner1", derived).unwrap().encode(),
            bytes
        );
    }

    #[test]
    fn credentials_verify_only_the_password() {
        let credential = Credential::derive(b"correct horse", [7; SALT_BYTES], MIN_ITERATIONS)
            .expect("credential");
        assert!(credential.verify(b"correct horse"));
        assert!(!credential.verify(b"correct horsf"));
        assert!(!credential.verify(b""));
        // The same password with another salt yields another hash.
        let other = Credential::derive(b"correct horse", [8; SALT_BYTES], MIN_ITERATIONS)
            .expect("credential");
        assert_ne!(credential.hash, other.hash);
        assert_eq!(
            Credential::derive(b"abc", [0; SALT_BYTES], MIN_ITERATIONS),
            None
        );
        assert_eq!(Credential::derive(b"abcd", [0; SALT_BYTES], 1), None);
    }

    #[test]
    fn account_records_round_trip_and_reject_corruption() {
        let credential =
            Credential::derive(b"nagi-pass", [3; SALT_BYTES], MIN_ITERATIONS).expect("credential");
        let record = AccountRecord::new(b"owner1", credential).expect("record");
        let bytes = record.encode();
        let decoded = AccountRecord::decode(&bytes).expect("decode");
        assert_eq!(decoded, record);
        assert_eq!(decoded.name(), b"owner1");
        assert!(decoded.credential.verify(b"nagi-pass"));

        let mut corrupt = bytes;
        corrupt[20] ^= 1;
        assert_eq!(
            AccountRecord::decode(&corrupt),
            Err(AccountRecordError::Checksum)
        );
        assert_eq!(
            AccountRecord::decode(&bytes[..ACCOUNT_RECORD_BYTES - 1]),
            Err(AccountRecordError::Malformed)
        );
        assert_eq!(
            AccountRecord::new(b"Owner", credential),
            Err(AccountRecordError::InvalidName)
        );
        assert_eq!(
            AccountRecord::new(b"1owner", credential),
            Err(AccountRecordError::InvalidName)
        );
    }
}
