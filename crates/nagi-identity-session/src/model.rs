use std::collections::HashSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{ProfileId, SessionId, UserId};

pub const IDENTITY_SCHEMA_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UnixMillis(u64);

impl UnixMillis {
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Clones must observe the same authoritative clock source so issued identity
/// leases can enforce session deadlines outside the service call path.
pub trait Clock: Clone + Send + Sync + 'static {
    fn now(&self) -> UnixMillis;
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalUserMetadata {
    pub schema_version: u32,
    pub display_name: String,
}

impl LocalUserMetadata {
    pub fn new(display_name: impl Into<String>) -> Result<Self, MetadataError> {
        let display_name = display_name.into();
        validate_display_name(&display_name)?;
        Ok(Self {
            schema_version: 1,
            display_name,
        })
    }

    pub fn validate(&self) -> Result<(), MetadataError> {
        if self.schema_version != 1 {
            return Err(MetadataError::UnsupportedVersion);
        }
        validate_display_name(&self.display_name)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileMetadata {
    pub schema_version: u32,
    pub display_name: String,
    pub locale: Option<String>,
}

impl ProfileMetadata {
    pub fn new(
        display_name: impl Into<String>,
        locale: Option<String>,
    ) -> Result<Self, MetadataError> {
        let value = Self {
            schema_version: 1,
            display_name: display_name.into(),
            locale,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), MetadataError> {
        if self.schema_version != 1 {
            return Err(MetadataError::UnsupportedVersion);
        }
        validate_display_name(&self.display_name)?;
        if self.locale.as_ref().is_some_and(|locale| {
            locale.is_empty()
                || locale.len() > 32
                || !locale
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        }) {
            return Err(MetadataError::InvalidLocale);
        }
        Ok(())
    }
}

fn validate_display_name(value: &str) -> Result<(), MetadataError> {
    if value.trim().is_empty() || value.len() > 128 || value.chars().any(char::is_control) {
        return Err(MetadataError::InvalidDisplayName);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetadataError {
    UnsupportedVersion,
    InvalidDisplayName,
    InvalidLocale,
}

impl fmt::Display for MetadataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::UnsupportedVersion => "unsupported identity metadata version",
            Self::InvalidDisplayName => "display name must be bounded, non-empty, and control-free",
            Self::InvalidLocale => "locale must be a bounded ASCII language tag",
        })
    }
}

impl std::error::Error for MetadataError {}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalUser {
    pub schema_version: u32,
    pub user_id: UserId,
    pub created_at: UnixMillis,
    pub metadata: LocalUserMetadata,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileKind {
    Persistent,
    GuestEphemeral,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileRecord {
    pub schema_version: u32,
    pub profile_id: ProfileId,
    pub owner_user_id: Option<UserId>,
    pub kind: ProfileKind,
    pub created_at: UnixMillis,
    pub metadata: ProfileMetadata,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionKind {
    LocalUser,
    Guest,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionEndReason {
    UserLogout,
    Expired,
    SystemRestart,
    Recovery,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GuestCleanupStatus {
    NotApplicable,
    Pending,
    Complete,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case", deny_unknown_fields)]
pub enum SessionLifecycle {
    Active,
    Ended {
        at: UnixMillis,
        reason: SessionEndReason,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionRecord {
    pub schema_version: u32,
    pub session_id: SessionId,
    pub kind: SessionKind,
    pub user_id: Option<UserId>,
    pub profile_id: ProfileId,
    pub created_at: UnixMillis,
    pub expires_at: Option<UnixMillis>,
    pub lifecycle: SessionLifecycle,
    pub guest_cleanup: GuestCleanupStatus,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IdentitySnapshot {
    pub schema_version: u32,
    pub revision: u64,
    pub users: Vec<LocalUser>,
    pub profiles: Vec<ProfileRecord>,
    pub sessions: Vec<SessionRecord>,
}

impl IdentitySnapshot {
    pub fn empty() -> Self {
        Self {
            schema_version: IDENTITY_SCHEMA_VERSION,
            revision: 0,
            users: Vec::new(),
            profiles: Vec::new(),
            sessions: Vec::new(),
        }
    }

    pub fn validate(&self) -> Result<(), SnapshotError> {
        if self.schema_version != IDENTITY_SCHEMA_VERSION {
            return Err(SnapshotError::UnsupportedVersion(self.schema_version));
        }

        let mut users = HashSet::new();
        for user in &self.users {
            if user.schema_version != IDENTITY_SCHEMA_VERSION
                || !users.insert(user.user_id)
                || user.metadata.validate().is_err()
            {
                return Err(SnapshotError::InvalidUser);
            }
        }

        let mut profiles = HashSet::new();
        for profile in &self.profiles {
            if profile.schema_version != IDENTITY_SCHEMA_VERSION
                || !profiles.insert(profile.profile_id.clone())
                || profile.metadata.validate().is_err()
            {
                return Err(SnapshotError::InvalidProfile);
            }
            match (profile.kind, profile.owner_user_id) {
                (ProfileKind::Persistent, Some(owner)) if users.contains(&owner) => {}
                (ProfileKind::GuestEphemeral, None) => {}
                _ => return Err(SnapshotError::InvalidProfileOwner),
            }
        }

        let mut sessions = HashSet::new();
        for session in &self.sessions {
            if session.schema_version != IDENTITY_SCHEMA_VERSION
                || !sessions.insert(session.session_id.clone())
                || session
                    .expires_at
                    .is_some_and(|expires| expires <= session.created_at)
            {
                return Err(SnapshotError::InvalidSession);
            }
            match (session.kind, session.user_id) {
                (SessionKind::LocalUser, Some(user_id)) => {
                    let Some(profile) = self
                        .profiles
                        .iter()
                        .find(|p| p.profile_id == session.profile_id)
                    else {
                        return Err(SnapshotError::InvalidSession);
                    };
                    if profile.owner_user_id != Some(user_id)
                        || profile.kind != ProfileKind::Persistent
                        || session.guest_cleanup != GuestCleanupStatus::NotApplicable
                    {
                        return Err(SnapshotError::InvalidSession);
                    }
                }
                (SessionKind::Guest, None) => {
                    if session.guest_cleanup == GuestCleanupStatus::NotApplicable {
                        return Err(SnapshotError::InvalidSession);
                    }
                    let profile_exists = self.profiles.iter().any(|p| {
                        p.profile_id == session.profile_id
                            && p.owner_user_id.is_none()
                            && p.kind == ProfileKind::GuestEphemeral
                    });
                    let cleaned = matches!(session.guest_cleanup, GuestCleanupStatus::Complete)
                        && matches!(session.lifecycle, SessionLifecycle::Ended { .. });
                    if !profile_exists && !cleaned {
                        return Err(SnapshotError::InvalidSession);
                    }
                }
                _ => return Err(SnapshotError::InvalidSession),
            }
        }
        Ok(())
    }

    pub fn to_json(&self) -> Result<Vec<u8>, SnapshotError> {
        self.validate()?;
        serde_json::to_vec(self).map_err(|_| SnapshotError::Malformed)
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, SnapshotError> {
        let value: Self = serde_json::from_slice(bytes).map_err(|_| SnapshotError::Malformed)?;
        value.validate()?;
        Ok(value)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SnapshotError {
    Malformed,
    UnsupportedVersion(u32),
    MigrationFailed,
    MigratedStateInvalid,
    InvalidUser,
    InvalidProfile,
    InvalidProfileOwner,
    InvalidSession,
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Malformed => "identity snapshot is malformed",
            Self::UnsupportedVersion(_) => "identity snapshot schema version is unsupported",
            Self::MigrationFailed => "identity snapshot migration failed",
            Self::MigratedStateInvalid => "identity snapshot migration produced invalid state",
            Self::InvalidUser => "identity snapshot contains an invalid or duplicate user",
            Self::InvalidProfile => "identity snapshot contains an invalid or duplicate profile",
            Self::InvalidProfileOwner => "identity snapshot profile owner is inconsistent",
            Self::InvalidSession => "identity snapshot contains an invalid session",
        })
    }
}

impl std::error::Error for SnapshotError {}
