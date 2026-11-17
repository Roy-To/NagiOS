use std::collections::BTreeMap;

use crate::ids::CapabilityId;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CapabilityDefinition {
    pub id: CapabilityId,
    pub description: String,
}

#[derive(Clone, Debug, Default)]
pub struct CapabilityRegistry {
    definitions: BTreeMap<CapabilityId, CapabilityDefinition>,
}

impl CapabilityRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, definition: CapabilityDefinition) -> Result<(), RegistryError> {
        if definition.description.is_empty()
            || definition.description.len() > 512
            || definition.description.chars().any(char::is_control)
        {
            return Err(RegistryError::InvalidDescription);
        }
        if self.definitions.contains_key(&definition.id) {
            return Err(RegistryError::DuplicateCapability);
        }
        self.definitions.insert(definition.id.clone(), definition);
        Ok(())
    }

    pub fn contains(&self, capability: &CapabilityId) -> bool {
        self.definitions.contains_key(capability)
    }

    pub fn get(&self, capability: &CapabilityId) -> Option<&CapabilityDefinition> {
        self.definitions.get(capability)
    }

    pub fn iter(&self) -> impl Iterator<Item = &CapabilityDefinition> {
        self.definitions.values()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegistryError {
    DuplicateCapability,
    InvalidDescription,
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::DuplicateCapability => "capability is already registered",
            Self::InvalidDescription => {
                "capability description must be bounded and contain no control characters"
            }
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for RegistryError {}
