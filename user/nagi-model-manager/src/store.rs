use alloc::string::String;
use core::fmt;

use crate::{IntegrityMetadata, LicenseMetadata, ManifestError, ModelId, ModelManifest};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InstallState {
    NotInstalled,
    InstallRequested,
    Installing,
    Verifying,
    Installed,
    UpdateAvailable,
    RemovalRequested,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RemovalEligibility {
    Removable,
    InUse,
    SystemRequired,
    NotInstalled,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ModelStoreRecord {
    pub manifest: ModelManifest,
    pub state: InstallState,
    pub installed_version: Option<String>,
    pub update_version: Option<String>,
    pub integrity: Option<IntegrityMetadata>,
    pub license: LicenseMetadata,
    pub license_acknowledged: bool,
    removal_constraint: RemovalEligibility,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StoreError {
    Manifest(ManifestError),
    LicenseAcknowledgementRequired,
    IntegrityRequired,
    InvalidLicenseReference,
    NotInstalled,
    InUse,
    SystemRequired,
    InvalidTransition,
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let description = match self {
            Self::Manifest(error) => return write!(formatter, "invalid model manifest: {error}"),
            Self::LicenseAcknowledgementRequired => {
                "model terms must be acknowledged before install"
            }
            Self::IntegrityRequired => "model artifact requires a verified integrity digest",
            Self::InvalidLicenseReference => "license acknowledgement does not match model terms",
            Self::NotInstalled => "model is not installed",
            Self::InUse => "model is currently in use",
            Self::SystemRequired => "model is required by the system",
            Self::InvalidTransition => "invalid model store state transition",
        };
        formatter.write_str(description)
    }
}

impl ModelStoreRecord {
    pub fn discovered(manifest: ModelManifest) -> Result<Self, StoreError> {
        manifest.validate().map_err(StoreError::Manifest)?;
        let license_acknowledged = !manifest.license.acknowledgement_required;
        Ok(Self {
            integrity: manifest.artifact.integrity.clone(),
            license: manifest.license.clone(),
            manifest,
            state: InstallState::NotInstalled,
            installed_version: None,
            update_version: None,
            license_acknowledged,
            removal_constraint: RemovalEligibility::Removable,
        })
    }

    pub fn model_id(&self) -> &ModelId {
        &self.manifest.model_id
    }

    pub fn acknowledge_terms(&mut self, reference: &str) -> Result<(), StoreError> {
        match self.license.terms_reference.as_deref() {
            Some(expected) if expected == reference => {
                self.license_acknowledged = true;
                Ok(())
            }
            _ => Err(StoreError::InvalidLicenseReference),
        }
    }

    pub fn request_install(&mut self) -> Result<(), StoreError> {
        if self.license.acknowledgement_required && !self.license_acknowledged {
            return Err(StoreError::LicenseAcknowledgementRequired);
        }
        if self.integrity.is_none() {
            return Err(StoreError::IntegrityRequired);
        }
        self.transition(InstallState::InstallRequested)
    }

    pub fn publish_update(&mut self, version: impl Into<String>) -> Result<(), StoreError> {
        let version = version.into();
        if self.state != InstallState::Installed
            || version.trim().is_empty()
            || self.installed_version.as_deref() == Some(version.as_str())
        {
            return Err(StoreError::InvalidTransition);
        }
        self.update_version = Some(version);
        self.state = InstallState::UpdateAvailable;
        Ok(())
    }

    pub fn set_removal_constraint(&mut self, constraint: RemovalEligibility) {
        self.removal_constraint = constraint;
    }

    pub fn removal_eligibility(&self) -> RemovalEligibility {
        if self.installed_version.is_none() {
            RemovalEligibility::NotInstalled
        } else {
            self.removal_constraint
        }
    }

    pub fn request_remove(&mut self) -> Result<(), StoreError> {
        match self.removal_eligibility() {
            RemovalEligibility::NotInstalled => return Err(StoreError::NotInstalled),
            RemovalEligibility::InUse => return Err(StoreError::InUse),
            RemovalEligibility::SystemRequired => return Err(StoreError::SystemRequired),
            RemovalEligibility::Removable => {}
        }
        self.transition(InstallState::RemovalRequested)
    }

    pub fn transition(&mut self, next: InstallState) -> Result<(), StoreError> {
        let allowed = matches!(
            (self.state, next),
            (InstallState::NotInstalled, InstallState::InstallRequested)
                | (InstallState::InstallRequested, InstallState::Installing)
                | (InstallState::InstallRequested, InstallState::Failed)
                | (InstallState::Installing, InstallState::Verifying)
                | (InstallState::Installing, InstallState::Failed)
                | (InstallState::Verifying, InstallState::Installed)
                | (InstallState::Verifying, InstallState::Failed)
                | (InstallState::Installed, InstallState::UpdateAvailable)
                | (InstallState::Installed, InstallState::RemovalRequested)
                | (
                    InstallState::UpdateAvailable,
                    InstallState::InstallRequested
                )
                | (
                    InstallState::UpdateAvailable,
                    InstallState::RemovalRequested
                )
                | (InstallState::RemovalRequested, InstallState::NotInstalled)
                | (InstallState::RemovalRequested, InstallState::Installed)
                | (InstallState::Failed, InstallState::InstallRequested)
                | (InstallState::Failed, InstallState::RemovalRequested)
                | (InstallState::Failed, InstallState::NotInstalled)
        );
        if !allowed
            || (self.state == InstallState::Failed
                && next == InstallState::NotInstalled
                && self.installed_version.is_some())
        {
            return Err(StoreError::InvalidTransition);
        }
        match next {
            InstallState::Installed => {
                self.installed_version = Some(
                    self.update_version
                        .take()
                        .unwrap_or_else(|| self.manifest.version.clone()),
                );
            }
            InstallState::NotInstalled => {
                self.installed_version = None;
                self.update_version = None;
            }
            _ => {}
        }
        self.state = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ModelManifest;

    fn manifest(name: &str) -> ModelManifest {
        let mut manifest = ModelManifest::parse_json(name.as_bytes()).unwrap();
        // Synthetic metadata is confined to Store state-machine tests.
        manifest.artifact.integrity = Some(crate::IntegrityMetadata {
            algorithm: alloc::string::String::from("sha256"),
            digest: "a".repeat(64),
        });
        manifest.artifact.size_bytes = Some(1);
        manifest
    }

    #[test]
    fn records_license_terms_before_allowing_install() {
        let gemma = manifest(include_str!("../tests/fixtures/gemma-3-1b.json"));
        let mut record = ModelStoreRecord::discovered(gemma).unwrap();
        assert_eq!(record.license.identifier, "google-gemma-terms");
        assert!(!record.license_acknowledged);
        assert_eq!(
            record.request_install(),
            Err(StoreError::LicenseAcknowledgementRequired)
        );
        assert_eq!(
            record.acknowledge_terms("provider-terms:wrong-model"),
            Err(StoreError::InvalidLicenseReference)
        );
        record
            .acknowledge_terms("provider-terms:google.gemma")
            .unwrap();
        record.request_install().unwrap();
        assert_eq!(record.state, InstallState::InstallRequested);
    }

    #[test]
    fn enforces_install_verification_update_and_removal_states() {
        let granite = manifest(include_str!("../tests/fixtures/granite-4.2-3b.json"));
        let mut record = ModelStoreRecord::discovered(granite).unwrap();
        record.request_install().unwrap();
        record.transition(InstallState::Installing).unwrap();
        record.transition(InstallState::Verifying).unwrap();
        record.transition(InstallState::Installed).unwrap();
        assert_eq!(record.installed_version.as_deref(), Some("4.2"));

        record.publish_update("4.2.1").unwrap();
        assert_eq!(record.state, InstallState::UpdateAvailable);
        record.request_install().unwrap();
        record.transition(InstallState::Installing).unwrap();
        record.transition(InstallState::Verifying).unwrap();
        record.transition(InstallState::Installed).unwrap();
        assert_eq!(record.installed_version.as_deref(), Some("4.2.1"));

        record.set_removal_constraint(RemovalEligibility::InUse);
        assert_eq!(record.request_remove(), Err(StoreError::InUse));
        record.set_removal_constraint(RemovalEligibility::SystemRequired);
        assert_eq!(record.request_remove(), Err(StoreError::SystemRequired));
        record.set_removal_constraint(RemovalEligibility::Removable);
        record.request_remove().unwrap();
        record.transition(InstallState::NotInstalled).unwrap();
        assert_eq!(
            record.removal_eligibility(),
            RemovalEligibility::NotInstalled
        );
    }

    #[test]
    fn disallows_skipping_install_verification() {
        let qwen = manifest(include_str!("../tests/fixtures/qwen3-4b.json"));
        let mut record = ModelStoreRecord::discovered(qwen).unwrap();
        assert_eq!(
            record.transition(InstallState::Installed),
            Err(StoreError::InvalidTransition)
        );
        assert_eq!(record.state, InstallState::NotInstalled);
    }

    #[test]
    fn does_not_install_catalog_profiles_without_an_integrity_digest() {
        let qwen =
            ModelManifest::parse_json(include_str!("../tests/fixtures/qwen3-4b.json").as_bytes())
                .unwrap();
        let mut record = ModelStoreRecord::discovered(qwen).unwrap();
        assert_eq!(record.request_install(), Err(StoreError::IntegrityRequired));
        assert_eq!(record.state, InstallState::NotInstalled);
    }

    #[test]
    fn failed_update_keeps_the_installed_version_and_retry_target() {
        let granite = manifest(include_str!("../tests/fixtures/granite-4.2-3b.json"));
        let mut record = ModelStoreRecord::discovered(granite).unwrap();
        record.request_install().unwrap();
        record.transition(InstallState::Installing).unwrap();
        record.transition(InstallState::Verifying).unwrap();
        record.transition(InstallState::Installed).unwrap();
        record.publish_update("4.2.1").unwrap();
        record.request_install().unwrap();
        record.transition(InstallState::Installing).unwrap();
        record.transition(InstallState::Failed).unwrap();
        assert_eq!(record.installed_version.as_deref(), Some("4.2"));
        assert_eq!(record.update_version.as_deref(), Some("4.2.1"));
        assert_eq!(
            record.transition(InstallState::NotInstalled),
            Err(StoreError::InvalidTransition)
        );
        record.request_install().unwrap();
        assert_eq!(record.state, InstallState::InstallRequested);
    }
}
