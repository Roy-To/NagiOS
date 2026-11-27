# Linux 0.1 release signing

**Draft for review.** The owner keeps one new Ed25519 key for the Linux 0.1 release. The tools run in the owner's local terminal with Python 3 and OpenSSL 3. Key generation produces an encrypted PKCS#8 file and a raw 32-byte public key. Signing verifies the expected public key and the reviewed material hashes, then writes raw detached signatures. The tools leave guest verification and image assembly to their existing owners.

The private key stays in owner-controlled storage outside Git repositories, worktrees and Codex storage. Use a private directory on the owner's machine with owner-only access, including any filesystem ACLs. Neither CI nor Codex runs these commands with a product key. Run privately with terminal logging disabled and keep the passphrase separately.

## Generate and hand off the public key

Choose absolute paths in an existing local directory. Both outputs must be new files.

```sh
python3 tools/linux-boot/generate_release_key.py \
  /absolute/local/private-directory/nagi-release.pem \
  /absolute/local/private-directory/nagi-release-public.raw
```

The command prompts for a passphrase and confirmation. Its stdout contains the raw public key in hexadecimal and its SHA-256. Send only the raw public key and confirm its SHA-256 through a separate trusted channel. The integration owner verifies that the raw key is exactly 32 bytes and independently hashes it with `shasum -a 256`. The published Developer Preview test vector is rejected. After review, register the raw key at `release/linux-0.1/public/ed25519.raw` in a dedicated trust-anchor change. The actual anchor and each product verifier binding require review. The catalog and C3 recovery builds already accept `NAGI_LINUX_BOOT_CATALOG_PUBLIC_KEY` as a path to raw public bytes.

## Prepare and sign a batch

The integration owner prepares exact signing preimages, delivers them with a manifest, and confirms the manifest SHA-256 independently. The manifest is UTF-8 JSON with `schema_version: 1` and a nonempty `materials` array. Each entry contains `file` (one filename adjacent to the manifest) and `sha256` (64 lowercase hexadecimal characters for the exact preimage). The owner reviews each artifact and the agreed signing contract before signing. Supply the approved raw public key, its independently confirmed SHA-256, and the reviewed manifest's SHA-256.

```sh
python3 tools/linux-boot/sign_release_materials.py \
  /absolute/local/private-directory/nagi-release.pem \
  /absolute/local/private-directory/nagi-release-public.raw \
  /absolute/local/reviewed-materials/manifest.json \
  /absolute/local/new-signatures-directory \
  --public-key-sha256 APPROVED_PUBLIC_KEY_SHA256 \
  --manifest-sha256 REVIEWED_MANIFEST_SHA256
```

The signer prompts once, signs the fixed bytes in a single batch, and verifies each signature against the reviewed public key before creating the output directory. Return only the 64-byte `<file>.sig` files. The assembly owner verifies each signature and the final artifact with the corresponding consumer. The key, passphrase and owner paths stay off the release records.

| Artifact | Existing signed bytes and assembly |
|---|---|
| Linux boot catalog | `Nagi Linux boot system service catalog v1` plus a NUL and the exact catalog JSON bytes. Append the detached signature to the original JSON, without the domain prefix. The catalog and C3 recovery verifiers must use the same approved build anchor. |
| XAPP v1 | The encoded header through license bytes, with the signed flag and 64-byte signature length set. Append the signature as the footer. Verify using the public key from the approved catalog. |
| Data selection | Existing format uses `nagi-data-selection-v1` plus a NUL and the canonical LF-terminated body. Append the signature to the body. The existing verifier is Developer Preview only. Product anchor and mount wiring require review before collection of a product signature. |
| READY and System boot metadata | The existing development READY plan binds the complete signed catalog. A Linux product READY path and signed System metadata consumer require review before collection of product signatures. System verity metadata from the image builder is unsigned staging material. |

The current catalog binds the package ID, entry and **executable** digest. The package owner separately binds the complete signed XAPP and its installed generation. This lets the owner sign a catalog and the referenced XAPP in one batch once both preimages are reviewed. The development READY plan binds the **complete signed catalog**, so any approved product format with that dependency needs its assembly and review order confirmed before signing.
