use crate::{
    validate_artifact_preflight_report, validate_descriptor_json, ArtifactPreflightRequest,
    DescriptorError, PackageBindingError, PreflightReportError, ValidatedSystemServiceDescriptor,
    VerifiedServicePackageBinding,
};
use nagi_init_package_provider::package_trust::PackageServiceSessionOwner;
use nagi_system_principal_contract::SystemServiceEndpointOwnerProof;
use std::os::fd::{AsFd, BorrowedFd};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LaunchPortError {
    Unavailable,
    Rejected,
    Failed,
    OutcomeUnknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProofValidationError {
    WrongVersion,
    WrongLaunchGeneration,
    WrongPeerCredentials,
    MissingEndpoint,
    NotLive,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LaunchError {
    Descriptor(DescriptorError),
    PackageBinding(PackageBindingError),
    Catalog(LaunchPortError),
    ArtifactPreflight(LaunchPortError),
    ArtifactReport(PreflightReportError),
    ProcessOwner(LaunchPortError),
    InvalidOwnerProof(ProofValidationError),
    OwnerGateUnavailable,
    Identity(LaunchPortError),
    IdentityOutcomeUnknown,
    IdentityReturnedDifferentProof,
    OwnerRevoke(LaunchPortError),
}

/// Authenticated catalog source for every field that affects role or launch.
/// Implementations must compare the full descriptor with the trusted catalog;
/// caller-supplied JSON and an artifact digest do not establish that match.
pub trait TrustedSystemServiceCatalog {
    fn authenticate(
        &mut self,
        descriptor: &ValidatedSystemServiceDescriptor,
    ) -> Result<(), LaunchPortError>;
}

/// Artifact adapter must inspect the requested file and return that same open
/// object. Returning a CLI report and reopening `artifact_path` is forbidden.
/// The current Python helper retains its FD in-process; a Rust implementation
/// needs a concrete FD handoff before this port can be connected in product.
pub trait ArtifactPreflight {
    type HeldArtifact: AsFd;

    fn inspect_and_retain(
        &mut self,
        request: &ArtifactPreflightRequest,
    ) -> Result<(Self::HeldArtifact, String), LaunchPortError>;
}

/// C3 implementation binds the exact retained FD to its maintained process
/// launch, Supervisor generation, and connected endpoint. It must use the
/// concrete proof v2 contract and never construct a proof from descriptor data.
pub trait SystemServiceLaunchOwner {
    fn launch_and_authenticate(
        &mut self,
        descriptor: &ValidatedSystemServiceDescriptor,
        package: &VerifiedServicePackageBinding<'_>,
        executable: BorrowedFd<'_>,
    ) -> Result<SystemServiceEndpointOwnerProof, LaunchPortError>;

    /// Revoke the exact C3 Supervisor slot identified by this service and
    /// proof generation. C3 owns registry mutation; A never calls its unsafe
    /// record constructor or substitutes UID/PID as a revocation key.
    fn revoke_exact(
        &mut self,
        descriptor: &ValidatedSystemServiceDescriptor,
        proof: &SystemServiceEndpointOwnerProof,
    ) -> Result<(), LaunchPortError>;
}

/// C1-owned private seal/recovery adapter. The implementation receives a
/// bounded request from the launch owner, maps the catalog service to canonical
/// C1 IDs, seals the C3 proof privately, and retains it in the returned runtime.
/// This bootstrap operation is independent of any user Session and must not
/// perform user-session-protected effects. Such effects need a separate adapter
/// that acquires the C1 Session gate before the C3/C2 owner gate. It must not
/// capture an IdentityService owner in a C1 session-effect callback.
pub trait C1SystemServiceRecoveryClient {
    type Runtime;

    fn recover_system_service(
        &mut self,
        descriptor: &ValidatedSystemServiceDescriptor,
        package: &VerifiedServicePackageBinding<'_>,
        proof: Arc<SystemServiceEndpointOwnerProof>,
    ) -> Result<Self::Runtime, C1RecoveryFailure>;
}

/// C1 returns the same proof on a rejected recovery so A can revoke its exact
/// C3 record. The error is deliberately bounded to the launch port contract.
pub struct C1RecoveryFailure {
    error: LaunchPortError,
    proof: Arc<SystemServiceEndpointOwnerProof>,
}

impl C1RecoveryFailure {
    pub fn new(error: LaunchPortError, proof: Arc<SystemServiceEndpointOwnerProof>) -> Self {
        Self { error, proof }
    }

    pub fn error(&self) -> LaunchPortError {
        self.error
    }

    pub fn proof(&self) -> &Arc<SystemServiceEndpointOwnerProof> {
        &self.proof
    }
}

/// A successfully recovered C1 runtime paired with the retained C3 proof.
/// This type has no `READY` method; liveness snapshots are for monitoring only.
pub struct SystemServiceRuntime<'owner, R> {
    descriptor: ValidatedSystemServiceDescriptor,
    identity_runtime: R,
    proof: Arc<SystemServiceEndpointOwnerProof>,
    package: VerifiedServicePackageBinding<'owner>,
}

impl<R> SystemServiceRuntime<'_, R> {
    pub fn service_id(&self) -> &nagi_service_contract::ServiceId {
        self.descriptor.service_id()
    }

    pub fn generation(&self) -> std::num::NonZeroU64 {
        self.descriptor.generation()
    }

    pub fn identity_runtime(&self) -> &R {
        &self.identity_runtime
    }

    pub fn package_binding(&self) -> &VerifiedServicePackageBinding<'_> {
        &self.package
    }

    pub fn owner_liveness_snapshot(&self) -> std::io::Result<bool> {
        self.proof.is_live()
    }
}

/// Validate, inspect, launch, authenticate the owner proof, then ask C1 to
/// recover. Failures stop before the next owner; after C3 launch, all failures
/// revoke the exact C3 slot. The C3 generation gate remains held during the
/// user-session-independent C1 bootstrap recovery. The captured `identity`
/// port must be a bounded recovery client, not an IdentityService owner. This
/// path does not perform user-session-protected effects; those require a
/// separate adapter that takes the C1 Session gate before C3/C2 owner gates,
/// without capturing the issuing IdentityService owner in the callback.
pub fn launch_identity_service<'owner, C, P, O, I>(
    descriptor_json: &str,
    package_bytes: &[u8],
    package_owner: &'owner PackageServiceSessionOwner<'_>,
    catalog: &mut C,
    preflight: &mut P,
    owner: &mut O,
    identity: &mut I,
) -> Result<SystemServiceRuntime<'owner, I::Runtime>, LaunchError>
where
    C: TrustedSystemServiceCatalog,
    P: ArtifactPreflight,
    O: SystemServiceLaunchOwner,
    I: C1SystemServiceRecoveryClient,
{
    let descriptor = validate_descriptor_json(descriptor_json).map_err(LaunchError::Descriptor)?;
    catalog
        .authenticate(&descriptor)
        .map_err(LaunchError::Catalog)?;

    let request = descriptor.artifact_preflight_request();
    let (artifact, report_json) = preflight
        .inspect_and_retain(&request)
        .map_err(LaunchError::ArtifactPreflight)?;
    validate_artifact_preflight_report(&report_json, &descriptor)
        .map_err(LaunchError::ArtifactReport)?;

    let package = crate::bind_installed_service_package(&descriptor, package_bytes, package_owner)
        .map_err(LaunchError::PackageBinding)?;

    let proof = owner
        .launch_and_authenticate(&descriptor, &package, artifact.as_fd())
        .map_err(LaunchError::ProcessOwner)?;
    let proof = Arc::new(proof);
    if let Err(error) = validate_owner_proof(&descriptor, &proof) {
        owner
            .revoke_exact(&descriptor, &proof)
            .map_err(LaunchError::OwnerRevoke)?;
        return Err(LaunchError::InvalidOwnerProof(error));
    }

    let recovery = proof.with_live_authority_effect(|| {
        identity.recover_system_service(&descriptor, &package, Arc::clone(&proof))
    });
    match recovery {
        Err(_) => {
            owner
                .revoke_exact(&descriptor, &proof)
                .map_err(LaunchError::OwnerRevoke)?;
            Err(LaunchError::OwnerGateUnavailable)
        }
        Ok(Err(failure)) => {
            if !Arc::ptr_eq(&proof, failure.proof()) {
                owner
                    .revoke_exact(&descriptor, &proof)
                    .map_err(LaunchError::OwnerRevoke)?;
                return Err(LaunchError::IdentityReturnedDifferentProof);
            }
            owner
                .revoke_exact(&descriptor, failure.proof())
                .map_err(LaunchError::OwnerRevoke)?;
            if failure.error() == LaunchPortError::OutcomeUnknown {
                // Stop the launched process and require C1 reconciliation
                // before any later attempt against this Identity store.
                Err(LaunchError::IdentityOutcomeUnknown)
            } else {
                Err(LaunchError::Identity(failure.error()))
            }
        }
        Ok(Ok(identity_runtime)) => Ok(SystemServiceRuntime {
            descriptor,
            identity_runtime,
            proof,
            package,
        }),
    }
}

fn validate_owner_proof(
    descriptor: &ValidatedSystemServiceDescriptor,
    proof: &SystemServiceEndpointOwnerProof,
) -> Result<(), ProofValidationError> {
    if proof.version()
        != nagi_system_principal_contract::SYSTEM_SERVICE_ENDPOINT_OWNER_PROOF_VERSION
    {
        return Err(ProofValidationError::WrongVersion);
    }
    if proof.launch_generation() != descriptor.generation() {
        return Err(ProofValidationError::WrongLaunchGeneration);
    }
    let (expected_uid, expected_gid) = descriptor.credentials();
    let peer = proof.peer_credentials();
    if peer.uid != expected_uid || peer.gid != expected_gid {
        return Err(ProofValidationError::WrongPeerCredentials);
    }
    if proof.endpoint().is_none() {
        return Err(ProofValidationError::MissingEndpoint);
    }
    if !proof.is_live().map_err(|_| ProofValidationError::NotLive)? {
        return Err(ProofValidationError::NotLive);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{bind_installed_service_package, GenerationTransition, SYSTEM_SERVICE_TARGET};
    use ed25519_dalek::{Signer, SigningKey};
    use nagi_capability::PrincipalKind;
    use nagi_init_package_provider::package_trust::PackageServiceSessionOwner;
    use nagi_package::{
        build_xapp, InstallPolicy, PackageService, MAX_PACKAGE_BYTES, SIGNATURE_BYTES,
        TRUSTED_SIGNING_PUBLIC_KEY,
    };
    use serde_json::{json, Value};
    use sha2::{Digest, Sha256};
    use std::fs::File;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const DIGEST: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const TEST_SIGNING_SEED: [u8; 32] = [
        0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c,
        0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae,
        0x7f, 0x60,
    ];
    const FIXTURE_MANIFEST: &[u8] =
        b"id=org.nagi.fixture\nname=Fixture\nversion=1.0.0\nentry=fixture.elf\nsurfaces=compact\n";

    fn valid_descriptor() -> Value {
        json!({
            "schema_version": 1,
            "service_id": "fixture.identity",
            "role": "system_service",
            "package": { "id": "org.nagi.fixture", "canonical_entry": "fixture.elf" },
            "executable": {
                "artifact_path": "/fixture/identity.elf",
                "expected_sha256": DIGEST,
                "target": SYSTEM_SERVICE_TARGET
            },
            "credentials": { "uid": 1001, "gid": 1002 },
            "generation": 9,
            "lifecycle": { "mode": "initial" },
            "endpoint": {
                "transport": "unix_stream",
                "owner": "c3_pidfd_supervisor_generation",
                "handoff": "connected_owned_fd",
                "generation_policy": "unique_per_connection",
                "expected_peer_uid": 1001,
                "expected_peer_gid": 1002
            }
        })
    }

    fn valid_report() -> String {
        report_for_digest(DIGEST)
    }

    fn report_for_digest(digest: &str) -> String {
        json!({
            "status": "artifact_preflight_passed",
            "schema_version": 1,
            "target": SYSTEM_SERVICE_TARGET,
            "artifact_sha256": digest,
            "elf_type": "ET_DYN",
            "program_header_count": 10,
            "checks": {
                "regular_file": true,
                "no_symlink_path_components": true,
                "sha256_matches_expected": true,
                "elf64_little_endian": true,
                "x86_64": true,
                "executable_elf": true,
                "no_pt_interp": true
            },
            "claim_scope": "artifact_preflight_only",
            "not_proven": [
                "signature", "canonical_package_identity", "service_role",
                "user_authentication", "policy_authorization", "installation",
                "runtime_ready"
            ]
        })
        .to_string()
    }

    fn digest_hex(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn fixture_package(executable: &[u8]) -> Vec<u8> {
        let key = SigningKey::from_bytes(&TEST_SIGNING_SEED);
        assert_eq!(key.verifying_key().to_bytes(), TRUSTED_SIGNING_PUBLIC_KEY);
        let placeholder = [0; SIGNATURE_BYTES];
        let mut bytes = [0; MAX_PACKAGE_BYTES];
        let length = build_xapp(
            FIXTURE_MANIFEST,
            executable,
            b"",
            b"",
            b"",
            &placeholder,
            &mut bytes,
        )
        .expect("build package fixture");
        let signature = key.sign(&bytes[..length - SIGNATURE_BYTES]);
        bytes[length - SIGNATURE_BYTES..length].copy_from_slice(&signature.to_bytes());
        bytes[..length].to_vec()
    }

    fn installed_fixture_owner(package_bytes: &[u8]) -> PackageServiceSessionOwner<'static> {
        let mut owner = PackageServiceSessionOwner::new(
            Some(&TRUSTED_SIGNING_PUBLIC_KEY),
            PackageService::new(),
        );
        owner
            .install(
                package_bytes,
                InstallPolicy {
                    developer_mode: false,
                    require_signature: true,
                },
            )
            .expect("install test package under the fixture key");
        owner
    }

    fn empty_package_owner() -> PackageServiceSessionOwner<'static> {
        PackageServiceSessionOwner::new(None, PackageService::new())
    }

    fn descriptor_with_digest(digest: &str) -> String {
        let mut descriptor = valid_descriptor();
        descriptor["executable"]["expected_sha256"] = json!(digest);
        descriptor.to_string()
    }

    #[test]
    fn accepts_only_explicit_system_service_descriptor_shape() {
        let parsed = validate_descriptor_json(&valid_descriptor().to_string()).unwrap();
        assert_eq!(parsed.role(), PrincipalKind::SystemService);
        assert_eq!(parsed.service_id().as_str(), "fixture.identity");
        assert_eq!(parsed.package_id(), "org.nagi.fixture");
        assert_eq!(parsed.canonical_entry(), "fixture.elf");
        assert_eq!(
            parsed.package_app_id(),
            nagi_model::AppId::from_identifier(b"org.nagi.fixture")
        );
        assert_eq!(parsed.credentials(), (1001, 1002));
        assert_eq!(parsed.generation().get(), 9);
        assert_eq!(parsed.transition(), GenerationTransition::Initial);
        let request: Value =
            serde_json::from_str(&parsed.artifact_preflight_request().to_json().unwrap()).unwrap();
        assert_eq!(request["schema_version"], 1);
        assert_eq!(request["artifact_path"], "/fixture/identity.elf");
        assert_eq!(request["expected_sha256"], DIGEST);
        assert_eq!(request["target"], SYSTEM_SERVICE_TARGET);
    }

    #[test]
    fn rejects_missing_unknown_duplicate_and_malformed_fields() {
        let mut missing = valid_descriptor();
        missing.as_object_mut().unwrap().remove("endpoint");
        assert_eq!(
            validate_descriptor_json(&missing.to_string()),
            Err(DescriptorError::InvalidJson)
        );

        let mut unknown = valid_descriptor();
        unknown["unexpected"] = json!(true);
        assert_eq!(
            validate_descriptor_json(&unknown.to_string()),
            Err(DescriptorError::InvalidJson)
        );

        let duplicate = valid_descriptor().to_string().replacen(
            "\"schema_version\":1",
            "\"schema_version\":1,\"schema_version\":1",
            1,
        );
        assert_eq!(
            validate_descriptor_json(&duplicate),
            Err(DescriptorError::InvalidJson)
        );
        assert_eq!(
            validate_descriptor_json(" "),
            Err(DescriptorError::InvalidJson)
        );
    }

    #[test]
    fn rejects_role_id_digest_target_and_path_mismatches() {
        let cases = [
            ("role", json!("first_party_app"), DescriptorError::WrongRole),
            (
                "service_id",
                json!("Identity"),
                DescriptorError::InvalidServiceId,
            ),
            (
                "package.id",
                json!("Org.Nagi.Fixture"),
                DescriptorError::InvalidPackageId,
            ),
            (
                "package.canonical_entry",
                json!("../identity.elf"),
                DescriptorError::InvalidCanonicalEntry,
            ),
            (
                "executable.expected_sha256",
                json!(DIGEST.to_uppercase()),
                DescriptorError::InvalidDigest,
            ),
            (
                "executable.target",
                json!("aarch64-unknown-linux-musl"),
                DescriptorError::UnsupportedTarget,
            ),
            (
                "executable.artifact_path",
                json!("/fixture/../identity.elf"),
                DescriptorError::InvalidArtifactPath,
            ),
        ];
        for (path, value, expected) in cases {
            let mut descriptor = valid_descriptor();
            set_path(&mut descriptor, path, value);
            assert_eq!(
                validate_descriptor_json(&descriptor.to_string()),
                Err(expected),
                "path: {path}"
            );
        }
    }

    #[test]
    fn rejects_zero_and_stale_generation_transitions() {
        let mut zero = valid_descriptor();
        zero["generation"] = json!(0);
        assert_eq!(
            validate_descriptor_json(&zero.to_string()),
            Err(DescriptorError::ZeroGeneration)
        );

        for previous in [0, 9, 10] {
            let mut stale = valid_descriptor();
            stale["lifecycle"] = json!({ "mode": "replace", "previous_generation": previous });
            assert_eq!(
                validate_descriptor_json(&stale.to_string()),
                Err(DescriptorError::InvalidGenerationTransition),
                "previous generation: {previous}"
            );
        }
        let mut replacement = valid_descriptor();
        replacement["lifecycle"] = json!({ "mode": "replace", "previous_generation": 8 });
        assert!(matches!(
            validate_descriptor_json(&replacement.to_string())
                .unwrap()
                .transition(),
            GenerationTransition::Replace { previous_generation } if previous_generation.get() == 8
        ));
    }

    #[test]
    fn rejects_endpoint_policy_and_credential_contradictions() {
        for (field, value) in [
            ("transport", json!("unix_datagram")),
            ("owner", json!("caller")),
            ("handoff", json!("path_reopen")),
            ("generation_policy", json!("shared")),
        ] {
            let mut descriptor = valid_descriptor();
            descriptor["endpoint"][field] = value;
            assert_eq!(
                validate_descriptor_json(&descriptor.to_string()),
                Err(DescriptorError::UnsupportedEndpoint),
                "endpoint field: {field}"
            );
        }
        let mut peer_uid = valid_descriptor();
        peer_uid["endpoint"]["expected_peer_uid"] = json!(1003);
        assert_eq!(
            validate_descriptor_json(&peer_uid.to_string()),
            Err(DescriptorError::PeerCredentialsMismatch)
        );
        let mut peer_gid = valid_descriptor();
        peer_gid["endpoint"]["expected_peer_gid"] = json!(1003);
        assert_eq!(
            validate_descriptor_json(&peer_gid.to_string()),
            Err(DescriptorError::PeerCredentialsMismatch)
        );
    }

    #[test]
    fn validates_preflight_report_without_promoting_its_claim_scope() {
        let descriptor = validate_descriptor_json(&valid_descriptor().to_string()).unwrap();
        let report = validate_artifact_preflight_report(&valid_report(), &descriptor).unwrap();
        assert_eq!(report.artifact_sha256(), DIGEST);
        assert_eq!(report.elf_type(), "ET_DYN");
        assert_eq!(report.program_header_count(), 10);

        let mut mismatch: Value = serde_json::from_str(&valid_report()).unwrap();
        mismatch["artifact_sha256"] = json!("b".repeat(64));
        assert_eq!(
            validate_artifact_preflight_report(&mismatch.to_string(), &descriptor),
            Err(PreflightReportError::DigestMismatch)
        );
        let mut interp: Value = serde_json::from_str(&valid_report()).unwrap();
        interp["checks"]["no_pt_interp"] = json!(false);
        assert_eq!(
            validate_artifact_preflight_report(&interp.to_string(), &descriptor),
            Err(PreflightReportError::CheckFailed)
        );
        let mut scope: Value = serde_json::from_str(&valid_report()).unwrap();
        scope["not_proven"] = json!(["runtime_ready"]);
        assert_eq!(
            validate_artifact_preflight_report(&scope.to_string(), &descriptor),
            Err(PreflightReportError::ClaimScopeMismatch)
        );
    }

    #[test]
    fn package_binding_checks_catalog_id_entry_digest_and_holds_installed_generation() {
        let executable = b"fixture system service executable";
        let package_bytes = fixture_package(executable);
        let package_owner = installed_fixture_owner(&package_bytes);
        let digest = digest_hex(executable);
        let descriptor = validate_descriptor_json(&descriptor_with_digest(&digest)).unwrap();
        let binding = bind_installed_service_package(&descriptor, &package_bytes, &package_owner)
            .expect("signed fixture matches catalog-selected package values");
        assert_eq!(binding.package_id(), "org.nagi.fixture");
        assert_eq!(binding.canonical_entry(), "fixture.elf");
        assert_eq!(binding.executable_sha256(), digest);
        assert_eq!(binding.app_id(), descriptor.package_app_id());
        assert_eq!(binding.package_generation(), 1);
        drop(binding);

        let mut wrong_entry: Value =
            serde_json::from_str(&descriptor_with_digest(&digest)).unwrap();
        wrong_entry["package"]["canonical_entry"] = json!("other.elf");
        let wrong_entry = validate_descriptor_json(&wrong_entry.to_string()).unwrap();
        let package_owner = installed_fixture_owner(&package_bytes);
        assert_eq!(
            bind_installed_service_package(&wrong_entry, &package_bytes, &package_owner).err(),
            Some(PackageBindingError::CanonicalEntryMismatch)
        );

        let wrong_digest =
            validate_descriptor_json(&descriptor_with_digest(&"b".repeat(64))).unwrap();
        let package_owner = installed_fixture_owner(&package_bytes);
        assert_eq!(
            bind_installed_service_package(&wrong_digest, &package_bytes, &package_owner).err(),
            Some(PackageBindingError::ExecutableDigestMismatch)
        );

        let mut wrong_id: Value = serde_json::from_str(&descriptor_with_digest(&digest)).unwrap();
        wrong_id["package"]["id"] = json!("org.nagi.other");
        let wrong_id = validate_descriptor_json(&wrong_id.to_string()).unwrap();
        let package_owner = installed_fixture_owner(&package_bytes);
        assert_eq!(
            bind_installed_service_package(&wrong_id, &package_bytes, &package_owner).err(),
            Some(PackageBindingError::ManifestIdMismatch)
        );
    }

    struct Catalog {
        error: Option<LaunchPortError>,
        calls: AtomicUsize,
    }

    impl TrustedSystemServiceCatalog for Catalog {
        fn authenticate(
            &mut self,
            _descriptor: &ValidatedSystemServiceDescriptor,
        ) -> Result<(), LaunchPortError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.error.map_or(Ok(()), Err)
        }
    }

    struct Preflight {
        error: Option<LaunchPortError>,
        report: String,
        calls: AtomicUsize,
    }

    impl ArtifactPreflight for Preflight {
        type HeldArtifact = File;

        fn inspect_and_retain(
            &mut self,
            _request: &ArtifactPreflightRequest,
        ) -> Result<(Self::HeldArtifact, String), LaunchPortError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some(error) = self.error {
                return Err(error);
            }
            File::open("/dev/null")
                .map(|file| (file, self.report.clone()))
                .map_err(|_| LaunchPortError::Unavailable)
        }
    }

    struct Owner {
        error: Option<LaunchPortError>,
        calls: AtomicUsize,
        revocations: AtomicUsize,
    }

    impl SystemServiceLaunchOwner for Owner {
        fn launch_and_authenticate(
            &mut self,
            descriptor: &ValidatedSystemServiceDescriptor,
            package: &VerifiedServicePackageBinding<'_>,
            _executable: std::os::fd::BorrowedFd<'_>,
        ) -> Result<SystemServiceEndpointOwnerProof, LaunchPortError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            assert_eq!(package.package_id(), descriptor.package_id());
            assert_eq!(package.canonical_entry(), descriptor.canonical_entry());
            Err(self.error.unwrap_or(LaunchPortError::Unavailable))
        }

        fn revoke_exact(
            &mut self,
            _descriptor: &ValidatedSystemServiceDescriptor,
            _proof: &SystemServiceEndpointOwnerProof,
        ) -> Result<(), LaunchPortError> {
            self.revocations.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    struct Identity {
        calls: AtomicUsize,
    }

    impl C1SystemServiceRecoveryClient for Identity {
        type Runtime = ();

        fn recover_system_service(
            &mut self,
            _descriptor: &ValidatedSystemServiceDescriptor,
            _package: &VerifiedServicePackageBinding<'_>,
            _proof: Arc<SystemServiceEndpointOwnerProof>,
        ) -> Result<Self::Runtime, C1RecoveryFailure> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            unreachable!("host negative paths never receive a fabricated C3 proof")
        }
    }

    fn launch_ports() -> (Catalog, Preflight, Owner, Identity) {
        (
            Catalog {
                error: None,
                calls: AtomicUsize::new(0),
            },
            Preflight {
                error: None,
                report: valid_report(),
                calls: AtomicUsize::new(0),
            },
            Owner {
                error: Some(LaunchPortError::Unavailable),
                calls: AtomicUsize::new(0),
                revocations: AtomicUsize::new(0),
            },
            Identity {
                calls: AtomicUsize::new(0),
            },
        )
    }

    #[test]
    fn descriptor_rejection_stops_before_catalog_preflight_c3_and_c1() {
        let (mut catalog, mut preflight, mut owner, mut identity) = launch_ports();
        let package_owner = empty_package_owner();
        let result = launch_identity_service(
            "{}",
            &[],
            &package_owner,
            &mut catalog,
            &mut preflight,
            &mut owner,
            &mut identity,
        );
        assert_eq!(
            result.err(),
            Some(LaunchError::Descriptor(DescriptorError::InvalidJson))
        );
        assert_eq!(catalog.calls.load(Ordering::SeqCst), 0);
        assert_eq!(preflight.calls.load(Ordering::SeqCst), 0);
        assert_eq!(owner.calls.load(Ordering::SeqCst), 0);
        assert_eq!(identity.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn catalog_rejection_stops_before_preflight_c3_and_c1() {
        let (mut catalog, mut preflight, mut owner, mut identity) = launch_ports();
        catalog.error = Some(LaunchPortError::Rejected);
        let package_owner = empty_package_owner();
        let result = launch_identity_service(
            &valid_descriptor().to_string(),
            &[],
            &package_owner,
            &mut catalog,
            &mut preflight,
            &mut owner,
            &mut identity,
        );
        assert_eq!(
            result.err(),
            Some(LaunchError::Catalog(LaunchPortError::Rejected))
        );
        assert_eq!(preflight.calls.load(Ordering::SeqCst), 0);
        assert_eq!(owner.calls.load(Ordering::SeqCst), 0);
        assert_eq!(identity.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn preflight_failure_or_report_mismatch_stops_before_c3_and_c1() {
        let (mut catalog, mut preflight, mut owner, mut identity) = launch_ports();
        preflight.error = Some(LaunchPortError::Unavailable);
        let package_owner = empty_package_owner();
        let result = launch_identity_service(
            &valid_descriptor().to_string(),
            &[],
            &package_owner,
            &mut catalog,
            &mut preflight,
            &mut owner,
            &mut identity,
        );
        assert_eq!(
            result.err(),
            Some(LaunchError::ArtifactPreflight(LaunchPortError::Unavailable))
        );
        assert_eq!(owner.calls.load(Ordering::SeqCst), 0);
        assert_eq!(identity.calls.load(Ordering::SeqCst), 0);

        preflight.error = None;
        preflight.report = "{}".to_owned();
        let result = launch_identity_service(
            &valid_descriptor().to_string(),
            &[],
            &package_owner,
            &mut catalog,
            &mut preflight,
            &mut owner,
            &mut identity,
        );
        assert_eq!(
            result.err(),
            Some(LaunchError::ArtifactReport(
                PreflightReportError::InvalidJson
            ))
        );
        assert_eq!(owner.calls.load(Ordering::SeqCst), 0);
        assert_eq!(identity.calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn c3_launch_failure_stops_before_c1_recovery() {
        let (mut catalog, mut preflight, mut owner, mut identity) = launch_ports();
        owner.error = Some(LaunchPortError::Failed);
        let executable = b"fixture system service executable";
        let package_bytes = fixture_package(executable);
        let package_owner = installed_fixture_owner(&package_bytes);
        let digest = digest_hex(executable);
        preflight.report = report_for_digest(&digest);
        let result = launch_identity_service(
            &descriptor_with_digest(&digest),
            &package_bytes,
            &package_owner,
            &mut catalog,
            &mut preflight,
            &mut owner,
            &mut identity,
        );
        assert_eq!(
            result.err(),
            Some(LaunchError::ProcessOwner(LaunchPortError::Failed))
        );
        assert_eq!(owner.calls.load(Ordering::SeqCst), 1);
        assert_eq!(identity.calls.load(Ordering::SeqCst), 0);
    }

    fn set_path(value: &mut Value, path: &str, replacement: Value) {
        let mut segments = path.split('.').peekable();
        let mut current = value;
        while let Some(segment) = segments.next() {
            if segments.peek().is_none() {
                current[segment] = replacement;
                return;
            }
            current = &mut current[segment];
        }
    }
}
