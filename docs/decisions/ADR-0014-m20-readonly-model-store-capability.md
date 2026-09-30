# ADR-0014: Read-only Model Store capability for the bootstrap process

**Status:** Accepted for the M20 artifact-reader foundation

**Date:** 2026-10-01

## Context

The M30 GPT image already contains a dedicated FAT32 Model Store partition, and
M20's `ModelArtifactReader` verifies artifact bytes before backend loading.
However, the kernel currently exposes only an 8 MiB User Data extent to the
bootstrap process. User-space therefore cannot read a model artifact from the
Model Store. Passing the whole disk or reusing the writable User Data
capability would weaken the partition boundary.

## Decision

The kernel may pass a separate capability for the validated Model Store GPT
extent to the bootstrap process. The existing block-read syscall interprets
sector numbers relative to the capability's partition. The capability grants
read access only; block-write and flush continue to accept only the bounded
User Data capability. The Model Store capability is absent when the GPT entry
is missing or invalid, and the kernel does not fall back to raw-disk access.

The first consumer is a user-space FAT32 artifact reader. It locates an
8.3-name file in the root directory and returns bounded random reads through
the `ModelArtifactReader` contract. Artifact integrity remains unverified until
the M20 runtime hashes the bytes against its manifest; GPT and FAT metadata do
not establish model authenticity.

This adds the capability to the existing bootstrap entry contract. The current
bootstrap process is still the only user process and already receives platform
capabilities. This change does not claim per-service or per-application
authority isolation; production service authentication remains a separate
requirement. Model installation and mutation of the Model Store are not
provided by this capability.

## Consequences

- User Data remains limited to the first 16,384 sectors and remains the only
  user-writable block extent.
- Model Manager can read from the exact validated Model Store partition
  without a host filesystem path or raw-disk capability.
- The external Granite artifact remains optional; missing or invalid model
  files do not prevent boot.
- Model Store artifact discovery, manifest trust, installation, backend
  loading, and in-guest inference still require separate implementation and
  acceptance evidence.
