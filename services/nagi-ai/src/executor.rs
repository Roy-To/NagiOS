use alloc::{string::String, vec::Vec};

use nagi_model::ObjectId;

use crate::{
    ActionInvocation, ActionPolicy, ActionRegistry, HandlerError, ObjectAccess, PolicyDenied,
    ValidatedPlan, ValidatedStep, MAX_OUTPUT_OBJECTS, MAX_OUTPUT_SUMMARY_BYTES,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionStatus {
    Succeeded,
    Failed,
    Partial,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompletedStep {
    pub action_id: String,
    pub summary: String,
    pub object_ids: Vec<ObjectId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExecutionError {
    UnsupportedAction,
    CapabilityDenied,
    ObjectDenied,
    Handler(HandlerError),
    InvalidOutput,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExecutionReport {
    pub status: ExecutionStatus,
    pub completed: Vec<CompletedStep>,
    pub failed_step: Option<usize>,
    pub error: Option<ExecutionError>,
}

pub fn execute_plan<P: ActionPolicy>(
    plan: ValidatedPlan,
    registry: &mut ActionRegistry<P>,
    policy: &P,
) -> ExecutionReport {
    let (caller, steps) = plan.into_steps();
    let mut completed = Vec::with_capacity(steps.len());
    for (index, step) in steps.iter().enumerate() {
        match execute_step(step, caller, registry, policy) {
            Ok(result) => completed.push(result),
            Err(error) => {
                return ExecutionReport {
                    status: if completed.is_empty() {
                        ExecutionStatus::Failed
                    } else {
                        ExecutionStatus::Partial
                    },
                    completed,
                    failed_step: Some(index),
                    error: Some(error),
                };
            }
        }
    }
    ExecutionReport {
        status: ExecutionStatus::Succeeded,
        completed,
        failed_step: None,
        error: None,
    }
}

fn execute_step<P: ActionPolicy>(
    step: &ValidatedStep,
    caller: crate::CallerIdentity,
    registry: &mut ActionRegistry<P>,
    policy: &P,
) -> Result<CompletedStep, ExecutionError> {
    let mut capability_grants = Vec::with_capacity(step.action.required_capabilities.len());
    for capability in &step.action.required_capabilities {
        capability_grants.push(
            policy
                .acquire_capability(caller, capability)
                .map_err(map_policy_error)?,
        );
    }
    let mut object_handles = Vec::with_capacity(step.object_ids.len());
    for object_id in &step.object_ids {
        object_handles.push(
            policy
                .resolve_object(caller, *object_id, step.action.object_access)
                .map_err(map_policy_error)?,
        );
    }

    let registered = registry
        .registered_mut(&step.action.action_id)
        .ok_or(ExecutionError::UnsupportedAction)?;
    let output = registered
        .handler
        .execute(ActionInvocation {
            caller,
            action: &registered.descriptor,
            object_ids: &step.object_ids,
            parameters: &step.parameters,
            capability_grants: &capability_grants,
            object_handles: &object_handles,
        })
        .map_err(ExecutionError::Handler)?;
    if output.summary.len() > MAX_OUTPUT_SUMMARY_BYTES
        || output.object_ids.len() > MAX_OUTPUT_OBJECTS
    {
        return Err(ExecutionError::InvalidOutput);
    }
    for object_id in &output.object_ids {
        policy
            .check_object_access(caller, *object_id, ObjectAccess::Read)
            .map_err(map_policy_error)?;
    }
    Ok(CompletedStep {
        action_id: step.action.action_id.clone(),
        summary: output.summary,
        object_ids: output.object_ids,
    })
}

fn map_policy_error(error: PolicyDenied) -> ExecutionError {
    match error {
        PolicyDenied::Capability => ExecutionError::CapabilityDenied,
        PolicyDenied::Object => ExecutionError::ObjectDenied,
    }
}
