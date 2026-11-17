use super::super::RuntimeIngress;
use super::*;
use crate::session_fixture::{poll_ready, Fixture, FixtureCaller};
use consumer::session_binding::AdmissionFailure;
use libnagi::security::Role;
use std::cell::Cell;
use std::sync::atomic::Ordering;
use svc::{CancellationToken, ContractVersion, OperationId, RequestEnvelope, RequestId, ServiceId};

const NAME: &[u8] = b"bound-test";
const PASSWORD: &[u8] = b"test-only-password";

// Explicit test-owned data, not a production account directory or authority.
struct IssuerLookup {
    subject: ProviderSubjectRef,
    user: Option<UserId>,
    error: Option<ProviderLookupFailure>,
    calls: Cell<usize>,
}
impl IdentityProvider for IssuerLookup {
    fn resolve_subject(
        &self,
        subject: &ProviderSubjectRef,
    ) -> Result<Option<UserId>, ProviderLookupFailure> {
        self.calls.set(self.calls.get() + 1);
        if let Some(error) = self.error {
            return Err(error);
        }
        Ok(if subject == &self.subject {
            self.user
        } else {
            None
        })
    }
}

struct Host {
    fixture: Fixture,
    accounts: AccountStore,
    bindings: Vec<ExplicitAccountBinding>,
    lookup: IssuerLookup,
}
fn host() -> Host {
    let mut fixture = Fixture::new();
    fixture
        .issuer
        .as_mut()
        .unwrap()
        .end_session(&fixture.caller, fixture.identity.session_id())
        .unwrap();
    let mut accounts = AccountStore::new();
    accounts
        .add_account(NAME, Role::Standard, PASSWORD)
        .unwrap();
    let subject = ProviderSubjectRef::new("test-issuer", "bound-account").unwrap();
    let user = fixture.identity.user_id().unwrap();
    let profile = fixture.identity.profile_id().clone();
    let lookup = IssuerLookup {
        subject: subject.clone(),
        user: Some(user),
        error: None,
        calls: Cell::new(0),
    };
    Host {
        fixture,
        accounts,
        bindings: vec![ExplicitAccountBinding {
            account_name: NAME.to_vec(),
            subject,
            user,
            profile,
        }],
        lookup,
    }
}
fn intent(key: &str) -> LoginIntent {
    LoginIntent {
        expires_at: Some(UnixMillis::new(200)),
        idempotency_key: IdempotencyKey::new(key).unwrap(),
    }
}
fn credentials() -> Credentials<'static> {
    Credentials {
        name: NAME,
        password: PASSWORD,
    }
}
fn request() -> RequestEnvelope {
    RequestEnvelope::new(
        RequestId::new(1).unwrap(),
        None,
        None,
        ServiceId::new("fixture.echo").unwrap(),
        ContractVersion::new(1, 0).unwrap(),
        OperationId::new("echo").unwrap(),
        b"bound login ingress".to_vec(),
    )
    .unwrap()
}

#[test]
fn binding_real_credentials_issue_existing_sdk_session_and_reach_ingress() {
    let mut host = host();
    let mut entry = AccountBindingIssuer::new(&mut host.accounts, &host.bindings, &host.lookup);
    let issuer = host.fixture.issuer.as_mut().unwrap();
    let count = issuer.snapshot().sessions.len();
    let session = entry
        .login(
            issuer,
            &host.fixture.caller,
            credentials(),
            intent("binding-login-1"),
        )
        .unwrap();
    assert_eq!(host.lookup.calls.get(), 1);
    assert_eq!(issuer.snapshot().sessions.len(), count + 1);
    let ingress = RuntimeIngress::supply(
        issuer,
        &host.fixture.caller,
        &host.fixture.client,
        CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(ingress.binding.identity().unwrap().session_id(), &session);
    assert_eq!(
        ingress.binding.identity().unwrap().user_id(),
        Some(host.bindings[0].user)
    );
    assert_eq!(
        ingress.binding.identity().unwrap().profile_id(),
        &host.bindings[0].profile
    );
    let response = poll_ready(ingress.call(request()).unwrap()).unwrap();
    assert_eq!(response.result().unwrap(), b"bound login ingress");
    assert_eq!(host.fixture.policy_calls.load(Ordering::SeqCst), 1);
    assert_eq!(host.fixture.provider_calls.load(Ordering::SeqCst), 1);
}

#[test]
fn binding_wrong_credentials_or_unknown_account_do_not_lookup_or_issue() {
    for unknown in [false, true] {
        let mut host = host();
        let issuer = host.fixture.issuer.as_mut().unwrap();
        let before = issuer.snapshot().clone();
        let mut entry = AccountBindingIssuer::new(&mut host.accounts, &host.bindings, &host.lookup);
        let credential = if unknown {
            Credentials {
                name: b"unknown-test",
                password: PASSWORD,
            }
        } else {
            Credentials {
                name: NAME,
                password: b"incorrect-test",
            }
        };
        let expected = if unknown {
            LoginError::InvalidAccount
        } else {
            LoginError::WrongPassword
        };
        assert_eq!(
            entry.login(
                issuer,
                &host.fixture.caller,
                credential,
                intent("binding-reject")
            ),
            Err(BoundLoginFailure::Credential(expected))
        );
        assert_eq!(host.lookup.calls.get(), 0);
        assert_eq!(issuer.snapshot(), &before);
    }
}

#[test]
fn binding_missing_or_ambiguous_records_do_not_issue() {
    for ambiguous in [false, true] {
        let mut host = host();
        if ambiguous {
            let first = &host.bindings[0];
            host.bindings.push(ExplicitAccountBinding {
                account_name: first.account_name.clone(),
                subject: first.subject.clone(),
                user: first.user,
                profile: first.profile.clone(),
            });
        } else {
            host.bindings.clear()
        }
        let issuer = host.fixture.issuer.as_mut().unwrap();
        let before = issuer.snapshot().clone();
        let mut entry = AccountBindingIssuer::new(&mut host.accounts, &host.bindings, &host.lookup);
        let expected = if ambiguous {
            BoundLoginFailure::AmbiguousBinding
        } else {
            BoundLoginFailure::MissingBinding
        };
        assert_eq!(
            entry.login(
                issuer,
                &host.fixture.caller,
                credentials(),
                intent("binding-reject")
            ),
            Err(expected)
        );
        assert_eq!(host.lookup.calls.get(), 0);
        assert_eq!(issuer.snapshot(), &before);
    }
}

#[test]
fn binding_unknown_mismatched_or_unavailable_subject_does_not_issue() {
    for mode in 0..3 {
        let mut host = host();
        let expected = match mode {
            0 => {
                host.lookup.user = None;
                BoundLoginFailure::UnmappedSubject
            }
            1 => {
                host.lookup.user = Some(UserId::new(999).unwrap());
                BoundLoginFailure::SubjectMismatch
            }
            _ => {
                host.lookup.error = Some(ProviderLookupFailure::Unavailable);
                BoundLoginFailure::Lookup(ProviderLookupFailure::Unavailable)
            }
        };
        let issuer = host.fixture.issuer.as_mut().unwrap();
        let before = issuer.snapshot().clone();
        let mut entry = AccountBindingIssuer::new(&mut host.accounts, &host.bindings, &host.lookup);
        assert_eq!(
            entry.login(
                issuer,
                &host.fixture.caller,
                credentials(),
                intent("binding-reject")
            ),
            Err(expected)
        );
        assert_eq!(host.lookup.calls.get(), 1);
        assert_eq!(issuer.snapshot(), &before);
    }
}

#[test]
fn binding_absent_sdk_user_or_profile_does_not_provision_or_issue() {
    for missing_user in [true, false] {
        let mut host = host();
        let expected = if missing_user {
            let user = UserId::new(999).unwrap();
            host.bindings[0].user = user;
            host.lookup.user = Some(user);
            BoundLoginFailure::UserNotFound
        } else {
            host.bindings[0].profile = ProfileId::new("profile_missing").unwrap();
            BoundLoginFailure::ProfileNotFound
        };
        let issuer = host.fixture.issuer.as_mut().unwrap();
        let before = issuer.snapshot().clone();
        let mut entry = AccountBindingIssuer::new(&mut host.accounts, &host.bindings, &host.lookup);
        assert_eq!(
            entry.login(
                issuer,
                &host.fixture.caller,
                credentials(),
                intent("binding-reject")
            ),
            Err(expected)
        );
        assert_eq!(issuer.snapshot(), &before);
    }
}

#[test]
fn binding_profile_owned_by_another_existing_user_does_not_issue() {
    let mut host = host();
    let issuer = host.fixture.issuer.as_mut().unwrap();
    let (_, profile) = issuer
        .create_local_user(
            &host.fixture.caller,
            identity::LocalUserMetadata::new("Other Test User").unwrap(),
            identity::ProfileMetadata::new("Other Test Profile", None).unwrap(),
        )
        .unwrap();
    host.bindings[0].profile = profile;
    let before = issuer.snapshot().clone();
    let mut entry = AccountBindingIssuer::new(&mut host.accounts, &host.bindings, &host.lookup);
    assert_eq!(
        entry.login(
            issuer,
            &host.fixture.caller,
            credentials(),
            intent("binding-reject")
        ),
        Err(BoundLoginFailure::ProfileOwnershipMismatch)
    );
    assert_eq!(issuer.snapshot(), &before);
}

#[test]
fn binding_correct_credentials_do_not_bypass_sdk_caller_authorization() {
    let mut host = host();
    let issuer = host.fixture.issuer.as_mut().unwrap();
    let before = issuer.snapshot().clone();
    let caller = FixtureCaller::new("fixture.other", "caller_fixture");
    let mut entry = AccountBindingIssuer::new(&mut host.accounts, &host.bindings, &host.lookup);
    assert_eq!(
        entry.login(issuer, &caller, credentials(), intent("binding-reject")),
        Err(BoundLoginFailure::Identity(
            IdentityError::AuthorizationDenied
        ))
    );
    assert_eq!(issuer.snapshot(), &before);
}

#[test]
fn binding_sdk_end_revokes_old_ingress_and_fresh_credentials_can_relogin() {
    let mut host = host();
    let issuer = host.fixture.issuer.as_mut().unwrap();
    let mut entry = AccountBindingIssuer::new(&mut host.accounts, &host.bindings, &host.lookup);
    let first = entry
        .login(
            issuer,
            &host.fixture.caller,
            credentials(),
            intent("binding-login-1"),
        )
        .unwrap();
    let ingress = RuntimeIngress::supply(
        issuer,
        &host.fixture.caller,
        &host.fixture.client,
        CancellationToken::new(),
    )
    .unwrap();
    issuer.end_session(&host.fixture.caller, &first).unwrap();
    assert!(matches!(
        ingress.call(request()),
        Err(AdmissionFailure::StaleIdentity)
    ));
    assert_eq!(host.fixture.provider_calls.load(Ordering::SeqCst), 0);
    let ended = issuer.snapshot().clone();
    assert_eq!(
        entry.login(
            issuer,
            &host.fixture.caller,
            Credentials {
                name: NAME,
                password: b"incorrect-test"
            },
            intent("binding-login-2")
        ),
        Err(BoundLoginFailure::Credential(LoginError::WrongPassword))
    );
    assert_eq!(issuer.snapshot(), &ended);
    let second = entry
        .login(
            issuer,
            &host.fixture.caller,
            credentials(),
            intent("binding-login-2"),
        )
        .unwrap();
    assert_ne!(first, second);
    let new_ingress = RuntimeIngress::supply(
        issuer,
        &host.fixture.caller,
        &host.fixture.client,
        CancellationToken::new(),
    )
    .unwrap();
    assert_eq!(
        new_ingress.binding.identity().unwrap().session_id(),
        &second
    );
    assert_eq!(
        poll_ready(new_ingress.call(request()).unwrap())
            .unwrap()
            .result()
            .unwrap(),
        b"bound login ingress"
    );
    assert_eq!(host.fixture.provider_calls.load(Ordering::SeqCst), 1);
}
