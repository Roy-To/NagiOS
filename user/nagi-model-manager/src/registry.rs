#[cfg(test)]
use alloc::vec;
use alloc::{string::String, vec::Vec};
use core::fmt;

use crate::{
    ArtifactDescriptor, BackendId, CapabilityId, ExecutionScope, FormatId, ManifestError, ModelId,
    ModelManifest, RoleId,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BackendDescriptor {
    pub backend_id: BackendId,
    pub artifact_formats: Vec<FormatId>,
    pub runtime_api_versions: Vec<String>,
    pub architectures: Vec<String>,
}

impl BackendDescriptor {
    fn supports_runtime(&self, manifest: &ModelManifest) -> bool {
        self.runtime_api_versions
            .iter()
            .any(|version| version == &manifest.runtime_api_version)
    }

    fn supports_manifest(&self, manifest: &ModelManifest, target_architecture: &str) -> bool {
        manifest
            .supported_backends
            .iter()
            .any(|backend| backend == &self.backend_id)
            && self
                .artifact_formats
                .iter()
                .any(|format| format == &manifest.artifact.format)
            && (self.architectures.is_empty()
                || self
                    .architectures
                    .iter()
                    .any(|architecture| architecture == target_architecture))
            && (manifest.compatibility.architectures.is_empty()
                || manifest
                    .compatibility
                    .architectures
                    .iter()
                    .any(|architecture| architecture == target_architecture))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ResourceBudget {
    pub available_ram_bytes: u64,
    pub available_storage_bytes: u64,
    pub cpu_cores: u8,
    pub gpu_available: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactStatus {
    Missing,
    Present { integrity_verified: bool },
}

pub trait ArtifactCatalog {
    fn inspect(&self, artifact: &ArtifactDescriptor) -> ArtifactStatus;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AvailabilityState {
    Available,
    MissingArtifact,
    MissingIntegrity,
    IntegrityMismatch,
    IncompatibleRuntime,
    IncompatibleBackend,
    IncompatibleResources,
    RemoteProviderUnavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelEntry {
    pub manifest: ModelManifest,
    pub availability: AvailabilityState,
    pub lifecycle: LifecycleState,
}

impl ModelEntry {
    pub fn is_selectable(&self) -> bool {
        self.availability == AvailabilityState::Available
            && matches!(
                self.lifecycle,
                LifecycleState::Unloaded | LifecycleState::Ready
            )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleState {
    Unloaded,
    Loading,
    Ready,
    Busy,
    Unloading,
    Failed,
    Disabled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RegistryError {
    Manifest(ManifestError),
    DuplicateModel,
    ModelNotFound,
    NotAvailable,
    InvalidLifecycleTransition,
}

impl fmt::Display for RegistryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Manifest(error) => write!(formatter, "invalid model manifest: {error}"),
            Self::DuplicateModel => formatter.write_str("model is already registered"),
            Self::ModelNotFound => formatter.write_str("model is not registered"),
            Self::NotAvailable => formatter.write_str("model is not available"),
            Self::InvalidLifecycleTransition => {
                formatter.write_str("invalid model lifecycle transition")
            }
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectionRequest<'a> {
    pub capability: &'a CapabilityId,
    pub role: Option<&'a RoleId>,
    pub minimum_context_tokens: u32,
    pub resources: ResourceBudget,
    pub target_architecture: &'a str,
    pub offline_only: bool,
    pub preferred_model_id: Option<&'a ModelId>,
    pub role_default_model_id: Option<&'a ModelId>,
}

impl<'a> SelectionRequest<'a> {
    pub fn use_role_policy(mut self, policy: &'a RoleDefaultPolicy) -> Self {
        self.role_default_model_id = self.role.and_then(|role| policy.default_model_for(role));
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoleDefault {
    pub role: RoleId,
    pub model_id: ModelId,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RoleDefaultPolicy {
    defaults: Vec<RoleDefault>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RolePolicyError {
    DuplicateRole,
}

impl RoleDefaultPolicy {
    pub fn new(defaults: Vec<RoleDefault>) -> Result<Self, RolePolicyError> {
        for (index, candidate) in defaults.iter().enumerate() {
            if defaults[..index]
                .iter()
                .any(|existing| existing.role == candidate.role)
            {
                return Err(RolePolicyError::DuplicateRole);
            }
        }
        Ok(Self { defaults })
    }

    pub fn default_model_for(&self, role: &RoleId) -> Option<&ModelId> {
        self.defaults
            .iter()
            .find(|default| &default.role == role)
            .map(|default| &default.model_id)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SelectionPlan {
    pub selected: ModelId,
    pub fallbacks: Vec<ModelId>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SelectionError {
    NoCompatibleModel,
}

impl fmt::Display for SelectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("no installed model matches the requested capability and constraints")
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ModelRegistry {
    entries: Vec<ModelEntry>,
}

impl ModelRegistry {
    pub const fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub fn entries(&self) -> &[ModelEntry] {
        &self.entries
    }

    pub fn get(&self, model_id: &ModelId) -> Option<&ModelEntry> {
        self.entries
            .iter()
            .find(|entry| &entry.manifest.model_id == model_id)
    }

    pub fn discover<A: ArtifactCatalog>(
        &mut self,
        manifest: ModelManifest,
        artifacts: &A,
        backends: &[BackendDescriptor],
        resources: ResourceBudget,
        target_architecture: &str,
    ) -> Result<AvailabilityState, RegistryError> {
        manifest.validate().map_err(RegistryError::Manifest)?;
        if self.get(&manifest.model_id).is_some() {
            return Err(RegistryError::DuplicateModel);
        }
        let availability = self.evaluate(
            &manifest,
            artifacts,
            backends,
            resources,
            target_architecture,
        );
        self.entries.push(ModelEntry {
            manifest,
            availability,
            lifecycle: LifecycleState::Unloaded,
        });
        Ok(availability)
    }

    pub fn transition_lifecycle(
        &mut self,
        model_id: &ModelId,
        next: LifecycleState,
    ) -> Result<(), RegistryError> {
        let entry = self
            .entries
            .iter_mut()
            .find(|entry| &entry.manifest.model_id == model_id)
            .ok_or(RegistryError::ModelNotFound)?;
        if entry.availability != AvailabilityState::Available {
            return Err(RegistryError::NotAvailable);
        }
        let allowed = matches!(
            (entry.lifecycle, next),
            (LifecycleState::Unloaded, LifecycleState::Loading)
                | (LifecycleState::Loading, LifecycleState::Ready)
                | (LifecycleState::Loading, LifecycleState::Failed)
                | (LifecycleState::Ready, LifecycleState::Busy)
                | (LifecycleState::Ready, LifecycleState::Unloading)
                | (LifecycleState::Busy, LifecycleState::Ready)
                | (LifecycleState::Busy, LifecycleState::Unloading)
                | (LifecycleState::Busy, LifecycleState::Failed)
                | (LifecycleState::Unloading, LifecycleState::Unloaded)
                | (LifecycleState::Unloading, LifecycleState::Failed)
                | (LifecycleState::Failed, LifecycleState::Loading)
                | (LifecycleState::Failed, LifecycleState::Disabled)
                | (LifecycleState::Disabled, LifecycleState::Unloaded)
        );
        if !allowed {
            return Err(RegistryError::InvalidLifecycleTransition);
        }
        entry.lifecycle = next;
        Ok(())
    }

    pub fn select(&self, request: &SelectionRequest<'_>) -> Result<SelectionPlan, SelectionError> {
        let mut compatible: Vec<&ModelEntry> = self
            .entries
            .iter()
            .filter(|entry| {
                entry.is_selectable()
                    && entry.manifest.has_capability(request.capability)
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
            })
            .collect();
        compatible.sort_by(|left, right| left.manifest.model_id.cmp(&right.manifest.model_id));
        if compatible.is_empty() {
            return Err(SelectionError::NoCompatibleModel);
        }
        let preferred_index = request.preferred_model_id.and_then(|id| {
            compatible
                .iter()
                .position(|entry| &entry.manifest.model_id == id)
        });
        let default_index = request.role_default_model_id.and_then(|id| {
            compatible
                .iter()
                .position(|entry| &entry.manifest.model_id == id)
        });
        let selected_index = preferred_index.or(default_index).unwrap_or(0);
        let selected = compatible.remove(selected_index).manifest.model_id.clone();
        let fallbacks = compatible
            .into_iter()
            .map(|entry| entry.manifest.model_id.clone())
            .collect();
        Ok(SelectionPlan {
            selected,
            fallbacks,
        })
    }

    fn evaluate<A: ArtifactCatalog>(
        &self,
        manifest: &ModelManifest,
        artifacts: &A,
        backends: &[BackendDescriptor],
        resources: ResourceBudget,
        target_architecture: &str,
    ) -> AvailabilityState {
        if manifest.execution == ExecutionScope::Remote {
            return AvailabilityState::RemoteProviderUnavailable;
        }
        if manifest.artifact.integrity.is_none() {
            return AvailabilityState::MissingIntegrity;
        }
        match artifacts.inspect(&manifest.artifact) {
            ArtifactStatus::Missing => return AvailabilityState::MissingArtifact,
            ArtifactStatus::Present {
                integrity_verified: false,
            } if manifest.artifact.integrity.is_some() => {
                return AvailabilityState::IntegrityMismatch;
            }
            ArtifactStatus::Present { .. } => {}
        }
        let supported_backends: Vec<&BackendDescriptor> = backends
            .iter()
            .filter(|backend| {
                manifest
                    .supported_backends
                    .iter()
                    .any(|id| id == &backend.backend_id)
            })
            .collect();
        if supported_backends.is_empty() {
            return AvailabilityState::IncompatibleBackend;
        }
        if !supported_backends
            .iter()
            .any(|backend| backend.supports_runtime(manifest))
        {
            return AvailabilityState::IncompatibleRuntime;
        }
        if !supported_backends
            .iter()
            .any(|backend| backend.supports_manifest(manifest, target_architecture))
        {
            return AvailabilityState::IncompatibleBackend;
        }
        if !resources_fit(manifest, resources) {
            return AvailabilityState::IncompatibleResources;
        }
        AvailabilityState::Available
    }
}

fn resources_fit(manifest: &ModelManifest, resources: ResourceBudget) -> bool {
    manifest.resources.minimum_ram_bytes <= resources.available_ram_bytes
        && manifest.resources.minimum_storage_bytes <= resources.available_storage_bytes
        && manifest.resources.minimum_cpu_cores <= resources.cpu_cores
        && (!manifest.resources.gpu_required || resources.gpu_available)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ArtifactReference, CapabilityId, ModelManifest, ProviderId, RoleId};

    struct PresentArtifact;

    impl ArtifactCatalog for PresentArtifact {
        fn inspect(&self, _: &ArtifactDescriptor) -> ArtifactStatus {
            ArtifactStatus::Present {
                integrity_verified: true,
            }
        }
    }

    struct MissingArtifact;

    impl ArtifactCatalog for MissingArtifact {
        fn inspect(&self, _: &ArtifactDescriptor) -> ArtifactStatus {
            ArtifactStatus::Missing
        }
    }

    fn manifest(json: &str) -> ModelManifest {
        let mut manifest = ModelManifest::parse_json(json.as_bytes()).expect("fixture manifest");
        // Synthetic installed-artifact metadata is only used by registry tests.
        manifest.artifact.size_bytes = Some(1);
        manifest.artifact.integrity = Some(crate::IntegrityMetadata {
            algorithm: String::from("sha256"),
            digest: "a".repeat(64),
        });
        manifest
    }

    fn backend() -> BackendDescriptor {
        BackendDescriptor {
            backend_id: BackendId::new("llama_cpp").unwrap(),
            artifact_formats: vec![FormatId::new("gguf").unwrap()],
            runtime_api_versions: vec![String::from(crate::MODEL_RUNTIME_API_VERSION)],
            architectures: vec![String::from("x86_64")],
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

    fn request<'a>(capability: &'a CapabilityId) -> SelectionRequest<'a> {
        SelectionRequest {
            capability,
            role: None,
            minimum_context_tokens: 1024,
            resources: budget(),
            target_architecture: "x86_64",
            offline_only: true,
            preferred_model_id: None,
            role_default_model_id: None,
        }
    }

    #[test]
    fn matches_capability_and_resource_requirements() {
        let json = include_str!("../tests/fixtures/granite-4.2-3b.json");
        let model = manifest(json);
        assert_eq!(model.provider.provider_id, ProviderId::new("ibm").unwrap());
        assert!(model.has_capability(&CapabilityId::new("text.generate").unwrap()));
        assert!(!model.has_capability(&CapabilityId::new("vision").unwrap()));

        let mut registry = ModelRegistry::new();
        assert_eq!(
            registry.discover(model, &PresentArtifact, &[backend()], budget(), "x86_64"),
            Ok(AvailabilityState::Available)
        );
        let capability = CapabilityId::new("text.generate").unwrap();
        let mut selection = request(&capability);
        let standard = RoleId::new("standard").unwrap();
        selection.role = Some(&standard);
        selection.resources.available_ram_bytes = 1;
        assert_eq!(
            registry.select(&selection),
            Err(SelectionError::NoCompatibleModel)
        );
    }

    #[test]
    fn reports_missing_artifact_and_backend_without_fabricating_availability() {
        let mut missing_registry = ModelRegistry::new();
        let missing = manifest(include_str!("../tests/fixtures/gemma-3-1b.json"));
        assert_eq!(
            missing_registry.discover(missing, &MissingArtifact, &[backend()], budget(), "x86_64"),
            Ok(AvailabilityState::MissingArtifact)
        );
        let capability = CapabilityId::new("text.generate").unwrap();
        assert_eq!(
            missing_registry.select(&request(&capability)),
            Err(SelectionError::NoCompatibleModel)
        );

        let mut incompatible_registry = ModelRegistry::new();
        let incompatible = manifest(include_str!("../tests/fixtures/qwen3-4b.json"));
        assert_eq!(
            incompatible_registry.discover(incompatible, &PresentArtifact, &[], budget(), "x86_64"),
            Ok(AvailabilityState::IncompatibleBackend)
        );
    }

    #[test]
    fn metadata_without_integrity_is_not_selectable() {
        let model = ModelManifest::parse_json(
            include_str!("../tests/fixtures/granite-4.2-3b.json").as_bytes(),
        )
        .unwrap();
        let mut registry = ModelRegistry::new();
        assert_eq!(
            registry.discover(model, &PresentArtifact, &[backend()], budget(), "x86_64"),
            Ok(AvailabilityState::MissingIntegrity)
        );
        let capability = CapabilityId::new("text.generate").unwrap();
        assert_eq!(
            registry.select(&request(&capability)),
            Err(SelectionError::NoCompatibleModel)
        );
    }

    #[test]
    fn selection_preference_role_default_and_fallback_are_deterministic() {
        let fixtures = [
            include_str!("../tests/fixtures/qwen3-4b.json"),
            include_str!("../tests/fixtures/granite-4.2-3b.json"),
            include_str!("../tests/fixtures/gemma-3-1b.json"),
        ];
        let mut registry = ModelRegistry::new();
        for fixture in fixtures {
            let model = manifest(fixture);
            assert_eq!(
                registry.discover(model, &PresentArtifact, &[backend()], budget(), "x86_64"),
                Ok(AvailabilityState::Available)
            );
        }
        let capability = CapabilityId::new("text.generate").unwrap();
        let granite = ModelId::new("ibm.granite-4.2-3b").unwrap();
        let qwen = ModelId::new("qwen.qwen3-4b").unwrap();
        let mut selection = request(&capability);
        let standard = RoleId::new("standard").unwrap();
        selection.role = Some(&standard);
        let role_policy = RoleDefaultPolicy::new(vec![RoleDefault {
            role: standard.clone(),
            model_id: granite.clone(),
        }])
        .unwrap();
        selection = selection.use_role_policy(&role_policy);
        let default_plan = registry.select(&selection).unwrap();
        assert_eq!(default_plan.selected, granite);
        assert_eq!(default_plan.fallbacks, vec![qwen.clone()]);
        selection.preferred_model_id = Some(&qwen);
        let preferred_plan = registry.select(&selection).unwrap();
        assert_eq!(preferred_plan.selected, qwen);
        assert_eq!(preferred_plan.fallbacks, vec![granite.clone()]);
        selection.role_default_model_id = None;
        selection.preferred_model_id = None;
        assert_eq!(
            registry.select(&selection).unwrap().selected.as_str(),
            "ibm.granite-4.2-3b"
        );

        let lite = RoleId::new("lite").unwrap();
        let gemma = ModelId::new("google.gemma-3-1b").unwrap();
        selection.role = Some(&lite);
        let lite_policy = RoleDefaultPolicy::new(vec![RoleDefault {
            role: lite.clone(),
            model_id: gemma.clone(),
        }])
        .unwrap();
        selection = selection.use_role_policy(&lite_policy);
        let lite_plan = registry.select(&selection).unwrap();
        assert_eq!(lite_plan.selected, gemma);
        assert!(lite_plan.fallbacks.is_empty());
    }

    #[test]
    fn rejects_one_bad_manifest_without_mutating_registered_models() {
        let mut registry = ModelRegistry::new();
        let good = manifest(include_str!("../tests/fixtures/granite-4.2-3b.json"));
        let id = good.model_id.clone();
        registry
            .discover(good, &PresentArtifact, &[backend()], budget(), "x86_64")
            .unwrap();
        let mut invalid = manifest(include_str!("../tests/fixtures/qwen3-4b.json"));
        invalid.schema_version = 99;
        assert_eq!(
            registry.discover(invalid, &PresentArtifact, &[backend()], budget(), "x86_64"),
            Err(RegistryError::Manifest(
                ManifestError::UnsupportedSchemaVersion
            ))
        );
        assert_eq!(registry.entries().len(), 1);
        assert!(registry.get(&id).is_some());
    }

    #[test]
    fn validates_model_store_reference_as_opaque_identifier() {
        let model = manifest(include_str!("../tests/fixtures/granite-4.2-3b.json"));
        match &model.artifact.reference {
            ArtifactReference::ModelStore { artifact_id } => {
                assert_eq!(artifact_id.as_str(), "ibm.granite-4.2-3b")
            }
        }
    }

    #[test]
    fn lifecycle_requires_valid_transitions() {
        let mut registry = ModelRegistry::new();
        let model = manifest(include_str!("../tests/fixtures/granite-4.2-3b.json"));
        let id = model.model_id.clone();
        registry
            .discover(model, &PresentArtifact, &[backend()], budget(), "x86_64")
            .unwrap();
        assert_eq!(
            registry.transition_lifecycle(&id, LifecycleState::Ready),
            Err(RegistryError::InvalidLifecycleTransition)
        );
        registry
            .transition_lifecycle(&id, LifecycleState::Loading)
            .unwrap();
        registry
            .transition_lifecycle(&id, LifecycleState::Ready)
            .unwrap();
        registry
            .transition_lifecycle(&id, LifecycleState::Busy)
            .unwrap();
        assert!(!registry.get(&id).unwrap().is_selectable());
        registry
            .transition_lifecycle(&id, LifecycleState::Ready)
            .unwrap();
        assert!(registry.get(&id).unwrap().is_selectable());
    }
}
