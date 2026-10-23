//! Host-only contract fixture. No disk, service, credential or runtime adapter.
//! The durable bytes below model an adapter's committed state, not real durability.
use std::sync::{Arc, Barrier, Mutex};
use std::thread;

use nagi_capability::{
    AccessRequest, CapabilityDefinition, CapabilityId, CapabilityRegistry, CapabilityScope,
    DecisionReason, EvaluationContext, GrantId, GrantLifetime, GrantSource, InMemoryPolicyStore,
    PermissionEvaluator, PolicyDocument, PolicyStore, PolicyStoreError, PrincipalId, UnixTimestamp,
};

#[derive(Clone, Copy, Debug, Default)]
enum Fault {
    #[default]
    None,
    BeforeCommit,
    AfterCommit,
}

struct State {
    committed: String,
    next_fault: Fault,
    operations: usize,
    commits: usize,
}

#[derive(Clone)]
struct SharedStore(Arc<Mutex<State>>);

impl SharedStore {
    fn new() -> Self {
        Self(Arc::new(Mutex::new(State {
            committed: PolicyDocument::new().to_json().unwrap(),
            next_fault: Fault::None,
            operations: 0,
            commits: 0,
        })))
    }

    fn inject(&self, fault: Fault) {
        self.0.lock().unwrap().next_fault = fault;
    }

    fn bytes(&self) -> String {
        self.0.lock().unwrap().committed.clone()
    }

    // Fresh handles must reload authoritative committed bytes, never a stale cache.
    fn reload(&self) -> Self {
        self.snapshot().unwrap();
        self.clone()
    }

    fn counts(&self) -> (usize, usize) {
        let state = self.0.lock().unwrap();
        (state.operations, state.commits)
    }
}

impl PolicyStore for SharedStore {
    fn transact<T>(
        &mut self,
        operation: impl FnOnce(&mut PolicyDocument) -> Result<T, PolicyStoreError>,
    ) -> Result<T, PolicyStoreError> {
        // Serialization covers the complete read/modify/commit transaction.
        let mut state = self.0.lock().unwrap();
        let fault = std::mem::take(&mut state.next_fault);
        let mut candidate = PolicyDocument::from_json(state.committed.as_bytes())?;
        state.operations += 1;
        let result = operation(&mut candidate)?;
        let bytes = candidate.to_json()?;
        if matches!(fault, Fault::BeforeCommit) {
            return Err(PolicyStoreError::Unavailable("before commit".into()));
        }
        state.committed = bytes;
        state.commits += 1;
        if matches!(fault, Fault::AfterCommit) {
            return Err(PolicyStoreError::Unavailable("commit response lost".into()));
        }
        Ok(result)
    }

    fn snapshot(&self) -> Result<PolicyDocument, PolicyStoreError> {
        PolicyDocument::from_json(self.0.lock().unwrap().committed.as_bytes())
    }
}

fn request() -> AccessRequest {
    AccessRequest::new(
        PrincipalId::new("app:contract.fake").unwrap(),
        CapabilityId::new("filesystem.read").unwrap(),
        CapabilityScope::Filesystem {
            path: "/fake/document".into(),
        },
    )
}

fn context() -> EvaluationContext {
    EvaluationContext::new(UnixTimestamp::from_unix_seconds(10), None)
}

fn evaluator(store: SharedStore) -> PermissionEvaluator<SharedStore> {
    let mut registry = CapabilityRegistry::new();
    registry
        .register(CapabilityDefinition {
            id: request().capability,
            description: "Host fake contract".into(),
        })
        .unwrap();
    PermissionEvaluator::new(registry, store)
}

fn grant(evaluator: &mut PermissionEvaluator<SharedStore>) -> GrantId {
    evaluator
        .grant(
            &request(),
            GrantLifetime::OneShot,
            GrantSource::User,
            None,
            UnixTimestamp::from_unix_seconds(1),
            None,
        )
        .unwrap()
}

#[test]
fn operation_then_precommit_failure_denies_without_consuming_and_retry_allows_once() {
    let store = SharedStore::new();
    let mut eval = evaluator(store.clone());
    let id = grant(&mut eval);
    let before = store.bytes();
    let counts = store.counts();
    store.inject(Fault::BeforeCommit);
    let decision = eval.check(&request(), &context());
    assert!(!decision.is_allowed());
    assert_eq!(decision.reason(), DecisionReason::PolicyStoreUnavailable);
    assert_eq!(decision.grant_id(), None);
    assert_eq!(store.counts(), (counts.0 + 1, counts.1));
    assert_eq!(store.bytes(), before);
    let mut restored = evaluator(store.reload());
    assert!(restored.explain(&request(), &context()).is_allowed());
    let allowed = restored.check(&request(), &context());
    assert!(allowed.is_allowed());
    assert_eq!(allowed.grant_id(), Some(id));
    assert_eq!(
        restored.check(&request(), &context()).reason(),
        DecisionReason::OneShotConsumed
    );
}

#[test]
fn committed_but_response_unknown_denies_and_never_regrants_on_reload() {
    let store = SharedStore::new();
    let mut eval = evaluator(store.clone());
    let id = grant(&mut eval);
    let counts = store.counts();
    store.inject(Fault::AfterCommit);
    let decision = eval.check(&request(), &context());
    assert!(!decision.is_allowed());
    assert_eq!(decision.reason(), DecisionReason::PolicyStoreUnavailable);
    assert_eq!(decision.grant_id(), None);
    assert_eq!(store.counts(), (counts.0 + 1, counts.1 + 1));
    let document = store.snapshot().unwrap();
    let record = document.grants().iter().find(|g| g.id == id).unwrap();
    assert_eq!(record.consumed_at, Some(context().now));
    let mut restored = evaluator(store.reload());
    for _ in 0..3 {
        assert_eq!(
            restored.check(&request(), &context()).reason(),
            DecisionReason::OneShotConsumed
        );
        assert_eq!(
            restored.explain(&request(), &context()).reason(),
            DecisionReason::OneShotConsumed
        );
    }
}

fn concurrent_checks(store: SharedStore, workers: usize) -> Vec<DecisionReason> {
    let barrier = Arc::new(Barrier::new(workers));
    let handles: Vec<_> = (0..workers)
        .map(|_| {
            let mut eval = evaluator(store.reload());
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                let decision = eval.check(&request(), &context());
                assert_eq!(
                    decision.is_allowed(),
                    decision.reason() == DecisionReason::AuthorizedByGrant
                );
                decision.reason()
            })
        })
        .collect();
    handles.into_iter().map(|h| h.join().unwrap()).collect()
}

#[test]
fn independent_concurrent_evaluators_allow_at_most_once_across_commit_faults() {
    for fault in [Fault::None, Fault::BeforeCommit, Fault::AfterCommit] {
        let store = SharedStore::new();
        grant(&mut evaluator(store.clone()));
        store.inject(fault);
        let results = concurrent_checks(store.clone(), 16);
        let allows = results
            .iter()
            .filter(|r| **r == DecisionReason::AuthorizedByGrant)
            .count();
        let failures = results
            .iter()
            .filter(|r| **r == DecisionReason::PolicyStoreUnavailable)
            .count();
        let consumed = results
            .iter()
            .filter(|r| **r == DecisionReason::OneShotConsumed)
            .count();
        assert!(allows <= 1);
        match fault {
            Fault::None => assert_eq!((allows, failures, consumed), (1, 0, 15)),
            Fault::BeforeCommit => assert_eq!((allows, failures, consumed), (1, 1, 14)),
            Fault::AfterCommit => assert_eq!((allows, failures, consumed), (0, 1, 15)),
        }
        assert_eq!(
            evaluator(store.reload())
                .check(&request(), &context())
                .reason(),
            DecisionReason::OneShotConsumed
        );
    }
}

#[test]
fn committed_revoke_and_explicit_deny_survive_failed_checks_and_reload() {
    for revoked in [false, true] {
        for fault in [Fault::BeforeCommit, Fault::AfterCommit] {
            let store = SharedStore::new();
            let mut eval = evaluator(store.clone());
            let id = grant(&mut eval);
            let expected = if revoked {
                eval.revoke(id, context().now).unwrap();
                DecisionReason::Revoked
            } else {
                eval.deny(
                    &request(),
                    GrantLifetime::Persistent,
                    GrantSource::Policy,
                    None,
                    UnixTimestamp::from_unix_seconds(2),
                    None,
                )
                .unwrap();
                DecisionReason::ExplicitlyDenied
            };
            let before = store.bytes();
            store.inject(fault);
            assert_eq!(
                eval.check(&request(), &context()).reason(),
                DecisionReason::PolicyStoreUnavailable
            );
            assert_eq!(store.bytes(), before);
            let mut restored = evaluator(store.reload());
            assert_eq!(restored.explain(&request(), &context()).reason(), expected);
            assert_eq!(restored.check(&request(), &context()).reason(), expected);
            assert!(concurrent_checks(store.clone(), 8)
                .iter()
                .all(|r| *r == expected));
            let document = store.snapshot().unwrap();
            assert_eq!(
                document
                    .grants()
                    .iter()
                    .find(|g| g.id == id)
                    .unwrap()
                    .consumed_at,
                None
            );
        }
    }
}

#[test]
fn uncertain_revoke_and_deny_commits_preserve_authoritative_policy() {
    for revoked in [false, true] {
        let store = SharedStore::new();
        let mut eval = evaluator(store.clone());
        let id = grant(&mut eval);
        store.inject(Fault::AfterCommit);
        let expected = if revoked {
            assert!(eval.revoke(id, context().now).is_err());
            DecisionReason::Revoked
        } else {
            assert!(eval
                .deny(
                    &request(),
                    GrantLifetime::Persistent,
                    GrantSource::Policy,
                    None,
                    UnixTimestamp::from_unix_seconds(2),
                    None
                )
                .is_err());
            DecisionReason::ExplicitlyDenied
        };
        let mut restored = evaluator(store.reload());
        assert_eq!(restored.explain(&request(), &context()).reason(), expected);
        assert_eq!(restored.check(&request(), &context()).reason(), expected);
    }
}

// Exercise rollback on the shipped deterministic store as well as the adapter model.
// A transaction must not publish partial consumption or grant-ID state on error.
#[test]
fn in_memory_store_rolls_back_operation_errors_and_invalid_candidates() {
    for operation_error in [true, false] {
        let shared = SharedStore::new();
        grant(&mut evaluator(shared.clone()));
        let mut store = InMemoryPolicyStore::from_json(shared.bytes().as_bytes()).unwrap();
        let before = store.to_json().unwrap();
        let result: Result<(), PolicyStoreError> = store.transact(|document| {
            let mut candidate = serde_json::to_value(&*document).unwrap();
            candidate["grants"][0]["consumed_at"] = serde_json::json!(10);
            candidate["next_grant_id"] = serde_json::json!(if operation_error { 3 } else { 1 });
            // Deserialize directly so the store, not this setup, validates the candidate.
            *document = serde_json::from_value(candidate).unwrap();
            if operation_error {
                document.validate().unwrap();
                Err(PolicyStoreError::Unavailable("operation aborted".into()))
            } else {
                Ok(())
            }
        });
        if operation_error {
            assert_eq!(
                result,
                Err(PolicyStoreError::Unavailable("operation aborted".into()))
            );
        } else {
            assert!(matches!(result, Err(PolicyStoreError::Malformed(_))));
        }
        assert_eq!(store.to_json().unwrap(), before);
        assert_eq!(store.snapshot().unwrap().grants()[0].consumed_at, None);
        // A new evaluator sees the original one-shot, with no partial ID allocation.
        let mut registry = CapabilityRegistry::new();
        registry
            .register(CapabilityDefinition {
                id: request().capability,
                description: "Host fake contract".into(),
            })
            .unwrap();
        let mut restored = PermissionEvaluator::new(registry, store);
        assert_eq!(
            restored.check(&request(), &context()).reason(),
            DecisionReason::AuthorizedByGrant
        );
        assert_eq!(
            restored.check(&request(), &context()).reason(),
            DecisionReason::OneShotConsumed
        );
        let next_id = restored
            .grant(
                &request(),
                GrantLifetime::OneShot,
                GrantSource::User,
                None,
                UnixTimestamp::from_unix_seconds(1),
                None,
            )
            .unwrap();
        assert_eq!(next_id.get(), 2);
    }
}
