use crate::AppId;

use super::{AppError, AppIdentity, ErrorCode};

/// Fixed-capacity host-side registry for validated canonical app identities.
/// It performs no persistence, package discovery, or runtime launch.
pub struct AppRegistry<'slots, 'identity> {
    slots: &'slots mut [Option<AppIdentity<'identity>>],
    len: usize,
}

impl<'slots, 'identity> AppRegistry<'slots, 'identity> {
    /// Create an empty registry backed by caller-owned storage.
    pub fn new(slots: &'slots mut [Option<AppIdentity<'identity>>]) -> Self {
        slots.fill(None);
        Self { slots, len: 0 }
    }

    /// Register a validated identity. A numeric AppId collision between two
    /// different canonical identifiers is rejected rather than shadowed.
    pub fn register(&mut self, identity: AppIdentity<'identity>) -> Result<(), AppError> {
        identity.validate()?;
        for existing in self.slots.iter().flatten() {
            if let Some(code) = registration_conflict(
                existing.app_id(),
                existing.identifier(),
                identity.app_id(),
                identity.identifier(),
            ) {
                return Err(AppError::new(code));
            }
        }
        let slot = self
            .slots
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(AppError::new(ErrorCode::RegistrationCapacityExceeded))?;
        *slot = Some(identity);
        self.len += 1;
        Ok(())
    }

    pub fn lookup(&self, app_id: AppId) -> Option<AppIdentity<'identity>> {
        self.slots
            .iter()
            .flatten()
            .find(|identity| identity.app_id() == app_id)
            .copied()
    }

    pub const fn len(&self) -> usize {
        self.len
    }

    pub const fn is_empty(&self) -> bool {
        self.len == 0
    }
}

fn registration_conflict(
    existing_app_id: AppId,
    existing_identifier: &str,
    candidate_app_id: AppId,
    candidate_identifier: &str,
) -> Option<ErrorCode> {
    if existing_app_id != candidate_app_id {
        None
    } else if existing_identifier == candidate_identifier {
        Some(ErrorCode::AppAlreadyRegistered)
    } else {
        Some(ErrorCode::AppIdentityCollision)
    }
}

#[cfg(test)]
mod tests {
    use super::{registration_conflict, AppRegistry};
    use crate::app_contract::{AppIdentity, AppOrigin, ErrorCode};
    use crate::AppId;

    #[test]
    fn registry_rejects_duplicate_and_capacity_overflow() {
        let first =
            AppIdentity::new("com.example.notes", "1.0.0", AppOrigin::ThirdParty, None).unwrap();
        let second =
            AppIdentity::new("org.example.files", "1.0.0", AppOrigin::ThirdParty, None).unwrap();
        let mut storage = [None; 1];
        let mut registry = AppRegistry::new(&mut storage);
        assert!(registry.is_empty());
        registry.register(first).unwrap();
        assert_eq!(registry.len(), 1);
        assert_eq!(registry.lookup(first.app_id()), Some(first));
        assert_eq!(
            registry.register(first),
            Err(crate::app_contract::AppError::new(
                ErrorCode::AppAlreadyRegistered
            ))
        );
        assert_eq!(
            registry.register(second),
            Err(crate::app_contract::AppError::new(
                ErrorCode::RegistrationCapacityExceeded
            ))
        );
    }

    #[test]
    fn registry_distinguishes_hash_collision_from_duplicate_identifier() {
        assert_eq!(
            registration_conflict(AppId(99), "com.example.one", AppId(99), "org.example.two"),
            Some(ErrorCode::AppIdentityCollision)
        );
        assert_eq!(
            registration_conflict(AppId(99), "com.example.one", AppId(99), "com.example.one"),
            Some(ErrorCode::AppAlreadyRegistered)
        );
        assert_eq!(
            registration_conflict(AppId(99), "com.example.one", AppId(100), "org.example.two"),
            None
        );
    }
}
