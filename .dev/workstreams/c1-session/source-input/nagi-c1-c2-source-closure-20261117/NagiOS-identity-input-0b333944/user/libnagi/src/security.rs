//! Bounded user-space account, session, and permission policy.
//!
//! This module deliberately does not grant kernel capabilities. It decides
//! whether a user-space service may ask a later resource service to act.

#![allow(clippy::module_name_repetitions)]

const MAX_ACCOUNTS: usize = 4;
const MAX_ACCOUNT_NAME: usize = 16;
const TOKEN_SEED: u64 = 0x4e41_4749_5345_5353;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Role {
    Owner = 0,
    Standard = 1,
    Guest = 2,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AppTrust {
    TrustedForeground,
    Untrusted,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Resource {
    FileRead,
    FileWrite,
    MicrophoneCapture,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionDecision {
    Allow,
    Ask,
    Deny(DenyReason),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DenyReason {
    SessionLocked,
    UntrustedApp,
    NoForegroundConsent,
    InvalidSession,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LoginError {
    InvalidAccount,
    WrongPassword,
    StoreFull,
    InvalidName,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PermissionRequest {
    pub app_trust: AppTrust,
    pub resource: Resource,
    pub foreground: bool,
    pub explicit_consent: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Session {
    account_index: u8,
    role: Role,
    token: u64,
    locked: bool,
    developer_mode: bool,
}

impl Session {
    pub const fn role(self) -> Role {
        self.role
    }

    pub const fn token(self) -> u64 {
        self.token
    }

    pub const fn is_locked(self) -> bool {
        self.locked
    }

    pub const fn developer_mode(self) -> bool {
        self.developer_mode
    }

    pub fn lock(&mut self) {
        self.locked = true;
    }
}

#[derive(Clone, Copy)]
struct Account {
    name: [u8; MAX_ACCOUNT_NAME],
    name_len: u8,
    role: Role,
    password_hash: u64,
}

impl Account {
    const fn empty() -> Self {
        Self {
            name: [0; MAX_ACCOUNT_NAME],
            name_len: 0,
            role: Role::Guest,
            password_hash: 0,
        }
    }
}

pub struct AccountStore {
    accounts: [Account; MAX_ACCOUNTS],
    count: usize,
    next_token: u64,
}

impl AccountStore {
    pub const fn new() -> Self {
        Self {
            accounts: [Account::empty(); MAX_ACCOUNTS],
            count: 0,
            next_token: TOKEN_SEED,
        }
    }

    pub fn add_account(
        &mut self,
        name: &[u8],
        role: Role,
        password: &[u8],
    ) -> Result<(), LoginError> {
        if name.is_empty() || name.len() > MAX_ACCOUNT_NAME {
            return Err(LoginError::InvalidName);
        }
        if self.count == MAX_ACCOUNTS {
            return Err(LoginError::StoreFull);
        }
        let mut account = Account::empty();
        let mut name_index = 0;
        while name_index < name.len() {
            unsafe {
                core::ptr::write(
                    account.name.as_mut_ptr().add(name_index),
                    core::ptr::read(name.as_ptr().add(name_index)),
                );
            }
            name_index += 1;
        }
        account.name_len = name.len() as u8;
        account.role = role;
        account.password_hash = password_hash(password);
        unsafe { core::ptr::write(self.accounts.as_mut_ptr().add(self.count), account) };
        self.count += 1;
        Ok(())
    }

    pub fn authenticate(&mut self, name: &[u8], password: &[u8]) -> Result<Session, LoginError> {
        let password_hash = password_hash(password);
        let mut index = 0;
        while index < self.count {
            let account = unsafe { core::ptr::read(self.accounts.as_ptr().add(index)) };
            if account.name_len as usize == name.len()
                && bytes_equal(account.name.as_ptr(), account.name_len as usize, name)
            {
                if account.password_hash != password_hash {
                    return Err(LoginError::WrongPassword);
                }
                self.next_token = self.next_token.wrapping_add(1);
                return Ok(Session {
                    account_index: index as u8,
                    role: account.role,
                    token: self.next_token,
                    locked: false,
                    developer_mode: false,
                });
            }
            index += 1;
        }
        Err(LoginError::InvalidAccount)
    }

    pub fn unlock(&self, session: &mut Session, password: &[u8]) -> bool {
        if session.account_index as usize >= self.count {
            return false;
        }
        let account =
            unsafe { core::ptr::read(self.accounts.as_ptr().add(session.account_index as usize)) };
        if account.password_hash != password_hash(password) {
            return false;
        }
        session.locked = false;
        true
    }

    pub fn enable_developer_mode(&self, session: &mut Session) -> bool {
        if session.locked || session.role != Role::Owner {
            return false;
        }
        session.developer_mode = true;
        true
    }
}

impl Default for AccountStore {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PermissionBroker {
    developer_mode: bool,
}

impl PermissionBroker {
    pub const fn new() -> Self {
        Self {
            developer_mode: false,
        }
    }

    pub const fn developer_mode(self) -> bool {
        self.developer_mode
    }

    pub fn sync_session(&mut self, session: Session) {
        self.developer_mode = session.developer_mode;
    }

    pub fn decide(&self, session: Session, request: PermissionRequest) -> PermissionDecision {
        if session.token == 0 {
            return PermissionDecision::Deny(DenyReason::InvalidSession);
        }
        if session.locked {
            return PermissionDecision::Deny(DenyReason::SessionLocked);
        }
        if request.app_trust == AppTrust::Untrusted {
            return PermissionDecision::Deny(DenyReason::UntrustedApp);
        }
        if !request.foreground || !request.explicit_consent {
            return PermissionDecision::Ask;
        }
        let _ = self.developer_mode;
        PermissionDecision::Allow
    }
}

impl Default for PermissionBroker {
    fn default() -> Self {
        Self::new()
    }
}

fn password_hash(password: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    let mut index = 0;
    while index < password.len() {
        hash ^= u64::from(password[index]);
        hash = hash.wrapping_mul(0x1000_0000_01b3);
        index += 1;
    }
    hash
}

fn bytes_equal(left: *const u8, left_len: usize, right: &[u8]) -> bool {
    if left_len != right.len() {
        return false;
    }
    let mut different = 0_u8;
    let mut index = 0;
    while index < left_len {
        let left_byte = unsafe { core::ptr::read(left.add(index)) };
        let right_byte = unsafe { core::ptr::read(right.as_ptr().add(index)) };
        different |= left_byte ^ right_byte;
        index += 1;
    }
    different == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> AccountStore {
        let mut store = AccountStore::new();
        store
            .add_account(b"owner", Role::Owner, b"owner-pass")
            .expect("owner");
        store
            .add_account(b"standard", Role::Standard, b"standard-pass")
            .expect("standard");
        store
    }

    fn request(resource: Resource, trust: AppTrust, consent: bool) -> PermissionRequest {
        PermissionRequest {
            app_trust: trust,
            resource,
            foreground: true,
            explicit_consent: consent,
        }
    }

    #[test]
    fn authenticates_role_without_retaining_plaintext() {
        let mut store = store();
        let session = store.authenticate(b"owner", b"owner-pass").expect("login");
        assert_eq!(session.role(), Role::Owner);
        assert_ne!(session.token(), 0);
        assert_eq!(
            store.authenticate(b"owner", b"wrong"),
            Err(LoginError::WrongPassword)
        );
    }

    #[test]
    fn lock_requires_the_matching_password_to_unlock() {
        let mut store = store();
        let mut session = store
            .authenticate(b"standard", b"standard-pass")
            .expect("login");
        session.lock();
        assert!(!store.unlock(&mut session, b"wrong"));
        assert!(session.is_locked());
        assert!(store.unlock(&mut session, b"standard-pass"));
        assert!(!session.is_locked());
    }

    #[test]
    fn only_owner_can_enable_developer_mode() {
        let mut store = store();
        let mut owner = store
            .authenticate(b"owner", b"owner-pass")
            .expect("owner login");
        let mut standard = store
            .authenticate(b"standard", b"standard-pass")
            .expect("standard login");
        assert!(store.enable_developer_mode(&mut owner));
        assert!(owner.developer_mode());
        assert!(!store.enable_developer_mode(&mut standard));
        assert!(!standard.developer_mode());
    }

    #[test]
    fn trusted_foreground_requests_ask_then_allow_with_consent() {
        let mut store = store();
        let session = store
            .authenticate(b"standard", b"standard-pass")
            .expect("login");
        let broker = PermissionBroker::new();
        assert_eq!(
            broker.decide(
                session,
                request(Resource::FileRead, AppTrust::TrustedForeground, false)
            ),
            PermissionDecision::Ask
        );
        assert_eq!(
            broker.decide(
                session,
                request(Resource::FileRead, AppTrust::TrustedForeground, true)
            ),
            PermissionDecision::Allow
        );
    }

    #[test]
    fn untrusted_file_and_microphone_are_denied_in_developer_mode() {
        let mut store = store();
        let mut owner = store.authenticate(b"owner", b"owner-pass").expect("login");
        assert!(store.enable_developer_mode(&mut owner));
        let mut broker = PermissionBroker::new();
        broker.sync_session(owner);
        for resource in [
            Resource::FileRead,
            Resource::FileWrite,
            Resource::MicrophoneCapture,
        ] {
            assert_eq!(
                broker.decide(owner, request(resource, AppTrust::Untrusted, true)),
                PermissionDecision::Deny(DenyReason::UntrustedApp)
            );
        }
    }

    #[test]
    fn locked_sessions_fail_closed_before_app_policy() {
        let mut store = store();
        let mut session = store.authenticate(b"owner", b"owner-pass").expect("login");
        session.lock();
        let broker = PermissionBroker::new();
        assert_eq!(
            broker.decide(
                session,
                request(Resource::FileRead, AppTrust::TrustedForeground, true)
            ),
            PermissionDecision::Deny(DenyReason::SessionLocked)
        );
    }
}
