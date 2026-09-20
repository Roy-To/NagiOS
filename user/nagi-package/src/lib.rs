#![no_std]

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use nagi_model::AppId;

pub const MAX_PACKAGE_BYTES: usize = 8192;
pub const MAX_MANIFEST_BYTES: usize = 1024;
pub const MAX_ID_BYTES: usize = 64;
pub const MAX_NAME_BYTES: usize = 64;
pub const MAX_VERSION_BYTES: usize = 32;
pub const MAX_ENTRY_BYTES: usize = 64;
pub const MAX_SLOTS: usize = 8;
pub const SIGNATURE_BYTES: usize = 64;
/// RFC 8032 test-vector public key, pinned as the Developer Preview signer.
/// Production trust-store provisioning remains a later milestone concern.
pub const TRUSTED_SIGNING_PUBLIC_KEY: [u8; 32] = [
    0xd7, 0x5a, 0x98, 0x01, 0x82, 0xb1, 0x0a, 0xb7, 0xd5, 0x4b, 0xfe, 0xd3, 0xc9, 0x64, 0x07, 0x3a,
    0x0e, 0xe1, 0x72, 0xf3, 0xda, 0xa6, 0x23, 0x25, 0xaf, 0x02, 0x1a, 0x68, 0xf7, 0x07, 0x51, 0x1a,
];

const HEADER_BYTES: usize = 24;
const FORMAT_VERSION: u16 = 1;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PackageError {
    InvalidMagic,
    UnsupportedVersion,
    Truncated,
    Oversized,
    InvalidManifest,
    MissingManifestField,
    InvalidIdentity,
    PathTraversal,
    EmptyExecutable,
    SignatureRequired,
    InvalidSignature,
    StoreFull,
    AlreadyInstalled,
    NotInstalled,
    OutputTooSmall,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Text<const N: usize> {
    bytes: [u8; N],
    length: u8,
}

impl<const N: usize> Text<N> {
    const EMPTY: Self = Self {
        bytes: [0; N],
        length: 0,
    };

    fn parse(value: &[u8]) -> Result<Self, PackageError> {
        if value.is_empty() || value.len() > N {
            return Err(PackageError::InvalidManifest);
        }
        if value
            .iter()
            .any(|byte| *byte == 0 || *byte == b'\r' || *byte == b'\n')
        {
            return Err(PackageError::InvalidManifest);
        }
        let mut text = Self::EMPTY;
        text.bytes[..value.len()].copy_from_slice(value);
        text.length = value.len() as u8;
        Ok(text)
    }

    fn as_slice(&self) -> &[u8] {
        &self.bytes[..self.length as usize]
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PackageManifest {
    app_id: AppId,
    id: Text<MAX_ID_BYTES>,
    name: Text<MAX_NAME_BYTES>,
    version: Text<MAX_VERSION_BYTES>,
    entry: Text<MAX_ENTRY_BYTES>,
    surfaces: u8,
}

impl PackageManifest {
    pub fn parse(text: &[u8]) -> Result<Self, PackageError> {
        if text.is_empty() || text.len() > MAX_MANIFEST_BYTES {
            return Err(PackageError::InvalidManifest);
        }
        let mut id = None;
        let mut name = None;
        let mut version = None;
        let mut entry = None;
        let mut surfaces = 0;
        for line in text.split(|byte| *byte == b'\n') {
            let line = line.strip_suffix(b"\r").unwrap_or(line);
            if line.is_empty() {
                continue;
            }
            let Some(separator) = line.iter().position(|byte| *byte == b'=') else {
                return Err(PackageError::InvalidManifest);
            };
            let (key, value_with_separator) = line.split_at(separator);
            let value = &value_with_separator[1..];
            match key {
                b"id" => id = Some(Text::parse(value)?),
                b"name" => name = Some(Text::parse(value)?),
                b"version" => version = Some(Text::parse(value)?),
                b"entry" => {
                    if value.windows(2).any(|window| window == b"..")
                        || value.iter().any(|byte| *byte == b'/' || *byte == b'\\')
                    {
                        return Err(PackageError::PathTraversal);
                    }
                    entry = Some(Text::parse(value)?);
                }
                b"surfaces" => {
                    surfaces = parse_surfaces(value)?;
                }
                _ => return Err(PackageError::InvalidManifest),
            }
        }
        let id = id.ok_or(PackageError::MissingManifestField)?;
        let name = name.ok_or(PackageError::MissingManifestField)?;
        let version = version.ok_or(PackageError::MissingManifestField)?;
        let entry = entry.ok_or(PackageError::MissingManifestField)?;
        if id
            .as_slice()
            .iter()
            .any(|byte| !matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'.' | b'-' | b'_'))
        {
            return Err(PackageError::InvalidIdentity);
        }
        Ok(Self {
            app_id: AppId::from_identifier(id.as_slice()),
            id,
            name,
            version,
            entry,
            surfaces,
        })
    }

    pub const fn app_id(self) -> AppId {
        self.app_id
    }

    pub fn id(&self) -> &[u8] {
        self.id.as_slice()
    }

    pub fn name(&self) -> &[u8] {
        self.name.as_slice()
    }

    pub fn version(&self) -> &[u8] {
        self.version.as_slice()
    }

    pub fn entry(&self) -> &[u8] {
        self.entry.as_slice()
    }

    pub const fn supports_surfaces(self) -> u8 {
        self.surfaces
    }
}

pub struct PackageView<'a> {
    manifest: PackageManifest,
    executable: &'a [u8],
    resources: &'a [u8],
    schemas: &'a [u8],
    license: &'a [u8],
    signature: &'a [u8],
    signed_region: &'a [u8],
}

impl<'a> PackageView<'a> {
    pub fn parse(bytes: &'a [u8]) -> Result<Self, PackageError> {
        if bytes.len() < HEADER_BYTES {
            return Err(PackageError::Truncated);
        }
        if &bytes[..4] != b"XAPP" {
            return Err(PackageError::InvalidMagic);
        }
        if read_u16(bytes, 4)? != FORMAT_VERSION {
            return Err(PackageError::UnsupportedVersion);
        }
        let manifest_length = usize::from(read_u16(bytes, 8)?);
        let executable_length =
            usize::try_from(read_u32(bytes, 10)?).map_err(|_| PackageError::Oversized)?;
        let resources_length = usize::from(read_u16(bytes, 14)?);
        let schemas_length = usize::from(read_u16(bytes, 16)?);
        let license_length = usize::from(read_u16(bytes, 18)?);
        let signature_length = usize::from(read_u16(bytes, 20)?);
        let total = HEADER_BYTES
            .checked_add(manifest_length)
            .and_then(|value| value.checked_add(executable_length))
            .and_then(|value| value.checked_add(resources_length))
            .and_then(|value| value.checked_add(schemas_length))
            .and_then(|value| value.checked_add(license_length))
            .and_then(|value| value.checked_add(signature_length))
            .ok_or(PackageError::Oversized)?;
        if total != bytes.len() || total > MAX_PACKAGE_BYTES {
            return Err(PackageError::Truncated);
        }
        if executable_length == 0 {
            return Err(PackageError::EmptyExecutable);
        }
        let mut offset = HEADER_BYTES;
        let manifest = PackageManifest::parse(&bytes[offset..offset + manifest_length])?;
        offset += manifest_length;
        let executable = &bytes[offset..offset + executable_length];
        offset += executable_length;
        let resources = &bytes[offset..offset + resources_length];
        offset += resources_length;
        let schemas = &bytes[offset..offset + schemas_length];
        offset += schemas_length;
        let license = &bytes[offset..offset + license_length];
        offset += license_length;
        let signed_region = &bytes[..offset];
        let signature = &bytes[offset..offset + signature_length];
        Ok(Self {
            manifest,
            executable,
            resources,
            schemas,
            license,
            signature,
            signed_region,
        })
    }

    pub const fn manifest(&self) -> PackageManifest {
        self.manifest
    }

    pub const fn executable(&self) -> &'a [u8] {
        self.executable
    }

    pub const fn resources(&self) -> &'a [u8] {
        self.resources
    }

    pub const fn schemas(&self) -> &'a [u8] {
        self.schemas
    }

    pub const fn license(&self) -> &'a [u8] {
        self.license
    }

    pub const fn signature(&self) -> &'a [u8] {
        self.signature
    }

    pub fn is_signed(&self) -> bool {
        if self.signature.len() != SIGNATURE_BYTES {
            return false;
        }
        let Ok(public_key) = VerifyingKey::from_bytes(&TRUSTED_SIGNING_PUBLIC_KEY) else {
            return false;
        };
        let Ok(signature) = Signature::from_slice(self.signature) else {
            return false;
        };
        public_key.verify(self.signed_region, &signature).is_ok()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InstallPolicy {
    pub developer_mode: bool,
    pub require_signature: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InstallReport {
    pub app_id: AppId,
    pub replaced: bool,
    pub unsigned_warning: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct InstalledPackage {
    pub manifest: PackageManifest,
    pub generation: u64,
}

pub struct PackageStore {
    slots: [Option<InstalledPackage>; MAX_SLOTS],
    next_generation: u64,
}

impl Default for PackageStore {
    fn default() -> Self {
        Self::new()
    }
}

/// User-space Package Service boundary. The store is deliberately private to
/// this service so callers cannot mutate catalog slots without install policy.
pub struct PackageService {
    store: PackageStore,
}

impl Default for PackageService {
    fn default() -> Self {
        Self::new()
    }
}

impl PackageService {
    pub const fn new() -> Self {
        Self {
            store: PackageStore::new(),
        }
    }

    pub fn install(
        &mut self,
        package: &PackageView<'_>,
        policy: InstallPolicy,
    ) -> Result<InstallReport, PackageError> {
        self.store.install(package, policy)
    }

    pub fn remove(&mut self, app_id: AppId) -> Result<(), PackageError> {
        self.store.remove(app_id)
    }

    pub fn info(&self, app_id: AppId) -> Result<InstalledPackage, PackageError> {
        self.store.info(app_id)
    }

    pub fn list(&self, output: &mut [InstalledPackage]) -> Result<usize, PackageError> {
        self.store.list(output)
    }
}

impl PackageStore {
    pub const fn new() -> Self {
        Self {
            slots: [None; MAX_SLOTS],
            next_generation: 1,
        }
    }

    pub fn install(
        &mut self,
        package: &PackageView<'_>,
        policy: InstallPolicy,
    ) -> Result<InstallReport, PackageError> {
        if !package.signature().is_empty() && !package.is_signed() {
            return Err(PackageError::InvalidSignature);
        }
        let unsigned_warning = !package.is_signed();
        if policy.require_signature && !package.is_signed() && !policy.developer_mode {
            return Err(PackageError::SignatureRequired);
        }
        if !package.is_signed() && !policy.developer_mode {
            return Err(PackageError::SignatureRequired);
        }
        let app_id = package.manifest.app_id();
        let existing = self
            .slots
            .iter()
            .position(|slot| slot.is_some_and(|installed| installed.manifest.app_id() == app_id));
        let index = match existing {
            Some(index) => index,
            None => self
                .slots
                .iter()
                .position(Option::is_none)
                .ok_or(PackageError::StoreFull)?,
        };
        let replaced = existing.is_some();
        let installed = InstalledPackage {
            manifest: package.manifest,
            generation: self.next_generation,
        };
        self.slots[index] = Some(installed);
        self.next_generation = self.next_generation.saturating_add(1);
        Ok(InstallReport {
            app_id,
            replaced,
            unsigned_warning,
        })
    }

    pub fn remove(&mut self, app_id: AppId) -> Result<(), PackageError> {
        let slot = self
            .slots
            .iter_mut()
            .find(|slot| slot.is_some_and(|installed| installed.manifest.app_id() == app_id))
            .ok_or(PackageError::NotInstalled)?;
        *slot = None;
        Ok(())
    }

    pub fn info(&self, app_id: AppId) -> Result<InstalledPackage, PackageError> {
        self.slots
            .iter()
            .flatten()
            .find(|installed| installed.manifest.app_id() == app_id)
            .copied()
            .ok_or(PackageError::NotInstalled)
    }

    pub fn list(&self, output: &mut [InstalledPackage]) -> Result<usize, PackageError> {
        let mut count = 0;
        for installed in self.slots.iter().flatten() {
            let Some(destination) = output.get_mut(count) else {
                return Err(PackageError::OutputTooSmall);
            };
            *destination = *installed;
            count += 1;
        }
        Ok(count)
    }
}

pub fn build_xapp(
    manifest: &[u8],
    executable: &[u8],
    resources: &[u8],
    schemas: &[u8],
    license: &[u8],
    signature: &[u8],
    output: &mut [u8],
) -> Result<usize, PackageError> {
    let _ = PackageManifest::parse(manifest)?;
    if executable.is_empty() {
        return Err(PackageError::EmptyExecutable);
    }
    if manifest.len() > MAX_MANIFEST_BYTES
        || executable.len() > u32::MAX as usize
        || resources.len() > u16::MAX as usize
        || schemas.len() > u16::MAX as usize
        || license.len() > u16::MAX as usize
        || signature.len() > u16::MAX as usize
    {
        return Err(PackageError::Oversized);
    }
    let total = HEADER_BYTES
        .checked_add(manifest.len())
        .and_then(|value| value.checked_add(executable.len()))
        .and_then(|value| value.checked_add(resources.len()))
        .and_then(|value| value.checked_add(schemas.len()))
        .and_then(|value| value.checked_add(license.len()))
        .and_then(|value| value.checked_add(signature.len()))
        .ok_or(PackageError::Oversized)?;
    if total > MAX_PACKAGE_BYTES {
        return Err(PackageError::Oversized);
    }
    if output.len() < total {
        return Err(PackageError::OutputTooSmall);
    }
    output[..total].fill(0);
    output[..4].copy_from_slice(b"XAPP");
    write_u16(output, 4, FORMAT_VERSION);
    write_u16(output, 6, u16::from(!signature.is_empty()));
    write_u16(output, 8, manifest.len() as u16);
    write_u32(output, 10, executable.len() as u32);
    write_u16(output, 14, resources.len() as u16);
    write_u16(output, 16, schemas.len() as u16);
    write_u16(output, 18, license.len() as u16);
    write_u16(output, 20, signature.len() as u16);
    let mut offset = HEADER_BYTES;
    for section in [manifest, executable, resources, schemas, license, signature] {
        output[offset..offset + section.len()].copy_from_slice(section);
        offset += section.len();
    }
    Ok(total)
}

fn parse_surfaces(value: &[u8]) -> Result<u8, PackageError> {
    let mut flags = 0;
    for surface in value.split(|byte| *byte == b',') {
        flags |= match surface {
            b"compact" => 1,
            b"medium" => 2,
            b"expanded" => 4,
            _ => return Err(PackageError::InvalidManifest),
        };
    }
    if flags == 0 {
        return Err(PackageError::InvalidManifest);
    }
    Ok(flags)
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, PackageError> {
    let value = bytes
        .get(offset..offset + 2)
        .ok_or(PackageError::Truncated)?;
    Ok(u16::from_le_bytes([value[0], value[1]]))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, PackageError> {
    let value = bytes
        .get(offset..offset + 4)
        .ok_or(PackageError::Truncated)?;
    Ok(u32::from_le_bytes([value[0], value[1], value[2], value[3]]))
}

fn write_u16(bytes: &mut [u8], offset: usize, value: u16) {
    bytes[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
}

fn write_u32(bytes: &mut [u8], offset: usize, value: u32) {
    bytes[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::{
        build_xapp, InstallPolicy, PackageError, PackageManifest, PackageStore, PackageView,
        MAX_PACKAGE_BYTES, SIGNATURE_BYTES, TRUSTED_SIGNING_PUBLIC_KEY,
    };
    use ed25519_dalek::{Signer, SigningKey};

    const MANIFEST: &[u8] = b"id=com.example.hello\nname=Hello Nagi\nversion=0.1.0\nentry=hello.elf\nsurfaces=compact,expanded\n";

    fn package(signature: &[u8]) -> [u8; MAX_PACKAGE_BYTES] {
        let mut bytes = [0; MAX_PACKAGE_BYTES];
        let length = build_xapp(
            MANIFEST,
            b"HELLO NAGI ELF",
            b"asset",
            b"schema",
            b"MIT",
            signature,
            &mut bytes,
        )
        .expect("package");
        bytes[length..].fill(0);
        bytes
    }

    #[test]
    fn parses_manifest_and_stable_logical_identity() {
        let manifest = PackageManifest::parse(MANIFEST).expect("manifest");
        assert_eq!(manifest.id(), b"com.example.hello");
        assert_eq!(manifest.entry(), b"hello.elf");
        assert_ne!(manifest.app_id().0, 0);
        assert_eq!(manifest.supports_surfaces(), 5);
    }

    #[test]
    fn builds_and_parses_all_xapp_sections() {
        let bytes = package(b"");
        let view =
            PackageView::parse(&bytes[..bytes.iter().rposition(|byte| *byte != 0).unwrap() + 1])
                .expect("view");
        assert_eq!(view.manifest().name(), b"Hello Nagi");
        assert_eq!(view.executable(), b"HELLO NAGI ELF");
        assert_eq!(view.resources(), b"asset");
        assert_eq!(view.schemas(), b"schema");
        assert_eq!(view.license(), b"MIT");
        assert!(!view.is_signed());
    }

    #[test]
    fn rejects_traversal_and_unsigned_install_without_developer_mode() {
        let traversal =
            b"id=com.example.hello\nname=Hello\nversion=0.1\nentry=../evil\nsurfaces=compact\n";
        assert_eq!(
            PackageManifest::parse(traversal),
            Err(PackageError::PathTraversal)
        );
        let bytes = package(b"");
        let length = bytes.iter().rposition(|byte| *byte != 0).unwrap() + 1;
        let view = PackageView::parse(&bytes[..length]).expect("view");
        let mut store = PackageStore::new();
        assert_eq!(
            store.install(
                &view,
                InstallPolicy {
                    developer_mode: false,
                    require_signature: false
                }
            ),
            Err(PackageError::SignatureRequired)
        );
        let report = store
            .install(
                &view,
                InstallPolicy {
                    developer_mode: true,
                    require_signature: false,
                },
            )
            .expect("developer install");
        assert!(report.unsigned_warning);
    }

    #[test]
    fn replacement_is_atomic_and_list_info_remove_are_bounded() {
        let mut bytes = [0; MAX_PACKAGE_BYTES];
        let unsigned_signature = [0; SIGNATURE_BYTES];
        let length = build_xapp(
            MANIFEST,
            b"HELLO NAGI ELF",
            b"asset",
            b"schema",
            b"MIT",
            &unsigned_signature,
            &mut bytes,
        )
        .expect("package");
        let signing_key = SigningKey::from_bytes(&[
            0x9d, 0x61, 0xb1, 0x9d, 0xef, 0xfd, 0x5a, 0x60, 0xba, 0x84, 0x4a, 0xf4, 0x92, 0xec,
            0x2c, 0xc4, 0x44, 0x49, 0xc5, 0x69, 0x7b, 0x32, 0x69, 0x19, 0x70, 0x3b, 0xac, 0x03,
            0x1c, 0xae, 0x7f, 0x60,
        ]);
        let signature = signing_key.sign(&bytes[..length - SIGNATURE_BYTES]);
        bytes[length - SIGNATURE_BYTES..length].copy_from_slice(&signature.to_bytes());
        let view = PackageView::parse(&bytes[..length]).expect("view");
        assert_eq!(
            view.manifest().app_id().0,
            PackageManifest::parse(MANIFEST).unwrap().app_id().0
        );
        assert_eq!(
            TRUSTED_SIGNING_PUBLIC_KEY,
            signing_key.verifying_key().to_bytes()
        );
        let mut store = PackageStore::new();
        let first = store
            .install(
                &view,
                InstallPolicy {
                    developer_mode: false,
                    require_signature: true,
                },
            )
            .unwrap();
        assert!(!first.replaced);
        let second = store
            .install(
                &view,
                InstallPolicy {
                    developer_mode: false,
                    require_signature: true,
                },
            )
            .unwrap();
        assert!(second.replaced);
        let mut installed = [store.info(first.app_id).unwrap(); 2];
        assert_eq!(store.list(&mut installed).unwrap(), 1);
        store.remove(first.app_id).unwrap();
        assert_eq!(store.info(first.app_id), Err(PackageError::NotInstalled));
    }

    #[test]
    fn rejects_forged_and_tampered_signatures() {
        let mut bytes = [0; MAX_PACKAGE_BYTES];
        let signature = [0x5a; SIGNATURE_BYTES];
        let length = build_xapp(
            MANIFEST,
            b"HELLO NAGI ELF",
            b"asset",
            b"schema",
            b"MIT",
            &signature,
            &mut bytes,
        )
        .expect("package");
        let forged = PackageView::parse(&bytes[..length]).expect("parse forged package");
        let mut store = PackageStore::new();
        assert_eq!(
            store.install(
                &forged,
                InstallPolicy {
                    developer_mode: false,
                    require_signature: true,
                }
            ),
            Err(PackageError::InvalidSignature)
        );
    }
}
