use nagi_legal::diff::compare_inventories;
use nagi_legal::model::{EvidenceRecord, Finding, Inventory, ModelLicenseRecord};
use nagi_legal::report::{render_notice_candidate, render_scan_report};
use nagi_legal::sbom::{build_spdx_document, validate_spdx_document};
use nagi_legal::scan::scan_repository;
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static NEXT_FIXTURE_ID: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos();
        let fixture_id = NEXT_FIXTURE_ID.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "nagi-legal-fixture-{}-{stamp}-{fixture_id}",
            std::process::id()
        ));
        fs::create_dir_all(&root).expect("create fixture root");
        let fixture = Self { root };
        fixture.write(
            "Cargo.toml",
            r#"[package]
name = "fixture-app"
version = "0.1.0"
license = "MIT"

[dependencies]
known-crate = "1"
"#,
        );
        fixture.write(
            "Cargo.lock",
            r#"version = 4

[[package]]
name = "fixture-app"
version = "0.1.0"
dependencies = ["known-crate"]

[[package]]
name = "known-crate"
version = "1.2.3"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
dependencies = ["transitive-crate 0.9.0"]

[[package]]
name = "transitive-crate"
version = "0.9.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
"#,
        );
        fixture.write(
            "third_party/sources.lock",
            r#"format_version = 1

[sources]

[sources.sample]
component = "sample-vendor"
repository = "https://example.org/sample-vendor.git"
revision = "0123456789abcdef0123456789abcdef01234567"
license = "Apache-2.0 OR MIT"
vendored_path = "third_party/sample-vendor"
"#,
        );
        fixture.write(
            "third_party/sample-vendor/LICENSE",
            "UPSTREAM_COPYRIGHT_TEXT_SHOULD_NOT_BE_COPIED\n",
        );
        fixture.write(
            "THIRD_PARTY_NOTICES.md",
            "| Component | Inclusion | Pin | License | Boundary | Review |\n| --- | --- | --- | --- | --- | --- |\n| sample-vendor | A | pinned | Apache-2.0 OR MIT | source | review upstream notice |\n",
        );
        fixture.write(
            "docs/legal/model-assets.json",
            r#"{
  "schema_version": 1,
  "assets": [{
    "model_id": "fixture.model",
    "display_name": "Fixture Model",
    "provider_id": "fixture",
    "version": "1",
    "variant": "small",
    "inclusion_status": "fixture-only",
    "artifact_present": false,
    "opaque_license_identifier": "provider-terms:fixture",
    "terms_reference": "provider-terms:fixture.model",
    "acknowledgement_required": null,
    "notices": [{"notice_id": "model-notice", "reference": "provider-notice:fixture", "required": true}],
    "declared_license_expression": null,
    "review_status": "manual_review_required",
    "evidence_source": "tests/fixtures/model.json"
  }]
}"#,
        );
        fixture
    }

    fn write(&self, relative: &str, contents: &str) {
        let path = self.root.join(relative);
        fs::create_dir_all(path.parent().expect("parent directory"))
            .expect("create fixture subdirectory");
        fs::write(path, contents).expect("write fixture file");
    }

    fn scan(&self) -> Inventory {
        scan_repository(&self.root).expect("fixture scans offline")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn find_component<'a>(
    inventory: &'a Inventory,
    name: &str,
) -> &'a nagi_legal::model::ComponentRecord {
    inventory
        .components
        .iter()
        .find(|component| component.name == name)
        .unwrap_or_else(|| panic!("component {name} is present"))
}

#[test]
fn offline_fixture_scan_parses_lock_metadata_and_sorts_components() {
    let fixture = Fixture::new();
    let first = fixture.scan();
    let second = fixture.scan();
    assert_eq!(first, second);
    assert!(first
        .components
        .windows(2)
        .all(|pair| pair[0].component_id <= pair[1].component_id));

    let known = find_component(&first, "known-crate");
    assert_eq!(known.version.as_deref(), Some("1.2.3"));
    assert_eq!(known.directness, "direct");
    assert_eq!(known.scopes, vec!["runtime"]);
    assert_eq!(
        known.checksum.as_deref(),
        Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa")
    );
    assert_eq!(known.purl.as_deref(), Some("pkg:cargo/known-crate@1.2.3"));
    assert_eq!(known.dependencies.len(), 1);
    assert_eq!(
        find_component(&first, "transitive-crate").directness,
        "transitive"
    );
    assert_eq!(first.model_assets.len(), 1);
    assert_eq!(first.model_assets[0].inclusion_status, "fixture-only");
}

#[test]
fn explicit_license_file_does_not_guess_an_spdx_identifier() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"fixture-app\"\nversion = \"0.1.0\"\nlicense-file = \"LICENSE\"\n",
    );
    fixture.write("LICENSE", "fixture license text\n");
    let inventory = fixture.scan();
    let app = find_component(&inventory, "fixture-app");
    assert_eq!(app.declared_license_expression, None);
    assert_eq!(app.license_status, "unknown");
    assert_eq!(app.license_file_declared.as_deref(), Some("LICENSE"));
    assert!(app
        .detected_license_files
        .iter()
        .any(|path| path == "LICENSE"));
}

#[test]
fn unknown_license_stays_unknown_and_is_reported_for_review() {
    let fixture = Fixture::new();
    let inventory = fixture.scan();
    let component = find_component(&inventory, "known-crate");
    assert_eq!(component.license_status, "unknown");
    assert_eq!(component.declared_license_expression, None);
    assert!(inventory
        .findings
        .iter()
        .any(|finding| finding.code == "UNKNOWN_LICENSE"
            && finding.component_id.as_deref() == Some(component.component_id.as_str())));
    assert!(render_scan_report(&inventory).contains("Components requiring manual review"));
}

#[test]
fn generated_spdx_is_deterministic_except_for_creation_time_and_has_valid_shape() {
    let fixture = Fixture::new();
    let inventory = fixture.scan();
    let created = "2026-09-28T00:00:00Z";
    let first = build_spdx_document(&inventory, created).expect("SPDX document builds");
    let second = build_spdx_document(&inventory, created).expect("SPDX document builds twice");
    assert_eq!(first, second);
    assert_eq!(first["spdxVersion"], "SPDX-2.3");
    assert_eq!(first["creationInfo"]["licenseListVersion"], "3.29.0");
    validate_spdx_document(&first).expect("generated SPDX profile is valid");
    let later = build_spdx_document(&inventory, "2026-09-29T00:00:00Z")
        .expect("document builds at a later creation time");
    assert_eq!(first["documentNamespace"], later["documentNamespace"]);
    let mut volatile = first.clone();
    volatile["creationInfo"]["created"] = serde_json::json!("2026-09-29T00:00:00Z");
    assert_ne!(first, volatile);
    assert!(first["packages"]
        .as_array()
        .expect("packages array")
        .iter()
        .all(|package| package["name"] != "Fixture Model"));

    let mut changed_project_metadata = inventory.clone();
    changed_project_metadata.project.declared_license_expression = Some("Apache-2.0".to_owned());
    let changed = build_spdx_document(&changed_project_metadata, created)
        .expect("document with changed project metadata builds");
    assert_ne!(first["documentNamespace"], changed["documentNamespace"]);

    let mut changed_evidence = inventory;
    changed_evidence.components[0]
        .evidence
        .push(EvidenceRecord {
            kind: "review-record".to_owned(),
            source: "docs/legal/new-evidence.md".to_owned(),
            detail: None,
        });
    let changed = build_spdx_document(&changed_evidence, created)
        .expect("document with changed component evidence builds");
    assert_ne!(first["documentNamespace"], changed["documentNamespace"]);
}

#[test]
fn sbom_and_notice_outputs_exclude_host_paths_credentials_and_upstream_text() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"fixture-app\"\nversion = \"0.1.0\"\nlicense-file = \"/Users/alice/private/LICENSE\"\n",
    );
    fixture.write(
        "Cargo.lock",
        r#"version = 4

[[package]]
name = "fixture-app"
version = "0.1.0"

[[package]]
name = "private-git-crate"
version = "2.0.0"
source = "git+https://alice:topsecret@example.invalid/private.git#abcdef0123456789"
"#,
    );
    let inventory = fixture.scan();
    let spdx =
        build_spdx_document(&inventory, "2026-09-28T00:00:00Z").expect("SPDX document builds");
    let serialized = serde_json::to_string(&spdx).expect("serialize SPDX");
    let notice = render_notice_candidate(&inventory);
    for private in ["/Users/alice", "topsecret", "alice:"] {
        assert!(!serialized.contains(private));
        assert!(!notice.contains(private));
    }
    assert!(!notice.contains("UPSTREAM_COPYRIGHT_TEXT_SHOULD_NOT_BE_COPIED"));
    assert!(notice.contains("third_party/sample-vendor/LICENSE"));
    assert!(notice.contains("Manual review required"));
}

#[test]
fn vendored_license_discovery_is_bounded_around_build_and_cache_trees() {
    let fixture = Fixture::new();
    fixture.write(
        "target/fake/Cargo.lock",
        "[[package]]\nname=\"poison-target\"\nversion=\"9\"\n",
    );
    fixture.write(
        "out/Cargo.lock",
        "[[package]]\nname=\"poison-out\"\nversion=\"9\"\n",
    );
    fixture.write(
        ".cargo/registry/Cargo.lock",
        "[[package]]\nname=\"poison-cache\"\nversion=\"9\"\n",
    );
    fixture.write(
        "third_party/servo/Cargo.lock",
        "[[package]]\nname=\"poison-servo-cache\"\nversion=\"9\"\n",
    );
    let inventory = fixture.scan();
    for name in [
        "poison-target",
        "poison-out",
        "poison-cache",
        "poison-servo-cache",
    ] {
        assert!(!inventory
            .components
            .iter()
            .any(|component| component.name == name));
    }
    let vendor = find_component(&inventory, "sample-vendor");
    assert!(vendor
        .detected_license_files
        .contains(&"third_party/sample-vendor/LICENSE".to_owned()));
}

#[test]
fn conflicting_duplicate_identity_is_detected() {
    let fixture = Fixture::new();
    fixture.write(
        "third_party/sources.lock",
        r#"format_version = 1
[sources]
[sources.one]
component = "shared"
repository = "https://crates.io/crates/shared/1.0.0"
version = "1.0.0"
license = "MIT"
vendored_path = "third_party/shared-one"
[sources.two]
component = "shared-alias"
repository = "https://crates.io/crates/shared/1.0.0"
version = "1.0.0"
license = "Apache-2.0"
vendored_path = "third_party/shared-two"
"#,
    );
    let inventory = fixture.scan();
    let shared = inventory
        .components
        .iter()
        .find(|component| component.purl.as_deref() == Some("pkg:cargo/shared@1.0.0"))
        .expect("duplicate package identity exists");
    assert_eq!(shared.license_status, "conflict");
    assert!(!shared.conflicts.is_empty());
    assert!(inventory
        .findings
        .iter()
        .any(|finding| finding.code == "CONFLICTING_COMPONENT_IDENTITY"));
}

#[test]
fn vendored_component_without_evidence_or_review_record_fails_check_policy() {
    let fixture = Fixture::new();
    fixture.write(
        "third_party/sources.lock",
        r#"format_version = 1
[sources]
[sources.unreviewed]
component = "unreviewed-vendor"
repository = "https://example.org/unreviewed.git"
revision = "0123456789abcdef"
vendored_path = "third_party/unreviewed"
"#,
    );
    fixture.write("THIRD_PARTY_NOTICES.md", "# no component review rows\n");
    let inventory = fixture.scan();
    assert!(inventory.findings.iter().any(|finding| {
        finding.code == "VENDORED_LICENSE_EVIDENCE_MISSING"
            && finding.component_id.as_deref()
                == Some("vcs:https://example.org/unreviewed.git#0123456789abcdef:unreviewedvendor")
    }));
}

#[test]
fn notice_candidate_distinguishes_metadata_evidence_from_review_work() {
    let fixture = Fixture::new();
    let inventory = fixture.scan();
    let notice = render_notice_candidate(&inventory);
    assert!(notice.contains("## Generated project metadata"));
    assert!(notice.contains("## Detected source text and reference locations"));
    assert!(notice.contains("## Model terms and notice references"));
    assert!(notice.contains("provider-terms:fixture.model"));
    assert!(notice.contains("fixture-only"));
    assert!(notice.contains("manual review"));
}

#[test]
fn inventory_diff_reports_version_license_and_new_evidence_changes() {
    let fixture = Fixture::new();
    let before = fixture.scan();
    let mut after = before.clone();
    let package = after
        .components
        .iter_mut()
        .find(|component| component.name == "known-crate")
        .expect("known package present");
    let old_id = package.component_id.clone();
    package.component_id = "pkg:cargo/known-crate@1.3.0".to_owned();
    package.purl = Some(package.component_id.clone());
    package.version = Some("1.3.0".to_owned());
    package.license_status = "declared".to_owned();
    package.declared_license_expression = Some("MIT".to_owned());
    package
        .detected_license_files
        .push("LICENSE-MIT".to_owned());
    let diff = compare_inventories(&before, &after);
    assert!(diff.version_changes.iter().any(|change| {
        change.name == "known-crate" && change.from == "1.2.3" && change.to == "1.3.0"
    }));
    assert!(diff
        .license_changes
        .iter()
        .any(|change| change.name == "known-crate"));
    assert!(!diff
        .new_missing_license_conditions
        .iter()
        .any(|component| component.component_id == old_id));
}

#[test]
fn model_record_schema_and_unknown_terms_are_preserved() {
    let fixture = Fixture::new();
    let inventory = fixture.scan();
    let record: &ModelLicenseRecord = &inventory.model_assets[0];
    assert_eq!(record.declared_license_expression, None);
    assert_eq!(
        record.opaque_license_identifier.as_deref(),
        Some("provider-terms:fixture")
    );
    assert_eq!(
        record.terms_reference.as_deref(),
        Some("provider-terms:fixture.model")
    );
    assert!(record.notices[0].required);
    let schema: serde_json::Value = serde_json::from_str(include_str!(
        "../../../docs/legal/model-license-record.schema.json"
    ))
    .expect("model schema JSON parses");
    assert_eq!(schema["title"], "Nagi model license evidence record v1");
}

#[test]
fn unresolved_or_ambiguous_lock_edges_fail_structural_check() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        "[package]\nname = \"fixture-app\"\nversion = \"0.1.0\"\nlicense = \"MIT\"\n",
    );
    fixture.write(
        "Cargo.lock",
        r#"version = 4

[[package]]
name = "fixture-app"
version = "0.1.0"
dependencies = ["bad dependency string extra", "missing-crate 1.0.0", "duplicate"]

[[package]]
name = "duplicate"
version = "1.0.0"
source = "git+https://example.org/first.git#0123456789abcdef"

[[package]]
name = "duplicate"
version = "1.0.0"
source = "git+https://example.org/second.git#abcdef0123456789"
"#,
    );
    let inventory = fixture.scan();
    for code in [
        "UNPARSEABLE_CARGO_DEPENDENCY",
        "UNRESOLVED_CARGO_DEPENDENCY",
        "AMBIGUOUS_CARGO_DEPENDENCY",
    ] {
        assert!(inventory
            .findings
            .iter()
            .any(|finding| finding.code == code && finding.severity == "error"));
    }
    assert_eq!(
        nagi_legal::cli::run_from(&["check".to_owned()], &fixture.root),
        1
    );
}

#[test]
fn notice_and_copyright_files_do_not_count_as_license_evidence() {
    let fixture = Fixture::new();
    fixture.write(
        "third_party/sources.lock",
        r#"format_version = 1
[sources]
[sources.notice_only]
component = "notice-only"
repository = "https://example.org/notice-only.git"
revision = "0123456789abcdef0123456789abcdef01234567"
vendored_path = "third_party/notice-only"
"#,
    );
    fixture.write("third_party/notice-only/NOTICE", "attribution only\n");
    fixture.write("third_party/notice-only/COPYRIGHT", "copyright only\n");
    fixture.write("THIRD_PARTY_NOTICES.md", "# no component review rows\n");
    let inventory = fixture.scan();
    let component = find_component(&inventory, "notice-only");
    assert!(component.detected_license_files.is_empty());
    assert!(component
        .notice_files
        .contains(&"third_party/notice-only/NOTICE".to_owned()));
    assert!(inventory.findings.iter().any(|finding| {
        finding.code == "VENDORED_LICENSE_EVIDENCE_MISSING"
            && finding.component_id.as_deref() == Some(component.component_id.as_str())
    }));
}

#[test]
fn nested_vendored_package_inherits_its_tracked_parent_license_and_review_row() {
    let fixture = Fixture::new();
    fixture.write(
        "third_party/sources.lock",
        r#"format_version = 1
[sources]
[sources.relibc]
component = "relibc"
repository = "https://example.org/relibc.git"
revision = "0123456789abcdef0123456789abcdef01234567"
vendored_path = "third_party/relibc"
"#,
    );
    fixture.write(
        "THIRD_PARTY_NOTICES.md",
        "| Component | Inclusion | Pin | License | Boundary | Review |\n| --- | --- | --- | --- | --- | --- |\n| relibc | A | pinned | unknown | source | manual review |\n",
    );
    fixture.write(
        "third_party/relibc/LICENSE",
        "license text reference only\n",
    );
    fixture.write(
        "third_party/relibc/tests/Cargo.toml",
        "[package]\nname = \"relibc-tests\"\nversion = \"0.1.0\"\n",
    );
    fixture.write(
        "third_party/relibc/tests/Cargo.lock",
        "version = 4\n\n[[package]]\nname = \"relibc-tests\"\nversion = \"0.1.0\"\n",
    );
    let inventory = fixture.scan();
    let nested = find_component(&inventory, "relibc-tests");
    assert!(nested
        .detected_license_files
        .contains(&"third_party/relibc/LICENSE".to_owned()));
    assert!(nested.review_recorded);
    assert!(!inventory.findings.iter().any(|finding| {
        finding.code == "VENDORED_LICENSE_EVIDENCE_MISSING"
            && finding.component_id.as_deref() == Some(nested.component_id.as_str())
    }));
}

#[test]
fn source_pin_identity_and_malformed_license_claims_are_reported() {
    let fixture = Fixture::new();
    fixture.write(
        "third_party/sources.lock",
        r#"format_version = 1
[sources]
[sources.no_identity]
component = ""
license = "MIT"
[sources.bad_license]
component = "bad-license"
version = "1.0.0"
license = "MIT OR"
"#,
    );
    let inventory = fixture.scan();
    assert!(inventory
        .findings
        .iter()
        .any(|finding| finding.code == "SOURCE_PIN_IDENTITY_UNRESOLVED"));
    assert!(inventory.findings.iter().any(|finding| {
        finding.code == "MALFORMED_LICENSE_EXPRESSION"
            && finding.component_id.as_deref() == Some("source:badlicense@1.0.0")
    }));
}

#[test]
fn model_evidence_is_repository_relative_and_model_license_claims_are_validated() {
    let fixture = Fixture::new();
    fixture.write(
        "docs/legal/model-assets.json",
        r#"{
  "schema_version": 1,
  "assets": [{
    "model_id": "fixture.model",
    "display_name": "Fixture Model",
    "provider_id": "fixture",
    "version": "1",
    "variant": "small",
    "inclusion_status": "fixture-only",
    "artifact_present": false,
    "opaque_license_identifier": "provider-terms:fixture",
    "terms_reference": "https://alice:topsecret@example.org/terms?token=topsecret",
    "acknowledgement_required": null,
    "notices": [{"notice_id": "model-notice", "reference": "provider-notice:fixture", "required": true}],
    "declared_license_expression": "DefinitelyNotAnSpdxId",
    "review_status": "manual_review_required",
    "evidence_source": "/Users/alice/private/model.json"
  }]
}"#,
    );
    let inventory = fixture.scan();
    let record = &inventory.model_assets[0];
    assert_eq!(record.evidence_source, "docs/legal/model-assets.json");
    assert_eq!(record.declared_license_expression, None);
    assert_eq!(
        record.raw_license_metadata.as_deref(),
        Some("DefinitelyNotAnSpdxId")
    );
    assert!(inventory
        .findings
        .iter()
        .any(|finding| finding.code == "UNSAFE_MODEL_EVIDENCE_PATH"));
    assert!(inventory
        .findings
        .iter()
        .any(|finding| finding.code == "MODEL_LICENSE_EXPRESSION_UNKNOWN"
            && finding.severity == "review"));
    let notice = render_notice_candidate(&inventory);
    assert!(!notice.contains("/Users/alice"));
    assert!(!notice.contains("topsecret"));
}

#[test]
fn human_report_groups_repetitive_findings_and_bounds_examples() {
    let fixture = Fixture::new();
    let mut inventory = fixture.scan();
    for index in 0..30 {
        inventory.findings.push(Finding {
            code: "UNKNOWN_LICENSE".to_owned(),
            severity: "review".to_owned(),
            component_id: Some(format!("fixture-{index:02}")),
            message: "no explicit SPDX license expression was resolved; inspect recorded evidence"
                .to_owned(),
            evidence: Vec::new(),
        });
    }
    let report = render_scan_report(&inventory);
    assert!(report.contains("32 occurrence(s)"));
    assert!(report.contains("+29 more in JSON inventory"));
    assert!(report.len() < 2_500, "report is {} bytes", report.len());
}
