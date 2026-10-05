use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum M26Model {
    Qwen,
    Gemma,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ModelArtifactPin {
    pub(crate) model_id: &'static str,
    pub(crate) repository: &'static str,
    pub(crate) revision: &'static str,
    pub(crate) file_name: &'static str,
    pub(crate) format: &'static str,
    pub(crate) size_bytes: u64,
    pub(crate) sha256: &'static str,
    pub(crate) license: &'static str,
    pub(crate) license_reference: &'static str,
    pub(crate) notice_id: &'static str,
    pub(crate) notice_reference: &'static str,
    pub(crate) acknowledgement_required: bool,
    pub(crate) artifact_id: &'static str,
    pub(crate) storage: &'static str,
}

impl M26Model {
    fn lock_section(self) -> &'static str {
        match self {
            Self::Qwen => "models.qwen3_4b",
            Self::Gemma => "models.gemma_3_1b",
        }
    }

    fn fixture_path(self) -> &'static str {
        match self {
            Self::Qwen => "user/nagi-model-manager/tests/fixtures/qwen3-4b.json",
            Self::Gemma => "user/nagi-model-manager/tests/fixtures/gemma-3-1b.json",
        }
    }

    fn reviewed_pin(self) -> ModelArtifactPin {
        match self {
            Self::Qwen => ModelArtifactPin {
                model_id: "qwen.qwen3-4b",
                repository: "https://huggingface.co/Qwen/Qwen3-4B-GGUF",
                revision: "bc640142c66e1fdd12af0bd68f40445458f3869b",
                file_name: "Qwen3-4B-Q4_K_M.gguf",
                format: "gguf",
                size_bytes: 2_497_280_256,
                sha256: "7485fe6f11af29433bc51cab58009521f205840f5b4ae3a32fa7f92e8534fdf5",
                license: "Apache-2.0",
                license_reference: "https://huggingface.co/Qwen/Qwen3-4B-GGUF/blob/bc640142c66e1fdd12af0bd68f40445458f3869b/LICENSE",
                notice_id: "apache-2.0",
                notice_reference: "https://www.apache.org/licenses/LICENSE-2.0",
                acknowledgement_required: false,
                artifact_id: "qwen.qwen3-4b",
                storage: "model_store",
            },
            Self::Gemma => ModelArtifactPin {
                model_id: "google.gemma-3-1b",
                repository: "https://huggingface.co/ggml-org/gemma-3-1b-it-GGUF",
                revision: "f9c28bcd85737ffc5aef028638d3341d49869c27",
                file_name: "gemma-3-1b-it-Q4_K_M.gguf",
                format: "gguf",
                size_bytes: 806_058_240,
                sha256: "8ccc5cd1f1b3602548715ae25a66ed73fd5dc68a210412eea643eb20eb75a135",
                license: "Gemma Terms of Use",
                license_reference: "https://ai.google.dev/gemma/terms",
                notice_id: "gemma-terms-of-use",
                notice_reference: "https://ai.google.dev/gemma/terms",
                acknowledgement_required: true,
                artifact_id: "google.gemma-3-1b",
                storage: "model_store",
            },
        }
    }
}

/// Validates the checked-in M26 source lock against a separately reviewed
/// immutable pin. Artifact digests are never accepted from the manifest alone.
pub(crate) fn validate_m26_model_lock(
    root: &Path,
    model: M26Model,
) -> Result<ModelArtifactPin, String> {
    let lock_path = root.join("third_party/models.lock");
    let lock = fs::read_to_string(&lock_path)
        .map_err(|error| format!("cannot read {}: {error}", lock_path.display()))?;
    let section = model.lock_section();
    let reviewed = model.reviewed_pin();
    let required = |key: &str| {
        crate::llama_cpp::lock_value(&lock, section, key)
            .ok_or_else(|| format!("third_party/models.lock is missing {section} field `{key}`"))
    };
    let size_bytes = required("size_bytes")?
        .parse::<u64>()
        .map_err(|_| format!("{section} size_bytes must be an unsigned integer"))?;
    let acknowledgement_required = match required("acknowledgement_required")?.as_str() {
        "true" => true,
        "false" => false,
        _ => {
            return Err(format!(
                "{section} acknowledgement_required must be true or false"
            ))
        }
    };
    let expected_fields = [
        ("model_id", reviewed.model_id),
        ("repository", reviewed.repository),
        ("revision", reviewed.revision),
        ("file_name", reviewed.file_name),
        ("format", reviewed.format),
        ("sha256", reviewed.sha256),
        ("license", reviewed.license),
        ("license_reference", reviewed.license_reference),
        ("notice_id", reviewed.notice_id),
        ("notice_reference", reviewed.notice_reference),
        ("artifact_id", reviewed.artifact_id),
        ("storage", reviewed.storage),
    ];
    for (key, expected) in expected_fields {
        if required(key)? != expected {
            return Err(format!(
                "third_party/models.lock {section} field `{key}` does not match the reviewed M26 artifact pin"
            ));
        }
    }
    if size_bytes != reviewed.size_bytes
        || acknowledgement_required != reviewed.acknowledgement_required
    {
        return Err(format!(
            "third_party/models.lock {section} size or acknowledgement metadata does not match the reviewed M26 artifact pin"
        ));
    }
    if reviewed.sha256.len() != 64
        || !reviewed
            .sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(format!(
            "{section} sha256 is not a lowercase SHA-256 digest"
        ));
    }
    Ok(reviewed)
}

/// Checks that the installable manifest fixture carries the same artifact and
/// license metadata as the independently reviewed model lock.
pub(crate) fn validate_m26_model_manifest(
    root: &Path,
    model: M26Model,
    pin: ModelArtifactPin,
) -> Result<(), String> {
    let manifest_path = root.join(model.fixture_path());
    let bytes = fs::read(&manifest_path)
        .map_err(|error| format!("cannot read {}: {error}", manifest_path.display()))?;
    let manifest = nagi_model_manager::ModelManifest::parse_json(&bytes)
        .map_err(|error| format!("{} is invalid: {error}", manifest_path.display()))?;
    let source = manifest
        .source
        .as_ref()
        .ok_or_else(|| "M26 manifest is missing source metadata".to_owned())?;
    let integrity = manifest
        .artifact
        .integrity
        .as_ref()
        .ok_or_else(|| "M26 manifest is missing artifact integrity metadata".to_owned())?;
    let artifact_id = match &manifest.artifact.reference {
        nagi_model_manager::ArtifactReference::ModelStore { artifact_id } => artifact_id.as_str(),
    };
    let has_notice = manifest.license.notices.iter().any(|notice| {
        notice.notice_id == pin.notice_id
            && notice.reference == pin.notice_reference
            && notice.required
    });
    if manifest.model_id.as_str() != pin.model_id
        || artifact_id != pin.artifact_id
        || source.uri != pin.repository
        || source.revision != pin.revision
        || source.file_name != pin.file_name
        || manifest.artifact.format.as_str() != pin.format
        || manifest.artifact.size_bytes != Some(pin.size_bytes)
        || integrity.algorithm != "sha256"
        || integrity.digest != pin.sha256
        || manifest.license.identifier != pin.license
        || manifest.license.terms_reference.as_deref() != Some(pin.license_reference)
        || manifest.license.acknowledgement_required != pin.acknowledgement_required
        || !has_notice
        || pin.storage != "model_store"
    {
        return Err(format!(
            "{} does not match the reviewed M26 source, artifact, or license metadata",
            manifest_path.display()
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{validate_m26_model_lock, validate_m26_model_manifest, M26Model};
    use std::path::Path;

    #[test]
    fn qwen_and_gemma_locks_match_the_reviewed_pins_and_manifests() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for model in [M26Model::Qwen, M26Model::Gemma] {
            let pin = validate_m26_model_lock(&root, model).expect("reviewed M26 model lock");
            validate_m26_model_manifest(&root, model, pin)
                .expect("M26 model manifest matches its lock");
        }
    }
}
