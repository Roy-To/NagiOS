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
fn bundled_model_profiles_are_valid_v1_manifest_examples() {
    let examples = [
        (
            include_bytes!("fixtures/qwen3-4b.json").as_slice(),
            "qwen.qwen3-4b",
            "standard",
        ),
        (
            include_bytes!("fixtures/granite-4.2-3b.json").as_slice(),
            "ibm.granite-4.2-3b",
            "standard",
        ),
        (
            include_bytes!("fixtures/gemma-3-1b.json").as_slice(),
            "google.gemma-3-1b",
            "lite",
        ),
    ];
    for (example, model_id, resource_class) in examples {
        let manifest =
            ModelManifest::parse_json(example).expect("manifest profile follows runtime schema");
        assert_eq!(manifest.model_id.as_str(), model_id);
        assert_eq!(
            manifest.runtime_class.as_ref().unwrap().as_str(),
            "generative_llm"
        );
        assert_eq!(
            manifest.resource_class.as_ref().unwrap().as_str(),
            resource_class
        );
    }

    let granite =
        ModelManifest::parse_json(include_bytes!("fixtures/granite-4.2-3b.json").as_slice())
            .expect("pinned Granite artifact profile");
    assert_eq!(granite.variant, "3b");
    assert_eq!(granite.artifact.size_bytes, Some(2_244_011_552));
    assert_eq!(
        granite.artifact.integrity.as_ref().unwrap().digest,
        "e0406663965846ae22a403456eb826ccce5f450840491f71952f18a7cb78e7d5"
    );
    assert_eq!(granite.license.identifier, "Apache-2.0");
    let source = granite.source.as_ref().unwrap();
    assert_eq!(source.revision, "c40945d71cd90f249a56985e8155551a9188dc30");
    assert_eq!(source.file_name, "granite-4.2-3b-Q4_K_M.gguf");
}
