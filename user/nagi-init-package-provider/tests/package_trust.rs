use ed25519_dalek::{Signer, SigningKey};
use nagi_init_package_provider::package_trust::{
    InstallationState, InstalledPackageInventory, InventoryReadError, PackageProviderError,
    PackageSecurityProvider, PackageServiceSessionOwner, PackageServiceSessionOwnerError,
};
use nagi_model::AppId;
use nagi_package::{
    build_xapp, InstallPolicy, InstalledPackage, PackageError, PackageManifest, PackageService,
    PackageView, MAX_PACKAGE_BYTES, MAX_SLOTS, SIGNATURE_BYTES,
};

const TEST_ONLY_RFC8032_SEED: [u8; 32] = [
    0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c, 0xc4,
    0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae, 0x7f, 0x60,
];
const OTHER_TEST_ONLY_SEED: [u8; 32] = [0x42; 32];
const MANIFEST: &[u8] =
    b"id=org.nagi.fixture\nname=Fixture\nversion=1.0.0\nentry=fixture.elf\nsurfaces=compact\n";

fn signed_package(manifest: &[u8], key: &SigningKey) -> Vec<u8> {
    signed_package_with_executable(manifest, b"fixture ELF bytes", key)
}

fn signed_package_with_executable(manifest: &[u8], executable: &[u8], key: &SigningKey) -> Vec<u8> {
    let mut bytes = [0; MAX_PACKAGE_BYTES];
    let placeholder = [0; SIGNATURE_BYTES];
    let length = build_xapp(
        manifest,
        executable,
        b"",
        b"",
        b"",
        &placeholder,
        &mut bytes,
    )
    .expect("build signed fixture package");
    let signature = key.sign(&bytes[..length - SIGNATURE_BYTES]);
    bytes[length - SIGNATURE_BYTES..length].copy_from_slice(&signature.to_bytes());
    bytes[..length].to_vec()
}

fn unsigned_package(manifest: &[u8]) -> Vec<u8> {
    let mut bytes = [0; MAX_PACKAGE_BYTES];
    let length = build_xapp(
        manifest,
        b"fixture ELF bytes",
        b"",
        b"",
        b"",
        b"",
        &mut bytes,
    )
    .expect("build unsigned fixture package");
    bytes[..length].to_vec()
}

fn raw_package(manifest: &[u8]) -> Vec<u8> {
    let executable = b"fixture ELF bytes";
    let mut bytes = vec![0; 24 + manifest.len() + executable.len()];
    bytes[..4].copy_from_slice(b"XAPP");
    bytes[4..6].copy_from_slice(&1u16.to_le_bytes());
    bytes[8..10].copy_from_slice(&(manifest.len() as u16).to_le_bytes());
    bytes[10..14].copy_from_slice(&(executable.len() as u32).to_le_bytes());
    bytes[24..24 + manifest.len()].copy_from_slice(manifest);
    bytes[24 + manifest.len()..].copy_from_slice(executable);
    bytes
}

#[test]
fn verifies_injected_fixture_key_and_reports_real_installed_or_missing_state() {
    // This deterministic key is test-only and deliberately differs from the
    // pinned Developer Preview vector. No product credential is present here.
    let test_key = SigningKey::from_bytes(&OTHER_TEST_ONLY_SEED);
    let package = signed_package(MANIFEST, &test_key);
    let mut service = PackageService::new();
    let parsed = PackageView::parse(&package).unwrap();
    assert_eq!(
        service.install(
            &parsed,
            InstallPolicy {
                developer_mode: false,
                require_signature: true,
            },
        ),
        Err(PackageError::InvalidSignature)
    );
    service
        .install_with_trusted_key(
            &parsed,
            InstallPolicy {
                developer_mode: false,
                require_signature: true,
            },
            &test_key.verifying_key().to_bytes(),
        )
        .unwrap();

    let key = test_key.verifying_key().to_bytes();
    let provider = PackageSecurityProvider::new(Some(&key), &service);
    let inspection = provider.inspect(&package).unwrap();
    assert_eq!(
        inspection.verified_package.manifest().id(),
        b"org.nagi.fixture"
    );
    assert_eq!(
        inspection.verified_package.view().executable(),
        b"fixture ELF bytes"
    );
    assert_eq!(
        inspection.installation,
        InstallationState::Installed { generation: 1 }
    );

    let empty_service = PackageService::new();
    let empty_provider = PackageSecurityProvider::new(Some(&key), &empty_service);
    assert_eq!(
        empty_provider.inspect(&package).unwrap().installation,
        InstallationState::NotInstalled
    );

    let mut listed = [service
        .info(AppId::from_identifier(b"org.nagi.fixture"))
        .unwrap(); MAX_SLOTS];
    assert_eq!(provider.list_installed(&mut listed).unwrap(), 1);
    assert_eq!(listed[0].manifest.id(), b"org.nagi.fixture");
}

#[test]
fn session_owner_adopts_before_first_install_and_fences_stale_or_foreign_snapshots() {
    let signing_key = SigningKey::from_bytes(&TEST_ONLY_RFC8032_SEED);
    let trusted_key = signing_key.verifying_key().to_bytes();
    let initial = signed_package(MANIFEST, &signing_key);
    let replacement_manifest =
        b"id=org.nagi.fixture\nname=Fixture\nversion=2.0.0\nentry=fixture.elf\nsurfaces=compact\n";
    let replacement = signed_package(replacement_manifest, &signing_key);
    let mut owner = PackageServiceSessionOwner::new(Some(&trusted_key), PackageService::new());

    assert_eq!(
        owner.capture(&initial, b"org.nagi.fixture"),
        Err(PackageServiceSessionOwnerError::NotInstalled)
    );

    let first_install = owner
        .install(
            &initial,
            InstallPolicy {
                developer_mode: false,
                require_signature: true,
            },
        )
        .expect("first install goes through the preexisting session owner");
    assert_eq!(
        first_install.app_id,
        AppId::from_identifier(b"org.nagi.fixture")
    );
    let first_snapshot = owner
        .capture(&initial, b"org.nagi.fixture")
        .expect("capture the installed generation through the same owner");
    assert_eq!(first_snapshot.generation(), 1);
    {
        let lease = owner
            .lease_for_dispatch(&first_snapshot)
            .expect("lease the exact retained service generation");
        let inspection = PackageSecurityProvider::new(Some(&trusted_key), lease.inventory())
            .inspect(&initial)
            .expect("inspect against the leased inventory");
        assert_eq!(
            inspection.installation,
            InstallationState::Installed { generation: 1 }
        );
    }

    let foreign_owner = PackageServiceSessionOwner::new(Some(&trusted_key), PackageService::new());
    assert_eq!(
        foreign_owner.lease_for_dispatch(&first_snapshot).err(),
        Some(PackageServiceSessionOwnerError::OwnerMismatch)
    );

    let replacement_install = owner
        .install(
            &replacement,
            InstallPolicy {
                developer_mode: false,
                require_signature: true,
            },
        )
        .expect("reinstall through the same owner");
    assert!(replacement_install.replaced);
    assert_eq!(
        owner.lease_for_dispatch(&first_snapshot).err(),
        Some(PackageServiceSessionOwnerError::StaleGeneration {
            captured: 1,
            current: 2,
        })
    );
    let current_snapshot = owner
        .capture(&replacement, b"org.nagi.fixture")
        .expect("capture the replacement generation");
    assert_eq!(current_snapshot.generation(), 2);
    assert!(owner.lease_for_dispatch(&current_snapshot).is_ok());

    owner
        .remove(AppId::from_identifier(b"org.nagi.fixture"))
        .expect("remove through the same owner");
    assert_eq!(
        owner.lease_for_dispatch(&current_snapshot).err(),
        Some(PackageServiceSessionOwnerError::NotInstalled)
    );
}

#[test]
fn quarantined_generation_rejects_retry_and_all_package_mutations() {
    let signing_key = SigningKey::from_bytes(&TEST_ONLY_RFC8032_SEED);
    let trusted_key = signing_key.verifying_key().to_bytes();
    let initial = signed_package(MANIFEST, &signing_key);
    let replacement = signed_package(
        b"id=org.nagi.fixture\nname=Fixture\nversion=2.0.0\nentry=fixture.elf\nsurfaces=compact\n",
        &signing_key,
    );
    let mut owner = PackageServiceSessionOwner::new(Some(&trusted_key), PackageService::new());
    owner
        .install(
            &initial,
            InstallPolicy {
                developer_mode: false,
                require_signature: true,
            },
        )
        .unwrap();
    let snapshot = owner.capture(&initial, b"org.nagi.fixture").unwrap();
    let lease = owner.lease_for_dispatch(&snapshot).unwrap();

    // Simulates the launch owner failing to prove exact process termination.
    // Quarantine is enforced by the real package session owner, not by a mock.
    lease.quarantine();

    assert_eq!(
        owner.capture(&initial, b"org.nagi.fixture"),
        Err(PackageServiceSessionOwnerError::Quarantined)
    );
    assert_eq!(
        owner.lease_for_dispatch(&snapshot).err(),
        Some(PackageServiceSessionOwnerError::Quarantined)
    );
    assert_eq!(
        owner.install(
            &replacement,
            InstallPolicy {
                developer_mode: false,
                require_signature: true,
            },
        ),
        Err(PackageServiceSessionOwnerError::Quarantined)
    );
    assert_eq!(
        owner.remove(AppId::from_identifier(b"org.nagi.fixture")),
        Err(PackageServiceSessionOwnerError::Quarantined)
    );

    assert_eq!(
        owner.reconcile_quarantine_after(|| Err::<(), _>("C1/C3 state unresolved")),
        Err("C1/C3 state unresolved")
    );
    assert_eq!(
        owner.capture(&initial, b"org.nagi.fixture"),
        Err(PackageServiceSessionOwnerError::Quarantined)
    );
    owner
        .reconcile_quarantine_after(|| Ok::<(), &str>(()))
        .unwrap();
    assert_eq!(
        owner
            .capture(&initial, b"org.nagi.fixture")
            .unwrap()
            .generation(),
        1
    );
}

#[test]
fn generation_lease_binds_the_exact_signed_executable_bytes() {
    let signing_key = SigningKey::from_bytes(&TEST_ONLY_RFC8032_SEED);
    let trusted_key = signing_key.verifying_key().to_bytes();
    let package_a = signed_package_with_executable(MANIFEST, b"executable A", &signing_key);
    let package_b = signed_package_with_executable(MANIFEST, b"executable B", &signing_key);
    let mut owner = PackageServiceSessionOwner::new(Some(&trusted_key), PackageService::new());
    owner
        .install(
            &package_a,
            InstallPolicy {
                developer_mode: false,
                require_signature: true,
            },
        )
        .unwrap();
    let snapshot_a = owner.capture(&package_a, b"org.nagi.fixture").unwrap();

    assert_eq!(
        owner.capture(&package_b, b"org.nagi.fixture"),
        Err(PackageServiceSessionOwnerError::InstalledPackageBytesMismatch)
    );
    assert_eq!(
        owner
            .capture(&package_a, b"org.nagi.fixture")
            .unwrap()
            .generation(),
        1
    );

    let replacement = owner
        .install(
            &package_b,
            InstallPolicy {
                developer_mode: false,
                require_signature: true,
            },
        )
        .unwrap();
    assert!(replacement.replaced);
    assert_eq!(
        owner.lease_for_dispatch(&snapshot_a).err(),
        Some(PackageServiceSessionOwnerError::StaleGeneration {
            captured: 1,
            current: 2,
        })
    );
    let snapshot_b = owner.capture(&package_b, b"org.nagi.fixture").unwrap();
    assert_eq!(snapshot_b.generation(), 2);
    assert_ne!(
        snapshot_a.signed_package_sha256(),
        snapshot_b.signed_package_sha256()
    );
}

#[test]
fn session_owner_strict_install_rejects_unsigned_input_without_inventory_mutation() {
    let signing_key = SigningKey::from_bytes(&TEST_ONLY_RFC8032_SEED);
    let trusted_key = signing_key.verifying_key().to_bytes();
    let signed = signed_package(MANIFEST, &signing_key);
    let unsigned = unsigned_package(MANIFEST);
    let mut owner = PackageServiceSessionOwner::new(Some(&trusted_key), PackageService::new());

    assert_eq!(
        owner.install(
            &unsigned,
            InstallPolicy {
                developer_mode: true,
                require_signature: true,
            },
        ),
        Err(PackageServiceSessionOwnerError::Package(
            PackageError::SignatureRequired
        ))
    );
    assert_eq!(
        owner.capture(&signed, b"org.nagi.fixture"),
        Err(PackageServiceSessionOwnerError::NotInstalled)
    );
    owner
        .install(
            &signed,
            InstallPolicy {
                developer_mode: false,
                require_signature: true,
            },
        )
        .expect("signed package remains installable after unsigned rejection");
    assert_eq!(
        owner
            .capture(&signed, b"org.nagi.fixture")
            .expect("capture signed install")
            .generation(),
        1
    );
}

#[test]
fn rejects_missing_key_unsigned_unknown_signer_and_tampering() {
    let test_key = SigningKey::from_bytes(&TEST_ONLY_RFC8032_SEED);
    let other_key = SigningKey::from_bytes(&OTHER_TEST_ONLY_SEED);
    let valid = signed_package(MANIFEST, &test_key);
    let empty_inventory = PackageService::new();

    assert!(matches!(
        PackageSecurityProvider::new(None, &empty_inventory).inspect(&valid),
        Err(PackageProviderError::TrustUnavailable)
    ));
    assert!(matches!(
        PackageSecurityProvider::new(Some(&test_key.verifying_key().to_bytes()), &empty_inventory)
            .inspect(&unsigned_package(MANIFEST)),
        Err(PackageProviderError::UnsignedPackage)
    ));
    assert!(matches!(
        PackageSecurityProvider::new(
            Some(&other_key.verifying_key().to_bytes()),
            &empty_inventory
        )
        .inspect(&valid),
        Err(PackageProviderError::UntrustedSigner)
    ));

    let mut tampered = valid.clone();
    let manifest_length = u16::from_le_bytes([tampered[8], tampered[9]]) as usize;
    tampered[24 + manifest_length] ^= 1;
    assert!(matches!(
        PackageSecurityProvider::new(Some(&test_key.verifying_key().to_bytes()), &empty_inventory)
            .inspect(&tampered),
        Err(PackageProviderError::UntrustedSigner)
    ));
}

#[test]
fn parses_real_package_bytes_and_rejects_duplicate_or_missing_manifest_fields() {
    let duplicate =
        raw_package(b"id=org.nagi.fixture\nid=second\nname=Fixture\nversion=1\nentry=fixture\n");
    let missing = raw_package(b"id=org.nagi.fixture\nname=Fixture\nentry=fixture\n");
    let service = PackageService::new();
    let key = SigningKey::from_bytes(&TEST_ONLY_RFC8032_SEED)
        .verifying_key()
        .to_bytes();
    let provider = PackageSecurityProvider::new(Some(&key), &service);
    assert!(matches!(
        provider.inspect(&duplicate),
        Err(PackageProviderError::InvalidPackage(
            PackageError::InvalidManifest
        ))
    ));
    assert!(matches!(
        provider.inspect(&missing),
        Err(PackageProviderError::InvalidPackage(
            PackageError::MissingManifestField
        ))
    ));
}

struct CorruptInventory([InstalledPackage; 2]);

impl InstalledPackageInventory for CorruptInventory {
    fn list_inventory(
        &self,
        output: &mut [Option<InstalledPackage>],
    ) -> Result<usize, InventoryReadError> {
        output[0] = Some(self.0[0]);
        output[1] = Some(self.0[1]);
        Ok(2)
    }
}

#[test]
fn refuses_duplicate_inventory_identity() {
    let manifest = PackageManifest::parse(MANIFEST).unwrap();
    let duplicate = InstalledPackage {
        manifest,
        generation: 9,
    };
    let inventory = CorruptInventory([duplicate; 2]);
    let provider = PackageSecurityProvider::new(None, &inventory);
    assert_eq!(
        provider.list_installed(&mut []).unwrap_err(),
        PackageProviderError::InventoryCorrupt
    );
}
