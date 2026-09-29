use alloc::vec::Vec;

use crate::{
    resources_fit, selection_role_priority, AvailabilityState, ExecutionScope, ModelId,
    ModelRegistry, ProviderId, RoleId, SelectionRequest,
};

/// The kind of provider path requested by an AI operation.
///
/// The capability and role remain the source of eligibility. This enum labels
/// the dispatch path and defines whether a decision request may use the
/// generative adapter as a fallback.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RouteKind {
    Generative,
    Decision,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderHealth {
    Available,
    Degraded,
    Unavailable,
    Disabled,
}

impl ProviderHealth {
    fn is_usable(self) -> bool {
        matches!(self, Self::Available | Self::Degraded)
    }

    fn preference(self) -> u8 {
        match self {
            Self::Available => 0,
            Self::Degraded => 1,
            Self::Unavailable => 2,
            Self::Disabled => 3,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProviderState {
    provider_id: ProviderId,
    health: ProviderHealth,
}

/// Runtime availability is separate from a model's artifact/backend
/// availability. A provider with no reported state is treated as unknown and
/// cannot be selected.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProviderAvailabilityRegistry {
    providers: Vec<ProviderState>,
}

impl ProviderAvailabilityRegistry {
    pub const fn new() -> Self {
        Self {
            providers: Vec::new(),
        }
    }

    pub fn set(&mut self, provider_id: ProviderId, health: ProviderHealth) {
        if let Some(existing) = self
            .providers
            .iter_mut()
            .find(|provider| provider.provider_id == provider_id)
        {
            existing.health = health;
        } else {
            self.providers.push(ProviderState {
                provider_id,
                health,
            });
        }
    }

    pub fn remove(&mut self, provider_id: &ProviderId) -> bool {
        let Some(index) = self
            .providers
            .iter()
            .position(|provider| &provider.provider_id == provider_id)
        else {
            return false;
        };
        self.providers.remove(index);
        true
    }

    pub fn health(&self, provider_id: &ProviderId) -> Option<ProviderHealth> {
        self.providers
            .iter()
            .find(|provider| &provider.provider_id == provider_id)
            .map(|provider| provider.health)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectedRoutePath {
    Generative,
    Decision,
    GenerativeDecisionAdapter,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SafeRouteFallback {
    DeterministicOrManual,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RouteCandidate {
    pub model_id: ModelId,
    pub provider_id: ProviderId,
    pub health: ProviderHealth,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelRoutePlan {
    pub requested_route: RouteKind,
    pub selected_path: SelectedRoutePath,
    pub selected: RouteCandidate,
    /// Compatible alternatives in the same selected path, in deterministic
    /// preference order. Decision routes do not mix decision and generative
    /// alternatives in this list.
    pub fallbacks: Vec<RouteCandidate>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ModelRouteResolution {
    Selected(ModelRoutePlan),
    Fallback(SafeRouteFallback),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManualOverrideFailure {
    UnknownModel,
    ModelNotSelectable,
    ProviderStateUnknown,
    ProviderUnavailable,
    CapabilityMismatch,
    RoleMismatch,
    ContextTooSmall,
    ResourceBudget,
    ArchitectureMismatch,
    OfflineOnly,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RoutingError {
    ManualOverrideUnavailable(ManualOverrideFailure),
}

impl core::fmt::Display for RoutingError {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::ManualOverrideUnavailable(reason) => {
                formatter.write_str("manual model override is unavailable for this request: ")?;
                let description = match reason {
                    ManualOverrideFailure::UnknownModel => "model is not registered",
                    ManualOverrideFailure::ModelNotSelectable => {
                        "model artifact, backend, or lifecycle is unavailable"
                    }
                    ManualOverrideFailure::ProviderStateUnknown => {
                        "provider availability has not been reported"
                    }
                    ManualOverrideFailure::ProviderUnavailable => {
                        "provider is unavailable or disabled"
                    }
                    ManualOverrideFailure::CapabilityMismatch => {
                        "model does not provide the requested capability"
                    }
                    ManualOverrideFailure::RoleMismatch => {
                        "model does not support the requested role"
                    }
                    ManualOverrideFailure::ContextTooSmall => {
                        "model context is smaller than the request"
                    }
                    ManualOverrideFailure::ResourceBudget => {
                        "model exceeds available system resources"
                    }
                    ManualOverrideFailure::ArchitectureMismatch => {
                        "model is incompatible with the target architecture"
                    }
                    ManualOverrideFailure::OfflineOnly => {
                        "remote model is disallowed by offline policy"
                    }
                };
                formatter.write_str(description)
            }
        }
    }
}

/// A request to route one capability operation. Decision requests can provide
/// a separate generative selection request; that path is used only when no
/// available specialized decision provider matches.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelRouteRequest<'a> {
    pub route: RouteKind,
    pub selection: SelectionRequest<'a>,
    pub provider_availability: &'a ProviderAvailabilityRegistry,
    pub manual_override: Option<&'a ModelId>,
    pub generative_fallback: Option<SelectionRequest<'a>>,
}

impl ModelRegistry {
    /// Selects a provider from registered, compatible models using runtime
    /// availability as an additional fail-closed constraint.
    ///
    /// In automatic mode, an unavailable route produces an explicit
    /// deterministic/manual fallback. A manual override is never silently
    /// replaced: it must match the requested route or the configured
    /// generative decision adapter path.
    pub fn route(
        &self,
        request: &ModelRouteRequest<'_>,
    ) -> Result<ModelRouteResolution, RoutingError> {
        if let Some(override_id) = request.manual_override {
            match self.manual_candidate(
                override_id,
                &request.selection,
                request.provider_availability,
            ) {
                Ok(candidate) => {
                    return Ok(ModelRouteResolution::Selected(ModelRoutePlan {
                        requested_route: request.route,
                        selected_path: selected_path(request.route),
                        selected: candidate,
                        fallbacks: Vec::new(),
                    }));
                }
                Err(primary_error) => {
                    if request.route == RouteKind::Decision {
                        if let Some(generative) = &request.generative_fallback {
                            match self.manual_candidate(
                                override_id,
                                generative,
                                request.provider_availability,
                            ) {
                                Ok(candidate) => {
                                    return Ok(ModelRouteResolution::Selected(ModelRoutePlan {
                                        requested_route: request.route,
                                        selected_path: SelectedRoutePath::GenerativeDecisionAdapter,
                                        selected: candidate,
                                        fallbacks: Vec::new(),
                                    }));
                                }
                                Err(fallback_error) => {
                                    return Err(RoutingError::ManualOverrideUnavailable(
                                        preferred_manual_failure(primary_error, fallback_error),
                                    ));
                                }
                            }
                        }
                    }
                    return Err(RoutingError::ManualOverrideUnavailable(primary_error));
                }
            }
        }

        let primary = self.candidates(&request.selection, request.provider_availability);
        if !primary.is_empty() {
            return Ok(ModelRouteResolution::Selected(route_plan(
                request.route,
                selected_path(request.route),
                primary,
            )));
        }
        if request.route == RouteKind::Decision {
            if let Some(generative) = &request.generative_fallback {
                let candidates = self.candidates(generative, request.provider_availability);
                if !candidates.is_empty() {
                    return Ok(ModelRouteResolution::Selected(route_plan(
                        request.route,
                        SelectedRoutePath::GenerativeDecisionAdapter,
                        candidates,
                    )));
                }
            }
        }
        Ok(ModelRouteResolution::Fallback(
            SafeRouteFallback::DeterministicOrManual,
        ))
    }

    fn candidates(
        &self,
        request: &SelectionRequest<'_>,
        providers: &ProviderAvailabilityRegistry,
    ) -> Vec<RouteCandidate> {
        let mut candidates: Vec<RouteCandidate> = self
            .entries()
            .iter()
            .filter_map(|entry| {
                if !selection_matches(entry, request) {
                    return None;
                }
                let health = providers.health(&entry.manifest.provider.provider_id)?;
                if !health.is_usable() {
                    return None;
                }
                Some(RouteCandidate {
                    model_id: entry.manifest.model_id.clone(),
                    provider_id: entry.manifest.provider.provider_id.clone(),
                    health,
                })
            })
            .collect();
        candidates.sort_by(|left, right| {
            let left_entry = self
                .get(&left.model_id)
                .expect("route candidate is backed by a registry entry");
            let right_entry = self
                .get(&right.model_id)
                .expect("route candidate is backed by a registry entry");
            left.health
                .preference()
                .cmp(&right.health.preference())
                .then_with(|| {
                    preference_rank(&left.model_id, request)
                        .cmp(&preference_rank(&right.model_id, request))
                })
                .then_with(|| {
                    selection_role_priority(left_entry, request.role.is_none()).cmp(
                        &selection_role_priority(right_entry, request.role.is_none()),
                    )
                })
                .then_with(|| left.provider_id.cmp(&right.provider_id))
                .then_with(|| left.model_id.cmp(&right.model_id))
        });
        candidates
    }

    fn manual_candidate(
        &self,
        model_id: &ModelId,
        request: &SelectionRequest<'_>,
        providers: &ProviderAvailabilityRegistry,
    ) -> Result<RouteCandidate, ManualOverrideFailure> {
        let entry = self
            .get(model_id)
            .ok_or(ManualOverrideFailure::UnknownModel)?;
        if !entry.is_selectable() || entry.availability != AvailabilityState::Available {
            return Err(ManualOverrideFailure::ModelNotSelectable);
        }
        let provider_id = &entry.manifest.provider.provider_id;
        let health = providers
            .health(provider_id)
            .ok_or(ManualOverrideFailure::ProviderStateUnknown)?;
        if !health.is_usable() {
            return Err(ManualOverrideFailure::ProviderUnavailable);
        }
        if !selection_matches(entry, request) {
            return Err(selection_failure(entry, request));
        }
        Ok(RouteCandidate {
            model_id: entry.manifest.model_id.clone(),
            provider_id: provider_id.clone(),
            health,
        })
    }
}

fn route_plan(
    requested_route: RouteKind,
    selected_path: SelectedRoutePath,
    mut candidates: Vec<RouteCandidate>,
) -> ModelRoutePlan {
    let selected = candidates.remove(0);
    ModelRoutePlan {
        requested_route,
        selected_path,
        selected,
        fallbacks: candidates,
    }
}

fn selected_path(route: RouteKind) -> SelectedRoutePath {
    match route {
        RouteKind::Generative => SelectedRoutePath::Generative,
        RouteKind::Decision => SelectedRoutePath::Decision,
    }
}

fn selection_matches(entry: &crate::ModelEntry, request: &SelectionRequest<'_>) -> bool {
    entry.is_selectable()
        && entry.supported_capabilities.contains(request.capability)
        && request
            .role
            .is_none_or(|role| entry.manifest.has_role(role))
        && entry.manifest.context.max_input_tokens >= request.minimum_context_tokens
        && resources_fit(&entry.manifest, request.resources)
        && (request.target_architecture.is_empty()
            || entry.manifest.compatibility.architectures.is_empty()
            || entry
                .manifest
                .compatibility
                .architectures
                .iter()
                .any(|arch| arch == request.target_architecture))
        && (!request.offline_only || entry.manifest.execution == ExecutionScope::Local)
}

fn selection_failure(
    entry: &crate::ModelEntry,
    request: &SelectionRequest<'_>,
) -> ManualOverrideFailure {
    if !entry.supported_capabilities.contains(request.capability) {
        ManualOverrideFailure::CapabilityMismatch
    } else if request
        .role
        .is_some_and(|role: &RoleId| !entry.manifest.has_role(role))
    {
        ManualOverrideFailure::RoleMismatch
    } else if entry.manifest.context.max_input_tokens < request.minimum_context_tokens {
        ManualOverrideFailure::ContextTooSmall
    } else if !resources_fit(&entry.manifest, request.resources) {
        ManualOverrideFailure::ResourceBudget
    } else if !request.target_architecture.is_empty()
        && !entry.manifest.compatibility.architectures.is_empty()
        && !entry
            .manifest
            .compatibility
            .architectures
            .iter()
            .any(|arch| arch == request.target_architecture)
    {
        ManualOverrideFailure::ArchitectureMismatch
    } else if request.offline_only && entry.manifest.execution != ExecutionScope::Local {
        ManualOverrideFailure::OfflineOnly
    } else {
        ManualOverrideFailure::ModelNotSelectable
    }
}

fn preferred_manual_failure(
    primary: ManualOverrideFailure,
    generative: ManualOverrideFailure,
) -> ManualOverrideFailure {
    match primary {
        ManualOverrideFailure::CapabilityMismatch | ManualOverrideFailure::RoleMismatch => {
            generative
        }
        _ => primary,
    }
}

fn preference_rank(model_id: &ModelId, request: &SelectionRequest<'_>) -> u8 {
    if request.preferred_model_id == Some(model_id) {
        0
    } else if request.role_default_model_id == Some(model_id) {
        1
    } else {
        2
    }
}

#[cfg(test)]
mod tests {
    use alloc::{string::ToString, vec};

    use super::*;
    use crate::{
        ArtifactCatalog, ArtifactDescriptor, ArtifactStatus, BackendDescriptor, BackendId,
        CapabilityId, FormatId, IntegrityMetadata, ModelManifest, ResourceBudget, RoleDefault,
        RoleDefaultPolicy, RuntimeClassId,
    };

    struct PresentArtifact;

    impl ArtifactCatalog for PresentArtifact {
        fn inspect(&self, _: &ArtifactDescriptor) -> ArtifactStatus {
            ArtifactStatus::Present {
                integrity_verified: true,
            }
        }
    }

    fn model_registry_with_defaults() -> ModelRegistry {
        let mut registry = ModelRegistry::new();
        for fixture in [
            include_str!("../tests/fixtures/granite-4.2-3b.json"),
            include_str!("../tests/fixtures/qwen3-4b.json"),
            include_str!("../tests/fixtures/gemma-3-1b.json"),
        ] {
            let mut manifest = ModelManifest::parse_json(fixture.as_bytes()).unwrap();
            // These synthetic values exercise selection only; they do not
            // represent or install model artifacts.
            manifest.artifact.size_bytes = Some(1);
            manifest.artifact.integrity = Some(IntegrityMetadata {
                algorithm: "sha256".to_string(),
                digest: "a".repeat(64),
            });
            registry
                .discover(manifest, &PresentArtifact, &[backend()], budget(), "x86_64")
                .unwrap();
        }
        registry
    }

    fn backend() -> BackendDescriptor {
        BackendDescriptor {
            backend_id: BackendId::new("llama_cpp").unwrap(),
            artifact_formats: vec![FormatId::new("gguf").unwrap()],
            runtime_api_versions: vec![crate::MODEL_RUNTIME_API_VERSION.to_string()],
            architectures: vec!["x86_64".to_string()],
            capabilities: [
                "text.generate",
                "text.stream",
                "structured.generate",
                "decision.boolean",
            ]
            .into_iter()
            .map(|value| CapabilityId::new(value).unwrap())
            .collect(),
            runtime_classes: vec![
                RuntimeClassId::new("generative_llm").unwrap(),
                RuntimeClassId::new("system_one").unwrap(),
            ],
        }
    }

    fn budget() -> ResourceBudget {
        ResourceBudget {
            available_ram_bytes: 8 * 1024 * 1024 * 1024,
            available_storage_bytes: 32 * 1024 * 1024 * 1024,
            cpu_cores: 4,
            gpu_available: false,
        }
    }

    fn selection<'a>(
        capability: &'a CapabilityId,
        role: Option<&'a RoleId>,
    ) -> SelectionRequest<'a> {
        SelectionRequest {
            capability,
            role,
            minimum_context_tokens: 1024,
            resources: budget(),
            target_architecture: "x86_64",
            offline_only: true,
            preferred_model_id: None,
            role_default_model_id: None,
        }
    }

    fn model_id(value: &str) -> ModelId {
        ModelId::new(value).unwrap()
    }

    fn available_providers() -> ProviderAvailabilityRegistry {
        let mut availability = ProviderAvailabilityRegistry::new();
        for provider in ["ibm", "alibaba", "google"] {
            availability.set(
                crate::ProviderId::new(provider).unwrap(),
                ProviderHealth::Available,
            );
        }
        availability
    }

    #[test]
    fn keeps_configured_standard_default_and_falls_back_when_its_provider_is_unavailable() {
        let registry = model_registry_with_defaults();
        let standard = RoleId::new("standard").unwrap();
        let default_policy = RoleDefaultPolicy::new(vec![RoleDefault {
            role: standard.clone(),
            model_id: model_id("ibm.granite-4.2-3b"),
        }])
        .unwrap();
        let capability = CapabilityId::new("text.generate").unwrap();
        let mut request = selection(&capability, Some(&standard));
        request = request.use_role_policy(&default_policy);
        let mut providers = available_providers();
        let route = ModelRouteRequest {
            route: RouteKind::Generative,
            selection: request.clone(),
            provider_availability: &providers,
            manual_override: None,
            generative_fallback: None,
        };

        let ModelRouteResolution::Selected(plan) = registry.route(&route).unwrap() else {
            panic!("available Standard route should select a provider");
        };
        assert_eq!(plan.selected.model_id.as_str(), "ibm.granite-4.2-3b");
        assert_eq!(plan.selected_path, SelectedRoutePath::Generative);

        providers.set(
            crate::ProviderId::new("ibm").unwrap(),
            ProviderHealth::Unavailable,
        );
        let route = ModelRouteRequest {
            route: RouteKind::Generative,
            selection: request,
            provider_availability: &providers,
            manual_override: None,
            generative_fallback: None,
        };
        let ModelRouteResolution::Selected(plan) = registry.route(&route).unwrap() else {
            panic!("an available compatible alternative should be selected");
        };
        assert_eq!(plan.selected.model_id.as_str(), "qwen.qwen3-4b");
        assert_eq!(plan.selected.provider_id.as_str(), "alibaba");
    }

    #[test]
    fn manual_override_is_validated_and_never_silently_replaced() {
        let registry = model_registry_with_defaults();
        let standard = RoleId::new("standard").unwrap();
        let capability = CapabilityId::new("text.generate").unwrap();
        let selection = selection(&capability, Some(&standard));
        let mut providers = available_providers();
        let override_id = model_id("qwen.qwen3-4b");
        let route = ModelRouteRequest {
            route: RouteKind::Generative,
            selection: selection.clone(),
            provider_availability: &providers,
            manual_override: Some(&override_id),
            generative_fallback: None,
        };
        let ModelRouteResolution::Selected(plan) = registry.route(&route).unwrap() else {
            panic!("valid manual override should be selected");
        };
        assert_eq!(plan.selected.model_id, override_id);
        assert!(plan.fallbacks.is_empty());

        providers.set(
            crate::ProviderId::new("alibaba").unwrap(),
            ProviderHealth::Disabled,
        );
        let route = ModelRouteRequest {
            route: RouteKind::Generative,
            selection: selection.clone(),
            provider_availability: &providers,
            manual_override: Some(&override_id),
            generative_fallback: None,
        };
        assert_eq!(
            registry.route(&route),
            Err(RoutingError::ManualOverrideUnavailable(
                ManualOverrideFailure::ProviderUnavailable
            ))
        );

        let bad_override = model_id("google.gemma-3-1b");
        let route = ModelRouteRequest {
            route: RouteKind::Generative,
            selection,
            provider_availability: &providers,
            manual_override: Some(&bad_override),
            generative_fallback: None,
        };
        assert_eq!(
            registry.route(&route),
            Err(RoutingError::ManualOverrideUnavailable(
                ManualOverrideFailure::RoleMismatch
            ))
        );
    }

    #[test]
    fn decision_routes_use_specialized_providers_then_bounded_generative_adapter_or_safe_fallback()
    {
        let mut registry = model_registry_with_defaults();
        let mut decision_model = ModelManifest::parse_json(
            include_str!("../tests/fixtures/granite-4.2-3b.json").as_bytes(),
        )
        .unwrap();
        decision_model.model_id = model_id("future.decision-model");
        decision_model.display_name = "Future decision provider".to_string();
        decision_model.provider.provider_id = crate::ProviderId::new("future_provider").unwrap();
        decision_model.provider.display_name = "Future Provider".to_string();
        decision_model.provider.family = "decision-family".to_string();
        decision_model.runtime_class = Some(RuntimeClassId::new("system_one").unwrap());
        decision_model.capabilities = vec![CapabilityId::new("decision.boolean").unwrap()];
        decision_model.roles = vec![RoleId::new("decision").unwrap()];
        decision_model.artifact.size_bytes = Some(1);
        decision_model.artifact.integrity = Some(IntegrityMetadata {
            algorithm: "sha256".to_string(),
            digest: "b".repeat(64),
        });
        registry
            .discover(
                decision_model,
                &PresentArtifact,
                &[backend()],
                budget(),
                "x86_64",
            )
            .unwrap();

        let decision_role = RoleId::new("decision").unwrap();
        let standard_role = RoleId::new("standard").unwrap();
        let decision_capability = CapabilityId::new("decision.boolean").unwrap();
        let generative_capability = CapabilityId::new("text.generate").unwrap();
        let decision_selection = selection(&decision_capability, Some(&decision_role));
        let default_policy = RoleDefaultPolicy::new(vec![RoleDefault {
            role: standard_role.clone(),
            model_id: model_id("ibm.granite-4.2-3b"),
        }])
        .unwrap();
        let generative_selection = selection(&generative_capability, Some(&standard_role))
            .use_role_policy(&default_policy);
        let mut providers = available_providers();
        providers.set(
            crate::ProviderId::new("future_provider").unwrap(),
            ProviderHealth::Available,
        );
        let route = ModelRouteRequest {
            route: RouteKind::Decision,
            selection: decision_selection.clone(),
            provider_availability: &providers,
            manual_override: None,
            generative_fallback: Some(generative_selection.clone()),
        };
        let ModelRouteResolution::Selected(plan) = registry.route(&route).unwrap() else {
            panic!("available specialized decision route should be selected");
        };
        assert_eq!(plan.selected.model_id.as_str(), "future.decision-model");
        assert_eq!(plan.selected_path, SelectedRoutePath::Decision);

        providers.set(
            crate::ProviderId::new("future_provider").unwrap(),
            ProviderHealth::Unavailable,
        );
        let route = ModelRouteRequest {
            route: RouteKind::Decision,
            selection: decision_selection.clone(),
            provider_availability: &providers,
            manual_override: None,
            generative_fallback: Some(generative_selection.clone()),
        };
        let ModelRouteResolution::Selected(plan) = registry.route(&route).unwrap() else {
            panic!("compatible generative adapter should be selected as fallback");
        };
        assert_eq!(
            plan.selected_path,
            SelectedRoutePath::GenerativeDecisionAdapter
        );
        assert_eq!(plan.selected.model_id.as_str(), "ibm.granite-4.2-3b");

        for provider in ["ibm", "alibaba", "google"] {
            providers.set(
                crate::ProviderId::new(provider).unwrap(),
                ProviderHealth::Unavailable,
            );
        }
        let route = ModelRouteRequest {
            route: RouteKind::Decision,
            selection: decision_selection,
            provider_availability: &providers,
            manual_override: None,
            generative_fallback: Some(generative_selection),
        };
        assert_eq!(
            registry.route(&route).unwrap(),
            ModelRouteResolution::Fallback(SafeRouteFallback::DeterministicOrManual)
        );
    }

    #[test]
    fn an_unreported_provider_is_not_treated_as_available() {
        let registry = model_registry_with_defaults();
        let standard = RoleId::new("standard").unwrap();
        let capability = CapabilityId::new("text.generate").unwrap();
        let route = ModelRouteRequest {
            route: RouteKind::Generative,
            selection: selection(&capability, Some(&standard)),
            provider_availability: &ProviderAvailabilityRegistry::new(),
            manual_override: None,
            generative_fallback: None,
        };
        assert_eq!(
            registry.route(&route).unwrap(),
            ModelRouteResolution::Fallback(SafeRouteFallback::DeterministicOrManual)
        );
    }
}
