use ed25519_dalek::{Signer, SigningKey};
use nagi_init_package_provider::package_trust::{
    InstallationState, PackageProviderError, PackageSecurityProvider, PackageServiceSessionOwner,
    PackageServiceSessionOwnerError,
};
use nagi_model::AppId;
use nagi_package::{
    build_xapp, InstallPolicy, InstalledPackage, PackageError, PackageManifest, PackageService,
    PackageView, MAX_PACKAGE_BYTES, MAX_SLOTS, SIGNATURE_BYTES, TRUSTED_SIGNING_PUBLIC_KEY,
};

// These are the same public RFC 8032 and deterministic alternate-key fixtures
// used by the package-provider's existing host tests. They are test vectors,
// not credentials or production signing keys.
const RFC8032_TEST_SEED: [u8; 32] = [
    0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec, 0x2c, 0xc4,
    0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03, 0x1c, 0xae, 0x7f, 0x60,
];
const OTHER_TEST_ONLY_SEED: [u8; 32] = [0x42; 32];
const PACKAGE_ID: &[u8] = b"org.nagi.fixture";
const EXPECTED_ENTRY: &[u8] = b"fixture.elf";
const EXECUTABLE: &[u8] = b"service package fixture executable";
const MANIFEST: &[u8] =
    b"id=org.nagi.fixture\nname=Fixture\nversion=1.0.0\nentry=fixture.elf\nsurfaces=compact\n";
const OTHER_ID_MANIFEST: &[u8] =
    b"id=org.nagi.substitute\nname=Fixture\nversion=1.0.0\nentry=fixture.elf\nsurfaces=compact\n";
const OTHER_ENTRY_MANIFEST: &[u8] =
    b"id=org.nagi.fixture\nname=Fixture\nversion=1.0.0\nentry=substitute.elf\nsurfaces=compact\n";
const STRICT_POLICY: InstallPolicy = InstallPolicy {
    developer_mode: false,
    require_signature: true,
};

fn fixture_signing_key() -> SigningKey {
    SigningKey::from_bytes(&RFC8032_TEST_SEED)
}

fn signed_package(manifest: &[u8], executable: &[u8], key: &SigningKey) -> Vec<u8> {
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
    .expect("build signed test package");
    let signature = key.sign(&bytes[..length - SIGNATURE_BYTES]);
    bytes[length - SIGNATURE_BYTES..length].copy_from_slice(&signature.to_bytes());
    bytes[..length].to_vec()
}

fn unsigned_package(manifest: &[u8], executable: &[u8]) -> Vec<u8> {
    let mut bytes = [0; MAX_PACKAGE_BYTES];
    let length = build_xapp(manifest, executable, b"", b"", b"", b"", &mut bytes)
        .expect("build unsigned test package");
    bytes[..length].to_vec()
}

fn mutate_once(bytes: &[u8], needle: &[u8], replacement: &[u8]) -> Vec<u8> {
    assert_eq!(needle.len(), replacement.len());
    let mut changed = bytes.to_vec();
    let offset = changed
        .windows(needle.len())
        .position(|window| window == needle)
        .expect("fixture contains mutation needle");
    changed[offset..offset + replacement.len()].copy_from_slice(replacement);
    changed
}

fn mutate_executable(bytes: &[u8]) -> Vec<u8> {
    let mut changed = bytes.to_vec();
    let manifest_length = usize::from(u16::from_le_bytes([changed[8], changed[9]]));
    let executable_offset = 24 + manifest_length;
    changed[executable_offset] ^= 1;
    changed
}

fn mutate_signature(bytes: &[u8]) -> Vec<u8> {
    let mut changed = bytes.to_vec();
    let last = changed.len() - 1;
    changed[last] ^= 1;
    changed
}

fn inventory_snapshot(service: &PackageService) -> [Option<InstalledPackage>; MAX_SLOTS] {
    let mut inventory = [None; MAX_SLOTS];
    service
        .list_inventory(&mut inventory)
        .expect("inventory fits the fixed slots");
    inventory
}

fn owner_inventory_snapshot(
    owner: &PackageServiceSessionOwner<'_>,
    valid_package: &[u8],
) -> [Option<InstalledPackage>; MAX_SLOTS] {
    let snapshot = owner
        .capture(valid_package, PACKAGE_ID)
        .expect("known package remains installed");
    let lease = owner
        .lease_for_dispatch(&snapshot)
        .expect("known package inventory remains dispatchable");
    let mut inventory = [None; MAX_SLOTS];
    let count = lease
        .inventory()
        .list_inventory(&mut inventory)
        .expect("inventory output is large enough");
    assert_eq!(count, 1);
    inventory
}

#[test]
fn signed_fixture_is_accepted_and_reports_the_installed_catalog_record() {
    let key = fixture_signing_key();
    let trusted_key = key.verifying_key().to_bytes();
    assert_eq!(trusted_key, TRUSTED_SIGNING_PUBLIC_KEY);

    let package_bytes = signed_package(MANIFEST, EXECUTABLE, &key);
    let parsed = PackageView::parse(&package_bytes).expect("signed fixture parses");
    let mut service = PackageService::new();
    let install = service
        .install_with_trusted_key(&parsed, STRICT_POLICY, &trusted_key)
        .expect("fixture signed by the injected test trust anchor installs");
    assert_eq!(
        install.app_id,
        PackageManifest::parse(MANIFEST).unwrap().app_id()
    );
    assert!(!install.replaced);

    let before = inventory_snapshot(&service);
    let inspection = PackageSecurityProvider::new(Some(&trusted_key), &service)
        .inspect(&package_bytes)
        .expect("provider authenticates the exact fixture bytes");
    assert_eq!(
        inspection.installation,
        InstallationState::Installed { generation: 1 }
    );
    assert_eq!(inspection.verified_package.manifest().id(), PACKAGE_ID);
    assert_eq!(
        inspection.verified_package.manifest().entry(),
        EXPECTED_ENTRY
    );
    assert_eq!(inspection.verified_package.view().executable(), EXECUTABLE);
    assert_eq!(inventory_snapshot(&service), before);
}

#[test]
fn signature_manifest_executable_signer_and_unsigned_rejections_preserve_inventory() {
    let key = fixture_signing_key();
    let trusted_key = key.verifying_key().to_bytes();
    let other_key = SigningKey::from_bytes(&OTHER_TEST_ONLY_SEED);
    let valid = signed_package(MANIFEST, EXECUTABLE, &key);
    let manifest_tampered = mutate_once(&valid, b"name=Fixture", b"name=Fiction");
    let executable_tampered = mutate_executable(&valid);
    let signature_tampered = mutate_signature(&valid);
    let untrusted_signer = signed_package(MANIFEST, EXECUTABLE, &other_key);
    let unsigned = unsigned_package(MANIFEST, EXECUTABLE);

    let mut owner = PackageServiceSessionOwner::new(Some(&trusted_key), PackageService::new());
    owner
        .install(&valid, STRICT_POLICY)
        .expect("install signed baseline before negative attempts");
    let baseline = owner_inventory_snapshot(&owner, &valid);

    let cases = [
        (
            "manifest changed after signing",
            manifest_tampered,
            PackageProviderError::UntrustedSigner,
            PackageServiceSessionOwnerError::Package(PackageError::InvalidSignature),
        ),
        (
            "executable changed after signing",
            executable_tampered,
            PackageProviderError::UntrustedSigner,
            PackageServiceSessionOwnerError::Package(PackageError::InvalidSignature),
        ),
        (
            "signature bytes changed",
            signature_tampered,
            PackageProviderError::UntrustedSigner,
            PackageServiceSessionOwnerError::Package(PackageError::InvalidSignature),
        ),
        (
            "different test signer",
            untrusted_signer,
            PackageProviderError::UntrustedSigner,
            PackageServiceSessionOwnerError::Package(PackageError::InvalidSignature),
        ),
        (
            "missing signature",
            unsigned,
            PackageProviderError::UnsignedPackage,
            PackageServiceSessionOwnerError::Package(PackageError::SignatureRequired),
        ),
    ];

    for (label, candidate, inspect_error, install_error) in cases {
        let snapshot = owner
            .capture(&valid, PACKAGE_ID)
            .expect("baseline remains capturable");
        let lease = owner
            .lease_for_dispatch(&snapshot)
            .expect("baseline remains leaseable");
        let observed_inspect_error =
            PackageSecurityProvider::new(Some(&trusted_key), lease.inventory())
                .inspect(&candidate)
                .err()
                .expect(label);
        assert_eq!(
            observed_inspect_error, inspect_error,
            "{label}: inspect error"
        );
        drop(lease);
        assert_eq!(
            owner_inventory_snapshot(&owner, &valid),
            baseline,
            "{label}: inspect must not mutate inventory"
        );

        assert_eq!(
            owner.install(&candidate, STRICT_POLICY),
            Err(install_error),
            "{label}"
        );
        assert_eq!(
            owner_inventory_snapshot(&owner, &valid),
            baseline,
            "{label}: rejected install must not mutate inventory"
        );
    }
}

#[test]
fn expected_identity_substitute_is_rejected_and_entry_substitute_is_fenced_by_lease() {
    let key = fixture_signing_key();
    let trusted_key = key.verifying_key().to_bytes();
    let valid = signed_package(MANIFEST, EXECUTABLE, &key);
    let other_identity = signed_package(OTHER_ID_MANIFEST, EXECUTABLE, &key);
    let other_entry = signed_package(OTHER_ENTRY_MANIFEST, EXECUTABLE, &key);

    let mut owner = PackageServiceSessionOwner::new(Some(&trusted_key), PackageService::new());
    owner
        .install(&valid, STRICT_POLICY)
        .expect("install expected package before substitution cases");
    let baseline = owner_inventory_snapshot(&owner, &valid);

    let actual = PackageManifest::parse(OTHER_ID_MANIFEST).unwrap().app_id();
    let expected = AppId::from_identifier(PACKAGE_ID);
    assert_eq!(
        owner.capture(&other_identity, PACKAGE_ID),
        Err(PackageServiceSessionOwnerError::IdentityMismatch { actual, expected })
    );
    assert_eq!(owner_inventory_snapshot(&owner, &valid), baseline);

    let snapshot = owner
        .capture(&other_entry, PACKAGE_ID)
        .expect("capture authenticates identity but has no expected-entry argument");
    assert_eq!(snapshot.generation(), 1);

    let installed_snapshot = owner
        .capture(&valid, PACKAGE_ID)
        .expect("baseline can still be captured");
    let lease = owner
        .lease_for_dispatch(&installed_snapshot)
        .expect("baseline lease remains available");
    let inspection = PackageSecurityProvider::new(Some(&trusted_key), lease.inventory())
        .inspect(&other_entry)
        .expect("inspect authenticates the signed substitute but does not take expected_entry");
    assert_eq!(
        inspection.verified_package.manifest().entry(),
        b"substitute.elf"
    );
    assert_eq!(
        inspection.installation,
        InstallationState::Installed { generation: 1 }
    );
    drop(lease);

    assert_eq!(
        owner.lease_for_dispatch(&snapshot).err(),
        Some(PackageServiceSessionOwnerError::ManifestChanged),
        "full-manifest comparison rejects an entry substitution before dispatch"
    );
    assert_eq!(owner_inventory_snapshot(&owner, &valid), baseline);
}
