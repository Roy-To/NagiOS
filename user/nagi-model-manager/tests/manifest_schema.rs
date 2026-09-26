use nagi_model_manager::ModelManifest;
use serde_json::Value;

const SCHEMA: &str = include_str!("../../../docs/schemas/nagi-model-manifest-v1.schema.json");

#[test]
fn schema_file_is_valid_json_and_declares_closed_versioned_manifest() {
    let schema: Value = serde_json::from_str(SCHEMA).expect("valid JSON Schema document");
    assert_eq!(schema["$id"], "urn:nagi:model-manifest:v1");
    assert_eq!(schema["properties"]["schema_version"]["const"], 1);
    assert_eq!(
        schema["properties"]["runtime_api_version"]["const"],
        "nagi.ai/1"
    );
    assert_eq!(schema["additionalProperties"], false);
    for required in [
        "model_id",
        "provider",
        "artifact",
        "capabilities",
        "resources",
        "license",
        "source",
    ] {
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .iter()
            .any(|field| field == required));
    }
}

#[test]
fn bundled_model_profiles_are_valid_v1_schema_examples() {
    let examples = [
        include_bytes!("fixtures/qwen3-4b.json").as_slice(),
        include_bytes!("fixtures/granite-4.2-3b.json").as_slice(),
        include_bytes!("fixtures/gemma-3-1b.json").as_slice(),
    ];
    for example in examples {
        ModelManifest::parse_json(example).expect("manifest profile follows runtime schema");
    }
}
