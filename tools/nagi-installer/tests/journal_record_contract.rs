use nagi_installer::{
    ErrorKind, FailureInjector, FailurePoint, Installer, MetadataReferences, NoopPolicy,
    PackageMetadata, PackageSource, VirtualFile,
};
use nagi_sdk::app_contract::APP_MANIFEST_SCHEMA_VERSION;
use serde_json::{json, Value};
use std::{
    fs,
    path::PathBuf,
    sync::atomic::{AtomicU64, Ordering},
};

const APP: &str = "com.nagi.journal-boundary";
const ENTRY: &str = "bin/app.napp";
static NEXT: AtomicU64 = AtomicU64::new(1);
struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "nagi-journal-contract-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&p).unwrap();
        Self(p)
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn source(version: &str, capabilities: &[&str]) -> PackageSource {
    PackageSource::from_virtual_files(
        PackageMetadata {
            app_id: APP.into(),
            version: version.into(),
            manifest_schema_version: APP_MANIFEST_SCHEMA_VERSION,
            entrypoint: ENTRY.into(),
            required_capabilities: capabilities.iter().map(|id| (*id).into()).collect(),
            references: MetadataReferences::default(),
        },
        vec![VirtualFile {
            path: ENTRY.into(),
            contents: version.as_bytes().to_vec(),
            executable: true,
        }],
    )
    .unwrap()
}
struct Interrupt;
impl FailureInjector for Interrupt {
    fn check(&mut self, point: FailurePoint) -> Result<(), String> {
        if point == FailurePoint::AfterActiveSwitch {
            Err("synthetic interruption".into())
        } else {
            Ok(())
        }
    }
}
fn check_rejection(record: &str, invalid: Value) {
    let root = Root::new();
    let installer = Installer::open(&root.0).unwrap();
    let prior = installer
        .install(source("1.0.0", &["storage.read"]))
        .unwrap();
    let prior_path = installer.package_path(&prior).unwrap().join(ENTRY);
    assert_eq!(
        installer
            .install_with(
                source("2.0.0", &["network.fetch"]),
                &mut NoopPolicy,
                &mut Interrupt
            )
            .unwrap_err()
            .kind(),
        ErrorKind::InjectedFailure
    );
    let marker = fs::read_dir(root.0.join("transactions"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let valid_marker = fs::read(&marker).unwrap();
    let mut journal: Value = serde_json::from_slice(&valid_marker).unwrap();
    let candidate_path = root
        .0
        .join(journal["target"]["active_location"].as_str().unwrap())
        .join(ENTRY);
    journal[record]["required_capabilities"] = invalid;
    fs::write(&marker, serde_json::to_vec(&journal).unwrap()).unwrap();
    let invalid_marker = fs::read(&marker).unwrap();
    let inventory = fs::read(root.0.join("inventory.json")).unwrap();
    let prior_bytes = fs::read(&prior_path).unwrap();
    let candidate_bytes = fs::read(&candidate_path).unwrap();
    drop(installer);
    let reopened = Installer::open(&root.0).unwrap();
    for _ in 0..2 {
        assert_eq!(
            reopened.recover().unwrap_err().kind(),
            ErrorKind::RecoveryFailure
        );
        assert_eq!(
            fs::read(&marker).unwrap(),
            invalid_marker,
            "retain corrupt marker unchanged"
        );
        assert_eq!(
            fs::read(root.0.join("inventory.json")).unwrap(),
            inventory,
            "do not write an unreadable inventory"
        );
        assert_eq!(fs::read(&prior_path).unwrap(), prior_bytes);
        assert_eq!(fs::read(&candidate_path).unwrap(), candidate_bytes);
    }
    // Removing only the test's corruption must permit the original recovery.
    fs::write(&marker, valid_marker).unwrap();
    reopened.recover().unwrap();
    assert_eq!(reopened.installed(APP).unwrap(), Some(prior));
    assert_eq!(fs::read(&prior_path).unwrap(), b"1.0.0");
    assert!(!candidate_path.exists());
    assert_eq!(
        fs::read_dir(root.0.join("transactions")).unwrap().count(),
        0
    );
    assert_eq!(fs::read_dir(root.0.join("staging")).unwrap().count(), 0);
}
#[test]
fn empty_capability_ids_in_prior_and_target_are_rejected_before_recovery_writes() {
    for record in ["prior", "target"] {
        check_rejection(record, json!([""]));
    }
}
#[test]
fn duplicate_capabilities_in_prior_and_target_are_rejected_before_recovery_writes() {
    for record in ["prior", "target"] {
        check_rejection(record, json!(["storage.read", "storage.read"]));
    }
}
#[test]
fn empty_capability_lists_remain_valid_in_prior_and_target() {
    let root = Root::new();
    let installer = Installer::open(&root.0).unwrap();
    let prior = installer.install(source("1.0.0", &[])).unwrap();
    assert_eq!(
        installer
            .install_with(source("2.0.0", &[]), &mut NoopPolicy, &mut Interrupt)
            .unwrap_err()
            .kind(),
        ErrorKind::InjectedFailure
    );
    drop(installer);
    let reopened = Installer::open(&root.0).unwrap();
    reopened.recover().unwrap();
    assert_eq!(reopened.installed(APP).unwrap(), Some(prior));
    reopened.install(source("2.0.0", &[])).unwrap();
    assert_eq!(reopened.installed(APP).unwrap().unwrap().version, "2.0.0");
}
