use nagi_installer::{
    AppSdkManifestAdapter, ErrorKind, FailureInjector, FailurePoint, Installer, InstallerError,
    ManifestAdapter, MetadataReferences, NoopPolicy, PackageMetadata, PackageSource, VirtualFile,
};
use nagi_sdk::app_contract::{
    AppEntrypoint, AppIdentity, AppManifestContract, AppOrigin, AppStateVersion, ContractVersion,
    DisplayName, EntrypointKind, StateCompatibility, APP_MANIFEST_SCHEMA_VERSION, EN_US,
    SDK_CONTRACT_VERSION,
};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

const APP: &str = "com.nagi.metadata-boundary";
const ENTRY: &str = "bin/app.napp";
static NEXT_ROOT: AtomicU64 = AtomicU64::new(1);
struct TestRoot(PathBuf);
impl TestRoot {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "nagi-metadata-contract-{}-{}",
            std::process::id(),
            NEXT_ROOT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
}
impl Drop for TestRoot {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn manifest(version: &str) -> AppManifestContract<'_> {
    AppManifestContract {
        schema_version: APP_MANIFEST_SCHEMA_VERSION,
        sdk_contract_version: SDK_CONTRACT_VERSION,
        identity: AppIdentity::new(APP, version, AppOrigin::ThirdParty, None).unwrap(),
        display_name: DisplayName {
            en_us: "Synthetic package",
            ja_jp: None,
        },
        supported_locales: &[EN_US],
        icon: None,
        entrypoint: AppEntrypoint {
            kind: EntrypointKind::Native,
            target: ENTRY,
        },
        resources: &[],
        intents: &[],
        requested_capabilities: &[],
        background_services: &[],
        state: StateCompatibility {
            current: AppStateVersion(1),
            minimum_readable: AppStateVersion(1),
        },
    }
}
// A synthetic adapter exercises the public metadata boundary independently of SDK projection.
struct FakeAdapter;
impl ManifestAdapter<str> for FakeAdapter {
    fn adapt(&self, version: &str) -> Result<PackageMetadata, InstallerError> {
        Ok(PackageMetadata {
            app_id: APP.into(),
            version: version.into(),
            manifest_schema_version: APP_MANIFEST_SCHEMA_VERSION,
            entrypoint: ENTRY.into(),
            required_capabilities: vec![],
            references: MetadataReferences::default(),
        })
    }
}
fn package(version: &str) -> Result<PackageSource, InstallerError> {
    PackageSource::from_virtual_files(
        FakeAdapter.adapt(version)?,
        vec![VirtualFile {
            path: ENTRY.into(),
            contents: version.as_bytes().to_vec(),
            executable: true,
        }],
    )
}
fn assert_active(installer: &Installer, version: &str) {
    let active = installer.installed(APP).unwrap().unwrap();
    assert_eq!(active.version, version);
    assert_eq!(
        fs::read(installer.package_path(&active).unwrap().join(ENTRY)).unwrap(),
        version.as_bytes()
    );
    assert_eq!(installer.inventory().unwrap().len(), 1);
}

#[test]
fn oversized_core_components_fail_at_sdk_and_fake_metadata_boundaries() {
    for version in [
        "18446744073709551616.0.0",
        "0.18446744073709551616.0",
        "0.0.18446744073709551616",
    ] {
        let manifest = manifest(version);
        assert!(
            manifest.validate().is_ok(),
            "SDK syntax fixture must be valid"
        );
        assert_eq!(
            AppSdkManifestAdapter.adapt(&manifest).unwrap_err().kind(),
            ErrorKind::InvalidMetadata
        );
        let projected = FakeAdapter.adapt(version).unwrap();
        assert_eq!(
            projected.validate().unwrap_err().kind(),
            ErrorKind::InvalidMetadata
        );
        assert_eq!(
            package(version).unwrap_err().kind(),
            ErrorKind::InvalidMetadata
        );
        // Metadata must fail before attempting any source directory I/O.
        assert_eq!(
            PackageSource::from_directory("no-such-synthetic-package", projected)
                .unwrap_err()
                .kind(),
            ErrorKind::InvalidMetadata
        );
    }
}

#[test]
fn maximum_core_components_roundtrip_without_poisoning_inventory() {
    for version in [
        "18446744073709551615.0.0",
        "0.18446744073709551615.0",
        "0.0.18446744073709551615",
    ] {
        let root = TestRoot::new();
        let installer = Installer::open(&root.0).unwrap();
        assert!(AppSdkManifestAdapter.adapt(&manifest(version)).is_ok());
        installer.install(package(version).unwrap()).unwrap();
        drop(installer);
        let reopened = Installer::open(&root.0).unwrap();
        reopened.recover().unwrap();
        assert_active(&reopened, version);
    }
}

#[test]
fn incompatible_manifest_and_sdk_are_rejected_without_active_state_changes() {
    let root = TestRoot::new();
    let installer = Installer::open(&root.0).unwrap();
    installer.install(package("1.0.0").unwrap()).unwrap();
    let before = fs::read(root.0.join("inventory.json")).unwrap();
    for schema in [0, APP_MANIFEST_SCHEMA_VERSION + 1, u16::MAX] {
        let mut input = manifest("2.0.0");
        input.schema_version = schema;
        assert!(
            matches!(AppSdkManifestAdapter.adapt(&input), Err(InstallerError::UnsupportedSchema { kind: "app manifest", version }) if version == u32::from(schema))
        );
    }
    let mut input = manifest("2.0.0");
    input.sdk_contract_version = ContractVersion {
        major: SDK_CONTRACT_VERSION.major + 1,
        minor: 0,
    };
    assert_eq!(
        AppSdkManifestAdapter.adapt(&input).unwrap_err().kind(),
        ErrorKind::InvalidMetadata
    );
    assert_eq!(fs::read(root.0.join("inventory.json")).unwrap(), before);
    assert_active(&installer, "1.0.0");
}

#[test]
fn prerelease_and_build_precedence_rejections_preserve_state_and_allow_upgrade() {
    let root = TestRoot::new();
    let installer = Installer::open(&root.0).unwrap();
    installer
        .install(package("1.0.0-rc.10+old").unwrap())
        .unwrap();
    let before = fs::read(root.0.join("inventory.json")).unwrap();
    for (version, expected) in [
        ("1.0.0-rc.2", ErrorKind::DowngradeRejected),
        ("1.0.0-rc.10+new", ErrorKind::VersionConflict),
    ] {
        assert_eq!(
            installer
                .install(package(version).unwrap())
                .unwrap_err()
                .kind(),
            expected
        );
        assert_eq!(fs::read(root.0.join("inventory.json")).unwrap(), before);
        assert_active(&installer, "1.0.0-rc.10+old");
        for dir in ["staging", "transactions"] {
            assert_eq!(fs::read_dir(root.0.join(dir)).unwrap().count(), 0);
        }
    }
    installer.install(package("1.0.0").unwrap()).unwrap();
    assert_active(&installer, "1.0.0");
}

struct FailAt(FailurePoint);
impl FailureInjector for FailAt {
    fn check(&mut self, point: FailurePoint) -> Result<(), String> {
        if point == self.0 {
            Err("synthetic interruption".into())
        } else {
            Ok(())
        }
    }
}
#[test]
fn interrupted_first_install_recovers_to_empty_then_accepts_retry() {
    for point in [
        FailurePoint::AfterJournalCreated,
        FailurePoint::AfterStaging,
        FailurePoint::BeforeJournalCommit,
    ] {
        let root = TestRoot::new();
        let installer = Installer::open(&root.0).unwrap();
        assert_eq!(
            installer
                .install_with(
                    package("1.0.0").unwrap(),
                    &mut NoopPolicy,
                    &mut FailAt(point)
                )
                .unwrap_err()
                .kind(),
            ErrorKind::InjectedFailure
        );
        drop(installer);
        let reopened = Installer::open(&root.0).unwrap();
        reopened.recover().unwrap();
        let recovered = reopened.inventory().unwrap();
        assert!(recovered.is_empty());
        reopened.recover().unwrap();
        assert_eq!(reopened.inventory().unwrap(), recovered);
        for dir in ["staging", "transactions"] {
            assert_eq!(fs::read_dir(root.0.join(dir)).unwrap().count(), 0);
        }
        reopened.install(package("1.0.0").unwrap()).unwrap();
        assert_active(&reopened, "1.0.0");
    }
}
