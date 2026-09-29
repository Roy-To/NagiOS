use alloc::vec::Vec;

use nagi_model::ObjectId;

use crate::{
    validate_parameters, ActionDescriptor, ActionPolicy, ActionRegistry, CallerIdentity, NagiPlan,
    ParameterError, PolicyDenied, ResolvedContext, MAX_INTENT_BYTES, MAX_OBJECTS_PER_STEP,
    MAX_PLAN_STEPS, NAGI_PLAN_VERSION,
};

pub struct ValidatedPlan {
    intent: alloc::string::String,
    steps: Vec<ValidatedStep>,
    caller: CallerIdentity,
}

#[derive(Clone)]
pub struct ValidatedStep {
    pub action: ActionDescriptor,
    pub object_ids: Vec<ObjectId>,
    pub parameters: alloc::collections::BTreeMap<alloc::string::String, serde_json::Value>,
}

impl ValidatedPlan {
    pub fn intent(&self) -> &str {
        &self.intent
    }

    pub fn step_count(&self) -> usize {
        self.steps.len()
    }

    pub(crate) fn into_steps(self) -> (CallerIdentity, alloc::string::String, Vec<ValidatedStep>) {
        (self.caller, self.intent, self.steps)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ValidationError {
    UnsupportedVersion,
    InvalidIntent,
    EmptyPlan,
    TooManySteps,
    UnsupportedAction,
    InvalidObjectCount,
    DuplicateObject,
    ObjectOutsideContext,
    ObjectDenied,
    CapabilityDenied,
    InvalidParameters(ParameterError),
}

pub fn validate_plan<P: ActionPolicy>(
    plan: NagiPlan,
    context: &ResolvedContext,
    registry: &ActionRegistry<P>,
    policy: &P,
) -> Result<ValidatedPlan, ValidationError> {
    if plan.plan_version != NAGI_PLAN_VERSION {
        return Err(ValidationError::UnsupportedVersion);
    }
    if plan.intent.trim().is_empty() || plan.intent.len() > MAX_INTENT_BYTES {
        return Err(ValidationError::InvalidIntent);
    }
    if plan.steps.is_empty() {
        return Err(ValidationError::EmptyPlan);
    }
    if plan.steps.len() > MAX_PLAN_STEPS {
        return Err(ValidationError::TooManySteps);
    }

    let caller = context.caller();
    let mut validated_steps = Vec::with_capacity(plan.steps.len());
    for step in plan.steps {
        let descriptor = registry
            .descriptor(&step.action)
            .ok_or(ValidationError::UnsupportedAction)?
            .clone();
        if step.object_ids.len() > MAX_OBJECTS_PER_STEP
            || step.object_ids.len() < descriptor.min_objects
            || step.object_ids.len() > descriptor.max_objects
        {
            return Err(ValidationError::InvalidObjectCount);
        }
        let mut object_ids = Vec::with_capacity(step.object_ids.len());
        for raw_id in step.object_ids {
            let object_id = ObjectId(raw_id);
            if object_ids.contains(&object_id) {
                return Err(ValidationError::DuplicateObject);
            }
            if !context.contains_object(object_id) {
                return Err(ValidationError::ObjectOutsideContext);
            }
            policy
                .check_object_access(caller, object_id, descriptor.object_access)
                .map_err(|error| match error {
                    PolicyDenied::Object => ValidationError::ObjectDenied,
                    PolicyDenied::Capability => ValidationError::CapabilityDenied,
                })?;
            object_ids.push(object_id);
        }
        for capability in &descriptor.required_capabilities {
            policy
                .check_capability(caller, capability)
                .map_err(|_| ValidationError::CapabilityDenied)?;
        }
        validate_parameters(&descriptor, &step.parameters)
            .map_err(ValidationError::InvalidParameters)?;
        validated_steps.push(ValidatedStep {
            action: descriptor,
            object_ids,
            parameters: step.parameters,
        });
    }

    Ok(ValidatedPlan {
        intent: plan.intent,
        steps: validated_steps,
        caller,
    })
}
