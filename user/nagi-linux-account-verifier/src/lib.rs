//! A-side password authentication against the account owner's persisted
//! credential and canonical identity mapping. This evidence must be joined to
//! C2's exact owner admission before requesting a C1 caller or Session.

#![cfg(target_os = "linux")]

use libnagi::credential::{AccountRecord, ACCOUNT_RECORD_BYTES};
use libnagi::login::{decode_login_failures, encode_login_failures, FREE_ATTEMPTS};
use nagi_capability::PrincipalId;
use rustix::fs::{linkat, openat, renameat, statat, unlinkat, AtFlags, Mode, OFlags};
use rustix::net::sockopt::socket_cookie;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{self, Read, Write};
use std::num::NonZeroU64;
use std::os::unix::fs::MetadataExt;
use std::os::unix::net::UnixStream;

const ACCOUNT_FILE: &str = "owner-account-identity.v1";
const MAX_RECORD_BYTES: usize = 2048;
const LOGIN_THROTTLE_FILE: &str = "login-throttle";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AccountAuthenticationError {
    Unavailable,
    InvalidRecord,
    Rejected,
    WrongVerifierOrConnection,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PersistedAccountProfile {
    version: u32,
    account_id: NonZeroU64,
    user_id: NonZeroU64,
    profile_id: String,
    principal_id: PrincipalId,
    account_record: Vec<u8>,
}

/// The account mapping supplied by the identity provisioning owner.
pub struct OwnerAccountBinding {
    pub account_id: NonZeroU64,
    pub user_id: NonZeroU64,
    pub profile_id: String,
    pub principal_id: PrincipalId,
}

#[derive(Debug)]
pub enum OwnerAccountSaveError {
    InvalidBinding,
    AlreadyExists,
    BeforePublication(io::Error),
    PublicationUnknown(io::Error),
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

    pub fn require_unprovisioned_private_directory(&self) -> Result<(), OwnerAccountSaveError> {
        use OwnerAccountSaveError::*;

        let metadata = self.directory.metadata().map_err(BeforePublication)?;
        if !metadata.is_dir() || metadata.uid() != 0 || metadata.mode() & 0o077 != 0 {
            return Err(BeforePublication(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "account directory is not private to the boot owner",
            )));
        }
        match statat(&self.directory, ACCOUNT_FILE, AtFlags::SYMLINK_NOFOLLOW) {
            Err(rustix::io::Errno::NOENT) => Ok(()),
            Ok(_) => Err(AlreadyExists),
            Err(error) => Err(BeforePublication(error.into())),
        }
    }

    /// Publish the first owner credential on the same retained Data account
    /// directory used for login. The canonical identity owner must supply the
    /// binding. A prior account is never replaced.
    pub fn publish_first_owner(
        &self,
        binding: OwnerAccountBinding,
        credential: AccountRecord,
    ) -> Result<(), OwnerAccountSaveError> {
        use OwnerAccountSaveError::*;

        self.require_unprovisioned_private_directory()?;
        let record = PersistedAccountProfile {
            version: 1,
            account_id: binding.account_id,
            user_id: binding.user_id,
            profile_id: binding.profile_id,
            principal_id: binding.principal_id,
            account_record: credential.encode().to_vec(),
        };
        if !record.has_supported_binding() {
            return Err(InvalidBinding);
        }
        let bytes = serde_json::to_vec(&record)
            .map_err(|error| BeforePublication(io::Error::other(error)))?;
        if bytes.len() > MAX_RECORD_BYTES {
            return Err(InvalidBinding);
        }
        let mut file = File::from(
            openat(
                &self.directory,
                ".",
                OFlags::TMPFILE | OFlags::WRONLY | OFlags::CLOEXEC,
                Mode::RUSR | Mode::WUSR,
            )
            .map_err(|error| BeforePublication(error.into()))?,
        );
        file.write_all(&bytes).map_err(BeforePublication)?;
        file.sync_all().map_err(BeforePublication)?;
        match linkat(
            &file,
            "",
            &self.directory,
            ACCOUNT_FILE,
            AtFlags::EMPTY_PATH,
        ) {
            Ok(()) => {}
            Err(rustix::io::Errno::EXIST) => return Err(AlreadyExists),
            Err(error) => return Err(PublicationUnknown(error.into())),
        }
        self.directory.sync_all().map_err(PublicationUnknown)?;
        Ok(())
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

    /// Read the display name for the trusted boot unlock screen from the
    /// same protected record used by authentication.
    pub fn owner_login_name(&self) -> Result<Vec<u8>, AccountAuthenticationError> {
        let (_, account) = read_account(&self.directory, self.owner_uid)?;
        Ok(account.name().to_vec())
    }

    /// Restore ADR 0064 from the same protected Data directory as the account.
    pub fn load_login_failures(&self) -> u32 {
        if validate_directory(&self.directory, self.owner_uid).is_err() {
            return FREE_ATTEMPTS;
        }
        let file = match openat(
            &self.directory,
            LOGIN_THROTTLE_FILE,
            OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
            Mode::empty(),
        ) {
            Ok(fd) => File::from(fd),
            Err(rustix::io::Errno::NOENT) => return 0,
            Err(_) => return FREE_ATTEMPTS,
        };
        if !file.metadata().is_ok_and(|metadata| {
            metadata.is_file() && metadata.uid() == self.owner_uid && metadata.mode() & 0o077 == 0
        }) {
            return FREE_ATTEMPTS;
        }
        let mut bytes = Vec::with_capacity(9);
        if file.take(9).read_to_end(&mut bytes).is_err() {
            return FREE_ATTEMPTS;
        }
        decode_login_failures(&bytes).unwrap_or(FREE_ATTEMPTS)
    }

    /// Publish a whole count before another attempt can begin. An interrupted
    /// write leaves the previous count in place.
    pub fn store_login_failures(&self, failures: u32) -> io::Result<()> {
        validate_directory(&self.directory, self.owner_uid)
            .map_err(|_| io::Error::other("login throttle directory unavailable"))?;
        let mut nonce = [0; 8];
        File::open("/dev/urandom")?.read_exact(&mut nonce)?;
        let temporary = format!(".login-throttle-{:016x}", u64::from_ne_bytes(nonce));
        let mut file = File::from(openat(
            &self.directory,
            temporary.as_str(),
            OFlags::WRONLY | OFlags::CLOEXEC | OFlags::CREATE | OFlags::EXCL,
            Mode::RUSR | Mode::WUSR,
        )?);
        let result = (|| {
            file.write_all(&encode_login_failures(failures))?;
            file.sync_all()?;
            renameat(
                &self.directory,
                temporary.as_str(),
                &self.directory,
                LOGIN_THROTTLE_FILE,
            )?;
            self.directory.sync_all()
        })();
        if result.is_err() {
            let _ = unlinkat(&self.directory, temporary.as_str(), AtFlags::empty());
        }
        result
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

impl PersistedAccountProfile {
    fn has_supported_binding(&self) -> bool {
        self.version == 1
            && self.account_record.len() == ACCOUNT_RECORD_BYTES
            && !self.profile_id.is_empty()
            && self.profile_id.len() <= 128
            && self.principal_id.as_str().starts_with("user:")
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
        use AccountAuthenticationError::Rejected;

        let (record, credential) = read_account(account_directory, owner_uid)?;
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

fn read_account(
    account_directory: &File,
    owner_uid: u32,
) -> Result<(PersistedAccountProfile, AccountRecord), AccountAuthenticationError> {
    use AccountAuthenticationError::{InvalidRecord, Unavailable};

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
    if !record.has_supported_binding() {
        return Err(InvalidRecord);
    }
    let credential = AccountRecord::decode(&record.account_record).map_err(|_| InvalidRecord)?;
    Ok((record, credential))
}

#[cfg(test)]
mod tests {
    use super::*;
    use libnagi::credential::{Credential, MIN_ITERATIONS, SALT_BYTES};
    use std::fs::{self, OpenOptions};
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
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
        assert!(matches!(
            verifier.authenticate(&accepted, name, b"wrong-fixture-input"),
            Err(AccountAuthenticationError::Rejected)
        ));
        assert_eq!(verifier.load_login_failures(), 0);
        verifier.store_login_failures(FREE_ATTEMPTS).unwrap();
        let reopened =
            LinuxAccountVerifier::from_owner_directory(File::open(&path).unwrap(), owner_uid)
                .unwrap();
        assert_eq!(reopened.load_login_failures(), FREE_ATTEMPTS);
        let evidence = verifier.authenticate(&accepted, name, password).unwrap();
        let account = verifier.consume_on_connection(evidence, &accepted).unwrap();
        verifier.store_login_failures(0).unwrap();
        assert_eq!(reopened.load_login_failures(), 0);
        fs::write(path.join(LOGIN_THROTTLE_FILE), b"corrupt").unwrap();
        assert_eq!(reopened.load_login_failures(), FREE_ATTEMPTS);
        fs::remove_file(path.join(LOGIN_THROTTLE_FILE)).unwrap();
        fs::remove_file(path.join(ACCOUNT_FILE)).unwrap();
        fs::remove_file(socket_path).unwrap();
        fs::remove_dir(path).unwrap();
        assert_eq!(account.account_id().get(), 11);
        assert_eq!(account.user_id().get(), 29);
        assert_eq!(account.profile_id(), "profile_owner");
        assert_eq!(account.principal_id().as_str(), "user:canonical-owner");
    }

    #[test]
    #[ignore = "requires a root-run Linux guest and a filesystem supporting O_TMPFILE"]
    fn publishes_private_first_owner_and_refuses_to_replace_it() {
        let path = std::env::temp_dir().join(format!("nagi-first-owner-{}", std::process::id()));
        fs::DirBuilder::new().mode(0o700).create(&path).unwrap();
        let verifier =
            LinuxAccountVerifier::from_protected_directory(File::open(&path).unwrap()).unwrap();
        let binding = || OwnerAccountBinding {
            account_id: NonZeroU64::new(11).unwrap(),
            user_id: NonZeroU64::new(29).unwrap(),
            profile_id: "profile_owner".into(),
            principal_id: serde_json::from_str(r#""user:canonical-owner""#).unwrap(),
        };
        let credential = |password| {
            let credential =
                Credential::derive(password, [0x71; SALT_BYTES], MIN_ITERATIONS).unwrap();
            AccountRecord::new(b"owner", credential).unwrap()
        };
        verifier.require_unprovisioned_private_directory().unwrap();
        verifier
            .publish_first_owner(binding(), credential(b"first-password"))
            .unwrap();
        assert!(matches!(
            verifier.publish_first_owner(binding(), credential(b"other-password")),
            Err(OwnerAccountSaveError::AlreadyExists)
        ));
        let account = AuthenticatedAccountProfile::authenticate(
            &verifier.directory,
            0,
            b"owner",
            b"first-password",
        )
        .unwrap();
        assert_eq!(account.user_id().get(), 29);
        assert_eq!(account.profile_id(), "profile_owner");
        assert_eq!(
            fs::metadata(path.join(ACCOUNT_FILE)).unwrap().mode() & 0o777,
            0o600
        );
        fs::remove_file(path.join(ACCOUNT_FILE)).unwrap();
        fs::remove_dir(path).unwrap();
    }
}
