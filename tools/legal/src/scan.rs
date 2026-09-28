use crate::license::{assess_license_expression, LicenseAssessment};
use crate::model::{
    ComponentRecord, EvidenceRecord, Finding, Inventory, ModelLicenseRecord, ProjectRecord,
};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs::{self, File};
use std::io::Read;
use std::path::{Component as PathComponent, Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};

const MAX_INDEX_PATHS: usize = 100_000;
const MAX_FALLBACK_ENTRIES: usize = 30_000;
const MAX_FALLBACK_DEPTH: usize = 8;
const MAX_COMPONENT_FILES: usize = 256;
const MAX_METADATA_BYTES: usize = 8 * 1024 * 1024;

#[derive(Deserialize, Default)]
struct CargoLock {
    #[serde(default)]
    package: Vec<LockPackage>,
}

#[derive(Clone, Deserialize)]
struct LockPackage {
    name: String,
    version: String,
    source: Option<String>,
    checksum: Option<String>,
    #[serde(default)]
    dependencies: Vec<String>,
}

#[derive(Deserialize, Default)]
struct CargoMetadata {
    #[serde(default)]
    packages: Vec<MetadataPackage>,
}

#[derive(Clone, Deserialize)]
struct MetadataPackage {
    name: String,
    version: String,
    manifest_path: String,
    license: Option<String>,
    license_file: Option<String>,
    repository: Option<String>,
    #[serde(default)]
    dependencies: Vec<MetadataDependency>,
}

#[derive(Clone, Deserialize)]
struct MetadataDependency {
    name: String,
    kind: Option<String>,
}

#[derive(Clone)]
struct ManifestMetadata {
    name: String,
    version: String,
    license: Option<String>,
    license_file: Option<String>,
    repository: Option<String>,
    relative_path: String,
    dependencies: BTreeMap<String, BTreeSet<String>>,
}

#[derive(Clone)]
struct SourcePin {
    component: String,
    version: Option<String>,
    revision: Option<String>,
    repository: Option<String>,
    source_hash: Option<String>,
    license: Option<String>,
    vendored_path: Option<String>,
    unsafe_vendored_path: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct LockKey {
    name: String,
    version: String,
    source: String,
}

#[derive(Clone)]
struct DependencyKey {
    name: String,
    version: Option<String>,
    source: Option<String>,
}

struct CargoComponentInput<'a> {
    package: &'a LockPackage,
    lock_path: &'a str,
    manifest: Option<&'a ManifestMetadata>,
    purl: Option<String>,
    component_id: &'a str,
}

struct ComponentEvidenceContext<'a> {
    evidence_paths: &'a [String],
    tracked_paths: &'a [String],
    notice_rows: &'a BTreeSet<String>,
    direct_scopes: &'a BTreeMap<String, BTreeSet<String>>,
}

pub fn scan_repository(root: &Path) -> Result<Inventory, String> {
    let root = root
        .canonicalize()
        .map_err(|error| format!("cannot resolve scan root: {error}"))?;
    if !root.is_dir() {
        return Err("scan root is not a directory".to_owned());
    }

    let paths = candidate_paths(&root)?;
    let evidence_paths: Vec<String> = paths
        .iter()
        .filter(|path| is_evidence_reference_path(path))
        .cloned()
        .collect();
    let notice_text = read_optional_text(&root.join("THIRD_PARTY_NOTICES.md"))?;
    let notice_rows = parse_notice_rows(notice_text.as_deref().unwrap_or_default());
    let mut findings = Vec::new();
    let mut components = BTreeMap::<String, ComponentRecord>::new();
    let mut source_pins = Vec::new();

    if let Some(contents) = read_optional_text(&root.join("third_party/sources.lock"))? {
        match parse_source_pins(&contents) {
            Ok(pins) => {
                for pin in &pins {
                    if pin.unsafe_vendored_path {
                        findings.push(Finding {
                            code: "UNSAFE_SOURCE_PATH".to_owned(),
                            severity: "error".to_owned(),
                            component_id: Some(pin.component.clone()),
                            message: "source-lock vendored_path is not a repository-relative path"
                                .to_owned(),
                            evidence: vec!["third_party/sources.lock".to_owned()],
                        });
                    }
                }
                source_pins = pins;
            }
            Err(_) => findings.push(Finding {
                code: "SOURCE_LOCK_UNREADABLE".to_owned(),
                severity: "error".to_owned(),
                component_id: None,
                message: "third_party/sources.lock is not valid source-lock TOML".to_owned(),
                evidence: vec!["third_party/sources.lock".to_owned()],
            }),
        }
    }

    add_source_pins(
        &source_pins,
        &evidence_paths,
        &notice_rows,
        &mut findings,
        &mut components,
    );

    let metadata_result = cargo_metadata(&root);
    let metadata_packages = match metadata_result {
        Ok(packages) => packages,
        Err(_) => {
            findings.push(Finding {
                code: "CARGO_METADATA_UNAVAILABLE".to_owned(),
                severity: "review".to_owned(),
                component_id: None,
                message: "stable cargo metadata was unavailable; lockfile identities are retained and unavailable license fields remain unknown".to_owned(),
                evidence: vec!["cargo metadata --no-deps --format-version 1 --locked --offline".to_owned()],
            });
            Vec::new()
        }
    };

    let manifest_metadata = collect_manifest_metadata(&root, &paths);
    let metadata_by_manifest: HashMap<String, MetadataPackage> = metadata_packages
        .iter()
        .filter_map(|package| {
            repo_relative_path(&root, Path::new(&package.manifest_path))
                .map(|path| (path, package.clone()))
        })
        .collect();
    let mut direct_scopes = BTreeMap::<String, BTreeSet<String>>::new();
    for manifest in &manifest_metadata {
        for (dependency, scopes) in &manifest.dependencies {
            direct_scopes
                .entry(dependency.clone())
                .or_default()
                .extend(scopes.iter().cloned());
        }
    }
    for package in &metadata_packages {
        for dependency in &package.dependencies {
            let scope = dependency
                .kind
                .as_deref()
                .map(cargo_scope)
                .unwrap_or("runtime")
                .to_owned();
            direct_scopes
                .entry(dependency.name.clone())
                .or_default()
                .insert(scope);
        }
    }

    let lock_paths = paths
        .iter()
        .filter(|path| path.rsplit('/').next() == Some("Cargo.lock"))
        .cloned()
        .collect::<Vec<_>>();
    let pin_by_path: HashMap<String, String> = source_pins
        .iter()
        .filter_map(|pin| {
            let path = pin.vendored_path.as_ref()?;
            let identity = pin_identity(pin)?;
            Some((normalize_repo_path(path), identity))
        })
        .collect();
    let pin_by_purl: HashMap<String, String> = source_pins
        .iter()
        .filter_map(|pin| {
            let purl =
                cargo_purl_from_repository(pin.repository.as_deref(), pin.version.as_deref())?;
            let identity = pin_identity(pin)?;
            Some((purl, identity))
        })
        .collect();
    let mut dependency_edges = BTreeMap::<String, BTreeSet<String>>::new();

    for lock_path in lock_paths {
        let Some(contents) = read_optional_text(&root.join(&lock_path))? else {
            continue;
        };
        if contents.len() > MAX_METADATA_BYTES {
            findings.push(Finding {
                code: "CARGO_LOCK_TOO_LARGE".to_owned(),
                severity: "error".to_owned(),
                component_id: None,
                message: "Cargo.lock exceeds the bounded metadata input size".to_owned(),
                evidence: vec![lock_path],
            });
            continue;
        }
        let lock = match toml::from_str::<CargoLock>(&contents) {
            Ok(lock) => lock,
            Err(_) => {
                findings.push(Finding {
                    code: "CARGO_LOCK_UNREADABLE".to_owned(),
                    severity: "error".to_owned(),
                    component_id: None,
                    message: "Cargo.lock is not valid Cargo lockfile TOML".to_owned(),
                    evidence: vec![lock_path],
                });
                continue;
            }
        };

        let mut per_lock_ids = BTreeMap::<LockKey, String>::new();
        for package in &lock.package {
            if package.name.trim().is_empty() || package.version.trim().is_empty() {
                findings.push(Finding {
                    code: "CARGO_PACKAGE_IDENTITY_UNRESOLVED".to_owned(),
                    severity: "error".to_owned(),
                    component_id: None,
                    message: "Cargo.lock package is missing a name or version".to_owned(),
                    evidence: vec![lock_path.clone()],
                });
                continue;
            }
            let lock_key = LockKey {
                name: package.name.clone(),
                version: package.version.clone(),
                source: package.source.clone().unwrap_or_default(),
            };
            let manifest = find_manifest(
                &root,
                &lock_path,
                package,
                &manifest_metadata,
                &metadata_by_manifest,
            );
            let source_kind = package.source.as_deref();
            let purl = source_kind
                .filter(|source| source.starts_with("registry+"))
                .map(|_| cargo_purl(&package.name, &package.version));
            let manifest_path = manifest.as_ref().map(|record| record.relative_path.clone());
            let exact_pin_id = manifest_path
                .as_ref()
                .and_then(|path| pin_by_path.get(path).cloned());
            let component_id = exact_pin_id
                .or_else(|| {
                    purl.as_ref()
                        .and_then(|purl| pin_by_purl.get(purl).cloned())
                })
                .unwrap_or_else(|| {
                    component_identity(
                        &package.name,
                        &package.version,
                        package.source.as_deref(),
                        manifest_path.as_deref(),
                        purl.as_deref(),
                    )
                });
            if manifest
                .as_ref()
                .and_then(|record| record.license_file.as_deref())
                .is_some_and(|path| safe_relative_reference(path).is_none())
            {
                findings.push(Finding {
                    code: "UNSAFE_LICENSE_FILE_PATH".to_owned(),
                    severity: "error".to_owned(),
                    component_id: Some(component_id.clone()),
                    message: "Cargo manifest license-file is not a repository-relative path"
                        .to_owned(),
                    evidence: manifest
                        .as_ref()
                        .map(|record| vec![record.relative_path.clone()])
                        .unwrap_or_default(),
                });
            }
            let mut record = cargo_component(
                CargoComponentInput {
                    package,
                    lock_path: &lock_path,
                    manifest: manifest.as_ref(),
                    purl,
                    component_id: &component_id,
                },
                &ComponentEvidenceContext {
                    evidence_paths: &evidence_paths,
                    tracked_paths: &paths,
                    notice_rows: &notice_rows,
                    direct_scopes: &direct_scopes,
                },
            );
            if package.source.is_none()
                && manifest_path
                    .as_deref()
                    .is_some_and(|path| path.starts_with("third_party/"))
            {
                record.vendored = true;
                record.directness = "vendored".to_owned();
                record.review_status = "manual_review_required".to_owned();
            }
            per_lock_ids.insert(lock_key, component_id.clone());
            merge_component(&mut components, record, &mut findings);
        }

        for package in &lock.package {
            let parent_key = LockKey {
                name: package.name.clone(),
                version: package.version.clone(),
                source: package.source.clone().unwrap_or_default(),
            };
            let Some(parent_id) = per_lock_ids.get(&parent_key) else {
                continue;
            };
            for raw_dependency in &package.dependencies {
                let Some(dependency) = parse_lock_dependency(raw_dependency) else {
                    findings.push(Finding {
                        code: "UNPARSEABLE_CARGO_DEPENDENCY".to_owned(),
                        severity: "error".to_owned(),
                        component_id: Some(parent_id.clone()),
                        message: "Cargo.lock contains a dependency entry that could not be parsed"
                            .to_owned(),
                        evidence: vec![lock_path.clone()],
                    });
                    continue;
                };
                let candidates = per_lock_ids
                    .iter()
                    .filter(|(key, _)| {
                        key.name == dependency.name
                            && dependency
                                .version
                                .as_ref()
                                .is_none_or(|version| &key.version == version)
                            && dependency
                                .source
                                .as_ref()
                                .is_none_or(|source| &key.source == source)
                    })
                    .map(|(_, id)| id.clone())
                    .collect::<BTreeSet<_>>();
                match candidates.len() {
                    1 => {
                        let child_id = candidates.into_iter().next().unwrap_or_default();
                        if &child_id != parent_id {
                            dependency_edges
                                .entry(parent_id.clone())
                                .or_default()
                                .insert(child_id);
                        }
                    }
                    0 => findings.push(Finding {
                        code: "UNRESOLVED_CARGO_DEPENDENCY".to_owned(),
                        severity: "error".to_owned(),
                        component_id: Some(parent_id.clone()),
                        message: "Cargo.lock dependency entry has no matching package identity"
                            .to_owned(),
                        evidence: vec![lock_path.clone()],
                    }),
                    _ => findings.push(Finding {
                        code: "AMBIGUOUS_CARGO_DEPENDENCY".to_owned(),
                        severity: "error".to_owned(),
                        component_id: Some(parent_id.clone()),
                        message: "Cargo.lock dependency entry matches multiple package identities"
                            .to_owned(),
                        evidence: vec![lock_path.clone()],
                    }),
                }
            }
        }
    }

    for (parent_id, children) in dependency_edges {
        if let Some(parent) = components.get_mut(&parent_id) {
            parent.dependencies.extend(children);
            parent.dependencies.sort();
            parent.dependencies.dedup();
        }
    }

    let model_assets = load_model_catalog(&root, &mut findings)?;
    for component in components.values() {
        if component.license_status == "unknown" {
            findings.push(Finding {
                code: "UNKNOWN_LICENSE".to_owned(),
                severity: "review".to_owned(),
                component_id: Some(component.component_id.clone()),
                message:
                    "no explicit SPDX license expression was resolved; inspect recorded evidence"
                        .to_owned(),
                evidence: component
                    .evidence
                    .iter()
                    .map(|item| item.source.clone())
                    .collect(),
            });
        }
        if component.license_status == "malformed" {
            findings.push(Finding {
                code: "MALFORMED_LICENSE_EXPRESSION".to_owned(),
                severity: "error".to_owned(),
                component_id: Some(component.component_id.clone()),
                message: "component metadata claims a malformed SPDX license expression".to_owned(),
                evidence: component
                    .evidence
                    .iter()
                    .map(|item| item.source.clone())
                    .collect(),
            });
        }
        if component.vendored
            && component.declared_license_expression.is_none()
            && component.detected_license_files.is_empty()
            && !component.review_recorded
        {
            findings.push(Finding {
                code: "VENDORED_LICENSE_EVIDENCE_MISSING".to_owned(),
                severity: "error".to_owned(),
                component_id: Some(component.component_id.clone()),
                message: "vendored component has no located license evidence and no explicit review record".to_owned(),
                evidence: component.evidence.iter().map(|item| item.source.clone()).collect(),
            });
        }
        if !component.conflicts.is_empty() {
            findings.push(Finding {
                code: "CONFLICTING_COMPONENT_IDENTITY".to_owned(),
                severity: "error".to_owned(),
                component_id: Some(component.component_id.clone()),
                message: "the same component identity has conflicting metadata".to_owned(),
                evidence: component.conflicts.clone(),
            });
        }
    }

    findings.sort_by(|left, right| {
        (&left.code, &left.component_id, &left.message).cmp(&(
            &right.code,
            &right.component_id,
            &right.message,
        ))
    });
    findings.dedup();

    let repository_revision = git_revision(&root);
    Ok(Inventory {
        schema_version: 1,
        project: ProjectRecord {
            name: "Nagi OS".to_owned(),
            version: "0.1 Developer Preview".to_owned(),
            declared_license_expression: None,
        },
        repository_revision,
        components: components.into_values().collect(),
        model_assets,
        findings,
    })
}

fn candidate_paths(root: &Path) -> Result<Vec<String>, String> {
    if let Some(repo_root) = git_root(root) {
        if repo_root == root {
            let mut command = Command::new("git");
            command
                .args(["-C", root.to_string_lossy().as_ref(), "ls-files", "-z"])
                .stderr(Stdio::null());
            let (status, stdout) = bounded_command_output(&mut command, 64 * 1024 * 1024)
                .map_err(|error| format!("cannot list tracked repository paths: {error}"))?;
            if !status.success() {
                return Err("git could not list tracked repository paths".to_owned());
            }
            let mut paths = stdout
                .split(|byte| *byte == 0)
                .filter(|path| !path.is_empty())
                .filter_map(|path| std::str::from_utf8(path).ok().map(str::to_owned))
                .collect::<Vec<_>>();
            if paths.len() > MAX_INDEX_PATHS {
                return Err("tracked repository path count exceeds its bound".to_owned());
            }
            paths.extend([
                "tools/legal/Cargo.toml".to_owned(),
                "tools/legal/Cargo.lock".to_owned(),
                "docs/legal/model-assets.json".to_owned(),
            ]);
            paths.retain(|path| {
                root.join(path)
                    .symlink_metadata()
                    .is_ok_and(|metadata| metadata.file_type().is_file())
            });
            paths.sort();
            paths.dedup();
            return Ok(paths);
        }
    }
    bounded_file_walk(root)
}

fn bounded_file_walk(root: &Path) -> Result<Vec<String>, String> {
    let mut pending = vec![(root.to_path_buf(), 0usize)];
    let mut paths = Vec::new();
    let mut visited = 0usize;
    while let Some((directory, depth)) = pending.pop() {
        if depth > MAX_FALLBACK_DEPTH {
            continue;
        }
        let entries = fs::read_dir(&directory)
            .map_err(|error| format!("cannot read scan directory: {error}"))?;
        for entry in entries {
            visited += 1;
            if visited > MAX_FALLBACK_ENTRIES {
                return Err("offline fixture scan exceeded its entry bound".to_owned());
            }
            let entry = entry.map_err(|error| format!("cannot read scan entry: {error}"))?;
            let name = entry.file_name();
            let Some(name_text) = name.to_str() else {
                continue;
            };
            let path = entry.path();
            let file_type = entry
                .file_type()
                .map_err(|error| format!("cannot inspect scan entry: {error}"))?;
            if file_type.is_symlink() {
                continue;
            }
            if file_type.is_dir() {
                if should_skip_directory(name_text) || is_large_vendor_cache(&path, root) {
                    continue;
                }
                pending.push((path, depth + 1));
            } else if file_type.is_file() {
                if let Some(relative) = repo_relative_path(root, &path) {
                    paths.push(relative);
                }
            }
        }
    }
    paths.sort();
    Ok(paths)
}

fn should_skip_directory(name: &str) -> bool {
    matches!(
        name,
        ".git"
            | ".cargo"
            | ".cache"
            | ".idea"
            | ".vscode"
            | "target"
            | "out"
            | "build"
            | "node_modules"
            | "vendor"
            | "__pycache__"
            | "cache"
            | "servo"
            | "mesa"
            | "mozjs"
    )
}

fn is_large_vendor_cache(path: &Path, root: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return true;
    };
    let normalized = normalize_path(relative);
    ["third_party/servo", "third_party/mesa", "third_party/mozjs"]
        .iter()
        .any(|prefix| normalized == *prefix || normalized.starts_with(&format!("{prefix}/")))
}

fn git_root(root: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .args([
            "-C",
            root.to_string_lossy().as_ref(),
            "rev-parse",
            "--show-toplevel",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    PathBuf::from(text.trim()).canonicalize().ok()
}

fn git_revision(root: &Path) -> Option<String> {
    let output = Command::new("git")
        .args([
            "-C",
            root.to_string_lossy().as_ref(),
            "rev-parse",
            "--verify",
            "HEAD",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let revision = String::from_utf8(output.stdout).ok()?.trim().to_owned();
    (revision.len() == 40 && revision.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then_some(revision)
}

fn cargo_metadata(root: &Path) -> Result<Vec<MetadataPackage>, ()> {
    if !root.join("Cargo.toml").is_file() || git_root(root).as_deref() != Some(root) {
        return Ok(Vec::new());
    }
    let mut command = Command::new("cargo");
    command
        .args([
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--locked",
            "--offline",
            "--manifest-path",
        ])
        .arg(root.join("Cargo.toml"))
        .current_dir(root)
        .stderr(Stdio::null());
    let (status, stdout) =
        bounded_command_output(&mut command, MAX_METADATA_BYTES).map_err(|_| ())?;
    if !status.success() {
        return Err(());
    }
    serde_json::from_slice::<CargoMetadata>(&stdout)
        .map(|metadata| metadata.packages)
        .map_err(|_| ())
}

fn bounded_command_output(
    command: &mut Command,
    limit: usize,
) -> Result<(ExitStatus, Vec<u8>), String> {
    command.stdout(Stdio::piped());
    let mut child = command
        .spawn()
        .map_err(|error| format!("cannot start metadata command: {error}"))?;
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return Err("metadata command has no readable output pipe".to_owned());
    };
    let mut bytes = Vec::new();
    let read_result = stdout
        .take(limit.saturating_add(1) as u64)
        .read_to_end(&mut bytes);
    if let Err(error) = read_result {
        let _ = child.kill();
        let _ = child.wait();
        return Err(format!("cannot read metadata command output: {error}"));
    }
    if bytes.len() > limit {
        let _ = child.kill();
        let _ = child.wait();
        return Err("metadata command output exceeded its size bound".to_owned());
    }
    let status = child
        .wait()
        .map_err(|error| format!("cannot wait for metadata command: {error}"))?;
    Ok((status, bytes))
}

fn collect_manifest_metadata(root: &Path, paths: &[String]) -> Vec<ManifestMetadata> {
    let mut manifests = Vec::new();
    for relative in paths
        .iter()
        .filter(|path| path.rsplit('/').next() == Some("Cargo.toml"))
    {
        let Ok(Some(contents)) = read_optional_text(&root.join(relative)) else {
            continue;
        };
        if contents.len() > MAX_METADATA_BYTES {
            continue;
        }
        let Ok(document) = toml::from_str::<toml::Value>(&contents) else {
            continue;
        };
        let Some(package) = document.get("package").and_then(toml::Value::as_table) else {
            continue;
        };
        let Some(name) = package.get("name").and_then(toml::Value::as_str) else {
            continue;
        };
        let Some(version) = package.get("version").and_then(toml::Value::as_str) else {
            continue;
        };
        let license = package
            .get("license")
            .and_then(toml::Value::as_str)
            .map(str::to_owned);
        let license_file = package
            .get("license-file")
            .and_then(toml::Value::as_str)
            .map(str::to_owned);
        let repository = package
            .get("repository")
            .and_then(toml::Value::as_str)
            .map(str::to_owned);
        let dependencies = manifest_dependencies(&document);
        manifests.push(ManifestMetadata {
            name: name.to_owned(),
            version: version.to_owned(),
            license,
            license_file,
            repository,
            relative_path: relative.clone(),
            dependencies,
        });
    }
    manifests.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    manifests
}

fn manifest_dependencies(document: &toml::Value) -> BTreeMap<String, BTreeSet<String>> {
    fn add_sections(value: &toml::Value, result: &mut BTreeMap<String, BTreeSet<String>>) {
        let Some(table) = value.as_table() else {
            return;
        };
        for (key, scope) in [
            ("dependencies", "runtime"),
            ("dev-dependencies", "dev"),
            ("build-dependencies", "build"),
        ] {
            let Some(dependencies) = table.get(key).and_then(toml::Value::as_table) else {
                continue;
            };
            for (dependency_name, dependency_value) in dependencies {
                let actual_name = dependency_value
                    .get("package")
                    .and_then(toml::Value::as_str)
                    .unwrap_or(dependency_name);
                result
                    .entry(actual_name.to_owned())
                    .or_default()
                    .insert(scope.to_owned());
            }
        }
    }
    let mut result = BTreeMap::new();
    add_sections(document, &mut result);
    if let Some(targets) = document.get("target").and_then(toml::Value::as_table) {
        for target in targets.values() {
            add_sections(target, &mut result);
        }
    }
    result
}

fn find_manifest(
    root: &Path,
    lock_path: &str,
    package: &LockPackage,
    manifests: &[ManifestMetadata],
    metadata_by_path: &HashMap<String, MetadataPackage>,
) -> Option<ManifestMetadata> {
    if package.source.is_some() {
        return None;
    }
    let lock_parent = Path::new(lock_path)
        .parent()
        .unwrap_or_else(|| Path::new(""));
    let sibling_manifest = normalize_path(&lock_parent.join("Cargo.toml"));
    if let Some(metadata) = metadata_by_path.get(&sibling_manifest) {
        return Some(ManifestMetadata {
            name: metadata.name.clone(),
            version: metadata.version.clone(),
            license: metadata.license.clone(),
            license_file: metadata.license_file.clone(),
            repository: metadata.repository.clone(),
            relative_path: sibling_manifest,
            dependencies: metadata.dependencies.iter().fold(
                BTreeMap::new(),
                |mut map, dependency| {
                    let scope = dependency
                        .kind
                        .as_deref()
                        .map(cargo_scope)
                        .unwrap_or("runtime")
                        .to_owned();
                    map.entry(dependency.name.clone())
                        .or_insert_with(BTreeSet::new)
                        .insert(scope);
                    map
                },
            ),
        });
    }
    if let Some(manifest) = manifests.iter().find(|manifest| {
        manifest.relative_path == sibling_manifest
            && manifest.name == package.name
            && manifest.version == package.version
    }) {
        return Some(manifest.clone());
    }
    let _ = root;
    manifests
        .iter()
        .find(|manifest| manifest.name == package.name && manifest.version == package.version)
        .cloned()
}

fn cargo_component(
    input: CargoComponentInput<'_>,
    context: &ComponentEvidenceContext<'_>,
) -> ComponentRecord {
    let CargoComponentInput {
        package,
        lock_path,
        manifest,
        purl,
        component_id,
    } = input;
    let ComponentEvidenceContext {
        evidence_paths,
        tracked_paths,
        notice_rows,
        direct_scopes,
    } = context;
    let local_path = manifest.map(|manifest| {
        normalize_repo_path(&normalize_path(
            Path::new(&manifest.relative_path)
                .parent()
                .unwrap_or(Path::new("")),
        ))
    });
    let vendored = local_path
        .as_deref()
        .is_some_and(|path| path.starts_with("third_party/"));
    let raw_license = manifest.and_then(|manifest| manifest.license.clone());
    let (license_expression, license_status) =
        match assess_license_expression(raw_license.as_deref()) {
            LicenseAssessment::DeclaredExpression(expression) => (Some(expression), "declared"),
            LicenseAssessment::MalformedExpression(_) => (None, "malformed"),
            LicenseAssessment::Unknown => (None, "unknown"),
        };
    let license_file_declared = manifest
        .and_then(|manifest| manifest.license_file.as_deref())
        .and_then(safe_relative_reference);
    let mut detected_license_files = local_path
        .as_deref()
        .map(|path| matching_license_paths(path, evidence_paths))
        .unwrap_or_default();
    if let (Some(local_path), Some(license_file)) = (&local_path, &license_file_declared) {
        let candidate = if local_path.is_empty() {
            license_file.clone()
        } else {
            format!("{local_path}/{license_file}")
        };
        if tracked_paths.iter().any(|path| path == &candidate) {
            detected_license_files.push(candidate);
        }
    }
    sort_unique(&mut detected_license_files);

    let review_recorded = local_path
        .as_deref()
        .is_some_and(|path| path_has_review_row(path, notice_rows))
        || notice_rows.contains(&normalize_component_name(&package.name));
    let mut notice_files = Vec::new();
    if review_recorded {
        notice_files.push("THIRD_PARTY_NOTICES.md".to_owned());
    }
    notice_files.extend(
        local_path
            .as_deref()
            .map(|path| matching_notice_paths(path, evidence_paths))
            .unwrap_or_default(),
    );
    sort_unique(&mut notice_files);

    let mut evidence = vec![EvidenceRecord {
        kind: "cargo-lock".to_owned(),
        source: lock_path.to_owned(),
        detail: Some("identity, checksum, source, and resolved dependency edges".to_owned()),
    }];
    if let Some(manifest) = manifest {
        evidence.push(EvidenceRecord {
            kind: "cargo-manifest".to_owned(),
            source: manifest.relative_path.clone(),
            detail: raw_license.clone(),
        });
    }
    evidence.extend(detected_license_files.iter().map(|path| EvidenceRecord {
        kind: "license-file".to_owned(),
        source: path.clone(),
        detail: None,
    }));
    evidence.extend(
        notice_files
            .iter()
            .filter(|path| path.as_str() != "THIRD_PARTY_NOTICES.md")
            .map(|path| EvidenceRecord {
                kind: "notice-file".to_owned(),
                source: path.clone(),
                detail: None,
            }),
    );
    if review_recorded {
        evidence.push(EvidenceRecord {
            kind: "review-record".to_owned(),
            source: "THIRD_PARTY_NOTICES.md".to_owned(),
            detail: Some("component has a tracked attribution/review entry".to_owned()),
        });
    }
    sort_evidence(&mut evidence);

    let source_origin = package.source.as_deref().and_then(sanitize_cargo_origin);
    let repository = manifest
        .and_then(|manifest| manifest.repository.as_deref())
        .and_then(sanitize_url);
    let origin = source_origin.or(repository);
    let directness = if vendored {
        "vendored"
    } else if local_path.is_some() {
        "project"
    } else if direct_scopes.contains_key(&package.name) {
        "direct"
    } else {
        "transitive"
    };
    let scopes = direct_scopes
        .get(&package.name)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .collect::<Vec<_>>();
    let review_status = if license_status == "declared" && !vendored {
        "metadata_recorded"
    } else {
        "manual_review_required"
    };

    ComponentRecord {
        component_id: component_id.to_owned(),
        name: package.name.clone(),
        package_type: "cargo".to_owned(),
        version: Some(package.version.clone()),
        revision: package.source.as_deref().and_then(cargo_revision),
        origin,
        local_path,
        purl,
        checksum: package.checksum.clone(),
        declared_license_expression: license_expression,
        raw_license_metadata: raw_license,
        license_status: license_status.to_owned(),
        license_file_declared,
        detected_license_files,
        notice_files,
        directness: directness.to_owned(),
        scopes,
        dependencies: Vec::new(),
        vendored,
        review_status: review_status.to_owned(),
        review_recorded,
        evidence,
        conflicts: Vec::new(),
    }
}

fn add_source_pins(
    pins: &[SourcePin],
    evidence_paths: &[String],
    notice_rows: &BTreeSet<String>,
    findings: &mut Vec<Finding>,
    components: &mut BTreeMap<String, ComponentRecord>,
) {
    for pin in pins {
        let Some(identity) = pin_identity(pin) else {
            findings.push(Finding {
                code: "SOURCE_PIN_IDENTITY_UNRESOLVED".to_owned(),
                severity: "error".to_owned(),
                component_id: None,
                message: "source-lock entry has no stable component identity".to_owned(),
                evidence: vec!["third_party/sources.lock".to_owned()],
            });
            continue;
        };
        let package_name =
            cargo_package_name(pin.repository.as_deref()).unwrap_or_else(|| pin.component.clone());
        let purl = cargo_purl_from_repository(pin.repository.as_deref(), pin.version.as_deref());
        let local_path = pin.vendored_path.as_deref().map(normalize_repo_path);
        let detected_license_files = local_path
            .as_deref()
            .map(|path| matching_license_paths(path, evidence_paths))
            .unwrap_or_default();
        let discovered_notice_files = local_path
            .as_deref()
            .map(|path| matching_notice_paths(path, evidence_paths))
            .unwrap_or_default();
        let normalized_name = normalize_component_name(&pin.component);
        let review_recorded = notice_rows.contains(&normalized_name)
            || notice_rows.contains(&normalize_component_name(&package_name));
        let mut notice_files = if review_recorded {
            vec!["THIRD_PARTY_NOTICES.md".to_owned()]
        } else {
            Vec::new()
        };
        notice_files.extend(discovered_notice_files);
        sort_unique(&mut notice_files);
        let (expression, license_status) = match assess_license_expression(pin.license.as_deref()) {
            LicenseAssessment::DeclaredExpression(expression) => (Some(expression), "declared"),
            LicenseAssessment::MalformedExpression(_) => (None, "malformed"),
            LicenseAssessment::Unknown => (None, "unknown"),
        };
        let mut evidence = vec![EvidenceRecord {
            kind: "source-pin".to_owned(),
            source: "third_party/sources.lock".to_owned(),
            detail: Some(format!("pin record for {}", pin.component)),
        }];
        evidence.extend(detected_license_files.iter().map(|path| EvidenceRecord {
            kind: "license-file".to_owned(),
            source: path.clone(),
            detail: None,
        }));
        evidence.extend(
            notice_files
                .iter()
                .filter(|path| path.as_str() != "THIRD_PARTY_NOTICES.md")
                .map(|path| EvidenceRecord {
                    kind: "notice-file".to_owned(),
                    source: path.clone(),
                    detail: None,
                }),
        );
        if review_recorded {
            evidence.push(EvidenceRecord {
                kind: "review-record".to_owned(),
                source: "THIRD_PARTY_NOTICES.md".to_owned(),
                detail: Some("component has a tracked attribution/review entry".to_owned()),
            });
        }
        sort_evidence(&mut evidence);
        let record = ComponentRecord {
            component_id: identity.clone(),
            name: package_name,
            package_type: "pinned-source".to_owned(),
            version: pin.version.clone(),
            revision: pin.revision.as_deref().and_then(safe_revision),
            origin: pin.repository.as_deref().and_then(sanitize_url),
            local_path,
            purl,
            checksum: pin
                .source_hash
                .as_deref()
                .and_then(parse_sha256_source_hash),
            declared_license_expression: expression,
            raw_license_metadata: pin.license.clone(),
            license_status: license_status.to_owned(),
            license_file_declared: None,
            detected_license_files,
            notice_files,
            directness: "pinned".to_owned(),
            scopes: vec!["unknown".to_owned()],
            dependencies: Vec::new(),
            vendored: true,
            review_status: "manual_review_required".to_owned(),
            review_recorded,
            evidence,
            conflicts: Vec::new(),
        };
        merge_component(components, record, &mut Vec::new());
    }
}

fn merge_component(
    components: &mut BTreeMap<String, ComponentRecord>,
    mut incoming: ComponentRecord,
    findings: &mut Vec<Finding>,
) {
    sort_unique(&mut incoming.detected_license_files);
    sort_unique(&mut incoming.notice_files);
    sort_unique(&mut incoming.scopes);
    sort_unique(&mut incoming.dependencies);
    sort_evidence(&mut incoming.evidence);
    let key = incoming.component_id.clone();
    let Some(existing) = components.get_mut(&key) else {
        components.insert(key, incoming);
        return;
    };
    if existing.name != incoming.name {
        existing
            .conflicts
            .push(format!("name: {} vs {}", existing.name, incoming.name));
    }
    if existing.version != incoming.version {
        existing.conflicts.push(format!(
            "version: {:?} vs {:?}",
            existing.version, incoming.version
        ));
    }
    if existing.checksum != incoming.checksum
        && existing.checksum.is_some()
        && incoming.checksum.is_some()
    {
        existing
            .conflicts
            .push("checksum declarations differ".to_owned());
    }
    if existing.declared_license_expression != incoming.declared_license_expression
        && existing.declared_license_expression.is_some()
        && incoming.declared_license_expression.is_some()
    {
        existing
            .conflicts
            .push("license expressions differ".to_owned());
        existing.declared_license_expression = None;
        existing.license_status = "conflict".to_owned();
    } else if existing.declared_license_expression.is_none()
        && incoming.declared_license_expression.is_some()
    {
        existing.declared_license_expression = incoming.declared_license_expression.take();
        existing.license_status = incoming.license_status.clone();
    }
    if existing.raw_license_metadata.is_none() {
        existing.raw_license_metadata = incoming.raw_license_metadata.take();
    } else if incoming.raw_license_metadata.is_some()
        && existing.raw_license_metadata != incoming.raw_license_metadata
    {
        existing
            .conflicts
            .push("raw license metadata differs".to_owned());
    }
    if existing.checksum.is_none() {
        existing.checksum = incoming.checksum;
    }
    if existing.license_file_declared.is_none() {
        existing.license_file_declared = incoming.license_file_declared;
    }
    if existing.origin.is_none() {
        existing.origin = incoming.origin;
    }
    if existing.local_path.is_none() {
        existing.local_path = incoming.local_path;
    }
    existing
        .detected_license_files
        .extend(incoming.detected_license_files);
    existing.notice_files.extend(incoming.notice_files);
    existing.scopes.extend(incoming.scopes);
    existing.dependencies.extend(incoming.dependencies);
    existing.evidence.extend(incoming.evidence);
    existing.vendored |= incoming.vendored;
    existing.review_recorded |= incoming.review_recorded;
    if existing.review_status != "manual_review_required"
        && incoming.review_status == "manual_review_required"
    {
        existing.review_status = incoming.review_status;
    }
    if !existing.conflicts.is_empty() {
        existing.license_status = "conflict".to_owned();
    }
    sort_unique(&mut existing.detected_license_files);
    sort_unique(&mut existing.notice_files);
    sort_unique(&mut existing.scopes);
    sort_unique(&mut existing.dependencies);
    sort_unique(&mut existing.conflicts);
    sort_evidence(&mut existing.evidence);
    if !existing.conflicts.is_empty() {
        findings.push(Finding {
            code: "CONFLICTING_COMPONENT_IDENTITY".to_owned(),
            severity: "error".to_owned(),
            component_id: Some(existing.component_id.clone()),
            message: "the same component identity has conflicting metadata".to_owned(),
            evidence: existing.conflicts.clone(),
        });
    }
}

fn parse_source_pins(contents: &str) -> Result<Vec<SourcePin>, ()> {
    let document = toml::from_str::<toml::Value>(contents).map_err(|_| ())?;
    let Some(sources) = document.get("sources").and_then(toml::Value::as_table) else {
        return Ok(Vec::new());
    };
    let mut pins = Vec::new();
    for (key, value) in sources {
        let Some(table) = value.as_table() else {
            continue;
        };
        let get = |name: &str| {
            table
                .get(name)
                .and_then(toml::Value::as_str)
                .map(str::to_owned)
        };
        let raw_vendored_path = get("vendored_path");
        let vendored_path = raw_vendored_path
            .as_deref()
            .and_then(safe_relative_reference)
            .map(|path| normalize_repo_path(&path));
        pins.push(SourcePin {
            component: get("component").unwrap_or_else(|| key.clone()),
            version: get("version").or_else(|| get("toolchain")),
            revision: get("revision"),
            repository: get("repository").or_else(|| get("source")),
            source_hash: get("source_hash"),
            license: get("license"),
            unsafe_vendored_path: raw_vendored_path.is_some() && vendored_path.is_none(),
            vendored_path,
        });
    }
    pins.sort_by(|left, right| left.component.cmp(&right.component));
    Ok(pins)
}

fn load_model_catalog(
    root: &Path,
    findings: &mut Vec<Finding>,
) -> Result<Vec<ModelLicenseRecord>, String> {
    let path = root.join("docs/legal/model-assets.json");
    let Some(contents) = read_optional_text(&path)? else {
        return Ok(Vec::new());
    };
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Catalog {
        schema_version: u32,
        assets: Vec<ModelLicenseRecord>,
    }
    let mut catalog = serde_json::from_str::<Catalog>(&contents).map_err(|_| {
        "docs/legal/model-assets.json has an invalid model license record".to_owned()
    })?;
    if catalog.schema_version != 1 {
        findings.push(Finding {
            code: "MODEL_LICENSE_SCHEMA_UNSUPPORTED".to_owned(),
            severity: "error".to_owned(),
            component_id: None,
            message: "model license catalog schema version is unsupported".to_owned(),
            evidence: vec!["docs/legal/model-assets.json".to_owned()],
        });
    }
    let mut ids = BTreeSet::new();
    for model in &mut catalog.assets {
        let safe_model_id = safe_model_id(&model.model_id);
        if model.model_id.trim().is_empty()
            || model.display_name.trim().is_empty()
            || model.provider_id.trim().is_empty()
            || model.version.trim().is_empty()
            || model.variant.trim().is_empty()
            || !matches!(
                model.inclusion_status.as_str(),
                "fixture-only" | "planned" | "present"
            )
            || !matches!(
                model.review_status.as_str(),
                "manual_review_required" | "reviewed_by_maintainer"
            )
            || model.notices.iter().any(|notice| {
                notice.notice_id.trim().is_empty() || notice.reference.trim().is_empty()
            })
        {
            findings.push(model_record_finding(
                "MODEL_LICENSE_SCHEMA_INVALID",
                safe_model_id.as_deref(),
                "model license record does not satisfy its required field constraints",
                "error",
            ));
        }
        if !ids.insert(model.model_id.clone()) {
            findings.push(model_record_finding(
                "DUPLICATE_MODEL_ID",
                safe_model_id.as_deref(),
                "model license catalog contains a duplicate model ID",
                "error",
            ));
        }
        if let Some(evidence_source) = safe_relative_reference(&model.evidence_source) {
            model.evidence_source = evidence_source;
        } else {
            findings.push(model_record_finding(
                "UNSAFE_MODEL_EVIDENCE_PATH",
                safe_model_id.as_deref(),
                "model evidence_source must be a repository-relative path without traversal",
                "error",
            ));
            model.evidence_source = "docs/legal/model-assets.json".to_owned();
        }
        if let Some(claimed) = model.declared_license_expression.take() {
            match assess_license_expression(Some(&claimed)) {
                LicenseAssessment::DeclaredExpression(expression) => {
                    model.declared_license_expression = Some(expression);
                }
                LicenseAssessment::Unknown => {
                    model.raw_license_metadata = Some(claimed);
                    findings.push(model_record_finding(
                        "MODEL_LICENSE_EXPRESSION_UNKNOWN",
                        safe_model_id.as_deref(),
                        "model license metadata is not a known SPDX expression and remains unclassified",
                        "review",
                    ));
                }
                LicenseAssessment::MalformedExpression(raw) => {
                    model.raw_license_metadata = Some(raw);
                    findings.push(model_record_finding(
                        "MODEL_MALFORMED_LICENSE_EXPRESSION",
                        safe_model_id.as_deref(),
                        "model metadata claims a value that is not a known SPDX expression",
                        "error",
                    ));
                }
            }
        }
        if model.artifact_present && model.declared_license_expression.is_none() {
            findings.push(model_record_finding(
                "MODEL_LICENSE_REVIEW_REQUIRED",
                safe_model_id.as_deref(),
                "a model asset is marked present without an explicit SPDX license expression",
                "review",
            ));
        }
    }
    let mut assets = catalog.assets;
    assets.sort_by(|left, right| left.model_id.cmp(&right.model_id));
    Ok(assets)
}

fn model_record_finding(
    code: &str,
    component_id: Option<&str>,
    message: &str,
    severity: &str,
) -> Finding {
    Finding {
        code: code.to_owned(),
        severity: severity.to_owned(),
        component_id: component_id.map(str::to_owned),
        message: message.to_owned(),
        evidence: vec!["docs/legal/model-assets.json".to_owned()],
    }
}

fn safe_model_id(value: &str) -> Option<String> {
    (!value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-')))
    .then(|| value.to_owned())
}

fn parse_notice_rows(text: &str) -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with('|')
            || trimmed.contains("---")
            || trimmed
                .to_ascii_lowercase()
                .contains("component | inclusion")
        {
            continue;
        }
        if let Some(name) = trimmed.split('|').nth(1) {
            let name = name.trim().trim_matches('`').trim();
            if !name.is_empty() {
                names.insert(normalize_component_name(name));
            }
        }
    }
    names
}

fn matching_license_paths(prefix: &str, paths: &[String]) -> Vec<String> {
    matching_evidence_paths(prefix, paths, is_license_filename)
}

fn matching_notice_paths(prefix: &str, paths: &[String]) -> Vec<String> {
    matching_evidence_paths(prefix, paths, is_notice_or_attribution_filename)
}

fn matching_evidence_paths(
    prefix: &str,
    paths: &[String],
    accepts: fn(&str) -> bool,
) -> Vec<String> {
    let prefix = normalize_repo_path(prefix);
    paths
        .iter()
        .filter(|path| {
            let in_component = if prefix.is_empty() {
                !path.contains('/')
            } else {
                path.starts_with(&format!("{prefix}/"))
            };
            let inherited_from_vendor_root = prefix.starts_with("third_party/")
                && Path::new(path).parent().is_some_and(|file_parent| {
                    let mut ancestor = Path::new(&prefix).parent();
                    while let Some(directory) = ancestor {
                        if file_parent == directory {
                            return true;
                        }
                        if directory == Path::new("third_party") {
                            break;
                        }
                        ancestor = directory.parent();
                    }
                    false
                });
            (in_component || inherited_from_vendor_root) && accepts(path)
        })
        .take(MAX_COMPONENT_FILES)
        .cloned()
        .collect()
}

fn is_evidence_reference_path(path: &str) -> bool {
    is_license_filename(path) || is_notice_or_attribution_filename(path)
}

fn is_license_filename(path: &str) -> bool {
    path.rsplit('/').next().is_some_and(|name| {
        let name = name.to_ascii_lowercase();
        name.starts_with("license") || name.starts_with("licence") || name.starts_with("copying")
    })
}

fn is_notice_or_attribution_filename(path: &str) -> bool {
    path.rsplit('/').next().is_some_and(|name| {
        let name = name.to_ascii_lowercase();
        name.starts_with("notice") || name.starts_with("copyright") || name == "patents"
    })
}

fn path_has_review_row(path: &str, notice_rows: &BTreeSet<String>) -> bool {
    if notice_rows.contains(&normalize_component_name(package_name_from_path(path))) {
        return true;
    }
    if !path.starts_with("third_party/") {
        return false;
    }
    let mut directory = Path::new(path);
    loop {
        if let Some(name) = directory.file_name().and_then(|name| name.to_str()) {
            if notice_rows.contains(&normalize_component_name(name)) {
                return true;
            }
        }
        if directory == Path::new("third_party") {
            break;
        }
        let Some(parent) = directory.parent() else {
            break;
        };
        directory = parent;
    }
    false
}

fn cargo_scope(kind: &str) -> &'static str {
    match kind {
        "build" => "build",
        "dev" => "dev",
        _ => "runtime",
    }
}

fn cargo_purl(name: &str, version: &str) -> String {
    format!("pkg:cargo/{}@{}", purl_encode(name), purl_encode(version))
}

fn cargo_purl_from_repository(repository: Option<&str>, version: Option<&str>) -> Option<String> {
    let version = version?;
    let name = cargo_package_name(Some(repository?))?;
    Some(cargo_purl(&name, version))
}

fn cargo_package_name(repository: Option<&str>) -> Option<String> {
    let url = sanitize_url(repository?)?;
    let segments = url.trim_end_matches('/').split('/').collect::<Vec<_>>();
    if segments.len() >= 2 && segments[segments.len() - 2] == "crates" {
        return Some(segments[segments.len() - 1].to_owned());
    }
    if segments.len() >= 3 && segments[segments.len() - 3] == "crates" {
        return Some(segments[segments.len() - 2].to_owned());
    }
    None
}

fn purl_encode(value: &str) -> String {
    value
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-') {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

fn component_identity(
    name: &str,
    version: &str,
    source: Option<&str>,
    local_path: Option<&str>,
    purl: Option<&str>,
) -> String {
    if let Some(purl) = purl {
        return purl.to_owned();
    }
    if let Some(source) = source {
        let (origin, revision) = split_git_source(source);
        return format!(
            "vcs:{}#{}:{}@{}",
            origin,
            revision
                .as_deref()
                .and_then(safe_revision)
                .unwrap_or_default(),
            name,
            version
        );
    }
    if let Some(path) = local_path {
        return format!("cargo-path:{path}:{name}@{version}");
    }
    format!("cargo-lock:{name}@{version}")
}

fn pin_identity(pin: &SourcePin) -> Option<String> {
    let component_name = normalize_component_name(&pin.component);
    if component_name.is_empty() {
        return None;
    }
    if let Some(purl) =
        cargo_purl_from_repository(pin.repository.as_deref(), pin.version.as_deref())
    {
        return Some(purl);
    }
    let origin = pin.repository.as_deref().and_then(sanitize_url);
    let revision = pin.revision.as_deref().and_then(safe_revision);
    let version = pin
        .version
        .as_deref()
        .filter(|value| !value.trim().is_empty());
    if let (Some(origin), Some(revision)) = (origin.as_deref(), revision.as_deref()) {
        return Some(format!("vcs:{origin}#{revision}:{component_name}"));
    }
    if let (Some(origin), Some(version)) = (origin.as_deref(), version) {
        return Some(format!("source:{origin}:{component_name}@{version}"));
    }
    if let Some(path) = pin.vendored_path.as_deref() {
        let immutable = revision
            .as_deref()
            .or(version)
            .or(pin.source_hash.as_deref())
            .unwrap_or("unversioned");
        return Some(format!(
            "source-path:{}:{component_name}@{immutable}",
            normalize_repo_path(path)
        ));
    }
    if let Some(version) = version {
        return Some(format!("source:{component_name}@{version}"));
    }
    if let Some(revision) = revision.as_deref() {
        return Some(format!("source:{component_name}#{revision}"));
    }
    if let Some(source_hash) = pin.source_hash.as_deref() {
        let hash = source_hash
            .strip_prefix("sha256:")
            .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
            .or_else(|| {
                source_hash.strip_prefix("git:").filter(|value| {
                    (7..=64).contains(&value.len())
                        && value.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
            });
        if let Some(hash) = hash {
            return Some(format!("source-hash:{}", hash.to_ascii_lowercase()));
        }
    }
    None
}

fn sanitize_cargo_origin(source: &str) -> Option<String> {
    if let Some(registry) = source.strip_prefix("registry+") {
        return sanitize_url(registry).map(|url| format!("registry+{url}"));
    }
    if source.starts_with("git+") {
        let (url, _) = split_git_source(source);
        return (!url.is_empty()).then_some(url);
    }
    None
}

fn cargo_revision(source: &str) -> Option<String> {
    split_git_source(source)
        .1
        .as_deref()
        .and_then(safe_revision)
}

fn safe_revision(value: &str) -> Option<String> {
    ((7..=64).contains(&value.len()) && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| value.to_ascii_lowercase())
}

fn split_git_source(source: &str) -> (String, Option<String>) {
    let without_prefix = source.strip_prefix("git+").unwrap_or(source);
    let (url, revision) = without_prefix
        .split_once('#')
        .map(|(url, revision)| (url, Some(revision.to_owned())))
        .unwrap_or((without_prefix, None));
    (
        sanitize_url(url).unwrap_or_else(|| "unknown-source".to_owned()),
        revision,
    )
}

fn sanitize_url(raw: &str) -> Option<String> {
    let value = raw.trim();
    let (scheme, rest) = value.split_once("://")?;
    if !matches!(scheme, "https" | "http") {
        return None;
    }
    let authority_end = rest.find('/').unwrap_or(rest.len());
    let authority = &rest[..authority_end];
    let host = authority.rsplit('@').next()?;
    if host.is_empty()
        || host
            .chars()
            .any(|ch| !(ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | ':' | '[' | ']')))
    {
        return None;
    }
    let path_and_query = &rest[authority_end..];
    let path = path_and_query.split(['?', '#']).next().unwrap_or_default();
    Some(format!("{scheme}://{host}{path}"))
}

fn parse_sha256_source_hash(raw: &str) -> Option<String> {
    let digest = raw.strip_prefix("sha256:")?;
    (digest.len() == 64 && digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| digest.to_ascii_lowercase())
}

fn parse_lock_dependency(raw: &str) -> Option<DependencyKey> {
    let raw = raw.trim();
    let (identity, source) = if let Some((identity, source)) = raw.rsplit_once(" (") {
        (identity, Some(source.trim_end_matches(')').to_owned()))
    } else {
        (raw, None)
    };
    let mut parts = identity.split_whitespace();
    let name = parts.next()?.to_owned();
    let version = parts.next().map(str::to_owned);
    if parts.next().is_some() {
        return None;
    }
    Some(DependencyKey {
        name,
        version,
        source,
    })
}

fn safe_relative_reference(raw: &str) -> Option<String> {
    let path = Path::new(raw);
    if path.is_absolute() || raw.contains(':') || raw.contains('\0') || raw.contains('\\') {
        return None;
    }
    let mut parts = Vec::new();
    for component in path.components() {
        match component {
            PathComponent::Normal(value) => {
                let value = value.to_str()?;
                if value.is_empty() {
                    return None;
                }
                parts.push(value.to_owned());
            }
            PathComponent::CurDir => {}
            _ => return None,
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn package_name_from_path(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

fn normalize_component_name(name: &str) -> String {
    name.chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn normalize_repo_path(path: &str) -> String {
    let normalized = path.replace('\\', "/");
    normalized
        .trim_start_matches("./")
        .trim_matches('/')
        .to_owned()
}

fn normalize_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn repo_relative_path(root: &Path, path: &Path) -> Option<String> {
    let relative = path.strip_prefix(root).ok()?;
    let value = normalize_path(relative);
    safe_relative_reference(&value)
}

fn read_optional_text(path: &Path) -> Result<Option<String>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.len() > MAX_METADATA_BYTES as u64 => {
            return Err("metadata file exceeds its size bound".to_owned());
        }
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err("metadata file must not be a symbolic link".to_owned());
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("cannot inspect repository metadata: {error}")),
    }
    let file = File::open(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            "metadata file disappeared while scanning".to_owned()
        } else {
            format!("cannot read repository metadata: {error}")
        }
    })?;
    let mut bytes = Vec::new();
    file.take(MAX_METADATA_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot read repository metadata: {error}"))?;
    if bytes.len() > MAX_METADATA_BYTES {
        return Err("metadata file exceeds its size bound".to_owned());
    }
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| "metadata file is not UTF-8".to_owned())
}

fn sort_unique(values: &mut Vec<String>) {
    values.sort();
    values.dedup();
}

fn sort_evidence(values: &mut Vec<EvidenceRecord>) {
    values.sort_by(|left, right| {
        (&left.kind, &left.source, &left.detail).cmp(&(&right.kind, &right.source, &right.detail))
    });
    values.dedup();
}

#[cfg(test)]
mod tests {
    use super::{parse_lock_dependency, sanitize_url};

    #[test]
    fn lock_dependency_parser_keeps_source_qualifier() {
        let parsed = parse_lock_dependency(
            "serde 1.0.229 (registry+https://github.com/rust-lang/crates.io-index)",
        )
        .expect("Cargo lock dependency parses");
        assert_eq!(parsed.name, "serde");
        assert_eq!(parsed.version.as_deref(), Some("1.0.229"));
        assert_eq!(
            parsed.source.as_deref(),
            Some("registry+https://github.com/rust-lang/crates.io-index")
        );
        let versionless =
            parse_lock_dependency("serde").expect("versionless Cargo.lock dependency parses");
        assert_eq!(versionless.name, "serde");
        assert_eq!(versionless.version, None);
    }

    #[test]
    fn url_credentials_are_removed() {
        assert_eq!(
            sanitize_url("https://user:secret@example.com/repo.git?token=hidden"),
            Some("https://example.com/repo.git".to_owned())
        );
    }
}
