use super::super::linux_init_c3_slot_journal::{C3DurableSlotReceipt, C3SlotState};
use super::super::linux_initial_login_c2::C1InitialLoginAuthority;
use super::super::linux_initial_login_owner::LinuxInitialLoginOwner;
use super::AuthenticatedBootService;
use crate::profile_volume_notes_owner::SelectedDataRoot;
use nagi_identity_session::{
    C1RecoveryOperationIdentity, C1SystemServiceRecoveryEffectGate,
    C1SystemServiceRecoveryReceiptAdapter, C1SystemServiceRuntimeHandoff,
    C1VerifiedBootSystemServicePrincipal, Clock, GuestProfileCleaner, IdempotencyKey,
    IdentityAuthorizer, IdentityError, IdentityEventSink, IdentityIdSource, IdentityService,
    LinuxIdentitySnapshotStore, RecoveryEffectGateError, UnixMillis,
};
use nagi_linux_account_verifier::LinuxAccountVerifier;
use nagi_system_service_recovery_contract::{
    C1RecoveryOperationCorrelation, SystemServiceRecoveryAdmissionScope,
    VerifiedRecoveryBootCatalog,
};
use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// Seal the caller while the exact C3 slot is live. The owned principal only
/// identifies the caller for C1; the recovery effect still needs its own gate.
pub(super) fn seal_boot_service_principal(
    boot: &AuthenticatedBootService<'_>,
    catalog: &VerifiedRecoveryBootCatalog,
) -> io::Result<C1VerifiedBootSystemServicePrincipal> {
    boot.with_verified_system_mapping(catalog, |binding| {
        C1VerifiedBootSystemServicePrincipal::from_live_binding(&binding)
    })?
    .map_err(|error| io::Error::new(io::ErrorKind::PermissionDenied, format!("{error:?}")))
}

#[derive(Debug)]
pub(super) enum BootIdentityRecoveryError {
    Principal(io::Error),
    Recovery(IdentityError),
}

/// Keep the authenticated C3 launch borrowed for as long as PID 1 retains
/// the recovered C1 runtime and the corresponding live C3 receipt.
pub(super) struct RecoveredBootIdentity<'boot, 'package, I, C, A, G, E> {
    _boot: &'boot AuthenticatedBootService<'package>,
    _receipt: C3DurableSlotReceipt,
    identity: C1InitialLoginAuthority<LinuxIdentitySnapshotStore, I, C, A, G, E>,
}

/// Recover the issuer and enter the first-login path while PID 1 still owns
/// the authenticated boot service. The Task24 launcher is supplied by the
/// existing owner and is consumed by the first-login adapter.
pub(super) fn recover_and_start_initial_login<'boot, 'package, I, C, A, G, E, Login>(
    boot: &'boot AuthenticatedBootService<'package>,
    catalog: &VerifiedRecoveryBootCatalog,
    mut identity: IdentityService<LinuxIdentitySnapshotStore, I, C, A, G, E>,
    verifier: LinuxAccountVerifier,
    request_key: IdempotencyKey,
    expires_at: Option<UnixMillis>,
    login: &mut Login,
) -> Result<
    (
        RecoveredBootIdentity<'boot, 'package, I, C, A, G, E>,
        Result<Login::Attached, Login::Error>,
    ),
    BootIdentityRecoveryError,
>
where
    I: IdentityIdSource,
    C: Clock,
    A: IdentityAuthorizer,
    G: GuestProfileCleaner,
    E: IdentityEventSink,
    Login:
        LinuxInitialLoginOwner<C1InitialLoginAuthority<LinuxIdentitySnapshotStore, I, C, A, G, E>>,
{
    let (handoff, receipt) = recover_boot_identity(boot, catalog, &mut identity)?;
    let runtime = RecoveredBootIdentity {
        _boot: boot,
        _receipt: receipt,
        identity: C1InitialLoginAuthority::new(
            identity,
            handoff,
            verifier,
            request_key,
            expires_at,
        )
        .map_err(BootIdentityRecoveryError::Recovery)?,
    };
    let attached = login.start_and_accept(&runtime.identity);
    Ok((runtime, attached))
}

struct BootRecoveryGate<'boot, 'package> {
    boot: &'boot AuthenticatedBootService<'package>,
    receipt: Option<C3DurableSlotReceipt>,
    used: bool,
}

impl C1SystemServiceRecoveryEffectGate for BootRecoveryGate<'_, '_> {
    fn apply_recovery_effect<R>(
        &mut self,
        scope: &dyn SystemServiceRecoveryAdmissionScope,
        operation: C1RecoveryOperationIdentity,
        effect: impl FnOnce() -> Result<R, IdentityError>,
    ) -> Result<Result<R, IdentityError>, RecoveryEffectGateError> {
        let admission = self.boot.recovery_scope();
        if self.used
            || !std::ptr::eq(scope, admission)
            || operation.binding_digest() != admission.binding_digest()
        {
            return Err(RecoveryEffectGateError::Unavailable);
        }
        self.used = true;
        let claim = C1RecoveryOperationCorrelation::new(
            operation.operation_id(),
            *operation.binding_digest(),
        );
        let (result, receipt) = self
            .boot
            .owner
            .c3
            .with_durable_live_effect_gate(admission, &self.boot.proof, claim, effect)
            .map_err(|_| RecoveryEffectGateError::Unavailable)?;
        if receipt.state() != C3SlotState::Live
            || receipt.c1_operation_id() != Some(claim.operation_id())
            || receipt.service_id() != admission.service_id()
            || receipt.catalog_generation() != admission.catalog_generation()
            || receipt.c3_launch_generation() != admission.c3_launch_generation()
            || receipt.scope_binding_digest() != claim.binding_digest()
        {
            return Err(RecoveryEffectGateError::Unavailable);
        }
        self.receipt = Some(receipt);
        Ok(result)
    }
}

/// Start recovery after principal issuance releases its gate and retain both
/// owners' evidence for the same authenticated boot launch.
pub(super) fn recover_boot_identity<I, C, A, G, E>(
    boot: &AuthenticatedBootService<'_>,
    catalog: &VerifiedRecoveryBootCatalog,
    identity: &mut IdentityService<LinuxIdentitySnapshotStore, I, C, A, G, E>,
) -> Result<(C1SystemServiceRuntimeHandoff, C3DurableSlotReceipt), BootIdentityRecoveryError>
where
    I: IdentityIdSource,
    C: Clock,
    A: IdentityAuthorizer,
    G: GuestProfileCleaner,
    E: IdentityEventSink,
{
    let caller =
        seal_boot_service_principal(boot, catalog).map_err(BootIdentityRecoveryError::Principal)?;
    let scope = boot.recovery_scope();
    let mut gate = BootRecoveryGate {
        boot,
        receipt: None,
        used: false,
    };
    let handoff =
        C1SystemServiceRecoveryReceiptAdapter::new_for_verified_boot_principal(identity, &caller)
            .recover_with_new_intent_into_runtime(scope, &mut gate)
            .map_err(BootIdentityRecoveryError::Recovery)?;
    let receipt = gate
        .receipt
        .filter(|receipt| {
            let operation = handoff.operation_identity();
            receipt.c1_operation_id() == Some(operation.operation_id())
                && receipt.scope_binding_digest() == operation.binding_digest()
        })
        .ok_or(BootIdentityRecoveryError::Recovery(
            IdentityError::StoreUnavailable,
        ))?;
    Ok((handoff, receipt))
}

/// Open the actual C1 snapshot store and retain the verifier on the same
/// owner-selected Data volume. Both relative paths come from the boot owner.
pub(super) fn open_boot_identity(
    data: &SelectedDataRoot,
    snapshot_path: &Path,
    account_path: &Path,
) -> io::Result<(LinuxIdentitySnapshotStore, LinuxAccountVerifier)> {
    let verifier = data.prepare_account_verifier(account_path)?;
    let directory = data.retain_profile_directory(snapshot_path)?;
    let metadata = directory.metadata()?;
    if metadata.uid() != 0 || metadata.mode() & 0o077 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "identity snapshot directory is not private to the boot owner",
        ));
    }
    let store = LinuxIdentitySnapshotStore::open(directory).map_err(|error| {
        io::Error::other(format!("identity snapshot directory rejected: {error:?}"))
    })?;
    Ok((store, verifier))
}
