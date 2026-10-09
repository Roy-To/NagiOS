//! Bind the OS-owned model service to one authenticated desktop session.
//! This check does not grant external apps an AI capability or expose context.

use libnagi::security::Session;
use nagi_model::AppId;

pub struct ModelSessionAccess {
    token: u64,
}

impl ModelSessionAccess {
    pub fn bind(user: &Session) -> Option<Self> {
        (!user.is_locked() && user.token() != 0).then_some(Self {
            token: user.token(),
        })
    }

    /// The live Session is supplied by the OS desktop, never by request text.
    pub fn allows(&self, user: &Session, external_caller: Option<AppId>) -> bool {
        !user.is_locked() && user.token() == self.token && external_caller.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use libnagi::security::{AccountStore, Role};

    fn sessions() -> (Session, Session) {
        let mut accounts = AccountStore::new();
        accounts
            .add_account(b"owner", Role::Owner, b"a test password")
            .unwrap();
        let first = accounts.authenticate(b"owner", b"a test password").unwrap();
        let second = accounts.authenticate(b"owner", b"a test password").unwrap();
        (first, second)
    }

    #[test]
    fn accepts_only_the_bound_signed_in_session() {
        let (first, second) = sessions();
        let access = ModelSessionAccess::bind(&first).unwrap();
        assert!(access.allows(&first, None));
        assert!(!access.allows(&second, None));
    }

    #[test]
    fn locking_blocks_inference_and_service_construction() {
        let (mut user, _) = sessions();
        let access = ModelSessionAccess::bind(&user).unwrap();
        user.lock();
        assert!(!access.allows(&user, None));
        assert!(ModelSessionAccess::bind(&user).is_none());
    }

    #[test]
    fn model_external_caller_is_not_authorized_by_a_desktop_session() {
        let (user, _) = sessions();
        let access = ModelSessionAccess::bind(&user).unwrap();
        assert!(!access.allows(&user, Some(AppId::from_identifier(b"org.nagi.files"))));
    }
}
