use alloc::{string::String, vec::Vec};
use nagi_model_manager::{
    CancellationToken, CapabilityId, GenerationOptions, GenerativeProvider, ModelRequest,
    RuntimeError,
};
use serde::Serialize;

use crate::{
    ActionDescriptor, ActionPolicy, ActionRegistry, NagiPlan, ParameterRule, ResolvedContext,
    MAX_INTENT_BYTES, MAX_PLAN_JSON_BYTES, MAX_PLAN_STEPS,
};

pub const MAX_PROMPT_ACTIONS: usize = 16;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PlanActionSchema {
    pub action_id: String,
    pub min_objects: usize,
    pub max_objects: usize,
    pub parameters: Vec<ParameterRule>,
}

impl From<&ActionDescriptor> for PlanActionSchema {
    fn from(descriptor: &ActionDescriptor) -> Self {
        Self {
            action_id: descriptor.action_id.clone(),
            min_objects: descriptor.min_objects,
            max_objects: descriptor.max_objects,
            parameters: descriptor.parameters.clone(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlanPrompt {
    request_id: u64,
    user_intent: String,
    context: ResolvedContext,
    action_schemas: Vec<PlanActionSchema>,
}

impl PlanPrompt {
    pub fn request_id(&self) -> u64 {
        self.request_id
    }

    pub fn user_intent(&self) -> &str {
        &self.user_intent
    }

    pub fn context(&self) -> &ResolvedContext {
        &self.context
    }

    pub fn action_schemas(&self) -> &[PlanActionSchema] {
        &self.action_schemas
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlanProviderError {
    Unavailable,
    InvalidResponse,
    InputTooLarge,
}

/// Provider output is an untrusted complete candidate. The interface returns
/// a whole document, never a stream that can be executed incrementally.
pub trait GenerativePlanProvider {
    fn generate_complete_plan(
        &mut self,
        prompt: &PlanPrompt,
        cancellation: &dyn CancellationToken,
    ) -> Result<String, PlanProviderError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PlannerError {
    InvalidIntent,
    NoAllowedActions,
    TooManyAllowedActions,
    UnsupportedAction,
    ProviderUnavailable,
    OutputTooLarge,
    InvalidPlan,
    InputTooLarge,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Planner;

impl Planner {
    /// Constructs a bounded prompt from the caller's already-filtered action
    /// candidates. It never passes the entire registry to a provider.
    pub fn prepare<P: ActionPolicy>(
        &self,
        request_id: u64,
        user_intent: String,
        context: ResolvedContext,
        filtered_action_ids: &[String],
        registry: &ActionRegistry<P>,
    ) -> Result<PlanPrompt, PlannerError> {
        if user_intent.trim().is_empty() || user_intent.len() > MAX_INTENT_BYTES {
            return Err(PlannerError::InvalidIntent);
        }
        if filtered_action_ids.is_empty() {
            return Err(PlannerError::NoAllowedActions);
        }
        if filtered_action_ids.len() > MAX_PROMPT_ACTIONS {
            return Err(PlannerError::TooManyAllowedActions);
        }
        let mut action_schemas = Vec::with_capacity(filtered_action_ids.len());
        for action_id in filtered_action_ids {
            if action_schemas
                .iter()
                .any(|existing: &PlanActionSchema| existing.action_id == *action_id)
            {
                return Err(PlannerError::UnsupportedAction);
            }
            let descriptor = registry
                .descriptor(action_id)
                .ok_or(PlannerError::UnsupportedAction)?;
            action_schemas.push(PlanActionSchema::from(descriptor));
        }
        Ok(PlanPrompt {
            request_id,
            user_intent,
            context,
            action_schemas,
        })
    }

    /// Accepts only a complete, bounded NagiPlan candidate. Callers must still
    /// run the deterministic Validator before Executor.
    pub fn generate(
        &self,
        provider: &mut impl GenerativePlanProvider,
        prompt: &PlanPrompt,
        cancellation: &dyn CancellationToken,
    ) -> Result<NagiPlan, PlannerError> {
        let response = provider
            .generate_complete_plan(prompt, cancellation)
            .map_err(|error| match error {
                PlanProviderError::Unavailable => PlannerError::ProviderUnavailable,
                PlanProviderError::InvalidResponse => PlannerError::InvalidPlan,
                PlanProviderError::InputTooLarge => PlannerError::InputTooLarge,
            })?;
        if response.len() > MAX_PLAN_JSON_BYTES {
            return Err(PlannerError::OutputTooLarge);
        }
        let plan = NagiPlan::parse_complete(&response).map_err(|error| match error {
            crate::PlanParseError::TooLarge => PlannerError::OutputTooLarge,
            crate::PlanParseError::InvalidJson => PlannerError::InvalidPlan,
        })?;
        if plan.steps.len() > MAX_PLAN_STEPS {
            return Err(PlannerError::InvalidPlan);
        }
        Ok(plan)
    }
}

struct PlanProviderInput<'a> {
    request_id: u64,
    user_intent: &'a str,
    app_id: u64,
    app_session_id: u64,
    node_id: u64,
    workspace_id: Option<u64>,
    visible_object_ids: Vec<u64>,
    allowed_actions: &'a [PlanActionSchema],
}

impl Serialize for PlanProviderInput<'_> {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("PlanProviderInput", 8)?;
        state.serialize_field("request_id", &self.request_id)?;
        state.serialize_field("user_intent", self.user_intent)?;
        state.serialize_field("app_id", &self.app_id)?;
        state.serialize_field("app_session_id", &self.app_session_id)?;
        state.serialize_field("node_id", &self.node_id)?;
        state.serialize_field("workspace_id", &self.workspace_id)?;
        state.serialize_field("visible_object_ids", &self.visible_object_ids)?;
        state.serialize_field("allowed_actions", self.allowed_actions)?;
        state.end()
    }
}

/// Adapter from the provider-neutral planning boundary to the existing
/// GenerativeProvider contract. A response is still only an untrusted plan
/// candidate and must pass Validator before Executor.
pub struct ModelManagerPlanAdapter<P> {
    provider: P,
    capability: CapabilityId,
    timeout_millis: u64,
}

impl<P> ModelManagerPlanAdapter<P> {
    pub fn new(provider: P, capability: CapabilityId, timeout_millis: u64) -> Self {
        Self {
            provider,
            capability,
            timeout_millis,
        }
    }

    pub fn provider_mut(&mut self) -> &mut P {
        &mut self.provider
    }
}

impl<P: GenerativeProvider> GenerativePlanProvider for ModelManagerPlanAdapter<P> {
    fn generate_complete_plan(
        &mut self,
        prompt: &PlanPrompt,
        cancellation: &dyn CancellationToken,
    ) -> Result<String, PlanProviderError> {
        let caller = prompt.context.caller();
        let input = PlanProviderInput {
            request_id: prompt.request_id,
            user_intent: &prompt.user_intent,
            app_id: caller.app_id.0,
            app_session_id: caller.app_session_id.0,
            node_id: caller.node_id.0,
            workspace_id: caller.workspace_id.map(|id| id.0),
            visible_object_ids: prompt
                .context
                .visible_objects()
                .iter()
                .map(|id| id.0)
                .collect(),
            allowed_actions: &prompt.action_schemas,
        };
        let input =
            serde_json::to_string(&input).map_err(|_| PlanProviderError::InvalidResponse)?;
        if input.len() > MAX_PLAN_JSON_BYTES {
            return Err(PlanProviderError::InputTooLarge);
        }
        let system_prompt = "Return one complete JSON NagiPlan@1 document. Use only supplied action IDs and visible Object IDs. Never include paths, shell commands, or authority claims.";
        let request = ModelRequest {
            request_id: prompt.request_id,
            caller: Some(caller.app_id),
            capability: &self.capability,
            system_prompt: Some(system_prompt),
            input: &input,
            input_tokens: None,
            max_output_tokens: 4096,
            options: GenerationOptions {
                temperature_milli: Some(0),
                top_p_milli: None,
                seed: Some(prompt.request_id),
            },
            timeout_millis: Some(self.timeout_millis),
        };
        let response =
            self.provider
                .generate(&request, cancellation)
                .map_err(|error| match error {
                    RuntimeError::InvalidBackendResponse => PlanProviderError::InvalidResponse,
                    _ => PlanProviderError::Unavailable,
                })?;
        if response.request_id != prompt.request_id {
            return Err(PlanProviderError::InvalidResponse);
        }
        Ok(response.text)
    }
}
