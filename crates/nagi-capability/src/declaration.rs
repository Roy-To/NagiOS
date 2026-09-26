use serde::{Deserialize, Serialize};

use crate::ids::{CapabilityId, IdentifierError, PrincipalId};
use crate::model::CapabilityScope;

pub const DECLARATION_SCHEMA_VERSION: u32 = 1;
const MAX_PURPOSE_BYTES: usize = 512;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityDeclaration {
    pub schema_version: u32,
    pub requested_capabilities: Vec<DeclaredCapability>,
}

impl CapabilityDeclaration {
    pub fn parse_json(bytes: &[u8]) -> Result<Self, DeclarationError> {
        let declaration: Self = serde_json::from_slice(bytes)
            .map_err(|error| DeclarationError::Malformed(error.to_string()))?;
        declaration.validate()?;
        Ok(declaration)
    }

    pub fn to_json(&self) -> Result<String, DeclarationError> {
        self.validate()?;
        serde_json::to_string_pretty(self)
            .map_err(|error| DeclarationError::Malformed(error.to_string()))
    }

    pub fn validate(&self) -> Result<(), DeclarationError> {
        if self.schema_version != DECLARATION_SCHEMA_VERSION {
            return Err(DeclarationError::UnsupportedVersion(self.schema_version));
        }
        for requested in &self.requested_capabilities {
            requested.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeclaredCapability {
    pub capability: CapabilityId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<CapabilityScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
}

impl DeclaredCapability {
    fn validate(&self) -> Result<(), DeclarationError> {
        if let Some(scope) = &self.scope {
            scope
                .validate()
                .map_err(|error| DeclarationError::InvalidScope(error.to_string()))?;
        }
        if let Some(purpose) = &self.purpose {
            if purpose.is_empty()
                || purpose.len() > MAX_PURPOSE_BYTES
                || purpose.chars().any(char::is_control)
            {
                return Err(DeclarationError::InvalidPurpose);
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundCapabilityRequest {
    pub principal: PrincipalId,
    pub capability: CapabilityId,
    pub scope: CapabilityScope,
}

pub fn bind_declaration(
    declaration: &CapabilityDeclaration,
    trusted_principal: &PrincipalId,
) -> Result<Vec<BoundCapabilityRequest>, DeclarationError> {
    declaration.validate()?;
    Ok(declaration
        .requested_capabilities
        .iter()
        .map(|requested| BoundCapabilityRequest {
            principal: trusted_principal.clone(),
            capability: requested.capability.clone(),
            scope: requested.scope.clone().unwrap_or(CapabilityScope::Unscoped),
        })
        .collect())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeclarationError {
    Malformed(String),
    UnsupportedVersion(u32),
    InvalidScope(String),
    InvalidPurpose,
    InvalidIdentifier(IdentifierError),
}

impl std::fmt::Display for DeclarationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Malformed(message) => {
                write!(formatter, "malformed capability declaration: {message}")
            }
            Self::UnsupportedVersion(version) => {
                write!(
                    formatter,
                    "unsupported capability declaration schema version {version}"
                )
            }
            Self::InvalidScope(message) => write!(formatter, "invalid capability scope: {message}"),
            Self::InvalidPurpose => formatter.write_str(
                "purpose must be non-empty, at most 512 bytes, and contain no control characters",
            ),
            Self::InvalidIdentifier(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for DeclarationError {}

impl From<IdentifierError> for DeclarationError {
    fn from(error: IdentifierError) -> Self {
        Self::InvalidIdentifier(error)
    }
}
