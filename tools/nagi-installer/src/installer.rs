use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use nagi_sdk::app_contract::is_valid_app_identifier;
use semver::Version;
use serde::Serialize;

use crate::error::{io_error, PolicyName};
use crate::state::{
    new_transaction_id, CapabilityDelta, InstallOperation, InstalledPackage, InventoryFile,
    InventorySnapshot, JournalOperation, TransactionJournal, TransactionPhase,
    INVENTORY_SCHEMA_VERSION, JOURNAL_SCHEMA_VERSION,
};
use crate::{InstallerError, PackageMetadata, PackageRelativePath, PackageSource};

const INVENTORY_FILE: &str = "inventory.json";
const TRANSACTIONS_DIR: &str = "transactions";
const STAGING_DIR: &str = "staging";
const PACKAGES_DIR: &str = "packages";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyCheck {
    CapabilityChanges,
    Sbom,
    License,
    Provenance,
    Trust,
    Uninstall,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyOperation {
    Install,
    Uninstall,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyDecision {
    NotApplicable,
    Allow,
    Deny,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct PolicyReferences<'a> {
    pub sbom: Option<&'a str>,
    pub license: Option<&'a str>,
    pub provenance: Option<&'a str>,
}

pub trait PolicyEvaluator {
    fn evaluate(
        &mut self,
        operation: PolicyOperation,
        check: PolicyCheck,
        app_id: &str,
        metadata: Option<&PackageMetadata>,
        delta: Option<&CapabilityDelta>,
        references: PolicyReferences<'_>,
    ) -> Result<PolicyDecision, String>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoopPolicy;

impl PolicyEvaluator for NoopPolicy {
    fn evaluate(
        &mut self,
        _operation: PolicyOperation,
        _check: PolicyCheck,
        _app_id: &str,
        _metadata: Option<&PackageMetadata>,
        _delta: Option<&CapabilityDelta>,
        _references: PolicyReferences<'_>,
    ) -> Result<PolicyDecision, String> {
        Ok(PolicyDecision::Allow)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FailurePoint {
    AfterJournalCreated,
    AfterFirstStagedFile,
    AfterStaging,
    BeforeActiveSwitch,
    AfterActiveSwitch,
    BeforeJournalCommit,
    BeforeUninstallActiveSwitch,
    AfterUninstallActiveSwitch,
    BeforeUninstallPackageCleanup,
}

pub trait FailureInjector {
    /// Return an error to simulate an interruption at a stable transaction
    /// boundary. Injected interruptions intentionally leave the journal for
    /// `Installer::recover` to process.
    fn check(&mut self, point: FailurePoint) -> Result<(), String>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoFailureInjector;

impl FailureInjector for NoFailureInjector {
    fn check(&mut self, _point: FailurePoint) -> Result<(), String> {
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct InstallPlan {
    source: PackageSource,
    operation: InstallOperation,
    previous: Option<InstalledPackage>,
    capability_delta: CapabilityDelta,
    required_policy_checks: Vec<PolicyCheck>,
    transaction_id: String,
    staging_location: String,
    active_location: String,
}

impl InstallPlan {
    pub fn app_id(&self) -> &str {
        &self.source.metadata.app_id
    }

    pub fn source_version(&self) -> &str {
        &self.source.metadata.version
    }

    pub fn current_version(&self) -> Option<&str> {
        self.previous
            .as_ref()
            .map(|package| package.version.as_str())
    }

    pub const fn operation(&self) -> InstallOperation {
        self.operation
    }

    pub fn transaction_id(&self) -> &str {
        &self.transaction_id
    }

    pub fn staged_destination(&self) -> &str {
        &self.staging_location
    }

    pub fn active_destination(&self) -> &str {
        &self.active_location
    }

    pub fn rollback_location(&self) -> Option<&str> {
        self.previous
            .as_ref()
            .map(|package| package.active_location.as_str())
    }

    pub fn capability_delta(&self) -> &CapabilityDelta {
        &self.capability_delta
    }

    pub fn required_policy_checks(&self) -> &[PolicyCheck] {
        &self.required_policy_checks
    }
}

#[derive(Clone, Debug)]
pub struct UninstallPlan {
    previous: InstalledPackage,
    transaction_id: String,
}

impl UninstallPlan {
    pub fn app_id(&self) -> &str {
        &self.previous.app_id
    }

    pub fn installed_version(&self) -> &str {
        &self.previous.version
    }

    pub fn transaction_id(&self) -> &str {
        &self.transaction_id
    }

    pub fn active_location(&self) -> &str {
        &self.previous.active_location
    }
}

pub struct Installer {
    root: PathBuf,
}

impl Installer {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, InstallerError> {
        fs::create_dir_all(root.as_ref())
            .map_err(|error| io_error("create installer root", error))?;
        let root = fs::canonicalize(root.as_ref())
            .map_err(|error| io_error("canonicalize installer root", error))?;
        let installer = Self { root };
        installer.ensure_layout()?;
        Ok(installer)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn plan_install(&self, source: PackageSource) -> Result<InstallPlan, InstallerError> {
        let _lock = self.acquire_lock()?;
        self.recover_locked()?;
        self.plan_install_locked(source)
    }

    pub fn install(&self, source: PackageSource) -> Result<InstalledPackage, InstallerError> {
        self.install_with(source, &mut NoopPolicy, &mut NoFailureInjector)
    }

    pub fn install_with(
        &self,
        source: PackageSource,
        policy: &mut dyn PolicyEvaluator,
        failures: &mut dyn FailureInjector,
    ) -> Result<InstalledPackage, InstallerError> {
        let plan = self.plan_install(source)?;
        self.execute_install_with(plan, policy, failures)
    }

    pub fn execute_install_with(
        &self,
        plan: InstallPlan,
        policy: &mut dyn PolicyEvaluator,
        failures: &mut dyn FailureInjector,
    ) -> Result<InstalledPackage, InstallerError> {
        let _lock = self.acquire_lock()?;
        self.recover_locked()?;
        let inventory = self.read_inventory()?;
        let current = inventory.packages.get(plan.app_id()).cloned();
        if current != plan.previous {
            return Err(InstallerError::ConcurrentModification(format!(
                "installed state for {} changed after the plan was created",
                plan.app_id()
            )));
        }
        self.revalidate_source(&plan.source)?;
        self.evaluate_install_policy(&plan, policy)?;
        self.execute_install_locked(plan, failures)
    }

    pub fn plan_uninstall(&self, app_id: &str) -> Result<UninstallPlan, InstallerError> {
        if !is_valid_app_identifier(app_id) {
            return Err(InstallerError::InvalidMetadata(
                "uninstall target is not a canonical SDK application identifier".into(),
            ));
        }
        let _lock = self.acquire_lock()?;
        self.recover_locked()?;
        let inventory = self.read_inventory()?;
        let previous = inventory
            .packages
            .get(app_id)
            .cloned()
            .ok_or_else(|| InstallerError::NotInstalled(app_id.to_owned()))?;
        Ok(UninstallPlan {
            previous,
            transaction_id: self.unique_transaction_id()?,
        })
    }

    pub fn uninstall(&self, app_id: &str) -> Result<crate::UninstallOutcome, InstallerError> {
        self.uninstall_with(app_id, &mut NoopPolicy, &mut NoFailureInjector)
    }

    pub fn uninstall_with(
        &self,
        app_id: &str,
        policy: &mut dyn PolicyEvaluator,
        failures: &mut dyn FailureInjector,
    ) -> Result<crate::UninstallOutcome, InstallerError> {
        let plan = self.plan_uninstall(app_id)?;
        self.execute_uninstall_with(plan, policy, failures)
    }

    pub fn execute_uninstall_with(
        &self,
        plan: UninstallPlan,
        policy: &mut dyn PolicyEvaluator,
        failures: &mut dyn FailureInjector,
    ) -> Result<crate::UninstallOutcome, InstallerError> {
        let _lock = self.acquire_lock()?;
        self.recover_locked()?;
        let inventory = self.read_inventory()?;
        if inventory.packages.get(plan.app_id()) != Some(&plan.previous) {
            return Err(InstallerError::ConcurrentModification(format!(
                "installed state for {} changed after the uninstall plan was created",
                plan.app_id()
            )));
        }
        self.evaluate_uninstall_policy(&plan, policy)?;
        self.execute_uninstall_locked(plan, failures)
    }

    pub fn inventory(&self) -> Result<InventorySnapshot, InstallerError> {
        let _lock = self.acquire_lock()?;
        self.recover_locked()?;
        Ok(self.read_inventory()?.snapshot())
    }

    pub fn installed(&self, app_id: &str) -> Result<Option<InstalledPackage>, InstallerError> {
        Ok(self.inventory()?.get(app_id).cloned())
    }

    /// Resolve a store-relative package location for a consumer. The stored
    /// inventory never contains an absolute host path.
    pub fn package_path(&self, package: &InstalledPackage) -> Result<PathBuf, InstallerError> {
        if !is_valid_app_identifier(&package.app_id) {
            return Err(InstallerError::InventoryFailure(
                "installed package has an invalid app ID".into(),
            ));
        }
        let path = PackageRelativePath::parse(&package.active_location)?;
        let expected_prefix = format!("{PACKAGES_DIR}/{}/versions/", package.app_id);
        if path.as_str() != format!("{expected_prefix}{}", package.transaction_id) {
            return Err(InstallerError::UnsafePath(format!(
                "active package location is outside its app version store: {}",
                path.as_str()
            )));
        }
        let absolute = resolve_relative(&self.root, &path);
        self.ensure_existing_path_has_no_symlinks(&absolute)?;
        Ok(absolute)
    }

    /// Recover incomplete transactions and safely remove orphan staging
    /// directories. Unknown or corrupt journals fail closed and are retained.
    pub fn recover(&self) -> Result<(), InstallerError> {
        let _lock = self.acquire_lock()?;
        self.recover_locked()
    }

    fn plan_install_locked(&self, source: PackageSource) -> Result<InstallPlan, InstallerError> {
        source.metadata.validate()?;
        self.revalidate_source(&source)?;
        let inventory = self.read_inventory()?;
        let previous = inventory.packages.get(&source.metadata.app_id).cloned();
        let operation = match &previous {
            None => InstallOperation::FreshInstall,
            Some(installed) => {
                let requested = Version::parse(&source.metadata.version).map_err(|error| {
                    InstallerError::InvalidMetadata(format!("malformed version: {error}"))
                })?;
                let current = Version::parse(&installed.version).map_err(|error| {
                    InstallerError::InventoryFailure(format!(
                        "installed version is malformed: {error}"
                    ))
                })?;
                match requested.cmp_precedence(&current) {
                    std::cmp::Ordering::Greater => InstallOperation::Upgrade,
                    std::cmp::Ordering::Equal => {
                        return Err(InstallerError::VersionConflict {
                            installed: installed.version.clone(),
                            requested: source.metadata.version.clone(),
                        });
                    }
                    std::cmp::Ordering::Less => {
                        return Err(InstallerError::DowngradeRejected {
                            installed: installed.version.clone(),
                            requested: source.metadata.version.clone(),
                        });
                    }
                }
            }
        };
        let capability_delta = CapabilityDelta::between(
            previous
                .as_ref()
                .map_or(&[], |record| record.required_capabilities.as_slice()),
            &source.metadata.required_capabilities,
        );
        let transaction_id = self.unique_transaction_id()?;
        let active_location = format!(
            "{PACKAGES_DIR}/{}/versions/{transaction_id}",
            source.metadata.app_id
        );
        Ok(InstallPlan {
            source,
            operation,
            previous,
            capability_delta,
            required_policy_checks: vec![
                PolicyCheck::CapabilityChanges,
                PolicyCheck::Sbom,
                PolicyCheck::License,
                PolicyCheck::Provenance,
                PolicyCheck::Trust,
            ],
            staging_location: format!("{STAGING_DIR}/{transaction_id}"),
            active_location,
            transaction_id,
        })
    }

    fn evaluate_install_policy(
        &self,
        plan: &InstallPlan,
        policy: &mut dyn PolicyEvaluator,
    ) -> Result<(), InstallerError> {
        let references = &plan.source.metadata.references;
        let input = PolicyReferences {
            sbom: references.sbom.as_deref(),
            license: references.license.as_deref(),
            provenance: references.provenance.as_deref(),
        };
        for check in &plan.required_policy_checks {
            evaluate_policy(
                policy,
                PolicyOperation::Install,
                *check,
                plan.app_id(),
                Some(&plan.source.metadata),
                Some(&plan.capability_delta),
                input,
            )?;
        }
        Ok(())
    }

    fn evaluate_uninstall_policy(
        &self,
        plan: &UninstallPlan,
        policy: &mut dyn PolicyEvaluator,
    ) -> Result<(), InstallerError> {
        evaluate_policy(
            policy,
            PolicyOperation::Uninstall,
            PolicyCheck::Uninstall,
            plan.app_id(),
            None,
            None,
            PolicyReferences::default(),
        )
    }

    fn execute_install_locked(
        &self,
        plan: InstallPlan,
        failures: &mut dyn FailureInjector,
    ) -> Result<InstalledPackage, InstallerError> {
        let target = InstalledPackage {
            app_id: plan.source.metadata.app_id.clone(),
            version: plan.source.metadata.version.clone(),
            active_location: plan.active_location.clone(),
            transaction_id: plan.transaction_id.clone(),
            required_capabilities: plan.source.metadata.required_capabilities.clone(),
        };
        let mut journal = TransactionJournal {
            schema_version: JOURNAL_SCHEMA_VERSION,
            transaction_id: plan.transaction_id.clone(),
            operation: JournalOperation::Install,
            app_id: target.app_id.clone(),
            prior: plan.previous.clone(),
            target: Some(target.clone()),
            staging_location: Some(plan.staging_location.clone()),
            phase: TransactionPhase::Created,
            failure: None,
        };
        self.write_journal(&journal)?;
        let transaction = (|| {
            inject(failures, FailurePoint::AfterJournalCreated)?;
            self.advance_journal(&mut journal, TransactionPhase::Validating)?;
            self.stage_package(&plan, failures)?;
            self.advance_journal(&mut journal, TransactionPhase::Staged)?;
            inject(failures, FailurePoint::AfterStaging)?;
            self.validate_staged_package(&plan)?;
            self.advance_journal(&mut journal, TransactionPhase::ReadyToCommit)?;
            self.promote_package(&plan)?;
            self.advance_journal(&mut journal, TransactionPhase::Committing)?;
            inject(failures, FailurePoint::BeforeActiveSwitch)?;
            self.replace_inventory_record(&target.app_id, Some(target.clone()))?;
            inject(failures, FailurePoint::AfterActiveSwitch)?;
            inject(failures, FailurePoint::BeforeJournalCommit)?;
            self.advance_journal(&mut journal, TransactionPhase::Committed)?;
            if self.cleanup_staging(&journal).is_ok() {
                let _ = self.remove_journal(&journal.transaction_id);
            }
            Ok(())
        })();

        if let Err(error) = transaction {
            if matches!(error, InstallerError::InjectedFailure { .. }) {
                return Err(error);
            }
            self.recover_journal(&self.journal_path(&journal.transaction_id)?)
                .map_err(|rollback| {
                    InstallerError::RollbackFailure(format!(
                        "install failed ({error}); rollback failed ({rollback})"
                    ))
                })?;
            if self.read_inventory()?.packages.get(&target.app_id) == Some(&target) {
                return Ok(target);
            }
            return Err(error);
        }
        Ok(target)
    }

    fn execute_uninstall_locked(
        &self,
        plan: UninstallPlan,
        failures: &mut dyn FailureInjector,
    ) -> Result<crate::UninstallOutcome, InstallerError> {
        let mut journal = TransactionJournal {
            schema_version: JOURNAL_SCHEMA_VERSION,
            transaction_id: plan.transaction_id.clone(),
            operation: JournalOperation::Uninstall,
            app_id: plan.previous.app_id.clone(),
            prior: Some(plan.previous.clone()),
            target: None,
            staging_location: None,
            phase: TransactionPhase::Created,
            failure: None,
        };
        self.write_journal(&journal)?;
        let transaction = (|| {
            self.advance_journal(&mut journal, TransactionPhase::Validating)?;
            self.advance_journal(&mut journal, TransactionPhase::Staged)?;
            self.advance_journal(&mut journal, TransactionPhase::ReadyToCommit)?;
            self.advance_journal(&mut journal, TransactionPhase::Committing)?;
            inject(failures, FailurePoint::BeforeUninstallActiveSwitch)?;
            self.replace_inventory_record(&plan.previous.app_id, None)?;
            inject(failures, FailurePoint::AfterUninstallActiveSwitch)?;
            inject(failures, FailurePoint::BeforeJournalCommit)?;
            self.advance_journal(&mut journal, TransactionPhase::Committed)?;
            inject(failures, FailurePoint::BeforeUninstallPackageCleanup)?;
            let package_files_removed = self
                .remove_app_packages(&plan.previous.app_id)
                .unwrap_or(false);
            if package_files_removed {
                let _ = self.remove_journal(&journal.transaction_id);
            }
            Ok(package_files_removed)
        })();

        let package_files_removed = match transaction {
            Ok(removed) => removed,
            Err(error) => {
                if matches!(error, InstallerError::InjectedFailure { .. }) {
                    return Err(error);
                }
                self.recover_journal(&self.journal_path(&journal.transaction_id)?)
                    .map_err(|rollback| {
                        InstallerError::RollbackFailure(format!(
                            "uninstall failed ({error}); rollback failed ({rollback})"
                        ))
                    })?;
                if !self
                    .read_inventory()?
                    .packages
                    .contains_key(&plan.previous.app_id)
                {
                    let package_files_removed = self
                        .remove_app_packages(&plan.previous.app_id)
                        .unwrap_or(false);
                    return Ok(crate::UninstallOutcome {
                        app_id: plan.previous.app_id,
                        removed_version: plan.previous.version,
                        package_files_removed,
                    });
                }
                return Err(error);
            }
        };
        Ok(crate::UninstallOutcome {
            app_id: plan.previous.app_id,
            removed_version: plan.previous.version,
            package_files_removed,
        })
    }

    fn stage_package(
        &self,
        plan: &InstallPlan,
        failures: &mut dyn FailureInjector,
    ) -> Result<(), InstallerError> {
        let stage_root = self.resolve_store_location(&plan.staging_location)?;
        self.create_directory_path(&stage_root)?;
        let payload = stage_root.join("payload");
        self.create_directory_path(&payload)?;
        for (index, file) in plan.source.files.iter().enumerate() {
            let contents = plan.source.read_file(file).map_err(|error| {
                InstallerError::StagingFailure(format!("cannot read package content: {error}"))
            })?;
            let destination = resolve_relative(&payload, &file.path);
            self.create_directory_path(
                destination
                    .parent()
                    .ok_or_else(|| InstallerError::UnsafePath("file has no parent".into()))?,
            )?;
            let mut output = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&destination)
                .map_err(|error| io_error("create staged package file", error))?;
            output
                .write_all(&contents)
                .map_err(|error| io_error("write staged package file", error))?;
            output
                .sync_all()
                .map_err(|error| io_error("sync staged package file", error))?;
            set_executable(&destination, file.executable)?;
            if index == 0 {
                inject(failures, FailurePoint::AfterFirstStagedFile)?;
            }
        }
        sync_directory_tree(&payload)?;
        Ok(())
    }

    fn validate_staged_package(&self, plan: &InstallPlan) -> Result<(), InstallerError> {
        let stage_root = self.resolve_store_location(&plan.staging_location)?;
        let payload = stage_root.join("payload");
        self.ensure_existing_path_has_no_symlinks(&payload)?;
        let staged = PackageSource::from_directory(&payload, plan.source.metadata.clone())
            .map_err(|error| InstallerError::StagingFailure(error.to_string()))?;
        if staged.files != plan.source.files {
            return Err(InstallerError::StagingFailure(
                "staged package file list or metadata differs from the validated source".into(),
            ));
        }
        Ok(())
    }

    fn promote_package(&self, plan: &InstallPlan) -> Result<(), InstallerError> {
        let source = self.resolve_store_location(&format!("{}/payload", plan.staging_location))?;
        let destination = self.resolve_store_location(&plan.active_location)?;
        self.ensure_existing_path_has_no_symlinks(&source)?;
        self.ensure_existing_path_has_no_symlinks(&destination)?;
        let parent = destination.parent().ok_or_else(|| {
            InstallerError::UnsafePath("package destination has no parent".into())
        })?;
        self.create_directory_path(parent)?;
        if path_exists(&destination)? {
            return Err(InstallerError::CommitFailure(
                "package transaction destination already exists".into(),
            ));
        }
        fs::rename(&source, &destination)
            .map_err(|error| InstallerError::CommitFailure(error.to_string()))?;
        sync_directory(parent)?;
        Ok(())
    }

    fn revalidate_source(&self, source: &PackageSource) -> Result<(), InstallerError> {
        source.metadata.validate()?;
        let refreshed = match source.directory_root() {
            Some(root) => PackageSource::from_directory(root, source.metadata.clone())?,
            None => return Ok(()),
        };
        if refreshed.files != source.files {
            return Err(InstallerError::InvalidPackage(
                "package directory changed after source validation".into(),
            ));
        }
        Ok(())
    }

    fn read_inventory(&self) -> Result<InventoryFile, InstallerError> {
        let path = self.root.join(INVENTORY_FILE);
        if !path_exists(&path)? {
            return Ok(InventoryFile::default());
        }
        self.ensure_existing_path_has_no_symlinks(&path)?;
        let contents =
            fs::read(&path).map_err(|error| InstallerError::InventoryFailure(error.to_string()))?;
        let inventory: InventoryFile = serde_json::from_slice(&contents).map_err(|error| {
            InstallerError::InventoryFailure(format!("invalid inventory: {error}"))
        })?;
        if inventory.schema_version != INVENTORY_SCHEMA_VERSION {
            return Err(InstallerError::UnsupportedSchema {
                kind: "installed package inventory",
                version: inventory.schema_version,
            });
        }
        for (app_id, package) in &inventory.packages {
            if app_id != &package.app_id
                || !is_valid_app_identifier(app_id)
                || Version::parse(&package.version).is_err()
                || !valid_transaction_id(&package.transaction_id)
                || package.transaction_id != final_location_component(&package.active_location)
                || package.active_location
                    != format!(
                        "{PACKAGES_DIR}/{app_id}/versions/{}",
                        package.transaction_id
                    )
            {
                return Err(InstallerError::InventoryFailure(format!(
                    "invalid inventory record for {app_id}"
                )));
            }
            let path = PackageRelativePath::parse(&package.active_location)?;
            let expected = format!("{PACKAGES_DIR}/{app_id}/versions/");
            if !path.as_str().starts_with(&expected) {
                return Err(InstallerError::InventoryFailure(format!(
                    "inventory location escapes app package storage: {}",
                    path.as_str()
                )));
            }
            if package.required_capabilities.iter().any(String::is_empty)
                || package
                    .required_capabilities
                    .iter()
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    != package.required_capabilities.len()
            {
                return Err(InstallerError::InventoryFailure(format!(
                    "invalid capability list for {app_id}"
                )));
            }
        }
        Ok(inventory)
    }

    fn replace_inventory_record(
        &self,
        app_id: &str,
        package: Option<InstalledPackage>,
    ) -> Result<(), InstallerError> {
        let mut inventory = self.read_inventory()?;
        inventory
            .update(app_id, package)
            .map_err(|error| InstallerError::InventoryFailure(error.to_string()))?;
        self.write_inventory(&inventory)
    }

    fn write_inventory(&self, inventory: &InventoryFile) -> Result<(), InstallerError> {
        let destination = self.root.join(INVENTORY_FILE);
        self.write_json_atomically(&destination, inventory)
            .map_err(|error| InstallerError::InventoryFailure(error.to_string()))
    }

    fn write_journal(&self, journal: &TransactionJournal) -> Result<(), InstallerError> {
        let destination = self.journal_path(&journal.transaction_id)?;
        self.write_json_atomically(&destination, journal)
            .map_err(|error| InstallerError::RecoveryFailure(error.to_string()))
    }

    fn advance_journal(
        &self,
        journal: &mut TransactionJournal,
        next: TransactionPhase,
    ) -> Result<(), InstallerError> {
        journal.transition(next)?;
        self.write_journal(journal)
    }

    fn write_json_atomically<T: Serialize>(
        &self,
        destination: &Path,
        value: &T,
    ) -> Result<(), InstallerError> {
        if let Some(parent) = destination.parent() {
            self.create_directory_path(parent)?;
        }
        self.ensure_existing_path_has_no_symlinks(destination)?;
        let sequence = new_transaction_id();
        let name = destination
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| InstallerError::UnsafePath("state file name is not UTF-8".into()))?;
        let temporary = destination.with_file_name(format!(".{name}.tmp-{sequence}"));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|error| io_error("create atomic state temporary", error))?;
        serde_json::to_writer(&mut file, value)
            .map_err(|error| InstallerError::InventoryFailure(error.to_string()))?;
        file.write_all(b"\n")
            .map_err(|error| io_error("finish atomic state temporary", error))?;
        file.sync_all()
            .map_err(|error| io_error("sync atomic state temporary", error))?;
        replace_file(&temporary, destination)
            .map_err(|error| io_error("atomically replace state file", error))?;
        if let Some(parent) = destination.parent() {
            sync_directory(parent)?;
        }
        Ok(())
    }

    fn recover_locked(&self) -> Result<(), InstallerError> {
        let directory = self.root.join(TRANSACTIONS_DIR);
        let mut journal_paths = fs::read_dir(&directory)
            .map_err(|error| io_error("enumerate transaction journals", error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| io_error("read transaction journal entry", error))?
            .into_iter()
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect::<Vec<_>>();
        journal_paths.sort();
        for path in journal_paths {
            self.recover_journal(&path)?;
        }
        self.cleanup_orphan_staging()
    }

    fn recover_journal(&self, path: &Path) -> Result<(), InstallerError> {
        self.ensure_existing_path_has_no_symlinks(path)?;
        let contents =
            fs::read(path).map_err(|error| InstallerError::RecoveryFailure(error.to_string()))?;
        let mut journal: TransactionJournal =
            serde_json::from_slice(&contents).map_err(|error| {
                InstallerError::RecoveryFailure(format!("invalid transaction marker: {error}"))
            })?;
        if journal.schema_version != JOURNAL_SCHEMA_VERSION {
            return Err(InstallerError::UnsupportedSchema {
                kind: "installer transaction journal",
                version: journal.schema_version,
            });
        }
        if !valid_transaction_id(&journal.transaction_id)
            || path.file_stem().and_then(|stem| stem.to_str()) != Some(&journal.transaction_id)
            || !is_valid_app_identifier(&journal.app_id)
        {
            return Err(InstallerError::RecoveryFailure(
                "transaction marker identity does not match its path".into(),
            ));
        }
        if let Some(location) = &journal.staging_location {
            let expected = format!("{STAGING_DIR}/{}", journal.transaction_id);
            if location != &expected {
                return Err(InstallerError::UnsafePath(
                    "transaction marker contains an invalid staging location".into(),
                ));
            }
        }
        self.validate_journal_records(&journal)?;

        if journal.phase.is_committed() {
            return self.finish_committed(&journal, path);
        }
        if journal.phase == TransactionPhase::RolledBack {
            self.cleanup_journal_artifacts(&journal)?;
            return self.remove_journal(&journal.transaction_id);
        }

        if journal.phase != TransactionPhase::Failed
            && journal.phase != TransactionPhase::RollingBack
        {
            if journal.phase != TransactionPhase::Committing {
                journal.transition(TransactionPhase::Failed)?;
                self.write_journal(&journal)?;
            }
            journal.transition(TransactionPhase::RollingBack)?;
            self.write_journal(&journal)?;
        } else if journal.phase == TransactionPhase::Failed {
            journal.transition(TransactionPhase::RollingBack)?;
            self.write_journal(&journal)?;
        }

        self.restore_prior_inventory(&journal)?;
        self.cleanup_journal_artifacts(&journal)?;
        if journal.phase != TransactionPhase::RolledBack {
            journal.transition(TransactionPhase::RolledBack)?;
            self.write_journal(&journal)?;
        }
        self.remove_journal(&journal.transaction_id)
    }

    fn validate_journal_records(&self, journal: &TransactionJournal) -> Result<(), InstallerError> {
        if let Some(record) = &journal.prior {
            self.validate_installed_record(&journal.app_id, record)?;
        }
        if let Some(record) = &journal.target {
            self.validate_installed_record(&journal.app_id, record)?;
        }
        match journal.operation {
            JournalOperation::Install if journal.target.is_none() => Err(
                InstallerError::RecoveryFailure("install journal has no target record".into()),
            ),
            JournalOperation::Uninstall if journal.prior.is_none() || journal.target.is_some() => {
                Err(InstallerError::RecoveryFailure(
                    "uninstall journal has an invalid record transition".into(),
                ))
            }
            _ => Ok(()),
        }
    }

    fn validate_installed_record(
        &self,
        app_id: &str,
        package: &InstalledPackage,
    ) -> Result<(), InstallerError> {
        if package.app_id != app_id
            || !is_valid_app_identifier(&package.app_id)
            || Version::parse(&package.version).is_err()
            || !valid_transaction_id(&package.transaction_id)
            || package.transaction_id != final_location_component(&package.active_location)
            || package.active_location
                != format!(
                    "{PACKAGES_DIR}/{app_id}/versions/{}",
                    package.transaction_id
                )
        {
            return Err(InstallerError::RecoveryFailure(
                "transaction marker contains an invalid package record".into(),
            ));
        }
        let path = PackageRelativePath::parse(&package.active_location)?;
        let expected = format!("{PACKAGES_DIR}/{app_id}/versions/");
        if !path.as_str().starts_with(&expected) {
            return Err(InstallerError::UnsafePath(
                "transaction marker package location escapes managed storage".into(),
            ));
        }
        Ok(())
    }

    fn restore_prior_inventory(&self, journal: &TransactionJournal) -> Result<(), InstallerError> {
        let mut inventory = self.read_inventory()?;
        let current = inventory.packages.get(&journal.app_id);
        if current == journal.prior.as_ref() {
            return Ok(());
        }
        if current != journal.target.as_ref() {
            return Err(InstallerError::RecoveryFailure(format!(
                "cannot recover {}: inventory contains an unexpected active record",
                journal.app_id
            )));
        }
        inventory.update(&journal.app_id, journal.prior.clone())?;
        self.write_inventory(&inventory)
    }

    fn finish_committed(
        &self,
        journal: &TransactionJournal,
        path: &Path,
    ) -> Result<(), InstallerError> {
        let inventory = self.read_inventory()?;
        let cleanup_complete = match journal.operation {
            JournalOperation::Install => {
                if inventory.packages.get(&journal.app_id) != journal.target.as_ref() {
                    return Err(InstallerError::RecoveryFailure(format!(
                        "committed install {} is not present in inventory",
                        journal.app_id
                    )));
                }
                self.cleanup_staging(journal)?;
                true
            }
            JournalOperation::Uninstall => {
                if inventory.packages.contains_key(&journal.app_id) {
                    return Err(InstallerError::RecoveryFailure(format!(
                        "committed uninstall {} is still active in inventory",
                        journal.app_id
                    )));
                }
                if let Some(previous) = &journal.prior {
                    self.remove_app_packages(&previous.app_id)?
                } else {
                    true
                }
            }
        };
        if !cleanup_complete {
            return Ok(());
        }
        let filename = path.file_name().ok_or_else(|| {
            InstallerError::UnsafePath("transaction journal path has no file name".into())
        })?;
        fs::remove_file(self.root.join(TRANSACTIONS_DIR).join(filename))
            .map_err(|error| io_error("remove completed transaction journal", error))?;
        sync_directory(&self.root.join(TRANSACTIONS_DIR))
    }

    fn cleanup_journal_artifacts(
        &self,
        journal: &TransactionJournal,
    ) -> Result<(), InstallerError> {
        if let Some(target) = &journal.target {
            if journal.prior.as_ref() != Some(target) {
                self.remove_package_location(&target.active_location)?;
            }
        }
        self.cleanup_staging(journal)
    }

    fn cleanup_staging(&self, journal: &TransactionJournal) -> Result<(), InstallerError> {
        let Some(location) = &journal.staging_location else {
            return Ok(());
        };
        let path = self.resolve_store_location(location)?;
        self.ensure_existing_path_has_no_symlinks(&path)?;
        remove_directory_if_present(&path)
    }

    fn cleanup_orphan_staging(&self) -> Result<(), InstallerError> {
        let directory = self.root.join(STAGING_DIR);
        let mut entries = fs::read_dir(&directory)
            .map_err(|error| io_error("enumerate staging directory", error))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| io_error("read staging entry", error))?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                continue;
            };
            if valid_transaction_id(&name) {
                self.ensure_existing_path_has_no_symlinks(&entry.path())?;
                remove_directory_if_present(&entry.path())?;
            }
        }
        Ok(())
    }

    fn remove_package_location(&self, location: &str) -> Result<(), InstallerError> {
        let path = PackageRelativePath::parse(location)?;
        if !path.as_str().starts_with(&format!("{PACKAGES_DIR}/")) {
            return Err(InstallerError::UnsafePath(
                "refusing to remove outside package storage".into(),
            ));
        }
        let absolute = resolve_relative(&self.root, &path);
        self.ensure_existing_path_has_no_symlinks(&absolute)?;
        remove_directory_if_present(&absolute)
    }

    fn remove_app_packages(&self, app_id: &str) -> Result<bool, InstallerError> {
        if !is_valid_app_identifier(app_id) {
            return Err(InstallerError::UnsafePath(
                "refusing to clean package data for an invalid app ID".into(),
            ));
        }
        let app_root = self.root.join(PACKAGES_DIR).join(app_id);
        let versions = app_root.join("versions");
        self.ensure_existing_path_has_no_symlinks(&versions)?;
        let mut entries = match fs::read_dir(&versions) {
            Ok(entries) => entries
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| io_error("read installed package version", error))?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return remove_empty_app_directory(&app_root)
            }
            Err(error) => return Err(io_error("enumerate installed versions", error)),
        };
        entries.sort_by_key(|entry| entry.file_name());
        let mut complete = true;
        for entry in entries {
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                complete = false;
                continue;
            };
            if !valid_transaction_id(&name) {
                // Preserve unknown content rather than treating it as a
                // package directory owned by this installer.
                complete = false;
                continue;
            }
            self.ensure_existing_path_has_no_symlinks(&entry.path())?;
            remove_directory_if_present(&entry.path())?;
        }
        if complete {
            fs::remove_dir(&versions)
                .map_err(|error| io_error("remove empty package version directory", error))?;
            return remove_empty_app_directory(&app_root);
        }
        Ok(complete)
    }

    fn remove_journal(&self, transaction_id: &str) -> Result<(), InstallerError> {
        let path = self.journal_path(transaction_id)?;
        if path_exists(&path)? {
            self.ensure_existing_path_has_no_symlinks(&path)?;
            fs::remove_file(&path)
                .map_err(|error| io_error("remove transaction journal", error))?;
            sync_directory(&self.root.join(TRANSACTIONS_DIR))?;
        }
        Ok(())
    }

    fn journal_path(&self, transaction_id: &str) -> Result<PathBuf, InstallerError> {
        if !valid_transaction_id(transaction_id) {
            return Err(InstallerError::UnsafePath("invalid transaction ID".into()));
        }
        Ok(self
            .root
            .join(TRANSACTIONS_DIR)
            .join(format!("{transaction_id}.json")))
    }

    fn unique_transaction_id(&self) -> Result<String, InstallerError> {
        for _ in 0..16 {
            let id = new_transaction_id();
            if !path_exists(&self.journal_path(&id)?)?
                && !path_exists(&self.root.join(STAGING_DIR).join(&id))?
            {
                return Ok(id);
            }
        }
        Err(InstallerError::CommitFailure(
            "could not allocate a unique transaction ID".into(),
        ))
    }

    fn resolve_store_location(&self, location: &str) -> Result<PathBuf, InstallerError> {
        let relative = PackageRelativePath::parse(location)?;
        Ok(resolve_relative(&self.root, &relative))
    }

    fn ensure_layout(&self) -> Result<(), InstallerError> {
        for name in [TRANSACTIONS_DIR, STAGING_DIR, PACKAGES_DIR] {
            self.create_directory_path(&self.root.join(name))?;
        }
        Ok(())
    }

    fn create_directory_path(&self, path: &Path) -> Result<(), InstallerError> {
        let relative = path.strip_prefix(&self.root).map_err(|_| {
            InstallerError::UnsafePath(format!(
                "refusing to create a directory outside installer root: {}",
                path.display()
            ))
        })?;
        let mut current = self.root.clone();
        for component in relative.components() {
            let segment = component.as_os_str();
            if segment.is_empty() || segment == "." || segment == ".." {
                return Err(InstallerError::UnsafePath(
                    "invalid installer directory component".into(),
                ));
            }
            current.push(segment);
            match fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                    return Err(InstallerError::UnsafePath(format!(
                        "installer directory is a symlink or non-directory: {}",
                        current.display()
                    )));
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    fs::create_dir(&current)
                        .map_err(|error| io_error("create installer directory", error))?;
                }
                Err(error) => return Err(io_error("inspect installer directory", error)),
            }
        }
        Ok(())
    }

    fn ensure_existing_path_has_no_symlinks(&self, path: &Path) -> Result<(), InstallerError> {
        let relative = path.strip_prefix(&self.root).map_err(|_| {
            InstallerError::UnsafePath(format!(
                "path is outside installer root: {}",
                path.display()
            ))
        })?;
        let mut current = self.root.clone();
        for component in relative.components() {
            current.push(component.as_os_str());
            match fs::symlink_metadata(&current) {
                Ok(metadata) if metadata.file_type().is_symlink() => {
                    return Err(InstallerError::UnsafePath(format!(
                        "managed path contains a symlink: {}",
                        current.display()
                    )));
                }
                Ok(_) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(io_error("inspect managed path", error)),
            }
        }
        Ok(())
    }

    fn acquire_lock(&self) -> Result<File, InstallerError> {
        let path = self.root.join(".installer.lock");
        self.ensure_existing_path_has_no_symlinks(&path)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .map_err(|error| io_error("open installer lock", error))?;
        lock.lock()
            .map_err(|error| io_error("acquire installer lock", error))?;
        Ok(lock)
    }
}

fn evaluate_policy(
    policy: &mut dyn PolicyEvaluator,
    operation: PolicyOperation,
    check: PolicyCheck,
    app_id: &str,
    metadata: Option<&PackageMetadata>,
    delta: Option<&CapabilityDelta>,
    references: PolicyReferences<'_>,
) -> Result<(), InstallerError> {
    let name = match check {
        PolicyCheck::CapabilityChanges => PolicyName::CapabilityChanges,
        PolicyCheck::Sbom => PolicyName::Sbom,
        PolicyCheck::License => PolicyName::License,
        PolicyCheck::Provenance => PolicyName::Provenance,
        PolicyCheck::Trust => PolicyName::Trust,
        PolicyCheck::Uninstall => PolicyName::Uninstall,
    };
    match policy
        .evaluate(operation, check, app_id, metadata, delta, references)
        .map_err(|message| InstallerError::PolicyFailure {
            policy: name,
            message,
        })? {
        PolicyDecision::Allow | PolicyDecision::NotApplicable => Ok(()),
        PolicyDecision::Deny => Err(InstallerError::PolicyDenied(name)),
    }
}

fn inject(failures: &mut dyn FailureInjector, point: FailurePoint) -> Result<(), InstallerError> {
    failures
        .check(point)
        .map_err(|message| InstallerError::InjectedFailure {
            point: format!("{point:?}"),
            message,
        })
}

fn resolve_relative(root: &Path, relative: &PackageRelativePath) -> PathBuf {
    relative
        .as_str()
        .split('/')
        .fold(root.to_path_buf(), |path, segment| path.join(segment))
}

fn final_location_component(location: &str) -> &str {
    location.rsplit('/').next().unwrap_or_default()
}

fn valid_transaction_id(id: &str) -> bool {
    let mut components = id.split('-');
    let Some(timestamp) = components.next() else {
        return false;
    };
    let Some(process) = components.next() else {
        return false;
    };
    let Some(sequence) = components.next() else {
        return false;
    };
    components.next().is_none()
        && timestamp.len() == 32
        && timestamp.bytes().all(|byte| byte.is_ascii_hexdigit())
        && process.len() == 8
        && process.bytes().all(|byte| byte.is_ascii_hexdigit())
        && sequence.len() == 16
        && sequence.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn path_exists(path: &Path) -> Result<bool, InstallerError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(io_error("inspect path", error)),
    }
}

fn remove_directory_if_present(path: &Path) -> Result<(), InstallerError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(InstallerError::UnsafePath(
            format!("refusing to remove a managed symlink: {}", path.display()),
        )),
        Ok(metadata) if metadata.is_dir() => {
            fs::remove_dir_all(path).map_err(|error| io_error("remove managed directory", error))
        }
        Ok(_) => Err(InstallerError::UnsafePath(format!(
            "managed directory path is not a directory: {}",
            path.display()
        ))),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error("inspect managed directory", error)),
    }
}

fn remove_empty_app_directory(path: &Path) -> Result<bool, InstallerError> {
    match fs::remove_dir(path) {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::DirectoryNotEmpty => Ok(false),
        Err(error) => Err(io_error("remove empty app package directory", error)),
    }
}

fn set_executable(path: &Path, executable: bool) -> Result<(), InstallerError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = if executable { 0o755 } else { 0o644 };
        let mut permissions = fs::metadata(path)
            .map_err(|error| io_error("inspect staged permissions", error))?
            .permissions();
        permissions.set_mode(mode);
        fs::set_permissions(path, permissions)
            .map_err(|error| io_error("set staged permissions", error))?;
    }
    #[cfg(not(unix))]
    let _ = (path, executable);
    Ok(())
}

fn sync_directory(path: &Path) -> Result<(), InstallerError> {
    #[cfg(unix)]
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| io_error("sync installer directory", error))?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn sync_directory_tree(path: &Path) -> Result<(), InstallerError> {
    let mut entries = fs::read_dir(path)
        .map_err(|error| io_error("enumerate staged directories", error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| io_error("read staged entry", error))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        if entry
            .file_type()
            .map_err(|error| io_error("inspect staged entry", error))?
            .is_dir()
        {
            sync_directory_tree(&entry.path())?;
        }
    }
    sync_directory(path)
}

fn replace_file(source: &Path, destination: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
        const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
        #[link(name = "Kernel32")]
        unsafe extern "system" {
            fn MoveFileExW(existing: *const u16, new: *const u16, flags: u32) -> i32;
        }
        let source = source
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        let destination = destination
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect::<Vec<_>>();
        // Both files are created below the same installer root, so replacement
        // stays on one volume and the destination pointer changes in one call.
        let result = unsafe {
            MoveFileExW(
                source.as_ptr(),
                destination.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if result == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        fs::rename(source, destination)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        evaluate_policy, valid_transaction_id, FailurePoint, InstallOperation, Installer,
        PolicyCheck, PolicyDecision, PolicyEvaluator, PolicyOperation, PolicyReferences,
    };
    use crate::state::{CapabilityDelta, TransactionPhase};
    use crate::{
        AppSdkManifestAdapter, FailureInjector, InstallerError, ManifestAdapter, NoopPolicy,
        PackageMetadata, PackageSource, VirtualFile,
    };
    use nagi_sdk::app_contract::AppStateVersion;
    use nagi_sdk::app_contract::{
        AppEntrypoint, AppIdentity, AppManifestContract, AppOrigin, CapabilityRequest, DisplayName,
        EntrypointKind, StateCompatibility, EN_US, SDK_CONTRACT_VERSION,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_ROOT: AtomicU64 = AtomicU64::new(1);

    struct TestRoot(PathBuf);

    impl TestRoot {
        fn new() -> Self {
            let id = NEXT_ROOT.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("nagi-installer-test-{}-{id}", std::process::id()));
            let _ = fs::remove_dir_all(&path);
            fs::create_dir_all(&path).expect("create test root");
            Self(path)
        }
    }

    impl Drop for TestRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn metadata(version: &str, capabilities: &[&str]) -> PackageMetadata {
        let requests = capabilities
            .iter()
            .map(|id| CapabilityRequest {
                id,
                purpose_key: None,
            })
            .collect::<Vec<_>>();
        let manifest = AppManifestContract {
            schema_version: 1,
            sdk_contract_version: SDK_CONTRACT_VERSION,
            identity: AppIdentity::new(
                "com.nagi.installer-fixture",
                version,
                AppOrigin::ThirdParty,
                None,
            )
            .expect("valid identity"),
            display_name: DisplayName {
                en_us: "Installer Fixture",
                ja_jp: None,
            },
            supported_locales: &[EN_US],
            icon: None,
            entrypoint: AppEntrypoint {
                kind: EntrypointKind::Native,
                target: "bin/app.napp",
            },
            resources: &[],
            intents: &[],
            requested_capabilities: &requests,
            background_services: &[],
            state: StateCompatibility {
                current: AppStateVersion(1),
                minimum_readable: AppStateVersion(1),
            },
        };
        AppSdkManifestAdapter
            .adapt(&manifest)
            .expect("valid package metadata")
    }

    fn source(version: &str, capabilities: &[&str], content: &[u8]) -> PackageSource {
        PackageSource::from_virtual_files(
            metadata(version, capabilities),
            vec![VirtualFile {
                path: "bin/app.napp".into(),
                contents: content.to_vec(),
                executable: true,
            }],
        )
        .expect("valid package source")
    }

    #[derive(Default)]
    struct FailAt(Option<FailurePoint>);

    impl FailureInjector for FailAt {
        fn check(&mut self, point: FailurePoint) -> Result<(), String> {
            if self.0 == Some(point) {
                self.0 = None;
                Err("simulated power loss".into())
            } else {
                Ok(())
            }
        }
    }

    #[derive(Default)]
    struct RecordingPolicy {
        checks: Vec<PolicyCheck>,
        deny: Option<PolicyCheck>,
        fail: Option<PolicyCheck>,
    }

    impl PolicyEvaluator for RecordingPolicy {
        fn evaluate(
            &mut self,
            _operation: PolicyOperation,
            check: PolicyCheck,
            _app_id: &str,
            _metadata: Option<&PackageMetadata>,
            _delta: Option<&CapabilityDelta>,
            _references: PolicyReferences<'_>,
        ) -> Result<PolicyDecision, String> {
            self.checks.push(check);
            if self.fail == Some(check) {
                return Err("validator unavailable".into());
            }
            if self.deny == Some(check) {
                return Ok(PolicyDecision::Deny);
            }
            Ok(PolicyDecision::Allow)
        }
    }

    fn app_content(installer: &Installer, app_id: &str) -> Vec<u8> {
        let installed = installer
            .installed(app_id)
            .expect("read inventory")
            .expect("installed package");
        fs::read(
            installer
                .package_path(&installed)
                .expect("package path")
                .join("bin/app.napp"),
        )
        .expect("read installed app")
    }

    #[test]
    fn sdk_manifest_adapter_uses_canonical_identity_and_version_contract() {
        let adapted = metadata("1.2.0", &["storage.read", "network.fetch"]);
        assert_eq!(adapted.app_id, "com.nagi.installer-fixture");
        assert_eq!(adapted.version, "1.2.0");
        assert_eq!(
            adapted.required_capabilities,
            ["network.fetch", "storage.read"]
        );
        assert_eq!(adapted.entrypoint, "bin/app.napp");
    }

    #[test]
    fn install_plan_is_read_only_and_fresh_install_commits_inventory_atomically() {
        let root = TestRoot::new();
        let installer = Installer::open(&root.0).expect("open installer");
        let package = source("1.0.0", &["storage.read"], b"version one");
        let plan = installer
            .plan_install(package.clone())
            .expect("plan install");
        assert_eq!(plan.operation(), InstallOperation::FreshInstall);
        assert_eq!(plan.current_version(), None);
        assert!(!root.0.join("inventory.json").exists());

        let installed = installer.install(package).expect("install package");
        assert_eq!(installed.version, "1.0.0");
        assert_eq!(app_content(&installer, &installed.app_id), b"version one");
        let inventory = fs::read_to_string(root.0.join("inventory.json")).expect("inventory");
        assert!(!inventory.contains(root.0.to_str().expect("utf8 test root")));
        assert_eq!(installer.inventory().expect("enumerate").len(), 1);
    }

    #[test]
    fn upgrades_compare_semver_and_expose_sorted_capability_delta() {
        let root = TestRoot::new();
        let installer = Installer::open(&root.0).unwrap();
        installer
            .install(source("1.9.0", &["storage.read", "files.read"], b"old"))
            .unwrap();
        let plan = installer
            .plan_install(source("1.10.0", &["storage.read", "network.fetch"], b"new"))
            .unwrap();
        assert_eq!(plan.operation(), InstallOperation::Upgrade);
        assert_eq!(plan.current_version(), Some("1.9.0"));
        assert_eq!(plan.capability_delta().unchanged, ["storage.read"]);
        assert_eq!(plan.capability_delta().added, ["network.fetch"]);
        assert_eq!(plan.capability_delta().removed, ["files.read"]);
        installer
            .execute_install_with(
                plan,
                &mut RecordingPolicy::default(),
                &mut FailAt::default(),
            )
            .unwrap();
        assert_eq!(
            app_content(&installer, "com.nagi.installer-fixture"),
            b"new"
        );
    }

    #[test]
    fn reinstall_downgrade_malformed_version_and_incompatible_schema_are_rejected() {
        let root = TestRoot::new();
        let installer = Installer::open(&root.0).unwrap();
        installer.install(source("2.0.0", &[], b"stable")).unwrap();
        assert!(matches!(
            installer.plan_install(source("2.0.0+different-build", &[], b"same precedence")),
            Err(InstallerError::VersionConflict { .. })
        ));
        assert!(matches!(
            installer.plan_install(source("1.9.9", &[], b"older")),
            Err(InstallerError::DowngradeRejected { .. })
        ));

        let mut invalid = metadata("2.1.0", &[]);
        invalid.version = "2.1".into();
        let bad_version = PackageSource::from_virtual_files(
            invalid,
            vec![VirtualFile {
                path: "bin/app.napp".into(),
                contents: b"bad".to_vec(),
                executable: true,
            }],
        );
        assert!(matches!(
            bad_version,
            Err(InstallerError::InvalidMetadata(_))
        ));

        let mut invalid = metadata("2.1.0", &[]);
        invalid.manifest_schema_version = 2;
        let bad_schema = PackageSource::from_virtual_files(
            invalid,
            vec![VirtualFile {
                path: "bin/app.napp".into(),
                contents: b"bad".to_vec(),
                executable: true,
            }],
        );
        assert!(matches!(
            bad_schema,
            Err(InstallerError::UnsupportedSchema { .. })
        ));
        assert_eq!(
            app_content(&installer, "com.nagi.installer-fixture"),
            b"stable"
        );
    }

    #[test]
    fn unsafe_virtual_paths_and_destination_conflicts_are_rejected() {
        for unsafe_path in [
            "/absolute",
            "../escape",
            "bin/../../escape",
            "C:/drive",
            "CON.txt",
        ] {
            assert!(matches!(
                PackageSource::from_virtual_files(
                    metadata("1.0.0", &[]),
                    vec![VirtualFile {
                        path: unsafe_path.into(),
                        contents: b"bad".to_vec(),
                        executable: false,
                    }]
                ),
                Err(InstallerError::UnsafePath(_))
            ));
        }
        assert!(matches!(
            PackageSource::from_virtual_files(
                metadata("1.0.0", &[]),
                vec![
                    VirtualFile {
                        path: "bin/app.napp".into(),
                        contents: b"app".to_vec(),
                        executable: true,
                    },
                    VirtualFile {
                        path: "bin/APP.NAPP".into(),
                        contents: b"other".to_vec(),
                        executable: false,
                    },
                ]
            ),
            Err(InstallerError::DuplicateDestination(_))
        ));
        assert!(matches!(
            PackageSource::from_virtual_files(
                metadata("1.0.0", &[]),
                vec![
                    VirtualFile {
                        path: "BIN".into(),
                        contents: b"file".to_vec(),
                        executable: false,
                    },
                    VirtualFile {
                        path: "bin/app.napp".into(),
                        contents: b"app".to_vec(),
                        executable: true,
                    },
                ]
            ),
            Err(InstallerError::DuplicateDestination(_))
        ));
        assert!(matches!(
            PackageSource::from_virtual_files(
                metadata("1.0.0", &[]),
                vec![
                    VirtualFile {
                        path: "bin".into(),
                        contents: b"file".to_vec(),
                        executable: false,
                    },
                    VirtualFile {
                        path: "bin/app.napp".into(),
                        contents: b"app".to_vec(),
                        executable: true,
                    },
                ]
            ),
            Err(InstallerError::DuplicateDestination(_))
        ));
    }

    #[test]
    fn directory_source_rejects_symlinks() {
        let root = TestRoot::new();
        let package_root = root.0.join("source");
        fs::create_dir_all(package_root.join("bin")).unwrap();
        fs::write(package_root.join("bin/app.napp"), b"app").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(root.0.join("outside"), package_root.join("escape")).unwrap();
        let result = PackageSource::from_directory(&package_root, metadata("1.0.0", &[]));
        #[cfg(unix)]
        assert!(matches!(result, Err(InstallerError::UnsafePath(_))));
        #[cfg(not(unix))]
        assert!(result.is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn directory_source_rejects_non_utf8_names_when_host_filesystem_allows_them() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let root = TestRoot::new();
        let package_root = root.0.join("source");
        fs::create_dir_all(package_root.join("bin")).unwrap();
        fs::write(package_root.join("bin/app.napp"), b"app").unwrap();
        let invalid_name = package_root.join(OsStr::from_bytes(b"bad-\xff-name"));
        if fs::write(invalid_name, b"bad").is_err() {
            // Some macOS filesystems reject invalid UTF-8 names at creation.
            return;
        }
        assert!(matches!(
            PackageSource::from_directory(&package_root, metadata("1.0.0", &[])),
            Err(InstallerError::UnsafePath(_))
        ));
    }

    #[test]
    fn staging_and_pre_switch_failures_leave_active_version_unchanged() {
        let root = TestRoot::new();
        let installer = Installer::open(&root.0).unwrap();
        installer.install(source("1.0.0", &[], b"old")).unwrap();
        for point in [
            FailurePoint::AfterFirstStagedFile,
            FailurePoint::BeforeActiveSwitch,
        ] {
            let plan = installer
                .plan_install(source("2.0.0", &[], b"new"))
                .unwrap();
            let result =
                installer.execute_install_with(plan, &mut NoopPolicy, &mut FailAt(Some(point)));
            assert!(matches!(
                result,
                Err(InstallerError::InjectedFailure { .. })
            ));
            installer
                .recover()
                .expect("rollback interrupted transaction");
            assert_eq!(
                app_content(&installer, "com.nagi.installer-fixture"),
                b"old"
            );
            assert_eq!(installer.inventory().unwrap().len(), 1);
            assert_eq!(
                fs::read_dir(root.0.join("transactions")).unwrap().count(),
                0
            );
        }
        assert_eq!(fs::read_dir(root.0.join("staging")).unwrap().count(), 0);
    }

    #[test]
    fn interruption_after_inventory_switch_restores_previous_version_on_recovery() {
        let root = TestRoot::new();
        let installer = Installer::open(&root.0).unwrap();
        let previous = installer
            .install(source("1.0.0", &[], b"known good"))
            .unwrap();
        let plan = installer
            .plan_install(source("2.0.0", &[], b"candidate"))
            .unwrap();
        let result = installer.execute_install_with(
            plan,
            &mut NoopPolicy,
            &mut FailAt(Some(FailurePoint::AfterActiveSwitch)),
        );
        assert!(matches!(
            result,
            Err(InstallerError::InjectedFailure { .. })
        ));
        let interrupted_inventory: serde_json::Value =
            serde_json::from_slice(&fs::read(root.0.join("inventory.json")).unwrap()).unwrap();
        assert_eq!(
            interrupted_inventory["packages"][&previous.app_id]["version"],
            "2.0.0"
        );
        installer.recover().expect("restore old inventory pointer");
        assert_eq!(app_content(&installer, &previous.app_id), b"known good");
        assert_eq!(
            installer.installed(&previous.app_id).unwrap(),
            Some(previous)
        );
        assert_eq!(
            fs::read_dir(root.0.join("transactions")).unwrap().count(),
            0
        );
    }

    #[test]
    fn denied_or_failed_mandatory_policy_prevents_staging_and_commit() {
        let root = TestRoot::new();
        let installer = Installer::open(&root.0).unwrap();
        let mut metadata = metadata("1.0.0", &["storage.read"]);
        metadata.references.sbom = Some("sbom:v1".into());
        metadata.references.license = Some("license:v1".into());
        metadata.references.provenance = Some("sha256:artifact".into());
        let source = PackageSource::from_virtual_files(
            metadata,
            vec![VirtualFile {
                path: "bin/app.napp".into(),
                contents: b"app".to_vec(),
                executable: true,
            }],
        )
        .unwrap();
        let mut policy = RecordingPolicy {
            deny: Some(PolicyCheck::Provenance),
            ..RecordingPolicy::default()
        };
        assert!(matches!(
            installer.install_with(source.clone(), &mut policy, &mut FailAt::default()),
            Err(InstallerError::PolicyDenied(_))
        ));
        assert_eq!(
            policy.checks,
            [
                PolicyCheck::CapabilityChanges,
                PolicyCheck::Sbom,
                PolicyCheck::License,
                PolicyCheck::Provenance
            ]
        );
        assert!(installer.inventory().unwrap().is_empty());
        assert_eq!(fs::read_dir(root.0.join("staging")).unwrap().count(), 0);

        let mut policy = RecordingPolicy {
            fail: Some(PolicyCheck::Sbom),
            ..RecordingPolicy::default()
        };
        assert!(matches!(
            installer.install_with(source, &mut policy, &mut FailAt::default()),
            Err(InstallerError::PolicyFailure { .. })
        ));
        assert!(installer.inventory().unwrap().is_empty());
    }

    #[test]
    fn inventory_round_trip_keeps_apps_isolated_and_uninstall_preserves_user_data() {
        let root = TestRoot::new();
        let installer = Installer::open(&root.0).unwrap();
        let first = installer.install(source("1.0.0", &[], b"first")).unwrap();
        let second_metadata = AppSdkManifestAdapter
            .adapt(&AppManifestContract {
                schema_version: 1,
                sdk_contract_version: SDK_CONTRACT_VERSION,
                identity: AppIdentity::new(
                    "com.nagi.second-fixture",
                    "1.0.0",
                    AppOrigin::ThirdParty,
                    None,
                )
                .unwrap(),
                display_name: DisplayName {
                    en_us: "Second",
                    ja_jp: None,
                },
                supported_locales: &[EN_US],
                icon: None,
                entrypoint: AppEntrypoint {
                    kind: EntrypointKind::Native,
                    target: "bin/app.napp",
                },
                resources: &[],
                intents: &[],
                requested_capabilities: &[],
                background_services: &[],
                state: StateCompatibility {
                    current: AppStateVersion(1),
                    minimum_readable: AppStateVersion(1),
                },
            })
            .unwrap();
        let second = PackageSource::from_virtual_files(
            second_metadata,
            vec![VirtualFile {
                path: "bin/app.napp".into(),
                contents: b"second".to_vec(),
                executable: true,
            }],
        )
        .unwrap();
        let second = installer.install(second).unwrap();
        drop(installer);
        let installer = Installer::open(&root.0).unwrap();
        assert_eq!(installer.inventory().unwrap().len(), 2);
        assert_eq!(app_content(&installer, &first.app_id), b"first");
        assert_eq!(app_content(&installer, &second.app_id), b"second");

        let data = root.0.join("data/com.nagi.installer-fixture");
        fs::create_dir_all(&data).unwrap();
        fs::write(data.join("notes.db"), b"user data").unwrap();
        let outcome = installer.uninstall(&first.app_id).unwrap();
        assert_eq!(outcome.removed_version, "1.0.0");
        assert!(outcome.package_files_removed);
        assert!(installer.installed(&first.app_id).unwrap().is_none());
        assert_eq!(fs::read(data.join("notes.db")).unwrap(), b"user data");
        assert_eq!(installer.inventory().unwrap().len(), 1);
        assert!(!installer.package_path(&first).unwrap().exists());
    }

    #[test]
    fn missing_uninstall_target_and_denied_uninstall_are_structured() {
        let root = TestRoot::new();
        let installer = Installer::open(&root.0).unwrap();
        assert!(matches!(
            installer.plan_uninstall("com.nagi.absent"),
            Err(InstallerError::NotInstalled(_))
        ));
        let package = installer.install(source("1.0.0", &[], b"active")).unwrap();
        let mut policy = RecordingPolicy {
            deny: Some(PolicyCheck::Uninstall),
            ..RecordingPolicy::default()
        };
        assert!(matches!(
            installer.uninstall_with(&package.app_id, &mut policy, &mut FailAt::default()),
            Err(InstallerError::PolicyDenied(_))
        ));
        assert!(installer.installed(&package.app_id).unwrap().is_some());
    }

    #[test]
    fn interrupted_uninstall_recovers_by_restoring_inventory_and_package() {
        let root = TestRoot::new();
        let installer = Installer::open(&root.0).unwrap();
        let installed = installer.install(source("1.0.0", &[], b"active")).unwrap();
        assert!(matches!(
            installer.uninstall_with(
                &installed.app_id,
                &mut NoopPolicy,
                &mut FailAt(Some(FailurePoint::AfterUninstallActiveSwitch))
            ),
            Err(InstallerError::InjectedFailure { .. })
        ));
        let interrupted_inventory: serde_json::Value =
            serde_json::from_slice(&fs::read(root.0.join("inventory.json")).unwrap()).unwrap();
        assert!(interrupted_inventory["packages"][&installed.app_id].is_null());
        installer.recover().unwrap();
        assert_eq!(app_content(&installer, &installed.app_id), b"active");
    }

    #[test]
    fn interrupted_committed_uninstall_finishes_package_cleanup_but_keeps_user_data() {
        let root = TestRoot::new();
        let installer = Installer::open(&root.0).unwrap();
        let installed = installer.install(source("1.0.0", &[], b"active")).unwrap();
        let package_path = installer.package_path(&installed).unwrap();
        let data = root.0.join("data/com.nagi.installer-fixture");
        fs::create_dir_all(&data).unwrap();
        fs::write(data.join("user.txt"), b"keep").unwrap();
        assert!(matches!(
            installer.uninstall_with(
                &installed.app_id,
                &mut NoopPolicy,
                &mut FailAt(Some(FailurePoint::BeforeUninstallPackageCleanup))
            ),
            Err(InstallerError::InjectedFailure { .. })
        ));
        assert!(package_path.exists());
        installer.recover().unwrap();
        assert!(!package_path.exists());
        assert_eq!(fs::read(data.join("user.txt")).unwrap(), b"keep");
    }

    #[test]
    fn uninstall_removes_retained_rollback_versions() {
        let root = TestRoot::new();
        let installer = Installer::open(&root.0).unwrap();
        let old = installer
            .install(source("1.0.0", &[], b"old version"))
            .unwrap();
        installer
            .install(source("2.0.0", &[], b"new version"))
            .unwrap();
        let old_path = installer.package_path(&old).unwrap();
        assert!(old_path.exists());

        let outcome = installer.uninstall(&old.app_id).unwrap();
        assert!(outcome.package_files_removed);
        assert!(!old_path.exists());
        assert!(!root.0.join("packages/com.nagi.installer-fixture").exists());
    }

    #[test]
    fn orphan_staging_is_cleaned_and_unknown_directories_are_left_alone() {
        let root = TestRoot::new();
        let installer = Installer::open(&root.0).unwrap();
        let orphan = root
            .0
            .join("staging/00000000000000000000000000000000-00000001-0000000000000001");
        fs::create_dir_all(&orphan).unwrap();
        let unrelated = root.0.join("staging/user-files");
        fs::create_dir_all(&unrelated).unwrap();
        installer.recover().unwrap();
        assert!(!orphan.exists());
        assert!(unrelated.exists());
    }

    #[test]
    fn changed_directory_source_is_rejected_before_staging_or_active_mutation() {
        let root = TestRoot::new();
        let installer = Installer::open(root.0.join("install-root")).unwrap();
        installer
            .install(source("1.0.0", &[], b"known good"))
            .unwrap();
        let package_root = root.0.join("source");
        fs::create_dir_all(package_root.join("bin")).unwrap();
        fs::write(package_root.join("bin/app.napp"), b"AAAA").unwrap();
        let package = PackageSource::from_directory(&package_root, metadata("2.0.0", &[])).unwrap();
        let plan = installer.plan_install(package).unwrap();
        // Same length, different bytes: metadata-only revalidation would miss it.
        fs::write(package_root.join("bin/app.napp"), b"BBBB").unwrap();
        assert!(matches!(
            installer.execute_install_with(plan, &mut NoopPolicy, &mut FailAt::default()),
            Err(InstallerError::InvalidPackage(_))
        ));
        assert_eq!(
            app_content(&installer, "com.nagi.installer-fixture"),
            b"known good"
        );
        assert_eq!(
            fs::read_dir(installer.root().join("transactions"))
                .unwrap()
                .count(),
            0
        );
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_package_parent_cannot_redirect_writes_or_cleanup_outside_root() {
        let root = TestRoot::new();
        let install_root = root.0.join("install-root");
        let outside = root.0.join("outside");
        fs::create_dir_all(&outside).unwrap();
        fs::write(outside.join("keep.txt"), b"must survive").unwrap();
        let installer = Installer::open(&install_root).unwrap();
        let package_parent = install_root.join("packages/com.nagi.installer-fixture");
        fs::create_dir_all(&package_parent).unwrap();
        std::os::unix::fs::symlink(&outside, package_parent.join("versions")).unwrap();

        let result = installer.install(source("1.0.0", &[], b"app"));
        assert!(result.is_err());
        assert_eq!(fs::read(outside.join("keep.txt")).unwrap(), b"must survive");
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 1);

        fs::remove_file(package_parent.join("versions")).unwrap();
        installer.recover().unwrap();
        assert!(fs::read_dir(install_root.join("transactions"))
            .unwrap()
            .next()
            .is_none());
    }

    #[test]
    fn stale_plans_and_unknown_state_schemas_fail_closed() {
        let root = TestRoot::new();
        let installer = Installer::open(&root.0).unwrap();
        let plan = installer
            .plan_install(source("2.0.0", &[], b"candidate"))
            .unwrap();
        installer
            .install(source("1.0.0", &[], b"installed meanwhile"))
            .unwrap();
        assert!(matches!(
            installer.execute_install_with(plan, &mut NoopPolicy, &mut FailAt::default()),
            Err(InstallerError::ConcurrentModification(_))
        ));

        fs::write(
            root.0.join("inventory.json"),
            br#"{"schema_version":9,"generation":0,"packages":{}}"#,
        )
        .unwrap();
        assert!(matches!(
            installer.inventory(),
            Err(InstallerError::UnsupportedSchema { .. })
        ));
    }

    #[test]
    fn unknown_transaction_journal_schema_is_retained_for_diagnosis() {
        let root = TestRoot::new();
        let installer = Installer::open(&root.0).unwrap();
        let id = "0123456789abcdef0123456789abcdef-00000001-0000000000000001";
        let journal = root.0.join(format!("transactions/{id}.json"));
        fs::write(
            &journal,
            format!(
                "{{\"schema_version\":9,\"transaction_id\":\"{id}\",\"operation\":\"install\",\"app_id\":\"com.nagi.fixture\",\"prior\":null,\"target\":null,\"staging_location\":null,\"phase\":\"created\",\"failure\":null}}"
            ),
        )
        .unwrap();
        assert!(matches!(
            installer.recover(),
            Err(InstallerError::UnsupportedSchema { .. })
        ));
        assert!(journal.exists());
    }

    #[test]
    fn corrupt_inventory_and_symlinked_package_sources_fail_closed() {
        let root = TestRoot::new();
        let installer = Installer::open(&root.0).unwrap();
        fs::write(root.0.join("inventory.json"), b"not json").unwrap();
        assert!(matches!(
            installer.inventory(),
            Err(InstallerError::InventoryFailure(_))
        ));

        #[cfg(unix)]
        {
            let source_root = root.0.join("source");
            fs::create_dir_all(&source_root).unwrap();
            std::os::unix::fs::symlink(root.0.join("escape"), source_root.join("bin")).unwrap();
            assert!(matches!(
                PackageSource::from_directory(source_root, metadata("1.0.0", &[])),
                Err(InstallerError::UnsafePath(_))
            ));
        }
    }

    #[test]
    fn state_machine_rejects_illegal_transitions_and_transaction_ids_are_bounded() {
        assert!(!TransactionPhase::Created.allows_transition_to(TransactionPhase::Committed));
        assert!(TransactionPhase::Created.allows_transition_to(TransactionPhase::Validating));
        assert!(valid_transaction_id(
            "0123456789abcdef0123456789abcdef-00000001-0000000000000001"
        ));
        assert!(!valid_transaction_id("../escape"));
    }

    #[test]
    fn capability_deltas_are_stable_and_sorted() {
        let delta = CapabilityDelta::between(
            &["z.read".into(), "a.read".into()],
            &["b.read".into(), "a.read".into()],
        );
        assert_eq!(delta.unchanged, ["a.read"]);
        assert_eq!(delta.added, ["b.read"]);
        assert_eq!(delta.removed, ["z.read"]);
    }

    #[test]
    fn policy_errors_remain_structured() {
        let mut policy = RecordingPolicy {
            fail: Some(PolicyCheck::Trust),
            ..RecordingPolicy::default()
        };
        let error = evaluate_policy(
            &mut policy,
            PolicyOperation::Install,
            PolicyCheck::Trust,
            "com.nagi.test",
            None,
            None,
            PolicyReferences::default(),
        )
        .unwrap_err();
        assert_eq!(error.kind(), crate::ErrorKind::PolicyFailure);
    }
}
