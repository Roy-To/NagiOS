//! Consumer of the existing public libnagi credential implementation.
//! Its legacy Session is not SDK Identity evidence or a trusted IPC caller.

use libnagi::security::{AccountStore, LoginError, Session};

struct CredentialEntry<'accounts> {
    accounts: &'accounts mut AccountStore,
}

impl<'accounts> CredentialEntry<'accounts> {
    fn new(accounts: &'accounts mut AccountStore) -> Self {
        Self { accounts }
    }

    // Credentials are borrowed for this call only. No hashing, enrollment or
    // login policy is reimplemented; the actual crate performs verification.
    fn authenticate(&mut self, name: &[u8], password: &[u8]) -> Result<Session, LoginError> {
        self.accounts.authenticate(name, password)
    }

    fn unlock(&self, session: &mut Session, password: &[u8]) -> bool {
        self.accounts.unlock(session, password)
    }
}

// Intentionally no conversion to UserId, ProfileId, PrincipalId, caller context
// or RuntimeIngress. The legacy public Session has no canonical SDK account
// binding, and its role/token are not authority for creating that binding.

#[cfg(test)]
mod tests {
    use super::*;
    use libnagi::security::{
        AppTrust, DenyReason, PermissionBroker, PermissionDecision, PermissionRequest, Resource,
        Role,
    };

    // All account/credential data is synthetic and belongs to these tests.
    const NAME: &[u8] = b"test-local";
    const PASSWORD: &[u8] = b"test-only-secret";

    fn accounts() -> AccountStore {
        let mut store = AccountStore::new();
        store.add_account(NAME, Role::Standard, PASSWORD).unwrap();
        store
    }

    fn decide(session: Session) -> PermissionDecision {
        PermissionBroker::new().decide(
            session,
            PermissionRequest {
                app_trust: AppTrust::TrustedForeground,
                resource: Resource::FileRead,
                foreground: true,
                explicit_consent: true,
            },
        )
    }

    #[test]
    fn credential_real_verifier_accepts_correct_test_credential() {
        let mut store = accounts();
        let mut entry = CredentialEntry::new(&mut store);
        let session = entry.authenticate(NAME, PASSWORD).unwrap();
        assert_eq!(session.role(), Role::Standard);
        assert_ne!(session.token(), 0);
        assert!(!session.is_locked());
    }

    #[test]
    fn credential_real_verifier_preserves_wrong_password_and_unknown_account_errors() {
        let mut store = accounts();
        let mut entry = CredentialEntry::new(&mut store);
        assert_eq!(
            entry.authenticate(NAME, b"wrong-test-secret"),
            Err(LoginError::WrongPassword)
        );
        assert_eq!(
            entry.authenticate(b"unknown-test", PASSWORD),
            Err(LoginError::InvalidAccount)
        );
        assert!(entry.authenticate(NAME, PASSWORD).is_ok());
    }

    #[test]
    fn credential_real_lock_and_unlock_require_the_correct_test_credential() {
        let mut store = accounts();
        let mut entry = CredentialEntry::new(&mut store);
        let mut session = entry.authenticate(NAME, PASSWORD).unwrap();
        session.lock();
        assert_eq!(
            decide(session),
            PermissionDecision::Deny(DenyReason::SessionLocked)
        );
        assert!(!entry.unlock(&mut session, b"wrong-test-secret"));
        assert!(session.is_locked());
        assert!(entry.unlock(&mut session, PASSWORD));
        assert!(!session.is_locked());
    }

    #[test]
    fn credential_relogin_changes_token_but_does_not_globally_revoke_legacy_copies() {
        let mut store = accounts();
        let mut entry = CredentialEntry::new(&mut store);
        let mut first = entry.authenticate(NAME, PASSWORD).unwrap();
        let earlier_copy = first;
        first.lock();
        let second = entry.authenticate(NAME, PASSWORD).unwrap();
        assert_ne!(first.token(), second.token());
        assert_eq!(
            decide(first),
            PermissionDecision::Deny(DenyReason::SessionLocked)
        );
        // Existing verifier/broker behavior: a copied token has no issuer
        // revocation/expiry check. Do not present it as an SDK live session.
        assert_eq!(decide(earlier_copy), PermissionDecision::Allow);
        assert_eq!(decide(second), PermissionDecision::Allow);
    }
}
