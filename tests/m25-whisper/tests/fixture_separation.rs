//! Source-level guards: fixture input and expected text stay out of the
//! production provider path, and the guest fixture check stays labelled as a
//! fixture check.

use std::fs;
use std::path::PathBuf;

fn repository_file(relative: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

const EMBEDDING_TOKENS: [&str; 4] = ["include_bytes!", "include_str!", "OUT_DIR", "env!("];

#[test]
fn shared_production_sources_embed_no_fixture_data() {
    for file in [
        "tools/whisper/provider_session.rs",
        "tools/whisper/provider_ffi.rs",
    ] {
        let source = repository_file(file);
        for token in EMBEDDING_TOKENS {
            assert!(!source.contains(token), "{file} must not use {token}");
        }
        assert!(
            !source.contains("EXPECTED"),
            "{file} must not reference expected text"
        );
    }
}

#[test]
fn guest_fixture_data_is_confined_to_fixture_module() {
    let source = repository_file("user/nagi-init/src/m25_whisper.rs");
    let fixture_start = source
        .find("mod fixture_acceptance {")
        .expect("fixture module present");
    let fixture_end = fixture_start
        + source[fixture_start..]
            .find("\n}\n")
            .expect("fixture module closes at top level");
    let (before, rest) = source.split_at(fixture_start);
    let after = &rest[fixture_end - fixture_start..];
    for production in [before, after] {
        for token in EMBEDDING_TOKENS {
            assert!(
                !production.contains(token),
                "production part of m25_whisper.rs uses {token}"
            );
        }
        assert!(!production.contains("EXPECTED_TEXT"));
        assert!(!production.contains("PCM_FIXTURE"));
    }
    let fixture = &source[fixture_start..fixture_end];
    assert!(fixture.contains("m25-whisper-input.pcm"));
    assert!(fixture.contains("m25-whisper-expected.txt"));
    assert!(before.contains("struct ModelStoreWhisperLoader"));
    assert!(before.contains("WhisperSession<ModelStoreWhisperLoader>"));
}

#[test]
fn unseen_eval_set_is_separate_from_fixture_and_licensed() {
    let manifest = repository_file("tests/m25-whisper/eval/fleurs-ja-validation.json");
    assert!(manifest.contains("\"license\": \"CC-BY-4.0\""));
    assert!(manifest.contains("arXiv:2205.12446"));
    assert!(manifest.contains("\"revision\": \"70bb2e84b976b7e960aa89f1c648e09c59f894dd\""));
    assert_eq!(manifest.matches("\"pcm_s16le_sha256\"").count(), 12);
    // The guest acceptance fixture is 23,605 samples (adapter static_assert);
    // no evaluation clip has that length.
    assert!(!manifest.contains("\"samples\": 23605,"));
}
