# ADR-0013: Canonical Media-Type Identifiers

- Status: Accepted for Nagi 0.2 host contracts
- Date: 2026-10-02

## Context

Two contracts validated media types differently:

| Rule | SDK manifest (`valid_media_type`, JSON Schema) | CLIP-01 `MediaType` |
|---|---|---|
| Case | upper and lower accepted | lowercase only |
| First character | `.`, `+`, `-` allowed | letter or digit |
| RFC 6838 `! # $ & ^ _` | rejected | accepted |
| Length | unbounded | 127 per token, 255 total |
| Parameters / wildcards | rejected / not checked | rejected / rejected |

The same identifier could therefore be valid in an app manifest and invalid on
the clipboard, or the reverse. Uppercase variants also allowed two spellings
of one type, and equality checks (for example, authorization of a restricted
clipboard format) compare exact strings.

## Decision

Nagi has one canonical media-type grammar, owned by `nagi-model`
(`nagi_model::validate_media_type`, `MediaTypeError`, `MAX_MEDIA_TYPE_BYTES`,
`MAX_MEDIA_TYPE_TOKEN_BYTES`):

- `type/subtype`, both lowercase;
- each token 1-127 bytes of RFC 6838 restricted-name characters
  (`a-z 0-9 ! # $ & - ^ _ . +`), starting with a letter or digit;
- at most 255 bytes in total;
- no parameters (`;`), whitespace, or wildcards (`*`).

Non-canonical spellings are rejected, not normalized, so the validated string
is exactly the one stored and compared.

- `nagi-model` is the shared, `no_std`, dependency-free model crate that the
  SDK and the clipboard foundation already use, so neither depends on the other.
- The SDK manifest validator and `sdk/rust/schemas/app-manifest.schema.json`
  (`mediaType` pattern and `maxLength`) use this grammar.
- `nagi-clipboard-core::MediaType` delegates to it and re-exports the same
  `MediaTypeError`.

## Consequences

- Manifests with uppercase or otherwise non-canonical `mediaType` values now
  fail validation. All checked-in fixtures already use canonical values
  (`image/svg+xml`). This is an intentional pre-1.0 tightening.
- Future contracts that name media types (drag-and-drop, Files metadata,
  Albert downloads) should reuse `nagi_model::validate_media_type` instead of
  defining another grammar.
- A media type is a naming rule only; it grants no authority.
