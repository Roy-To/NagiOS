use nagi_identity_session::{IdSourceFailure, IdentityIdSource, ProfileId, SessionId, UserId};

pub struct LinuxIdentityIdSource;

fn entropy() -> Result<[u8; 16], IdSourceFailure> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|_| IdSourceFailure::Unavailable)?;
    if bytes.iter().all(|byte| *byte == 0) {
        return Err(IdSourceFailure::Unavailable);
    }
    Ok(bytes)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

impl IdentityIdSource for LinuxIdentityIdSource {
    fn next_user_id(&mut self) -> Result<UserId, IdSourceFailure> {
        let mut bytes = [0_u8; 8];
        getrandom::fill(&mut bytes).map_err(|_| IdSourceFailure::Unavailable)?;
        UserId::new(u64::from_le_bytes(bytes)).map_err(|_| IdSourceFailure::Unavailable)
    }

    fn next_profile_id(&mut self) -> Result<ProfileId, IdSourceFailure> {
        ProfileId::new(format!("profile_{}", hex(&entropy()?)))
            .map_err(|_| IdSourceFailure::Unavailable)
    }

    fn next_session_id(&mut self) -> Result<SessionId, IdSourceFailure> {
        SessionId::new(format!("session_{}", hex(&entropy()?)))
            .map_err(|_| IdSourceFailure::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_source_produces_valid_distinct_identifiers() {
        let mut source = LinuxIdentityIdSource;
        let user = source.next_user_id().unwrap();
        let profile_a = source.next_profile_id().unwrap();
        let profile_b = source.next_profile_id().unwrap();
        let session_a = source.next_session_id().unwrap();
        let session_b = source.next_session_id().unwrap();
        assert_ne!(user.get(), 0);
        assert_ne!(profile_a, profile_b);
        assert_ne!(session_a, session_b);
    }
}
