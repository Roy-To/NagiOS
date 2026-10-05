use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::ledger::{ActivityLedger, LedgerError};
use crate::model::*;
use crate::store::{LedgerStore, SnapshotStore};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "snake_case")]
pub enum RestoreTarget {
    Transaction(TransactionId),
    RestorePoint(RestorePointId),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum RestoreStep {
    ExactUndo {
        entry_id: EntryId,
    },
    Compensation {
        entry_id: EntryId,
        contract_id: String,
    },
    SnapshotRestore {
        snapshot_id: SnapshotId,
        scope: ScopeReference,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum PlanBlocker {
    TransactionNotCommitted {
        transaction_id: TransactionId,
    },
    PartialTransaction {
        transaction_id: TransactionId,
    },
    IrreversibleAction {
        entry_id: EntryId,
        reason: String,
    },
    UnknownReversibility {
        entry_id: EntryId,
        reason: String,
    },
    MissingSnapshotReference {
        entry_id: EntryId,
    },
    MissingSnapshot {
        snapshot_id: SnapshotId,
    },
    SnapshotBackendUnavailable {
        snapshot_id: SnapshotId,
    },
    InvalidSnapshotManifest {
        snapshot_id: SnapshotId,
        reason: String,
    },
    SnapshotScopeMismatch {
        snapshot_id: SnapshotId,
    },
    SnapshotDependencyCycle {
        snapshot_id: SnapshotId,
    },
    RestorePointScopeMissingSnapshot {
        scope: ScopeReference,
    },
    RestorePointScopeNotCaptured {
        entry_id: EntryId,
        scope: ScopeReference,
    },
    SnapshotNotBeforeAction {
        snapshot_id: SnapshotId,
        entry_id: EntryId,
    },
    CompatibilityMismatch {
        snapshot_id: SnapshotId,
        expected: String,
        actual: String,
    },
    NoRestorableActions,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionBoundary {
    PlanOnlyFutureAuthorizedExecutorRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RestorePlan {
    pub schema_version: u32,
    pub target: RestoreTarget,
    pub steps: Vec<RestoreStep>,
    pub required_snapshots: Vec<SnapshotId>,
    pub blockers: Vec<PlanBlocker>,
    pub affected_scopes: Vec<ScopeReference>,
    /// True means structurally safe to offer to a future authorized executor; it never runs restore.
    pub executable: bool,
    pub execution_boundary: ExecutionBoundary,
}

impl RestorePlan {
    pub fn validate_version(&self) -> Result<(), LedgerError> {
        if self.schema_version != SCHEMA_VERSION {
            return Err(LedgerError::UnsupportedVersion {
                kind: "restore plan",
                version: self.schema_version,
            });
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UndoCandidate {
    pub schema_version: u32,
    pub transaction: Transaction,
    pub plan: RestorePlan,
}

impl UndoCandidate {
    pub fn requires_compensation(&self) -> bool {
        self.plan
            .steps
            .iter()
            .any(|step| matches!(step, RestoreStep::Compensation { .. }))
    }

    pub fn requires_snapshot(&self) -> bool {
        self.plan
            .steps
            .iter()
            .any(|step| matches!(step, RestoreStep::SnapshotRestore { .. }))
            || self.plan.blockers.iter().any(|blocker| {
                matches!(
                    blocker,
                    PlanBlocker::MissingSnapshotReference { .. }
                        | PlanBlocker::MissingSnapshot { .. }
                        | PlanBlocker::SnapshotBackendUnavailable { .. }
                        | PlanBlocker::InvalidSnapshotManifest { .. }
                )
            })
    }
}

fn scope_for_entry(entry: &ActivityEntry) -> ScopeReference {
    ScopeReference {
        scope_type: entry.target.resource_type.clone(),
        external_id: entry.target.external_id.clone(),
        app_id: entry.app_id.clone().or_else(|| entry.target.app_id.clone()),
    }
}

fn add_scope(scopes: &mut BTreeSet<ScopeReference>, scope: ScopeReference) {
    scopes.insert(scope);
}

#[derive(Clone, Copy)]
struct SnapshotExpectation<'a> {
    scope: Option<&'a ScopeReference>,
    compatibility: Option<&'a str>,
    before_sequence: Option<u64>,
    entry_id: Option<&'a EntryId>,
}

#[derive(Default)]
struct SnapshotDependencyState {
    required: BTreeSet<SnapshotId>,
    resolved: BTreeSet<SnapshotId>,
    resolving: BTreeSet<SnapshotId>,
    resolved_scopes: BTreeSet<ScopeReference>,
}

fn resolve_snapshot_inner<S: SnapshotStore>(
    snapshot_store: &S,
    snapshot_id: &SnapshotId,
    expectation: SnapshotExpectation<'_>,
    steps: &mut Vec<RestoreStep>,
    dependencies: &mut SnapshotDependencyState,
) -> Result<(), PlanBlocker> {
    if dependencies.resolved.contains(snapshot_id) {
        return Ok(());
    }
    if !dependencies.resolving.insert(snapshot_id.clone()) {
        return Err(PlanBlocker::SnapshotDependencyCycle {
            snapshot_id: snapshot_id.clone(),
        });
    }
    dependencies.required.insert(snapshot_id.clone());
    let result = (|| {
        let manifest = match snapshot_store.get_manifest(snapshot_id) {
            Ok(Some(manifest)) => manifest,
            Ok(None) => {
                return Err(PlanBlocker::MissingSnapshot {
                    snapshot_id: snapshot_id.clone(),
                });
            }
            Err(_) => {
                return Err(PlanBlocker::SnapshotBackendUnavailable {
                    snapshot_id: snapshot_id.clone(),
                });
            }
        };
        if let Err(error) = manifest.validate() {
            return Err(PlanBlocker::InvalidSnapshotManifest {
                snapshot_id: snapshot_id.clone(),
                reason: error.to_string(),
            });
        }
        if let Some(expected_scope) = expectation.scope {
            if &manifest.scope != expected_scope {
                return Err(PlanBlocker::SnapshotScopeMismatch {
                    snapshot_id: snapshot_id.clone(),
                });
            }
        }
        if let Some(expected) = expectation.compatibility {
            if manifest.compatibility_version != expected {
                return Err(PlanBlocker::CompatibilityMismatch {
                    snapshot_id: snapshot_id.clone(),
                    expected: expected.to_owned(),
                    actual: manifest.compatibility_version,
                });
            }
        }
        if let (Some(boundary), Some(entry_id)) =
            (expectation.before_sequence, expectation.entry_id)
        {
            if manifest.source_ledger_boundary.sequence >= boundary {
                return Err(PlanBlocker::SnapshotNotBeforeAction {
                    snapshot_id: snapshot_id.clone(),
                    entry_id: entry_id.clone(),
                });
            }
        }
        if let Some(parent_snapshot) = &manifest.parent_snapshot {
            resolve_snapshot_inner(
                snapshot_store,
                parent_snapshot,
                expectation,
                steps,
                dependencies,
            )?;
        }
        let step = RestoreStep::SnapshotRestore {
            snapshot_id: snapshot_id.clone(),
            scope: manifest.scope.clone(),
        };
        if !steps.contains(&step) {
            steps.push(step);
        }
        dependencies.resolved_scopes.insert(manifest.scope);
        Ok(())
    })();
    dependencies.resolving.remove(snapshot_id);
    if result.is_ok() {
        dependencies.resolved.insert(snapshot_id.clone());
    }
    result
}

fn resolve_snapshot<S: SnapshotStore>(
    snapshot_store: &S,
    snapshot_id: &SnapshotId,
    expectation: SnapshotExpectation<'_>,
    steps: &mut Vec<RestoreStep>,
    blockers: &mut Vec<PlanBlocker>,
    dependencies: &mut SnapshotDependencyState,
) {
    if let Err(blocker) = resolve_snapshot_inner(
        snapshot_store,
        snapshot_id,
        expectation,
        steps,
        dependencies,
    ) {
        blockers.push(blocker);
    }
}

fn finish_plan(
    target: RestoreTarget,
    steps: Vec<RestoreStep>,
    required_snapshots: BTreeSet<SnapshotId>,
    blockers: Vec<PlanBlocker>,
    affected_scopes: BTreeSet<ScopeReference>,
) -> RestorePlan {
    let executable = blockers.is_empty() && !steps.is_empty();
    RestorePlan {
        schema_version: SCHEMA_VERSION,
        target,
        steps,
        required_snapshots: required_snapshots.into_iter().collect(),
        blockers,
        affected_scopes: affected_scopes.into_iter().collect(),
        executable,
        execution_boundary: ExecutionBoundary::PlanOnlyFutureAuthorizedExecutorRequired,
    }
}

impl<S: LedgerStore> ActivityLedger<S> {
    pub fn plan_transaction_restore<P: SnapshotStore>(
        &self,
        transaction_id: &TransactionId,
        snapshots: &P,
        expected_compatibility: Option<&str>,
    ) -> Result<RestorePlan, LedgerError> {
        let transaction = self
            .transaction(transaction_id)?
            .ok_or(LedgerError::NotFound("transaction"))?;
        let target = RestoreTarget::Transaction(transaction_id.clone());
        let mut steps = Vec::new();
        let mut blockers = Vec::new();
        let mut snapshot_dependencies = SnapshotDependencyState::default();
        let mut affected_scopes = BTreeSet::new();
        match transaction.state {
            TransactionState::Committed { .. } => {}
            TransactionState::PartiallyApplied { .. } => {
                blockers.push(PlanBlocker::PartialTransaction {
                    transaction_id: transaction_id.clone(),
                });
            }
            TransactionState::Open | TransactionState::Aborted { .. } => {
                blockers.push(PlanBlocker::TransactionNotCommitted {
                    transaction_id: transaction_id.clone(),
                });
            }
        }
        let entries = self.transaction_entries(transaction_id)?;
        for entry in entries.iter().rev() {
            let scope = scope_for_entry(entry);
            add_scope(&mut affected_scopes, scope.clone());
            match &entry.reversibility {
                Reversibility::ExactReversible => steps.push(RestoreStep::ExactUndo {
                    entry_id: entry.id.clone(),
                }),
                Reversibility::CompensationRequired { contract_id } => {
                    steps.push(RestoreStep::Compensation {
                        entry_id: entry.id.clone(),
                        contract_id: contract_id.clone(),
                    });
                }
                Reversibility::SnapshotRequired { snapshot_id } => {
                    let snapshot_id = snapshot_id.as_ref().or_else(|| entry.snapshot_refs.first());
                    if let Some(snapshot_id) = snapshot_id {
                        resolve_snapshot(
                            snapshots,
                            snapshot_id,
                            SnapshotExpectation {
                                scope: Some(&scope),
                                compatibility: expected_compatibility,
                                before_sequence: Some(entry.integrity.sequence),
                                entry_id: Some(&entry.id),
                            },
                            &mut steps,
                            &mut blockers,
                            &mut snapshot_dependencies,
                        );
                    } else {
                        blockers.push(PlanBlocker::MissingSnapshotReference {
                            entry_id: entry.id.clone(),
                        });
                    }
                }
                Reversibility::Irreversible { reason } => {
                    blockers.push(PlanBlocker::IrreversibleAction {
                        entry_id: entry.id.clone(),
                        reason: reason.clone(),
                    });
                }
                Reversibility::Unknown { reason } => {
                    blockers.push(PlanBlocker::UnknownReversibility {
                        entry_id: entry.id.clone(),
                        reason: reason.clone(),
                    });
                }
            }
        }
        if entries.is_empty() {
            blockers.push(PlanBlocker::NoRestorableActions);
        }
        Ok(finish_plan(
            target,
            steps,
            snapshot_dependencies.required,
            blockers,
            affected_scopes,
        ))
    }

    pub fn plan_restore_point<P: SnapshotStore>(
        &self,
        restore_point_id: &RestorePointId,
        snapshots: &P,
        expected_compatibility: Option<&str>,
    ) -> Result<RestorePlan, LedgerError> {
        let point = self
            .restore_point(restore_point_id)?
            .ok_or(LedgerError::NotFound("restore point"))?;
        let target = RestoreTarget::RestorePoint(restore_point_id.clone());
        let mut steps = Vec::new();
        let mut blockers = Vec::new();
        let mut snapshot_dependencies = SnapshotDependencyState::default();
        let mut affected_scopes = BTreeSet::new();
        let expected = expected_compatibility.unwrap_or(&point.compatibility_version);
        for scope in &point.scopes {
            add_scope(&mut affected_scopes, scope.clone());
        }
        for snapshot_id in &point.snapshot_ids {
            let snapshot_scope = snapshots
                .get_manifest(snapshot_id)
                .ok()
                .flatten()
                .map(|manifest| manifest.scope);
            let expected_scope = snapshot_scope
                .as_ref()
                .filter(|scope| point.scopes.contains(scope));
            if expected_scope.is_none() {
                // resolve_snapshot reports a missing manifest or a scope mismatch below.
                if snapshot_scope.is_some() {
                    blockers.push(PlanBlocker::SnapshotScopeMismatch {
                        snapshot_id: snapshot_id.clone(),
                    });
                    snapshot_dependencies.required.insert(snapshot_id.clone());
                    continue;
                }
                resolve_snapshot(
                    snapshots,
                    snapshot_id,
                    SnapshotExpectation {
                        scope: None,
                        compatibility: Some(expected),
                        before_sequence: None,
                        entry_id: None,
                    },
                    &mut steps,
                    &mut blockers,
                    &mut snapshot_dependencies,
                );
                continue;
            }
            resolve_snapshot(
                snapshots,
                snapshot_id,
                SnapshotExpectation {
                    scope: expected_scope,
                    compatibility: Some(expected),
                    before_sequence: None,
                    entry_id: None,
                },
                &mut steps,
                &mut blockers,
                &mut snapshot_dependencies,
            );
        }
        for scope in &point.scopes {
            if !snapshot_dependencies.resolved_scopes.contains(scope) {
                blockers.push(PlanBlocker::RestorePointScopeMissingSnapshot {
                    scope: scope.clone(),
                });
            }
        }
        let mut entries = self.entries_after(point.ledger_boundary.sequence)?;
        entries.reverse();
        let mut checked_transactions = BTreeSet::new();
        for entry in entries {
            let scope = scope_for_entry(&entry);
            add_scope(&mut affected_scopes, scope.clone());
            if let Some(transaction_id) = &entry.transaction_id {
                if checked_transactions.insert(transaction_id.clone()) {
                    let transaction = self
                        .transaction(transaction_id)?
                        .ok_or(LedgerError::NotFound("transaction"))?;
                    match transaction.state {
                        TransactionState::Committed { .. } => {}
                        TransactionState::PartiallyApplied { .. } => {
                            blockers.push(PlanBlocker::PartialTransaction {
                                transaction_id: transaction_id.clone(),
                            });
                        }
                        TransactionState::Open | TransactionState::Aborted { .. } => {
                            blockers.push(PlanBlocker::TransactionNotCommitted {
                                transaction_id: transaction_id.clone(),
                            });
                        }
                    }
                }
            }
            match &entry.reversibility {
                Reversibility::Irreversible { reason } => {
                    blockers.push(PlanBlocker::IrreversibleAction {
                        entry_id: entry.id.clone(),
                        reason: reason.clone(),
                    });
                }
                Reversibility::Unknown { reason } => {
                    blockers.push(PlanBlocker::UnknownReversibility {
                        entry_id: entry.id.clone(),
                        reason: reason.clone(),
                    });
                }
                Reversibility::CompensationRequired { contract_id } => {
                    steps.push(RestoreStep::Compensation {
                        entry_id: entry.id.clone(),
                        contract_id: contract_id.clone(),
                    });
                }
                Reversibility::ExactReversible => {
                    if !snapshot_dependencies.resolved_scopes.contains(&scope) {
                        steps.push(RestoreStep::ExactUndo {
                            entry_id: entry.id.clone(),
                        });
                    }
                }
                Reversibility::SnapshotRequired { .. } => {
                    if !snapshot_dependencies.resolved_scopes.contains(&scope) {
                        blockers.push(PlanBlocker::RestorePointScopeNotCaptured {
                            entry_id: entry.id.clone(),
                            scope,
                        });
                    }
                }
            }
        }
        if steps.is_empty() && blockers.is_empty() {
            blockers.push(PlanBlocker::NoRestorableActions);
        }
        Ok(finish_plan(
            target,
            steps,
            snapshot_dependencies.required,
            blockers,
            affected_scopes,
        ))
    }

    pub fn undo_candidates<P: SnapshotStore>(
        &self,
        limit: usize,
        snapshots: &P,
        expected_compatibility: Option<&str>,
    ) -> Result<Vec<UndoCandidate>, LedgerError> {
        let mut candidates = Vec::new();
        for transaction in self
            .committed_transactions_newest_first()?
            .into_iter()
            .take(limit)
        {
            let plan =
                self.plan_transaction_restore(&transaction.id, snapshots, expected_compatibility)?;
            candidates.push(UndoCandidate {
                schema_version: SCHEMA_VERSION,
                transaction,
                plan,
            });
        }
        Ok(candidates)
    }
}
