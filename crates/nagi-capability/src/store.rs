use std::collections::BTreeSet;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::model::{Grant, GrantId, GrantValidationError, UnixTimestamp};

pub const POLICY_SCHEMA_VERSION: u32 = 1;

/// Each transaction must be atomic and durable before success is reported.
/// One-shot grants depend on this to prevent replay across concurrent checks.
pub trait PolicyStore {
    fn transact<T>(
        &mut self,
        operation: impl FnOnce(&mut PolicyDocument) -> Result<T, PolicyStoreError>,
    ) -> Result<T, PolicyStoreError>;

    fn snapshot(&self) -> Result<PolicyDocument, PolicyStoreError>;
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyDocument {
    schema_version: u32,
    next_grant_id: u64,
    grants: Vec<Grant>,
}

impl PolicyDocument {
    pub fn new() -> Self {
        Self {
            schema_version: POLICY_SCHEMA_VERSION,
            next_grant_id: 1,
            grants: Vec::new(),
        }
    }

    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub fn grants(&self) -> &[Grant] {
        &self.grants
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, PolicyStoreError> {
        let document: Self = serde_json::from_slice(bytes)
            .map_err(|error| PolicyStoreError::Malformed(error.to_string()))?;
        document.validate()?;
        Ok(document)
    }

    pub fn to_json(&self) -> Result<String, PolicyStoreError> {
        self.validate()?;
        serde_json::to_string_pretty(self)
            .map_err(|error| PolicyStoreError::Malformed(error.to_string()))
    }

    pub fn validate(&self) -> Result<(), PolicyStoreError> {
        if self.schema_version != POLICY_SCHEMA_VERSION {
            return Err(PolicyStoreError::UnsupportedSchemaVersion(
                self.schema_version,
            ));
        }
        if self.next_grant_id == 0 {
            return Err(PolicyStoreError::Malformed(
                "next_grant_id must be positive".into(),
            ));
        }
        let mut ids = BTreeSet::new();
        let mut maximum_id = 0_u64;
        for grant in &self.grants {
            grant.validate().map_err(PolicyStoreError::InvalidGrant)?;
            if !ids.insert(grant.id) {
                return Err(PolicyStoreError::Malformed(
                    "persisted policy contains duplicate grant IDs".into(),
                ));
            }
            maximum_id = maximum_id.max(grant.id.get());
        }
        if self.next_grant_id <= maximum_id {
            return Err(PolicyStoreError::Malformed(
                "next_grant_id must be greater than every stored grant ID".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn allocate_grant_id(&mut self) -> Result<GrantId, PolicyStoreError> {
        let id = self.next_grant_id;
        self.next_grant_id = id
            .checked_add(1)
            .ok_or(PolicyStoreError::GrantIdExhausted)?;
        Ok(GrantId::new(id))
    }

    pub(crate) fn push_grant(&mut self, grant: Grant) {
        self.grants.push(grant);
    }

    pub(crate) fn grant_mut(&mut self, id: GrantId) -> Option<&mut Grant> {
        self.grants.iter_mut().find(|grant| grant.id == id)
    }

    pub(crate) fn consume_one_shot(
        &mut self,
        id: GrantId,
        at: UnixTimestamp,
    ) -> Result<(), PolicyStoreError> {
        let grant = self.grant_mut(id).ok_or_else(|| {
            PolicyStoreError::Malformed(
                "selected grant disappeared during policy transaction".into(),
            )
        })?;
        grant.consumed_at = Some(at);
        Ok(())
    }
}

impl Default for PolicyDocument {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Debug, Default)]
pub struct InMemoryPolicyStore {
    document: PolicyDocument,
}

impl InMemoryPolicyStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn from_json(bytes: &[u8]) -> Result<Self, PolicyStoreError> {
        Ok(Self {
            document: PolicyDocument::from_json(bytes)?,
        })
    }

    pub fn to_json(&self) -> Result<String, PolicyStoreError> {
        self.document.to_json()
    }
}

impl PolicyStore for InMemoryPolicyStore {
    fn transact<T>(
        &mut self,
        operation: impl FnOnce(&mut PolicyDocument) -> Result<T, PolicyStoreError>,
    ) -> Result<T, PolicyStoreError> {
        let mut candidate = self.document.clone();
        let result = operation(&mut candidate)?;
        candidate.validate()?;
        self.document = candidate;
        Ok(result)
    }

    fn snapshot(&self) -> Result<PolicyDocument, PolicyStoreError> {
        self.document.validate()?;
        Ok(self.document.clone())
    }
}

/// Deterministic failure fixture for verifying fail-closed storage behavior.
#[derive(Clone, Copy, Debug, Default)]
pub struct FailingPolicyStore;

impl PolicyStore for FailingPolicyStore {
    fn transact<T>(
        &mut self,
        _operation: impl FnOnce(&mut PolicyDocument) -> Result<T, PolicyStoreError>,
    ) -> Result<T, PolicyStoreError> {
        Err(PolicyStoreError::Unavailable("test storage failure".into()))
    }

    fn snapshot(&self) -> Result<PolicyDocument, PolicyStoreError> {
        Err(PolicyStoreError::Unavailable("test storage failure".into()))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PolicyStoreError {
    Malformed(String),
    UnsupportedSchemaVersion(u32),
    InvalidGrant(GrantValidationError),
    Unavailable(String),
    GrantIdExhausted,
}

impl fmt::Display for PolicyStoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(message) => write!(formatter, "invalid persisted policy: {message}"),
            Self::UnsupportedSchemaVersion(version) => {
                write!(
                    formatter,
                    "unsupported persistent policy schema version {version}"
                )
            }
            Self::InvalidGrant(error) => write!(formatter, "invalid persisted grant: {error}"),
            Self::Unavailable(message) => write!(formatter, "policy store unavailable: {message}"),
            Self::GrantIdExhausted => formatter.write_str("grant identifier space is exhausted"),
        }
    }
}

impl std::error::Error for PolicyStoreError {}
