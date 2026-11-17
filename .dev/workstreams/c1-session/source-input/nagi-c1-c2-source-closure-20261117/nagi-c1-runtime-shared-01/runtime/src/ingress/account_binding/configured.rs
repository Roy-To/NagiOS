//! Owned in-memory configuration, not an authority or persistent generation.

use super::{AccountBindingIssuer, ExplicitAccountBinding};
use identity::{IdentityProvider, ProviderLookupFailure, ProviderSubjectRef, UserId};
use libnagi::security::AccountStore;

#[derive(Clone)]
pub(super) struct SubjectMapping {
    pub(super) subject: ProviderSubjectRef,
    pub(super) user: UserId,
}

pub(super) struct SubjectSnapshot {
    // Owned values only: no callback, Cell, shared mutable registry or mutator.
    rows: Vec<SubjectMapping>,
}

impl SubjectSnapshot {
    fn from_supplied(rows: Vec<SubjectMapping>) -> Result<Self, ProviderLookupFailure> {
        for (index, row) in rows.iter().enumerate() {
            if rows[..index]
                .iter()
                .any(|earlier| earlier.subject == row.subject)
            {
                return Err(ProviderLookupFailure::Unavailable);
            }
        }
        Ok(Self { rows })
    }
}

impl IdentityProvider for SubjectSnapshot {
    fn resolve_subject(
        &self,
        subject: &ProviderSubjectRef,
    ) -> Result<Option<UserId>, ProviderLookupFailure> {
        Ok(self
            .rows
            .iter()
            .find(|row| &row.subject == subject)
            .map(|row| row.user))
    }
}

pub(super) struct ConfiguredAccountInputs {
    accounts: AccountStore,
    bindings: Vec<ExplicitAccountBinding>,
    subjects: SubjectSnapshot,
}

impl ConfiguredAccountInputs {
    // The production owner must supply a coherent set from its authoritative
    // account configuration. This constructor cannot establish that truth.
    pub(super) fn from_supplied(
        accounts: AccountStore,
        bindings: Vec<ExplicitAccountBinding>,
        subjects: Vec<SubjectMapping>,
    ) -> Result<Self, ProviderLookupFailure> {
        Ok(Self {
            accounts,
            bindings,
            subjects: SubjectSnapshot::from_supplied(subjects)?,
        })
    }

    // This mutable borrow covers the whole cohort; callers cannot replace its
    // accounts or bindings while using the issuer view. Its concrete subject
    // provider also has no internal mutation, unlike the generic trait alone.
    pub(super) fn issuer(&mut self) -> AccountBindingIssuer<'_, '_, SubjectSnapshot> {
        AccountBindingIssuer::new(&mut self.accounts, &self.bindings, &self.subjects)
    }
}

// No hot-update API is provided. Replacing a cohort does not automatically
// invalidate SDK sessions: the runtime owner still arranges authorized
// end_session/recovery and cancellation before its configuration transition.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ingress::account_binding::{Credentials, LoginIntent};
    use crate::ingress::RuntimeIngress;
    use crate::session_fixture::{poll_ready, Fixture};
    use identity::{IdempotencyKey, UnixMillis};
    use libnagi::security::Role;
    use std::cell::Cell;
    use svc::{
        CancellationToken, ContractVersion, OperationId, RequestEnvelope, RequestId, ServiceId,
    };

    #[test]
    fn configured_snapshot_owns_values_while_generic_provider_can_mutate() {
        struct MutableLookup(Cell<UserId>);
        impl IdentityProvider for MutableLookup {
            fn resolve_subject(
                &self,
                _: &ProviderSubjectRef,
            ) -> Result<Option<UserId>, ProviderLookupFailure> {
                Ok(Some(self.0.get()))
            }
        }
        let subject = ProviderSubjectRef::new("test-issuer", "snapshot-account").unwrap();
        let first = UserId::new(1).unwrap();
        let later = UserId::new(2).unwrap();
        let live = MutableLookup(Cell::new(first));
        let alias = &live;
        let mut source_rows = vec![SubjectMapping {
            subject: subject.clone(),
            user: first,
        }];
        let snapshot = SubjectSnapshot::from_supplied(source_rows.clone()).unwrap();
        source_rows[0].user = later;
        live.0.set(later);
        assert_eq!(alias.resolve_subject(&subject).unwrap(), Some(later));
        assert_eq!(snapshot.resolve_subject(&subject).unwrap(), Some(first));
    }

    #[test]
    fn configured_snapshot_rejects_duplicate_subjects_without_claiming_authority() {
        let subject = ProviderSubjectRef::new("test-issuer", "duplicate-account").unwrap();
        let rows = vec![
            SubjectMapping {
                subject: subject.clone(),
                user: UserId::new(1).unwrap(),
            },
            SubjectMapping {
                subject,
                user: UserId::new(2).unwrap(),
            },
        ];
        assert!(matches!(
            SubjectSnapshot::from_supplied(rows),
            Err(ProviderLookupFailure::Unavailable)
        ));
    }

    #[test]
    fn configured_owned_cohort_reaches_real_sdk_login_and_ingress() {
        let mut fixture = Fixture::new();
        fixture
            .issuer
            .as_mut()
            .unwrap()
            .end_session(&fixture.caller, fixture.identity.session_id())
            .unwrap();
        let mut accounts = AccountStore::new();
        accounts
            .add_account(b"owned-test", Role::Standard, b"owned-test-secret")
            .unwrap();
        let subject = ProviderSubjectRef::new("test-issuer", "owned-account").unwrap();
        let user = fixture.identity.user_id().unwrap();
        let profile = fixture.identity.profile_id().clone();
        let bindings = vec![ExplicitAccountBinding {
            account_name: b"owned-test".to_vec(),
            subject: subject.clone(),
            user,
            profile,
        }];
        let mut inputs = ConfiguredAccountInputs::from_supplied(
            accounts,
            bindings,
            vec![SubjectMapping { subject, user }],
        )
        .unwrap();
        let session = inputs
            .issuer()
            .login(
                fixture.issuer.as_mut().unwrap(),
                &fixture.caller,
                Credentials {
                    name: b"owned-test",
                    password: b"owned-test-secret",
                },
                LoginIntent {
                    expires_at: Some(UnixMillis::new(200)),
                    idempotency_key: IdempotencyKey::new("owned-login").unwrap(),
                },
            )
            .unwrap();
        let ingress = RuntimeIngress::supply(
            fixture.issuer.as_mut().unwrap(),
            &fixture.caller,
            &fixture.client,
            CancellationToken::new(),
        )
        .unwrap();
        assert_eq!(ingress.binding.identity().unwrap().session_id(), &session);
        let request = RequestEnvelope::new(
            RequestId::new(1).unwrap(),
            None,
            None,
            ServiceId::new("fixture.echo").unwrap(),
            ContractVersion::new(1, 0).unwrap(),
            OperationId::new("echo").unwrap(),
            b"owned cohort ingress".to_vec(),
        )
        .unwrap();
        assert_eq!(
            poll_ready(ingress.call(request).unwrap())
                .unwrap()
                .result()
                .unwrap(),
            b"owned cohort ingress"
        );
    }
}
