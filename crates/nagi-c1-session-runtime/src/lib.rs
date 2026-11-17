//! Linux C1 runtime adapter for durable Sessions and the existing local
//! AccountRecord verifier. C2 still supplies the trusted caller context and
//! C3 still owns the authorized directory handle and login UI application.

#![forbid(unsafe_code)]

use std::collections::HashSet;
use std::fmt;
use std::fs::File;

use libnagi::credential::AccountRecord;
use libnagi::security::{AccountStore, Role};
use nagi_identity_session::{
    AuthenticatedInteractiveCaller, AuthenticatedSystemServicePrincipal, CurrentIdentityResolver,
    DurableStoreError, GuestProfileCleaner, IdentityAuthorizer, IdentityError, IdentityEventSink,
    IdentityService, InteractiveSessionContext, LinuxIdentitySnapshotStore, LiveSnapshot,
    LocalUserMetadata, NoopIdentityEventSink, ProfileId, ProfileKind, ProfileMetadata,
    RecoveryReport, ResolutionError, SessionId, TrustedCallerContext, TryLiveError, UnixMillis,
    UserId,
};

mod clock;
mod ids;

pub use clock::{ClockFailure, LinuxIdentityClock};
pub use ids::LinuxIdentityIdSource;

/// A verifier record and an explicit mapping to an already persisted local
/// User/Profile. This is configuration supplied by the trusted integration;
/// the account name and IDs are never accepted from an app payload as proof.
///
/// The verifier record is copied in memory but is never written into the C1
/// Session store. Only the authorized caller that loads the OS account record
/// should construct and pass this value.
///
/// C1 persists the mapped User/Profile records and Session lifecycle. The
/// account-to-User/Profile mapping remains trusted integration configuration;
/// its authoritative source and any persistence belong to the account or
/// provisioning owner. This adapter checks that each mapping resolves to an
/// existing persistent profile owned by the mapped User.
pub struct LocalAccountBinding {
    account: AccountRecord,
    user_id: UserId,
    profile_id: ProfileId,
}

impl LocalAccountBinding {
    pub fn new(account: AccountRecord, user_id: UserId, profile_id: ProfileId) -> Self {
        Self {
            account,
            user_id,
            profile_id,
        }
    }

    fn account_name(&self) -> &[u8] {
        self.account.name()
    }
}

/// Failure from the C1 Linux Session runtime. Authentication errors have one
/// public result so callers cannot distinguish an unknown account from a bad
/// password.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RuntimeError {
    Store(DurableStoreError),
    Identity(IdentityError),
    Resolution(ResolutionError),
    AuthenticationFailed,
    AccountMappingUnavailable,
    InvalidAccountMapping,
    ClockUnavailable,
    Live(TryLiveError),
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Store(DurableStoreError::Unavailable) => "C1 durable store is unavailable",
            Self::Store(DurableStoreError::Corrupt) => "C1 durable store is corrupt",
            Self::Store(DurableStoreError::Locked) => "C1 durable store already has an issuer",
            Self::Identity(error) => return write!(f, "C1 identity operation failed: {error}"),
            Self::Resolution(_) => "C1 current identity could not be resolved",
            Self::AuthenticationFailed => "local account authentication failed",
            Self::AccountMappingUnavailable => "authenticated account has no local profile mapping",
            Self::InvalidAccountMapping => "local account profile mapping is invalid",
            Self::ClockUnavailable => "C1 clock is unavailable",
            Self::Live(_) => "C1 Session liveness could not be observed",
        })
    }
}

impl std::error::Error for RuntimeError {}

/// A single-process issuer over the durable Linux snapshot store.
///
/// The caller supplies the trusted identity authorizer, guest cleaner, event
/// sink, service-recovery caller, and private directory descriptor. The C1
/// runtime does not create a service principal from an interactive Session.
pub struct LinuxSessionRuntime<A, G, E = NoopIdentityEventSink> {
    service: IdentityService<
        LinuxIdentitySnapshotStore,
        LinuxIdentityIdSource,
        LinuxIdentityClock,
        A,
        G,
        E,
    >,
    accounts: Vec<LocalAccountBinding>,
    recovery: RecoveryReport,
    clock: LinuxIdentityClock,
}

impl<A, G, E> LinuxSessionRuntime<A, G, E>
where
    A: IdentityAuthorizer,
    G: GuestProfileCleaner,
    E: IdentityEventSink,
{
    /// Start a real issuer from an externally authorized store directory and
    /// authenticated account records. Recovery ends all saved active Sessions;
    /// this method never restores one as current.
    pub fn start(
        directory: File,
        accounts: Vec<LocalAccountBinding>,
        authorizer: A,
        guest_cleaner: G,
        events: E,
        recovery_caller: &impl AuthenticatedSystemServicePrincipal,
    ) -> Result<Self, RuntimeError> {
        validate_account_names(&accounts)?;
        let store = LinuxIdentitySnapshotStore::open(directory).map_err(RuntimeError::Store)?;
        let clock = LinuxIdentityClock::open().map_err(|_| RuntimeError::ClockUnavailable)?;
        let mut service = IdentityService::new(
            store,
            LinuxIdentityIdSource,
            clock.clone(),
            authorizer,
            guest_cleaner,
            events,
        );
        let recovery = service
            .recover(recovery_caller)
            .map_err(RuntimeError::Identity)?;
        clock
            .checked_now()
            .map_err(|_| RuntimeError::ClockUnavailable)?;
        validate_account_profiles(&accounts, service.snapshot())?;
        Ok(Self {
            service,
            accounts,
            recovery,
            clock,
        })
    }

    pub fn recovery_report(&self) -> &RecoveryReport {
        &self.recovery
    }

    pub fn snapshot(&self) -> &nagi_identity_session::IdentitySnapshot {
        self.service.snapshot()
    }

    /// Create a durable User/Profile using the existing C1 service policy.
    /// Account mapping remains explicit configuration; this operation does
    /// not infer it from a caller label or account name.
    pub fn create_local_user(
        &mut self,
        caller: &impl TrustedCallerContext,
        user: LocalUserMetadata,
        profile: ProfileMetadata,
    ) -> Result<(UserId, ProfileId), RuntimeError> {
        self.ensure_clock()?;
        self.service
            .create_local_user(caller, user, profile)
            .map_err(RuntimeError::Identity)
    }

    /// Authenticate against the existing persisted local AccountRecord, then
    /// ask C1 to issue and commit a new Session for its explicit User/Profile
    /// mapping. The legacy libnagi Session is discarded and is never C1
    /// authority. Expiry is required from the caller; this adapter chooses no
    /// Session lifetime.
    pub fn authenticate_and_login(
        &mut self,
        caller: &impl AuthenticatedInteractiveCaller,
        account_name: &[u8],
        password: &[u8],
        expires_at: UnixMillis,
        idempotency_key: &nagi_identity_session::IdempotencyKey,
    ) -> Result<SessionId, RuntimeError> {
        self.ensure_clock()?;
        let binding = self
            .accounts
            .iter()
            .find(|binding| binding.account_name() == account_name)
            .ok_or(RuntimeError::AuthenticationFailed)?;
        let mut verifier = AccountStore::new();
        verifier
            .add_account_with_credential(
                binding.account.name(),
                Role::Owner,
                binding.account.credential,
            )
            .map_err(|_| RuntimeError::AuthenticationFailed)?;
        let _ = verifier
            .authenticate(account_name, password)
            .map_err(|_| RuntimeError::AuthenticationFailed)?;
        self.ensure_clock()?;

        self.service
            .login_local(
                caller,
                binding.user_id,
                binding.profile_id.clone(),
                Some(expires_at),
                idempotency_key.clone(),
            )
            .map_err(RuntimeError::Identity)
    }

    /// Resolve only the Session currently issued to the trusted caller.
    pub fn interactive_session<'caller>(
        &mut self,
        caller: &'caller impl AuthenticatedInteractiveCaller,
    ) -> Result<InteractiveSessionContext<'caller>, RuntimeError> {
        self.ensure_clock()?;
        self.service
            .interactive_session(caller)
            .map_err(|error| match error {
                nagi_identity_session::InteractiveSessionError::CallerMismatch => {
                    RuntimeError::Resolution(ResolutionError::Unavailable)
                }
                nagi_identity_session::InteractiveSessionError::Identity(error) => {
                    RuntimeError::Resolution(error)
                }
                nagi_identity_session::InteractiveSessionError::Live(error) => {
                    RuntimeError::Live(error)
                }
            })
    }

    /// Revalidate an existing context immediately before protected work.
    pub fn revalidate(&self, context: &InteractiveSessionContext<'_>) -> Result<(), RuntimeError> {
        self.ensure_clock()?;
        context.check().map_err(|error| match error {
            nagi_identity_session::InteractiveSessionError::CallerMismatch => {
                RuntimeError::Resolution(ResolutionError::Unavailable)
            }
            nagi_identity_session::InteractiveSessionError::Identity(error) => {
                RuntimeError::Resolution(error)
            }
            nagi_identity_session::InteractiveSessionError::Live(error) => {
                RuntimeError::Live(error)
            }
        })
    }

    /// Nonblocking point-in-time observation for connection maintenance. It
    /// is not a substitute for `revalidate` before a protected operation.
    pub fn try_live_snapshot(
        &self,
        context: &InteractiveSessionContext<'_>,
    ) -> Result<LiveSnapshot, RuntimeError> {
        context.try_live_snapshot().map_err(|error| match error {
            nagi_identity_session::InteractiveSessionError::CallerMismatch => {
                RuntimeError::Resolution(ResolutionError::Unavailable)
            }
            nagi_identity_session::InteractiveSessionError::Identity(error) => {
                RuntimeError::Resolution(error)
            }
            nagi_identity_session::InteractiveSessionError::Live(error) => {
                RuntimeError::Live(error)
            }
        })
    }

    /// End the current Session without accepting a caller-selected SessionId.
    /// The old lease is revalidated first and all following access fails closed.
    pub fn logout_interactive(
        &mut self,
        caller: &impl AuthenticatedInteractiveCaller,
    ) -> Result<(), RuntimeError> {
        let context = self.interactive_session(caller)?;
        let session_id = context
            .identity()
            .map_err(|error| match error {
                nagi_identity_session::InteractiveSessionError::CallerMismatch => {
                    RuntimeError::Resolution(ResolutionError::Unavailable)
                }
                nagi_identity_session::InteractiveSessionError::Identity(error) => {
                    RuntimeError::Resolution(error)
                }
                nagi_identity_session::InteractiveSessionError::Live(error) => {
                    RuntimeError::Live(error)
                }
            })?
            .session_id()
            .clone();
        drop(context);
        self.service
            .end_session(caller, &session_id)
            .map(|_| ())
            .map_err(RuntimeError::Identity)
    }

    /// Repeat explicit service recovery. Old leases are revoked even when the
    /// selected User/Profile remains available for later authenticated login.
    pub fn recover(
        &mut self,
        service_caller: &impl AuthenticatedSystemServicePrincipal,
    ) -> Result<RecoveryReport, RuntimeError> {
        self.ensure_clock()?;
        self.service
            .recover(service_caller)
            .map_err(RuntimeError::Identity)
    }

    fn ensure_clock(&self) -> Result<(), RuntimeError> {
        self.clock
            .checked_now()
            .map(|_| ())
            .map_err(|_| RuntimeError::ClockUnavailable)
    }
}

impl<A, G, E> CurrentIdentityResolver for LinuxSessionRuntime<A, G, E>
where
    A: IdentityAuthorizer,
    G: GuestProfileCleaner,
    E: IdentityEventSink,
{
    fn resolve_current_identity(
        &mut self,
        caller: &impl AuthenticatedInteractiveCaller,
    ) -> Result<nagi_identity_session::ResolvedIdentity, ResolutionError> {
        self.ensure_clock()
            .map_err(|_| ResolutionError::Unavailable)?;
        self.service.resolve_current_identity(caller)
    }
}

fn validate_account_names(accounts: &[LocalAccountBinding]) -> Result<(), RuntimeError> {
    let mut names = HashSet::new();
    if accounts
        .iter()
        .any(|account| account.account_name().is_empty() || !names.insert(account.account_name()))
    {
        return Err(RuntimeError::InvalidAccountMapping);
    }
    Ok(())
}

fn validate_account_profiles(
    accounts: &[LocalAccountBinding],
    snapshot: &nagi_identity_session::IdentitySnapshot,
) -> Result<(), RuntimeError> {
    for account in accounts {
        let user_exists = snapshot
            .users
            .iter()
            .any(|user| user.user_id == account.user_id);
        let profile_exists = snapshot.profiles.iter().any(|profile| {
            profile.profile_id == account.profile_id
                && profile.owner_user_id == Some(account.user_id)
                && profile.kind == ProfileKind::Persistent
        });
        if !user_exists || !profile_exists {
            return Err(RuntimeError::InvalidAccountMapping);
        }
    }
    Ok(())
}
