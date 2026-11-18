//! A-side password authentication against the account owner's persisted
//! credential and canonical identity mapping. This evidence must be joined to
//! C2's exact owner admission before requesting a C1 caller or Session.

#![cfg(target_os = "linux")]

use libnagi::credential::{AccountRecord, ACCOUNT_RECORD_BYTES};
use nagi_capability::PrincipalId;
use rustix::fs::{openat, Mode, OFlags};
use rustix::net::sockopt::socket_cookie;
use serde::Deserialize;
use std::fs::File;
use std::io::Read;
use std::num::NonZeroU64;
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;

const ACCOUNT_FILE: &str = "owner-account-identity.v1";
const MAX_RECORD_BYTES: usize = 2048;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountAuthenticationError {
    Unavailable,
    InvalidRecord,
    Rejected,
    WrongVerifierOrConnection,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedAccountProfile {
    version: u32,
    account_id: NonZeroU64,
    user_id: NonZeroU64,
    profile_id: String,
    principal_id: PrincipalId,
    account_record: Vec<u8>,
}

/// Authenticated account evidence with no public constructor. Keep this
/// evidence intact until it can be paired with the C2 owner admission.
pub struct AuthenticatedAccountProfile {
    account_id: NonZeroU64,
    user_id: NonZeroU64,
    profile_id: String,
    principal_id: PrincipalId,
}

/// A successful password check kept on the exact accepted connection for the
/// pre-Session login handler. Matching the socket only correlates the account
/// evidence. The C2 root must independently prove the spawn/endpoint owner
/// before the C1 issuer may use it.
pub struct AuthenticatedAccountOnConnection<'verifier, 'connection> {
    verifier: &'verifier LinuxAccountVerifier,
    connection: &'connection UnixStream,
    cookie: NonZeroU64,
    account: AuthenticatedAccountProfile,
}

/// A boot-selected account verifier. The C1 issuer must retain the same
/// configured instance and consume evidence through it after C2 owner
/// validation. Evidence from a separately opened directory or another
/// verifier instance is rejected, even when its records have the same IDs.
pub struct LinuxAccountVerifier {
    directory: File,
    owner_uid: u32,
}

impl LinuxAccountVerifier {
    /// Adopt the protected directory descriptor selected by the boot owner.
    pub fn from_protected_directory(directory: File) -> Result<Self, AccountAuthenticationError> {
        Self::from_owner_directory(directory, 0)
    }

    fn from_owner_directory(
        directory: File,
        owner_uid: u32,
    ) -> Result<Self, AccountAuthenticationError> {
        validate_directory(&directory, owner_uid)?;
        Ok(Self {
            directory,
            owner_uid,
        })
    }

    /// Verify the provisioned credential while borrowing the exact accepted
    /// stream. This proof carries no C2 owner or C1 Session authority.
    pub fn authenticate<'verifier, 'connection>(
        &'verifier self,
        connection: &'connection UnixStream,
        account_name: &[u8],
        password: &[u8],
    ) -> Result<AuthenticatedAccountOnConnection<'verifier, 'connection>, AccountAuthenticationError>
    {
        let cookie = connected_socket_cookie(connection)?;
        let account = AuthenticatedAccountProfile::authenticate(
            &self.directory,
            self.owner_uid,
            account_name,
            password,
        )?;
        Ok(AuthenticatedAccountOnConnection {
            verifier: self,
            connection,
            cookie,
            account,
        })
    }

    /// Consume one password proof using the same boot-selected verifier and
    /// the stream supplied by C2's pre-Session owner check.
    pub fn consume_on_connection(
        &self,
        evidence: AuthenticatedAccountOnConnection<'_, '_>,
        connection: &UnixStream,
    ) -> Result<AuthenticatedAccountProfile, AccountAuthenticationError> {
        if !std::ptr::eq(self, evidence.verifier)
            || !std::ptr::eq(evidence.connection, connection)
            || connected_socket_cookie(connection) != Ok(evidence.cookie)
        {
            return Err(AccountAuthenticationError::WrongVerifierOrConnection);
        }
        Ok(evidence.account)
    }
}

fn validate_directory(directory: &File, owner_uid: u32) -> Result<(), AccountAuthenticationError> {
    use AccountAuthenticationError::Unavailable;

    let metadata = directory.metadata().map_err(|_| Unavailable)?;
    if !metadata.is_dir() || metadata.uid() != owner_uid || metadata.mode() & 0o022 != 0 {
        return Err(Unavailable);
    }
    Ok(())
}

fn connected_socket_cookie(
    connection: &UnixStream,
) -> Result<NonZeroU64, AccountAuthenticationError> {
    use AccountAuthenticationError::Unavailable;

    connection.peer_addr().map_err(|_| Unavailable)?;
    let cookie = socket_cookie(connection).map_err(|_| Unavailable)?;
    NonZeroU64::new(cookie).ok_or(Unavailable)
}

impl AuthenticatedAccountProfile {
    pub fn account_id(&self) -> NonZeroU64 {
        self.account_id
    }

    pub fn user_id(&self) -> NonZeroU64 {
        self.user_id
    }

    /// C1 applies its canonical ProfileId validator before Session issuance.
    pub fn profile_id(&self) -> &str {
        &self.profile_id
    }

    pub fn principal_id(&self) -> &PrincipalId {
        &self.principal_id
    }

    /// Read the credential and mapping from one persisted record so a
    /// password change can replace them atomically without remapping the IDs.
    fn authenticate(
        account_directory: &File,
        owner_uid: u32,
        account_name: &[u8],
        password: &[u8],
    ) -> Result<Self, AccountAuthenticationError> {
        use AccountAuthenticationError::{InvalidRecord, Rejected, Unavailable};

        validate_directory(account_directory, owner_uid)?;
        let fd = openat(
            account_directory,
            ACCOUNT_FILE,
            OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::empty(),
        )
        .map_err(|_| Unavailable)?;
        let file = File::from(fd);
        let metadata = file.metadata().map_err(|_| Unavailable)?;
        if !metadata.is_file() || metadata.uid() != owner_uid || metadata.mode() & 0o077 != 0 {
            return Err(Unavailable);
        }
        let mut bytes = Vec::new();
        file.take((MAX_RECORD_BYTES + 1) as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| Unavailable)?;
        if bytes.len() > MAX_RECORD_BYTES {
            return Err(InvalidRecord);
        }
        let record: PersistedAccountProfile =
            serde_json::from_slice(&bytes).map_err(|_| InvalidRecord)?;
        if record.version != 1
            || record.account_record.len() != ACCOUNT_RECORD_BYTES
            || record.profile_id.is_empty()
            || record.profile_id.len() > 128
            || !record.principal_id.as_str().starts_with("user:")
        {
            return Err(InvalidRecord);
        }
        let credential =
            AccountRecord::decode(&record.account_record).map_err(|_| InvalidRecord)?;
        if credential.name() != account_name || !credential.credential.verify(password) {
            return Err(Rejected);
        }
        Ok(Self {
            account_id: record.account_id,
            user_id: record.user_id,
            profile_id: record.profile_id,
            principal_id: record.principal_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libnagi::credential::{Credential, MIN_ITERATIONS, SALT_BYTES};
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    use std::os::unix::net::UnixListener;

    #[test]
    fn authenticates_the_provisioned_account_and_retains_canonical_ids() {
        let path = std::env::temp_dir().join(format!(
            "nagi-account-profile-binding-{}",
            std::process::id()
        ));
        fs::create_dir(&path).unwrap();
        let directory = File::open(&path).unwrap();
        let name = b"owner";
        let password = b"account-fixture-password";
        let credential = Credential::derive(password, [0x71; SALT_BYTES], MIN_ITERATIONS).unwrap();
        let account = AccountRecord::new(name, credential).unwrap();
        let bytes = serde_json::to_vec(&serde_json::json!({
            "version": 1,
            "account_id": 11,
            "user_id": 29,
            "profile_id": "profile_owner",
            "principal_id": "user:canonical-owner",
            "account_record": account.encode().to_vec(),
        }))
        .unwrap();
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path.join(ACCOUNT_FILE))
            .unwrap()
            .write_all(&bytes)
            .unwrap();

        let socket_path = path.join("login.sock");
        let listener = UnixListener::bind(&socket_path).unwrap();
        let _client = UnixStream::connect(&socket_path).unwrap();
        let (accepted, _) = listener.accept().unwrap();
        let owner_uid = directory.metadata().unwrap().uid();
        let verifier = if owner_uid == 0 {
            LinuxAccountVerifier::from_protected_directory(directory)
        } else {
            LinuxAccountVerifier::from_owner_directory(directory, owner_uid)
        }
        .unwrap();
        let evidence = verifier.authenticate(&accepted, name, password).unwrap();
        let account = verifier.consume_on_connection(evidence, &accepted).unwrap();
        fs::remove_file(path.join(ACCOUNT_FILE)).unwrap();
        fs::remove_file(socket_path).unwrap();
        fs::remove_dir(path).unwrap();
        assert_eq!(account.account_id().get(), 11);
        assert_eq!(account.user_id().get(), 29);
        assert_eq!(account.profile_id(), "profile_owner");
        assert_eq!(account.principal_id().as_str(), "user:canonical-owner");
    }
}
