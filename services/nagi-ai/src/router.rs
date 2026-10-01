use alloc::{string::String, vec::Vec};
use serde::Deserialize;

use nagi_model::AppId;
use nagi_model_manager::{
    CancellationToken, CapabilityId, GenerationOptions, GenerativeProvider, ModelRequest,
    RuntimeError,
};

pub const MAX_DECISION_CANDIDATES: usize = 32;
pub const MAX_DECISION_INTENT_BYTES: usize = 2048;
pub const MAX_DECISION_OUTPUT_BYTES: usize = 2048;

pub struct DecisionRequest<'a> {
    pub request_id: u64,
    pub caller: AppId,
    pub capability: &'a CapabilityId,
    pub intent: &'a str,
    pub candidate_action_ids: &'a [String],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DecisionProviderError {
    InvalidRequest,
    Unavailable,
    InvalidOutput,
    UnsupportedCandidate,
}

pub trait DecisionProvider {
    fn decide(
        &mut self,
        request: &DecisionRequest<'_>,
        cancellation: &dyn CancellationToken,
    ) -> Result<DecisionCandidate, DecisionProviderError>;
}

pub struct LlmDecisionAdapter<P> {
    provider: P,
    timeout_millis: u64,
}

impl<P> LlmDecisionAdapter<P> {
    pub fn new(provider: P, timeout_millis: u64) -> Self {
        Self {
            provider,
            timeout_millis,
        }
    }

    pub fn provider_mut(&mut self) -> &mut P {
        &mut self.provider
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DecisionOutput {
    action_id: String,
    confidence_percent: u8,
}

impl<P: GenerativeProvider> DecisionProvider for LlmDecisionAdapter<P> {
    fn decide(
        &mut self,
        request: &DecisionRequest<'_>,
        cancellation: &dyn CancellationToken,
    ) -> Result<DecisionCandidate, DecisionProviderError> {
        if request.intent.trim().is_empty()
            || request.intent.len() > MAX_DECISION_INTENT_BYTES
            || request.candidate_action_ids.is_empty()
            || request.candidate_action_ids.len() > MAX_DECISION_CANDIDATES
        {
            return Err(DecisionProviderError::InvalidRequest);
        }
        for (index, action_id) in request.candidate_action_ids.iter().enumerate() {
            if !crate::valid_action_id(action_id)
                || request.candidate_action_ids[..index].contains(action_id)
            {
                return Err(DecisionProviderError::InvalidRequest);
            }
        }
        let input = serde_json::to_string(&serde_json::json!({
            "intent": request.intent,
            "allowed_action_ids": request.candidate_action_ids,
        }))
        .map_err(|_| DecisionProviderError::InvalidRequest)?;
        let system_prompt = "Choose exactly one allowed action ID. Return one JSON object with action_id and confidence_percent (0 through 100). This choice is advisory and grants no authority.";
        let model_request = ModelRequest {
            request_id: request.request_id,
            caller: Some(request.caller),
            capability: request.capability,
            system_prompt: Some(system_prompt),
            input: &input,
            input_tokens: None,
            max_output_tokens: 64,
            options: GenerationOptions {
                temperature_milli: Some(0),
                top_p_milli: None,
                seed: Some(request.request_id),
            },
            timeout_millis: Some(self.timeout_millis),
            structured_output: None,
        };
        let response = self
            .provider
            .generate(&model_request, cancellation)
            .map_err(|error| match error {
                RuntimeError::InvalidBackendResponse => DecisionProviderError::InvalidOutput,
                _ => DecisionProviderError::Unavailable,
            })?;
        if response.request_id != request.request_id
            || response.text.len() > MAX_DECISION_OUTPUT_BYTES
        {
            return Err(DecisionProviderError::InvalidOutput);
        }
        let parsed: DecisionOutput = serde_json::from_str(&response.text)
            .map_err(|_| DecisionProviderError::InvalidOutput)?;
        if parsed.confidence_percent > 100 {
            return Err(DecisionProviderError::InvalidOutput);
        }
        if !request
            .candidate_action_ids
            .iter()
            .any(|candidate| candidate == &parsed.action_id)
        {
            return Err(DecisionProviderError::UnsupportedCandidate);
        }
        Ok(DecisionCandidate {
            action_id: parsed.action_id,
            confidence_percent: parsed.confidence_percent,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecisionCandidate {
    pub action_id: String,
    /// A routing hint only. This never grants authority or validates a plan.
    pub confidence_percent: u8,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FallbackRoute {
    GenerativePlanner,
    DeterministicOrManual,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DecisionRoute {
    Candidate(String),
    Fallback(FallbackRoute),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RouterError {
    TooManyCandidates,
    InvalidConfidence,
}

pub fn route_decision_candidate(
    allowed_action_ids: &[String],
    candidate: Option<DecisionCandidate>,
    minimum_confidence_percent: u8,
    generative_available: bool,
) -> Result<DecisionRoute, RouterError> {
    if allowed_action_ids.len() > MAX_DECISION_CANDIDATES {
        return Err(RouterError::TooManyCandidates);
    }
    if minimum_confidence_percent > 100 {
        return Err(RouterError::InvalidConfidence);
    }
    let fallback = if generative_available {
        FallbackRoute::GenerativePlanner
    } else {
        FallbackRoute::DeterministicOrManual
    };
    let Some(candidate) = candidate else {
        return Ok(DecisionRoute::Fallback(fallback));
    };
    if candidate.confidence_percent > 100 {
        return Err(RouterError::InvalidConfidence);
    }
    if candidate.confidence_percent < minimum_confidence_percent
        || !allowed_action_ids
            .iter()
            .any(|action_id| action_id == &candidate.action_id)
    {
        return Ok(DecisionRoute::Fallback(fallback));
    }
    Ok(DecisionRoute::Candidate(candidate.action_id))
}

/// Uses a DecisionProvider only to choose from a prefiltered action set. Any
/// provider failure, malformed confidence, or unsupported candidate follows
/// the explicit generative/manual fallback. Validator and Policy still gate
/// every plan.
pub fn route_with_decision_provider(
    provider: Option<&mut dyn DecisionProvider>,
    request: &DecisionRequest<'_>,
    minimum_confidence_percent: u8,
    generative_available: bool,
    cancellation: &dyn CancellationToken,
) -> Result<DecisionRoute, RouterError> {
    if request.candidate_action_ids.len() > MAX_DECISION_CANDIDATES {
        return Err(RouterError::TooManyCandidates);
    }
    let candidate = provider.and_then(|provider| provider.decide(request, cancellation).ok());
    let candidate = candidate.filter(|candidate| candidate.confidence_percent <= 100);
    route_decision_candidate(
        request.candidate_action_ids,
        candidate,
        minimum_confidence_percent,
        generative_available,
    )
}

pub fn bounded_candidates(action_ids: &[String]) -> Vec<String> {
    action_ids
        .iter()
        .take(MAX_DECISION_CANDIDATES)
        .cloned()
        .collect()
}
