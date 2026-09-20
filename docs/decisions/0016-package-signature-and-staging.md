# ADR-0016: M16 package signature and guest staging

## Status

Accepted for Nagi OS 0.1 Developer Preview.

## Decision

`.xapp` signatures use the pinned RustCrypto `ed25519-dalek` verifier in the
guest Package Service. The Developer Preview trusts one explicitly embedded
public key; the corresponding private test key is host-tool-only and is never
included in the Nagi image. A non-empty signature that does not verify is
rejected, while unsigned packages are accepted only with Developer Mode and
an explicit unsigned warning.

The host `nagi-pkg` flow is a real artifact chain: the out-of-tree SDK sample
emits a bounded NAPP artifact, `nagi-pkg` packages those bytes, and the M16
image build stages the resulting `.xapp` into the init ELF. Guest install,
launch, update and remove then operate on the package loaded from Nagi VFS.

Package replacement uses the VFS `replace` transaction to keep the published
destination name and switch the inode in one directory update; the old inode
is released only after the new directory entry is visible. Future recovery
work may add a journal, but M16 does not expose a remove-then-rename gap.

## Consequences

- No custom cryptographic primitive is implemented by Nagi.
- Changing the Developer Preview signer requires an explicit trust-store
  decision and an acceptance update.
- The host signing key is test tooling, not a production secret integration.
