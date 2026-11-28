//! Authenticated inputs for the Linux release System mount.

use super::{checked_product_key, verify_signed_payload, BootCatalogError, SIGNATURE_BYTES};
use serde::{Deserialize, Serialize};

const DOMAIN: &[u8] = b"nagi-system-boot-metadata-v1\0";
const SCHEMA: &str = "nagi-system-verity-unsigned-v1";
const BLOCK_BYTES: u64 = 4096;
const HASHES_PER_BLOCK: u64 = BLOCK_BYTES / 32;
pub const MAX_SIGNED_SYSTEM_METADATA_BYTES: usize = 4096 + SIGNATURE_BYTES;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Record {
    schema: String,
    system: Image,
    verity: Verity,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Image {
    data_sha256: String,
    sha256: String,
    size_bytes: u64,
}

// Field order matches the builder's sorted, indented JSON with a final LF.
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Verity {
    algorithm: String,
    data_block_size: u64,
    format_version: u8,
    hash_block_size: u64,
    hash_start_block: u64,
    num_data_blocks: u64,
    root_digest: String,
    salt: String,
    tree_blocks: u64,
}

pub struct VerifiedSystemBootMetadata(Record);

impl VerifiedSystemBootMetadata {
    /// The initramfs file contains canonical JSON followed by a raw Ed25519 signature.
    pub fn verify(file: &[u8], public_key: &[u8; 32]) -> Result<Self, BootCatalogError> {
        if file.len() > MAX_SIGNED_SYSTEM_METADATA_BYTES {
            return Err(BootCatalogError::InvalidLength);
        }
        let key = checked_product_key(public_key).ok_or(BootCatalogError::UntrustedSigner)?;
        let payload = verify_signed_payload(file, DOMAIN, &key)?;
        let record: Record =
            serde_json::from_slice(payload).map_err(|_| BootCatalogError::InvalidRecord)?;
        let canonical =
            serde_json::to_string_pretty(&record).map_err(|_| BootCatalogError::InvalidRecord)?;
        if payload.strip_suffix(b"\n") != Some(canonical.as_bytes()) || !record.is_supported() {
            return Err(BootCatalogError::InvalidRecord);
        }
        Ok(Self(record))
    }

    pub fn image_size_bytes(&self) -> u64 {
        self.0.system.size_bytes
    }

    pub fn sectors(&self) -> u64 {
        self.0.verity.num_data_blocks * (BLOCK_BYTES / 512)
    }

    /// Device numbers come from PID 1's opened System block device.
    pub fn verity_parameters(&self, major: u32, minor: u32) -> String {
        let data = &self.0.verity;
        format!(
            "1 {major}:{minor} {major}:{minor} {BLOCK_BYTES} {BLOCK_BYTES} {} {} sha256 {} {}",
            data.num_data_blocks, data.hash_start_block, data.root_digest, data.salt
        )
    }
}

impl Record {
    fn is_supported(&self) -> bool {
        let v = &self.verity;
        if self.schema != SCHEMA
            || v.algorithm != "sha256"
            || v.format_version != 1
            || v.data_block_size != BLOCK_BYTES
            || v.hash_block_size != BLOCK_BYTES
            || !(4096..=u64::from(u32::MAX)).contains(&v.num_data_blocks)
            || v.hash_start_block != v.num_data_blocks
            || ![
                &v.salt,
                &v.root_digest,
                &self.system.data_sha256,
                &self.system.sha256,
            ]
            .into_iter()
            .all(|digest| is_sha256(digest))
        {
            return false;
        }
        let mut blocks = v.num_data_blocks.div_ceil(HASHES_PER_BLOCK);
        let mut tree_blocks = blocks;
        while blocks > 1 {
            blocks = blocks.div_ceil(HASHES_PER_BLOCK);
            tree_blocks += blocks;
        }
        v.tree_blocks == tree_blocks
            && self.system.size_bytes == (v.num_data_blocks + tree_blocks) * BLOCK_BYTES
    }
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    fn payload() -> Vec<u8> {
        let record = serde_json::json!({
            "schema": SCHEMA,
            "system": {
                "data_sha256": "a".repeat(64),
                "sha256": "b".repeat(64),
                "size_bytes": 16912384,
            },
            "verity": {
                "algorithm": "sha256", "data_block_size": 4096, "format_version": 1,
                "hash_block_size": 4096, "hash_start_block": 4096, "num_data_blocks": 4096,
                "root_digest": "c".repeat(64), "salt": "d".repeat(64), "tree_blocks": 33,
            },
        });
        (serde_json::to_string_pretty(&record).unwrap() + "\n").into_bytes()
    }

    fn envelope(payload: &[u8], domain: &[u8], signer: &SigningKey) -> Vec<u8> {
        let mut message = domain.to_vec();
        message.extend_from_slice(payload);
        let signature = signer.sign(&message);
        let mut file = payload.to_vec();
        file.extend_from_slice(&signature.to_bytes());
        file
    }

    fn rejection(file: &[u8], signer: &SigningKey) -> BootCatalogError {
        VerifiedSystemBootMetadata::verify(file, signer.verifying_key().as_bytes())
            .err()
            .expect("unsigned or malformed metadata must be rejected")
    }

    #[test]
    fn authenticates_canonical_json_and_derives_the_table() {
        let signer = SigningKey::from_bytes(&[7; 32]);
        let file = envelope(&payload(), DOMAIN, &signer);
        let verified =
            VerifiedSystemBootMetadata::verify(&file, signer.verifying_key().as_bytes()).unwrap();
        assert_eq!(verified.image_size_bytes(), 16912384);
        assert_eq!(verified.sectors(), 32768);
        assert_eq!(
            verified.verity_parameters(254, 1),
            format!(
                "1 254:1 254:1 4096 4096 4096 4096 sha256 {} {}",
                "c".repeat(64),
                "d".repeat(64)
            )
        );
    }

    #[test]
    fn rejects_missing_invalid_or_cross_domain_signatures() {
        let signer = SigningKey::from_bytes(&[7; 32]);
        let payload = payload();
        assert_eq!(rejection(&[], &signer), BootCatalogError::InvalidLength);
        assert_eq!(
            rejection(&payload, &signer),
            BootCatalogError::InvalidSignature
        );
        let wrong_domain = envelope(&payload, super::super::SIGNATURE_DOMAIN, &signer);
        assert_eq!(
            rejection(&wrong_domain, &signer),
            BootCatalogError::InvalidSignature
        );
        let mut file = envelope(&payload, DOMAIN, &signer);
        let other_signer = SigningKey::from_bytes(&[11; 32]);
        assert_eq!(
            rejection(&file, &other_signer),
            BootCatalogError::InvalidSignature
        );
        file[0] ^= 1;
        assert_eq!(
            rejection(&file, &signer),
            BootCatalogError::InvalidSignature
        );
    }

    #[test]
    fn rejects_signed_malformed_or_inconsistent_metadata() {
        let signer = SigningKey::from_bytes(&[7; 32]);
        let json = String::from_utf8(payload()).unwrap();
        for invalid in [
            json.trim_end().to_string(),
            json.replace("\"tree_blocks\": 33", "\"tree_blocks\": 32"),
            json.replace("\"hash_start_block\": 4096", "\"hash_start_block\": 4095"),
            json.replace("\"schema\":", "\"unknown\": true, \"schema\":"),
            json.replace(
                "\"tree_blocks\": 33",
                "\"tree_blocks\": 33, \"tree_blocks\": 33",
            ),
            json.replace(&"c".repeat(64), &"C".repeat(64)),
            json.replace("16912384", "16912385"),
            "{".to_string(),
        ] {
            let file = envelope(invalid.as_bytes(), DOMAIN, &signer);
            assert_eq!(rejection(&file, &signer), BootCatalogError::InvalidRecord);
        }
    }
}
