use std::collections::BTreeSet;

use nagi_model::AppId;
use nagi_package::{InstalledPackage, PackageManifest, PackageService, MAX_SLOTS};

use crate::actions::{ActionAvailability, CapabilityId, TypedAction};
use crate::localization::{Locale, LocalizationCatalog};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IconMetadata {
    /// Stable asset identifier. The host preview uses the glyph and color
    /// fallback until an asset registry can provide an icon resource.
    pub asset_key: String,
    pub fallback_glyph: char,
    pub accent_rgb: [u8; 3],
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppAvailability {
    NagiRuntime,
    HostPreview,
    ComingSoon,
    Unavailable { reason_key: String },
    Unlaunchable { reason_key: String },
}

impl AppAvailability {
    pub fn action_state(&self) -> ActionAvailability {
        match self {
            Self::NagiRuntime => ActionAvailability::Ready,
            Self::HostPreview => ActionAvailability::HostPreviewOnly,
            Self::ComingSoon => ActionAvailability::ComingSoon {
                reason_key: "availability.coming_soon".to_owned(),
            },
            Self::Unavailable { reason_key } => ActionAvailability::Unavailable {
                reason_key: reason_key.clone(),
            },
            Self::Unlaunchable { reason_key } => ActionAvailability::Unlaunchable {
                reason_key: reason_key.clone(),
            },
        }
    }

    pub fn is_launchable(&self) -> bool {
        matches!(self, Self::NagiRuntime | Self::HostPreview)
    }

    pub fn message_key(&self) -> &'static str {
        match self {
            Self::NagiRuntime => "availability.available",
            Self::HostPreview => "availability.host_preview",
            Self::ComingSoon => "availability.coming_soon",
            Self::Unavailable { .. } => "availability.unavailable",
            Self::Unlaunchable { .. } => "availability.unlaunchable",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppDescriptor {
    pub app_id: AppId,
    pub localization_name_key: String,
    pub localization_description_key: String,
    /// Package manifests currently carry a display name but do not yet carry
    /// locale resources. Retain that name as a fallback instead of displaying
    /// a missing localization key.
    pub manifest_name_fallback: Option<String>,
    pub manifest_description_fallback: Option<String>,
    pub icon: IconMetadata,
    pub launcher_order: u16,
    pub availability: AppAvailability,
    pub launch_capability: Option<CapabilityId>,
    /// Host preview navigation metadata. It never represents a Nagi runtime
    /// launch and is populated only by host preview registry entries.
    pub preview_route: Option<String>,
    /// The identity of the launch request is present even for disabled rows.
    /// Callers must check `availability` before dispatching it.
    pub launch_action: TypedAction,
}

impl AppDescriptor {
    pub fn display_name(&self, catalog: &LocalizationCatalog, locale: Locale) -> String {
        self.manifest_name_fallback
            .clone()
            .unwrap_or_else(|| catalog.resolve(&self.localization_name_key, locale))
    }

    pub fn description(&self, catalog: &LocalizationCatalog, locale: Locale) -> String {
        self.manifest_description_fallback
            .clone()
            .unwrap_or_else(|| catalog.resolve(&self.localization_description_key, locale))
    }
}

pub trait AppRegistry: Send + Sync {
    fn list(&self) -> Result<Vec<AppDescriptor>, RegistryError>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RegistryError {
    DuplicateAppId(AppId),
    InvalidDescriptor(&'static str),
    BackendUnavailable,
}

/// Immutable registry snapshot adapter. A future package/service adapter can
/// implement `AppRegistry`; Home consumes this contract instead of hardcoding
/// application behavior in its UI.
#[derive(Clone, Debug, Default)]
pub struct RegistrySnapshot {
    entries: Vec<AppDescriptor>,
}

impl RegistrySnapshot {
    pub fn new(entries: Vec<AppDescriptor>) -> Result<Self, RegistryError> {
        let mut seen = BTreeSet::new();
        for entry in &entries {
            if !seen.insert(entry.app_id.0) {
                return Err(RegistryError::DuplicateAppId(entry.app_id));
            }
            if (entry.localization_name_key.is_empty()
                && entry
                    .manifest_name_fallback
                    .as_deref()
                    .unwrap_or_default()
                    .is_empty())
                || (entry.localization_description_key.is_empty()
                    && entry
                        .manifest_description_fallback
                        .as_deref()
                        .unwrap_or_default()
                        .is_empty())
                || entry.icon.asset_key.is_empty()
            {
                return Err(RegistryError::InvalidDescriptor(
                    "app identity, localization keys, and icon metadata are required",
                ));
            }
            if !matches!(&entry.launch_action, TypedAction::LaunchApp { app_id } if *app_id == entry.app_id)
            {
                return Err(RegistryError::InvalidDescriptor(
                    "launch action must target the registered app identity",
                ));
            }
        }
        let mut entries = entries;
        entries.sort_by(|left, right| {
            left.launcher_order
                .cmp(&right.launcher_order)
                .then_with(|| left.app_id.0.cmp(&right.app_id.0))
        });
        Ok(Self { entries })
    }
}

/// Read-only adapter from the existing M16 Package Service into Home's app
/// registry contract. Installed packages are listed by manifest identity, but
/// remain explicitly unlaunchable until an app process launcher is available.
pub struct PackageRegistryAdapter<'a> {
    packages: &'a PackageService,
}

impl<'a> PackageRegistryAdapter<'a> {
    pub const fn new(packages: &'a PackageService) -> Self {
        Self { packages }
    }
}

impl AppRegistry for PackageRegistryAdapter<'_> {
    fn list(&self) -> Result<Vec<AppDescriptor>, RegistryError> {
        // PackageService exposes a bounded caller-provided output buffer. A
        // parsed sentinel initializes it safely; only the returned count is
        // read, so an empty package slot can never appear in the app list.
        let sentinel = PackageManifest::parse(
            b"id=com.nagi.registry-sentinel\nname=Registry Sentinel\nversion=0\nentry=sentinel\nsurfaces=compact\n",
        )
        .map_err(|_| RegistryError::BackendUnavailable)?;
        let placeholder = InstalledPackage {
            manifest: sentinel,
            generation: 0,
        };
        let mut installed = [placeholder; MAX_SLOTS];
        let count = self
            .packages
            .list(&mut installed)
            .map_err(|_| RegistryError::BackendUnavailable)?;
        let mut entries = Vec::with_capacity(count);
        for package in installed.into_iter().take(count) {
            entries.push(descriptor_from_manifest(package.manifest)?);
        }
        RegistrySnapshot::new(entries)?.list()
    }
}

fn descriptor_from_manifest(manifest: PackageManifest) -> Result<AppDescriptor, RegistryError> {
    let id = std::str::from_utf8(manifest.id())
        .map_err(|_| RegistryError::InvalidDescriptor("manifest app ID is not UTF-8"))?;
    let name = std::str::from_utf8(manifest.name())
        .map_err(|_| RegistryError::InvalidDescriptor("manifest app name is not UTF-8"))?;
    let _version = std::str::from_utf8(manifest.version())
        .map_err(|_| RegistryError::InvalidDescriptor("manifest version is not UTF-8"))?;
    let glyph = name.chars().next().unwrap_or('?');
    let hash = manifest.app_id().0;
    let color = [
        48 + ((hash >> 16) as u8 % 144),
        48 + ((hash >> 8) as u8 % 144),
        48 + (hash as u8 % 144),
    ];
    Ok(AppDescriptor {
        app_id: manifest.app_id(),
        localization_name_key: String::new(),
        localization_description_key: "app.package.description".to_owned(),
        manifest_name_fallback: Some(name.to_owned()),
        manifest_description_fallback: None,
        icon: IconMetadata {
            asset_key: format!("package/{id}"),
            fallback_glyph: glyph,
            accent_rgb: color,
        },
        launcher_order: 1_000,
        availability: AppAvailability::Unlaunchable {
            reason_key: "app.package.launcher_unavailable".to_owned(),
        },
        launch_capability: Some(
            CapabilityId::new("apps.launch").expect("static capability identifier is valid"),
        ),
        preview_route: None,
        launch_action: TypedAction::LaunchApp {
            app_id: manifest.app_id(),
        },
    })
}

impl AppRegistry for RegistrySnapshot {
    fn list(&self) -> Result<Vec<AppDescriptor>, RegistryError> {
        Ok(self.entries.clone())
    }
}

/// Merges trusted registry sources into one stable launcher catalog. The
/// composite validates the merged result so duplicate app identities fail
/// closed instead of producing ambiguous launch targets.
pub struct CompositeAppRegistry<'a> {
    sources: Vec<&'a dyn AppRegistry>,
}

impl<'a> CompositeAppRegistry<'a> {
    pub fn new(sources: Vec<&'a dyn AppRegistry>) -> Self {
        Self { sources }
    }
}

impl AppRegistry for CompositeAppRegistry<'_> {
    fn list(&self) -> Result<Vec<AppDescriptor>, RegistryError> {
        let mut entries = Vec::new();
        for source in &self.sources {
            entries.extend(source.list()?);
        }
        RegistrySnapshot::new(entries)?.list()
    }
}

#[cfg(test)]
mod tests {
    use nagi_model::AppId;
    use nagi_package::{build_xapp, InstallPolicy, PackageService, PackageView, MAX_PACKAGE_BYTES};

    use crate::actions::TypedAction;

    use super::{
        AppAvailability, AppDescriptor, AppRegistry, CompositeAppRegistry, IconMetadata,
        PackageRegistryAdapter, RegistryError, RegistrySnapshot,
    };

    fn descriptor(id: &[u8], order: u16) -> AppDescriptor {
        let app_id = AppId::from_identifier(id);
        AppDescriptor {
            app_id,
            localization_name_key: "app.home".to_owned(),
            localization_description_key: "home.title".to_owned(),
            manifest_name_fallback: None,
            manifest_description_fallback: None,
            icon: IconMetadata {
                asset_key: "app.home".to_owned(),
                fallback_glyph: 'H',
                accent_rgb: [20, 70, 120],
            },
            launcher_order: order,
            availability: AppAvailability::HostPreview,
            launch_capability: None,
            preview_route: None,
            launch_action: TypedAction::LaunchApp { app_id },
        }
    }

    #[test]
    fn registry_is_data_driven_and_has_stable_order() {
        let first = descriptor(b"com.nagi.search", 20);
        let second = descriptor(b"com.nagi.home", 10);
        let registry = RegistrySnapshot::new(vec![first, second.clone()]).unwrap();
        let entries = registry.list().unwrap();
        assert_eq!(entries[0].app_id, second.app_id);
        assert_eq!(
            entries[0].launch_action,
            TypedAction::LaunchApp {
                app_id: second.app_id
            }
        );
    }

    #[test]
    fn duplicate_registry_identity_fails_closed() {
        let entry = descriptor(b"com.nagi.home", 10);
        assert!(matches!(
            RegistrySnapshot::new(vec![entry.clone(), entry]),
            Err(RegistryError::DuplicateAppId(_))
        ));
    }

    #[test]
    fn coming_soon_app_is_not_launchable() {
        assert!(!AppAvailability::ComingSoon.is_launchable());
        assert!(AppAvailability::HostPreview.is_launchable());
    }

    #[test]
    fn package_service_entries_merge_with_first_party_registry_as_unlaunchable_apps() {
        const MANIFEST: &[u8] = b"id=com.example.calendar\nname=Calendar\nversion=1.0\nentry=calendar\nsurfaces=compact\n";
        let mut package_bytes = [0; MAX_PACKAGE_BYTES];
        let package_len = build_xapp(
            MANIFEST,
            b"package payload",
            &[],
            &[],
            &[],
            &[],
            &mut package_bytes,
        )
        .unwrap();
        let package = PackageView::parse(&package_bytes[..package_len]).unwrap();
        let mut packages = PackageService::new();
        packages
            .install(
                &package,
                InstallPolicy {
                    developer_mode: true,
                    require_signature: false,
                },
            )
            .unwrap();

        let first_party = RegistrySnapshot::new(vec![descriptor(b"com.nagi.home", 10)]).unwrap();
        let installed = PackageRegistryAdapter::new(&packages);
        let composite = CompositeAppRegistry::new(vec![&first_party, &installed]);
        let entries = composite.list().unwrap();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].app_id, AppId::from_identifier(b"com.nagi.home"));
        let package_app = &entries[1];
        assert_eq!(
            package_app.display_name(&crate::LocalizationCatalog, crate::Locale::EnUs),
            "Calendar"
        );
        assert_eq!(
            package_app.launch_action,
            TypedAction::LaunchApp {
                app_id: package_app.app_id
            }
        );
        assert!(matches!(
            package_app.availability,
            AppAvailability::Unlaunchable { .. }
        ));
    }

    #[test]
    fn composite_rejects_colliding_app_identity() {
        let first = RegistrySnapshot::new(vec![descriptor(b"com.nagi.home", 10)]).unwrap();
        let second = RegistrySnapshot::new(vec![descriptor(b"com.nagi.home", 20)]).unwrap();
        let composite = CompositeAppRegistry::new(vec![&first, &second]);
        assert!(matches!(
            composite.list(),
            Err(RegistryError::DuplicateAppId(_))
        ));
    }
}
