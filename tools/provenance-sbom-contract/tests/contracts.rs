use nagi_cli::development::execute_fingerprint_cli;
use nagi_legal::sbom::{build_spdx_document, validate_spdx_document};
use nagi_legal::scan::scan_repository;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

// Independent wire fixture, deliberately not produced by the validator under test.
// Key order follows schema v1's canonical typed input serialization.
const INPUTS: &str = r#"{"repository_id":"example.invalid/nagi","source":{"commit_sha":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","dirty":false,"dirty_tree_digest":null},"toolchain":{"rustc":{"version":"rustc fixture"},"cargo":"cargo fixture","host_triple":"aarch64-apple-darwin","native_tools":{}},"target_triple":"x86_64-unknown-nagi","enabled_features":[],"build_profile":null,"build_command_digest":null,"build_flags":[],"environment_inputs":{},"third_party_revisions":{},"submodule_revisions":{},"nagi_patch_digest":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","build_configuration_digest":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","generated_inputs":[],"runtime_harness":{"qemu_version":null,"firmware_inputs":[]}}"#;

static NEXT: AtomicU64 = AtomicU64::new(0);
fn fixture() -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "nagi-provenance-sbom-contract-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&root).unwrap();
    root
}
fn document() -> Value {
    json!({"schema_version":1,"compatibility":{
        "digest_algorithm":"sha256", "digest":format!("{:x}", Sha256::digest(INPUTS.as_bytes())),
        "inputs":serde_json::from_str::<Value>(INPUTS).unwrap()},
        "artifacts":[{"role":"image","path":"out/nagi.img","byte_length":3,"sha256":"d".repeat(64)}],
        "provenance":{"created_at_unix_seconds":1,"producer":"independent-contract-fixture","producer_version":"1"}})
}
fn compare(left: &Value, right: &Value) -> (i32, String) {
    let root = fixture();
    fs::write(root.join("left.json"), serde_json::to_vec(left).unwrap()).unwrap();
    fs::write(root.join("right.json"), serde_json::to_vec(right).unwrap()).unwrap();
    let result = execute_fingerprint_cli(
        &["compare", "left.json", "right.json", "--json"].map(str::to_owned),
        &root,
    );
    (result.exit_code, result.lines.join("\n"))
}
fn rejected(value: Value, diagnostic: &str) {
    let (code, output) = compare(&value, &value);
    assert_eq!(code, 4, "{output}");
    assert!(output.contains(diagnostic), "{output}");
}
#[test]
fn valid_wire_control_and_volatile_timestamp() {
    let left = document();
    let mut right = left.clone();
    right["provenance"]["created_at_unix_seconds"] = json!(2);
    let (code, output) = compare(&left, &right);
    assert_eq!(code, 0, "{output}");
    assert_eq!(
        serde_json::from_str::<Value>(&output).unwrap()["match"],
        true
    );
}
#[test]
fn missing_manifest_and_missing_required_field_fail_closed() {
    let root = fixture();
    let result = execute_fingerprint_cli(
        &["compare", "missing.json", "missing.json"].map(str::to_owned),
        &root,
    );
    assert_eq!(result.exit_code, 4);
    assert!(result.lines.join("\n").contains("cannot read fingerprint"));
    let mut value = document();
    value.as_object_mut().unwrap().remove("artifacts");
    rejected(value, "missing field `artifacts`");
}
#[test]
fn duplicate_json_field_fails_closed() {
    let root = fixture();
    let encoded = serde_json::to_string(&document()).unwrap();
    let encoded = encoded.replacen(
        "\"schema_version\":1",
        "\"schema_version\":1,\"schema_version\":1",
        1,
    );
    fs::write(root.join("duplicate.json"), encoded).unwrap();
    let result = execute_fingerprint_cli(
        &["compare", "duplicate.json", "duplicate.json"].map(str::to_owned),
        &root,
    );
    assert_eq!(result.exit_code, 4);
    assert!(result.lines.join("\n").contains("duplicate field"));
}
#[test]
fn duplicate_artifact_record_fails_closed() {
    let mut value = document();
    let row = value["artifacts"][0].clone();
    value["artifacts"].as_array_mut().unwrap().push(row);
    rejected(value, "duplicate artifact");
}
#[test]
fn artifact_manifest_paths_reject_traversal_and_absolute_names() {
    for path in [
        "../outside.img",
        "out/../outside.img",
        "/tmp/image",
        "C:\\image",
        "out\\..\\image",
        "",
        "out//image",
        "./image",
    ] {
        let mut value = document();
        value["artifacts"][0]["path"] = json!(path);
        rejected(value, "unsafe artifact path");
    }
}
#[test]
fn tampered_source_digest_cannot_compare_equal_to_itself() {
    let mut value = document();
    value["compatibility"]["inputs"]["source"]["commit_sha"] = json!("e".repeat(40));
    rejected(value, "does not match its normalized inputs");
}
#[test]
fn artifact_hash_and_size_mismatch_are_not_compatible() {
    let left = document();
    for (field, changed) in [("sha256", json!("e".repeat(64))), ("byte_length", json!(4))] {
        let mut right = left.clone();
        right["artifacts"][0][field] = changed;
        let (code, output) = compare(&left, &right);
        assert_eq!(code, 4, "{output}");
        let report: Value = serde_json::from_str(&output).unwrap();
        assert_eq!(report["inputs_compatible"], true);
        assert_eq!(report["artifact_inventories_match"], false);
        assert_eq!(report["match"], false);
    }
    let mut value = left;
    value["artifacts"][0]["sha256"] = json!("not-a-hash");
    rejected(value, "invalid SHA-256");
}
fn unknown_inventory() -> nagi_legal::model::Inventory {
    let root = fixture();
    fs::write(
        root.join("Cargo.toml"),
        "[package]\nname='fixture'\nversion='0.1.0'\n",
    )
    .unwrap();
    fs::write(
        root.join("Cargo.lock"),
        "version=4\n[[package]]\nname='fixture'\nversion='0.1.0'\n",
    )
    .unwrap();
    scan_repository(&root).unwrap()
}
#[test]
fn unknown_license_survives_inventory_sbom_and_roundtrip() {
    let inventory = unknown_inventory();
    let component = inventory
        .components
        .iter()
        .find(|c| c.name == "fixture")
        .unwrap();
    assert_eq!(component.license_status, "unknown");
    assert!(component.declared_license_expression.is_none());
    assert!(inventory
        .findings
        .iter()
        .any(|f| f.code == "UNKNOWN_LICENSE"));
    let sbom = build_spdx_document(&inventory, "2026-10-23T00:00:00Z").unwrap();
    let package = sbom["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "fixture")
        .unwrap();
    assert_eq!(package["licenseDeclared"], "NOASSERTION");
    assert_eq!(package["licenseConcluded"], "NOASSERTION");
    let decoded = serde_json::from_slice(&serde_json::to_vec(&inventory).unwrap()).unwrap();
    assert_eq!(inventory, decoded);
}
#[test]
fn spdx_missing_and_duplicate_package_records_fail_closed() {
    let inventory = unknown_inventory();
    let sbom = build_spdx_document(&inventory, "2026-10-23T00:00:00Z").unwrap();
    validate_spdx_document(&sbom).unwrap();
    let mut missing = sbom.clone();
    missing.as_object_mut().unwrap().remove("packages");
    assert!(validate_spdx_document(&missing)
        .unwrap_err()
        .iter()
        .any(|e| e.contains("packages")));
    let mut duplicate = sbom;
    let row = duplicate["packages"][0].clone();
    duplicate["packages"].as_array_mut().unwrap().push(row);
    assert!(validate_spdx_document(&duplicate)
        .unwrap_err()
        .iter()
        .any(|e| e.contains("duplicate")));
}

fn with_inputs(inputs: &str) -> Value {
    let mut value = document();
    value["compatibility"]["digest"] = json!(format!("{:x}", Sha256::digest(inputs.as_bytes())));
    value["compatibility"]["inputs"] = serde_json::from_str(inputs).unwrap();
    value
}
#[test]
fn resealed_source_revision_change_is_still_a_mismatch() {
    let inputs = INPUTS.replace(&"a".repeat(40), &"e".repeat(40));
    let (code, output) = compare(&document(), &with_inputs(&inputs));
    assert_eq!(code, 4, "{output}");
    let report: Value = serde_json::from_str(&output).unwrap();
    assert_eq!(report["inputs_compatible"], false);
    assert!(report["mismatches"]
        .as_array()
        .unwrap()
        .iter()
        .any(|m| m["class"] == "source_revision"));
}
#[test]
fn generated_and_firmware_inventory_paths_and_duplicates_fail_closed() {
    for field in ["generated_inputs", "firmware_inputs"] {
        let record = |path: &str| {
            format!(
                r#"{{"kind":"file","path":{},"byte_length":3,"sha256":"{}"}}"#,
                serde_json::to_string(path).unwrap(),
                "d".repeat(64)
            )
        };
        let old = format!("\"{field}\":[]");
        let safe = record("out/input");
        let valid = with_inputs(&INPUTS.replace(&old, &format!("\"{field}\":[{safe}]")));
        assert_eq!(compare(&valid, &valid).0, 0);
        let bad = record("../outside");
        rejected(
            with_inputs(&INPUTS.replace(&old, &format!("\"{field}\":[{bad}]"))),
            "unsafe input path",
        );
        rejected(
            with_inputs(&INPUTS.replace(&old, &format!("\"{field}\":[{safe},{safe}]"))),
            "duplicate input",
        );
    }
}

#[test]
fn repository_root_directory_generate_read_compare_roundtrip() {
    let parent = fixture();
    let root = parent.join("repo");
    fs::create_dir(&root).unwrap();
    fs::create_dir(root.join("third_party")).unwrap();
    fs::write(
        root.join("README"),
        "small independent repository fixture\n",
    )
    .unwrap();
    fs::write(
        root.join("third_party/sources.lock"),
        "format_version = 1\n[sources]\n",
    )
    .unwrap();
    for args in [
        vec!["init", "--quiet"],
        vec!["add", "README", "third_party/sources.lock"],
        vec![
            "-c",
            "user.name=Contract Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "-m",
            "fixture",
        ],
    ] {
        let result = std::process::Command::new("git")
            .args(args)
            .current_dir(&root)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let output = parent.join("root-fingerprint.json");
    let result = execute_fingerprint_cli(
        &[
            "--input".into(),
            ".".into(),
            "--output".into(),
            output.to_str().unwrap().into(),
        ],
        &root,
    );
    assert_eq!(result.exit_code, 0, "{}", result.lines.join("\n"));
    let document: Value = serde_json::from_slice(&fs::read(&output).unwrap()).unwrap();
    let inputs = document["compatibility"]["inputs"]["generated_inputs"]
        .as_array()
        .unwrap();
    assert!(inputs
        .iter()
        .any(|input| input["kind"] == "directory" && input["path"] == ""));
    let result = execute_fingerprint_cli(
        &[
            "compare".into(),
            output.to_str().unwrap().into(),
            output.to_str().unwrap().into(),
            "--json".into(),
        ],
        &root,
    );
    assert_eq!(result.exit_code, 0, "{}", result.lines.join("\n"));
    assert_eq!(
        serde_json::from_str::<Value>(&result.lines.join("\n")).unwrap()["match"],
        true
    );
}

#[test]
fn empty_inventory_path_is_not_a_file_or_tool() {
    for field in ["generated_inputs", "firmware_inputs"] {
        for kind in ["file", "tool"] {
            let row = format!(
                r#"{{"kind":"{kind}","path":"","byte_length":0,"sha256":"{}"}}"#,
                "d".repeat(64)
            );
            let inputs =
                INPUTS.replace(&format!("\"{field}\":[]"), &format!("\"{field}\":[{row}]"));
            rejected(with_inputs(&inputs), "unsafe input path");
        }
    }
    // The existing artifact negative table includes the empty path as well.
}
