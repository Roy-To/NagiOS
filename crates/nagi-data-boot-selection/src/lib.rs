//! Development boot selection authenticated independently of the kernel command line.
//!
//! This key is only a Developer Preview test signer. It is not a product trust
//! anchor; production key provisioning remains governed by DEC-087.

#![no_std]

use ed25519_dalek::{Signature, VerifyingKey};

pub const SIGNATURE_BYTES: usize = 64;
pub const MAX_CONFIG_TEXT_BYTES: usize = 512;
pub const MAX_CONFIG_BYTES: usize = MAX_CONFIG_TEXT_BYTES + SIGNATURE_BYTES;
pub const SIGNATURE_DOMAIN: &[u8] = b"nagi-data-selection-v1\0";
const HEADER: &str = "nagi-data-selection 1";

/// Developer Preview test-vector signer; never a production trust anchor.
pub const DEVELOPMENT_PREVIEW_PUBLIC_KEY: [u8; 32] = [
    0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64, 0x07, 0x3a,
    0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0x1a, 0x68, 0xf7, 0x07, 0x51, 0x1a,
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectionMode {
    Identity,
    SmokeWrite,
    SmokeRead,
    SmokeReject,
    SmokeProvisionIdentityDirectories,
}

impl SelectionMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::SmokeWrite => "smoke-write",
            Self::SmokeRead => "smoke-read",
            Self::SmokeReject => "smoke-reject",
            Self::SmokeProvisionIdentityDirectories => "smoke-provision-identity-directories",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        match value {
            "identity" => Some(Self::Identity),
            "smoke-write" => Some(Self::SmokeWrite),
            "smoke-read" => Some(Self::SmokeRead),
            "smoke-reject" => Some(Self::SmokeReject),
            "smoke-provision-identity-directories" => Some(Self::SmokeProvisionIdentityDirectories),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectionError {
    InvalidLength,
    Signature,
    Malformed,
    FilesystemUuidMismatch,
}

/// Fields from a signature-verified Data selection file.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DataBootSelection<'a> {
    pub mode: SelectionMode,
    pub filesystem_uuid: &'a str,
    pub accounts: &'a str,
    pub snapshots: &'a str,
}

impl<'a> DataBootSelection<'a> {
    /// Verify the complete text before parsing any selection field.
    pub fn verify(bytes: &'a [u8]) -> Result<Self, SelectionError> {
        if bytes.len() <= SIGNATURE_BYTES || bytes.len() > MAX_CONFIG_BYTES {
            return Err(SelectionError::InvalidLength);
        }
        let (text, signature) = bytes.split_at(bytes.len() - SIGNATURE_BYTES);
        let signature = Signature::from_slice(signature).map_err(|_| SelectionError::Signature)?;
        let key = VerifyingKey::from_bytes(&DEVELOPMENT_PREVIEW_PUBLIC_KEY)
            .map_err(|_| SelectionError::Signature)?;
        let mut message = [0; SIGNATURE_DOMAIN.len() + MAX_CONFIG_TEXT_BYTES];
        message[..SIGNATURE_DOMAIN.len()].copy_from_slice(SIGNATURE_DOMAIN);
        message[SIGNATURE_DOMAIN.len()..SIGNATURE_DOMAIN.len() + text.len()].copy_from_slice(text);
        key.verify_strict(&message[..SIGNATURE_DOMAIN.len() + text.len()], &signature)
            .map_err(|_| SelectionError::Signature)?;
        Self::parse_text(text)
    }

    /// Accept the signed selection only when the observed device UUID is exact.
    pub fn verify_for_filesystem(
        bytes: &'a [u8],
        observed_uuid: &str,
    ) -> Result<Self, SelectionError> {
        let selection = Self::verify(bytes)?;
        if selection.filesystem_uuid != observed_uuid {
            return Err(SelectionError::FilesystemUuidMismatch);
        }
        Ok(selection)
    }

    fn parse_text(text: &'a [u8]) -> Result<Self, SelectionError> {
        let text = core::str::from_utf8(text).map_err(|_| SelectionError::Malformed)?;
        let body = text.strip_suffix('\n').ok_or(SelectionError::Malformed)?;
        let mut lines = body.lines();
        if lines.next() != Some(HEADER) {
            return Err(SelectionError::Malformed);
        }
        let mode = lines
            .next()
            .and_then(|line| line.strip_prefix("mode="))
            .and_then(SelectionMode::parse)
            .ok_or(SelectionError::Malformed)?;
        let filesystem_uuid = lines
            .next()
            .and_then(|line| line.strip_prefix("filesystem-uuid="))
            .filter(|value| valid_uuid(value))
            .ok_or(SelectionError::Malformed)?;
        let accounts = lines
            .next()
            .and_then(|line| line.strip_prefix("accounts="))
            .filter(|value| valid_relative_path(value))
            .ok_or(SelectionError::Malformed)?;
        let snapshots = lines
            .next()
            .and_then(|line| line.strip_prefix("snapshots="))
            .filter(|value| valid_relative_path(value))
            .ok_or(SelectionError::Malformed)?;
        if lines.next().is_some() {
            return Err(SelectionError::Malformed);
        }
        Ok(Self {
            mode,
            filesystem_uuid,
            accounts,
            snapshots,
        })
    }
}

fn valid_uuid(value: &str) -> bool {
    value.len() == 36
        && value.bytes().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => byte == b'-',
            _ => byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte),
        })
}

fn valid_relative_path(value: &str) -> bool {
    !value.is_empty()
        && value.split('/').all(|component| {
            !component.is_empty()
                && component != "."
                && component != ".."
                && component
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
        })
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};
    use std::{format, vec::Vec};

    const TEST_SECRET: [u8; 32] = [
        0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c,
        0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae,
        0x7f, 0x60,
    ];

    fn signed_config(uuid: &str) -> Vec<u8> {
        let text = format!(
            "{HEADER}\nmode=identity\nfilesystem-uuid={uuid}\naccounts=accounts\nsnapshots=snapshots\n"
        );
        let mut message = SIGNATURE_DOMAIN.to_vec();
        message.extend_from_slice(text.as_bytes());
        let signature = SigningKey::from_bytes(&TEST_SECRET)
            .sign(&message)
            .to_bytes();
        let mut bytes = text.into_bytes();
        bytes.extend_from_slice(&signature);
        bytes
    }

    #[test]
    fn valid_signed_selection_matches_observed_filesystem_uuid() {
        let bytes = signed_config("12345678-1234-4234-8234-123456789abc");
        let selection = DataBootSelection::verify_for_filesystem(
            &bytes,
            "12345678-1234-4234-8234-123456789abc",
        )
        .expect("verified selection");
        assert_eq!(selection.mode, SelectionMode::Identity);
        assert_eq!(selection.accounts, "accounts");
        assert_eq!(selection.snapshots, "snapshots");
    }

    #[test]
    fn altered_signature_is_rejected_before_selection_is_used() {
        let mut bytes = signed_config("12345678-1234-4234-8234-123456789abc");
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        assert_eq!(
            DataBootSelection::verify(&bytes),
            Err(SelectionError::Signature)
        );
    }

    #[test]
    fn filesystem_uuid_mismatch_is_rejected() {
        let bytes = signed_config("12345678-1234-4234-8234-123456789abc");
        assert_eq!(
            DataBootSelection::verify_for_filesystem(
                &bytes,
                "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa",
            ),
            Err(SelectionError::FilesystemUuidMismatch)
        );
    }

    #[test]
    fn signed_but_noncanonical_fields_are_rejected() {
        let text = format!(
            "{HEADER}\nmode=identity\nfilesystem-uuid=12345678-1234-4234-8234-123456789ABC\naccounts=../accounts\nsnapshots=snapshots\n"
        );
        let mut message = SIGNATURE_DOMAIN.to_vec();
        message.extend_from_slice(text.as_bytes());
        let signature = SigningKey::from_bytes(&TEST_SECRET)
            .sign(&message)
            .to_bytes();
        let mut bytes = text.into_bytes();
        bytes.extend_from_slice(&signature);
        assert_eq!(
            DataBootSelection::verify(&bytes),
            Err(SelectionError::Malformed)
        );
    }
}
