use alloc::{boxed::Box, collections::BTreeMap, string::String, vec::Vec};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use nagi_model::ObjectId;
use nagi_model_manager::CapabilityId;

use crate::{CallerIdentity, MAX_OBJECTS_PER_STEP, MAX_PARAMETERS_PER_STEP};

pub const MAX_REGISTERED_ACTIONS: usize = 64;
pub const MAX_ACTION_ID_BYTES: usize = 96;
pub const MAX_PARAMETER_NAME_BYTES: usize = 64;
pub const MAX_ACTION_STRING_BYTES: usize = 4096;
pub const MAX_OUTPUT_SUMMARY_BYTES: usize = 1024;
pub const MAX_OUTPUT_OBJECTS: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObjectAccess {
    Read,
    Modify,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum ParameterKind {
    String { max_bytes: usize },
    Integer { min: i64, max: i64 },
    Boolean,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ParameterRule {
    pub name: String,
    pub kind: ParameterKind,
    pub required: bool,
}

impl ParameterRule {
    pub fn new(name: impl Into<String>, kind: ParameterKind, required: bool) -> Self {
        Self {
            name: name.into(),
            kind,
            required,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ActionDescriptor {
    pub(crate) action_id: String,
    pub(crate) required_capabilities: Vec<CapabilityId>,
    pub(crate) object_access: ObjectAccess,
    pub(crate) min_objects: usize,
    pub(crate) max_objects: usize,
    pub(crate) parameters: Vec<ParameterRule>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegistryError {
    InvalidActionId,
    MissingCapability,
    InvalidObjectBounds,
    TooManyParameters,
    InvalidParameterName,
    DuplicateParameter,
    InvalidParameterBounds,
    DuplicateCapability,
    DuplicateAction,
    Capacity,
}

impl ActionDescriptor {
    pub fn new(
        action_id: impl Into<String>,
        required_capabilities: Vec<CapabilityId>,
        object_access: ObjectAccess,
        min_objects: usize,
        max_objects: usize,
        parameters: Vec<ParameterRule>,
    ) -> Result<Self, RegistryError> {
        let action_id = action_id.into();
        if !valid_action_id(&action_id) {
            return Err(RegistryError::InvalidActionId);
        }
        if required_capabilities.is_empty() {
            return Err(RegistryError::MissingCapability);
        }
        for (index, capability) in required_capabilities.iter().enumerate() {
            if required_capabilities[..index].contains(capability) {
                return Err(RegistryError::DuplicateCapability);
            }
        }
        if min_objects > max_objects || max_objects > MAX_OBJECTS_PER_STEP {
            return Err(RegistryError::InvalidObjectBounds);
        }
        if parameters.len() > MAX_PARAMETERS_PER_STEP {
            return Err(RegistryError::TooManyParameters);
        }
        for (index, parameter) in parameters.iter().enumerate() {
            if !valid_parameter_name(&parameter.name) {
                return Err(RegistryError::InvalidParameterName);
            }
            if parameters[..index]
                .iter()
                .any(|existing| existing.name == parameter.name)
            {
                return Err(RegistryError::DuplicateParameter);
            }
            if let ParameterKind::String { max_bytes } = parameter.kind {
                if max_bytes == 0 || max_bytes > MAX_ACTION_STRING_BYTES {
                    return Err(RegistryError::InvalidParameterBounds);
                }
            }
            if let ParameterKind::Integer { min, max } = parameter.kind {
                if min > max {
                    return Err(RegistryError::InvalidParameterBounds);
                }
            }
        }
        Ok(Self {
            action_id,
            required_capabilities,
            object_access,
            min_objects,
            max_objects,
            parameters,
        })
    }

    pub fn action_id(&self) -> &str {
        &self.action_id
    }

    pub fn required_capabilities(&self) -> &[CapabilityId] {
        &self.required_capabilities
    }

    pub fn object_access(&self) -> ObjectAccess {
        self.object_access
    }

    pub fn object_count_bounds(&self) -> (usize, usize) {
        (self.min_objects, self.max_objects)
    }

    pub fn parameters(&self) -> &[ParameterRule] {
        &self.parameters
    }
}

pub fn valid_action_id(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_ACTION_ID_BYTES {
        return false;
    }
    let mut previous_dot = true;
    for byte in value.bytes() {
        match byte {
            b'a'..=b'z' | b'0'..=b'9' | b'_' | b'-' => previous_dot = false,
            b'.' if !previous_dot => previous_dot = true,
            _ => return false,
        }
    }
    !previous_dot
}

fn valid_parameter_name(value: &str) -> bool {
    if value.is_empty() || value.len() > MAX_PARAMETER_NAME_BYTES {
        return false;
    }
    // Plans contain logical Object IDs, never model-invented resource paths or
    // shell instructions. These names cannot be admitted by action metadata.
    if value.contains("path")
        || matches!(
            value,
            "command" | "shell" | "argv" | "script" | "executable"
        )
    {
        return false;
    }
    let mut bytes = value.bytes();
    if !bytes.next().is_some_and(|byte| byte.is_ascii_lowercase()) {
        return false;
    }
    bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

pub struct ActionOutput {
    pub summary: String,
    pub object_ids: Vec<ObjectId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum HandlerError {
    Failed,
    Unavailable,
}

pub struct ActionInvocation<'a, P: ActionPolicy> {
    pub(crate) caller: CallerIdentity,
    pub(crate) plan_intent: &'a str,
    pub(crate) action: &'a ActionDescriptor,
    pub(crate) object_ids: &'a [ObjectId],
    pub(crate) parameters: &'a BTreeMap<String, Value>,
    pub(crate) capability_grants: &'a [P::CapabilityGrant],
    pub(crate) object_handles: &'a [P::ObjectHandle],
}

impl<P: ActionPolicy> ActionInvocation<'_, P> {
    pub fn caller(&self) -> CallerIdentity {
        self.caller
    }

    /// The validated user intent associated with this action plan.
    pub fn plan_intent(&self) -> &str {
        self.plan_intent
    }

    pub fn action(&self) -> &ActionDescriptor {
        self.action
    }

    pub fn object_ids(&self) -> &[ObjectId] {
        self.object_ids
    }

    pub fn parameters(&self) -> &BTreeMap<String, Value> {
        self.parameters
    }

    /// Reads a validated string parameter without exposing the JSON parser to
    /// every first-party action adapter.
    pub fn string_parameter(&self, name: &str) -> Option<&str> {
        self.parameters.get(name).and_then(Value::as_str)
    }

    pub fn capability_grants(&self) -> &[P::CapabilityGrant] {
        self.capability_grants
    }

    pub fn object_handles(&self) -> &[P::ObjectHandle] {
        self.object_handles
    }
}

pub trait ActionHandler<P: ActionPolicy> {
    fn execute(
        &mut self,
        invocation: ActionInvocation<'_, P>,
    ) -> Result<ActionOutput, HandlerError>;
}

/// Implementations must derive grants and object handles from the trusted
/// caller/service authority. Caller metadata and provider confidence are not
/// authority. There is intentionally no allow-all production implementation.
pub trait ActionPolicy {
    type CapabilityGrant;
    type ObjectHandle;

    fn check_capability(
        &self,
        caller: CallerIdentity,
        capability: &CapabilityId,
    ) -> Result<(), PolicyDenied>;

    fn check_object_access(
        &self,
        caller: CallerIdentity,
        object_id: ObjectId,
        access: ObjectAccess,
    ) -> Result<(), PolicyDenied>;

    fn acquire_capability(
        &self,
        caller: CallerIdentity,
        capability: &CapabilityId,
    ) -> Result<Self::CapabilityGrant, PolicyDenied>;

    fn resolve_object(
        &self,
        caller: CallerIdentity,
        object_id: ObjectId,
        access: ObjectAccess,
    ) -> Result<Self::ObjectHandle, PolicyDenied>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyDenied {
    Capability,
    Object,
}

pub struct RegisteredAction<P: ActionPolicy> {
    pub descriptor: ActionDescriptor,
    pub handler: Box<dyn ActionHandler<P>>,
}

pub struct ActionRegistry<P: ActionPolicy> {
    actions: BTreeMap<String, RegisteredAction<P>>,
}

impl<P: ActionPolicy> Default for ActionRegistry<P> {
    fn default() -> Self {
        Self::new()
    }
}

impl<P: ActionPolicy> ActionRegistry<P> {
    pub fn new() -> Self {
        Self {
            actions: BTreeMap::new(),
        }
    }

    pub fn register(
        &mut self,
        descriptor: ActionDescriptor,
        handler: impl ActionHandler<P> + 'static,
    ) -> Result<(), RegistryError> {
        if self.actions.contains_key(&descriptor.action_id) {
            return Err(RegistryError::DuplicateAction);
        }
        if self.actions.len() >= MAX_REGISTERED_ACTIONS {
            return Err(RegistryError::Capacity);
        }
        self.actions.insert(
            descriptor.action_id.clone(),
            RegisteredAction {
                descriptor,
                handler: Box::new(handler),
            },
        );
        Ok(())
    }

    pub fn descriptor(&self, action_id: &str) -> Option<&ActionDescriptor> {
        self.actions.get(action_id).map(|entry| &entry.descriptor)
    }

    pub(crate) fn registered_mut(&mut self, action_id: &str) -> Option<&mut RegisteredAction<P>> {
        self.actions.get_mut(action_id)
    }
}

pub(crate) fn validate_parameters(
    descriptor: &ActionDescriptor,
    parameters: &BTreeMap<String, Value>,
) -> Result<(), ParameterError> {
    if parameters.len() > MAX_PARAMETERS_PER_STEP {
        return Err(ParameterError::TooMany);
    }
    for (name, value) in parameters {
        let rule = descriptor
            .parameters
            .iter()
            .find(|rule| rule.name == *name)
            .ok_or(ParameterError::Unknown)?;
        let valid = match rule.kind {
            ParameterKind::String { max_bytes } => value
                .as_str()
                .is_some_and(|text| !text.is_empty() && text.len() <= max_bytes),
            ParameterKind::Integer { min, max } => value
                .as_i64()
                .is_some_and(|integer| integer >= min && integer <= max),
            ParameterKind::Boolean => value.is_boolean(),
        };
        if !valid {
            return Err(ParameterError::InvalidValue);
        }
    }
    for rule in &descriptor.parameters {
        if rule.required && !parameters.contains_key(&rule.name) {
            return Err(ParameterError::Missing);
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ParameterError {
    TooMany,
    Unknown,
    Missing,
    InvalidValue,
}
