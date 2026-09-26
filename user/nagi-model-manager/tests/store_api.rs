use nagi_model_manager::{InstallState, ModelManifest, ModelStoreRecord};

#[test]
fn exposes_store_state_through_read_only_consumer_accessors() {
    let manifest = ModelManifest::parse_json(include_bytes!("fixtures/qwen3-4b.json"))
        .expect("valid model manifest");
    let record = ModelStoreRecord::discovered(manifest).expect("valid store record");

    assert_eq!(record.model_id().as_str(), "qwen.qwen3-4b");
    assert_eq!(record.manifest().schema_version, 1);
    assert_eq!(record.state(), InstallState::NotInstalled);
    assert_eq!(record.installed_version(), None);
    assert_eq!(record.update_version(), None);
    assert_eq!(record.integrity(), None);
    assert_eq!(record.license(), &record.manifest().license);
    assert_eq!(
        record.license_acknowledged(),
        !record.license().acknowledgement_required
    );

    let gemma = ModelManifest::parse_json(include_bytes!("fixtures/gemma-3-1b.json"))
        .expect("valid model manifest");
    let gemma = ModelStoreRecord::discovered(gemma).expect("valid store record");
    assert!(!gemma.license_acknowledged());
}
