use crate::domain::NoteDocument;
use crate::identity::ObjectIdSource;
use sha2::{Digest, Sha256};
use std::collections::HashMap;

pub use crate::retained_markdown::{export_markdown, import_markdown};
pub use crate::retained_markdown::{MarkdownError, StoredNoteError};

const VERSION_2: &str = "nagi-notes-version: 2";
const VERSION_3: &str = "nagi-notes-version: 3";
const DOCUMENT_DIGEST: &str = "document-sha256";

/// Write the retained Notes fields/body with a whole-document corruption check.
/// The checksum field itself is omitted from its input; the version, all other
/// front matter, and the body are included byte-for-byte. This is not authentication.
pub(crate) fn encode_stored(note: &NoteDocument) -> String {
    let retained = crate::retained_markdown::encode_stored(note);
    let (metadata, body) = split_front_matter(&retained)
        .expect("retained Notes encoder always emits valid front matter");
    let metadata = metadata.replacen(VERSION_2, VERSION_3, 1);
    let checksum_input = format!("---\n{metadata}\n---\n{body}");
    let digest = hex(&Sha256::digest(checksum_input.as_bytes()));
    format!("---\n{metadata}\n{DOCUMENT_DIGEST}: {digest}\n---\n{body}")
}

/// Encode only when the current stored Markdown representation can reproduce the
/// exact Notes model. The candidate writer still uses the retained format; this
/// guard prevents an ambiguous value from being reported as durably committed.
pub(crate) fn encode_stored_checked(note: &NoteDocument) -> Result<String, &'static str> {
    let encoded = encode_stored(note);
    let decoded = decode_stored(&encoded, &crate::identity::HostObjectIdSource::new())
        .map_err(|_| "stored Markdown cannot reproduce this Notes document")?;
    if decoded != *note {
        return Err("stored Markdown would change this Notes document");
    }
    Ok(encoded)
}

/// Decode stored Notes. V1 and V2 retain their historical import-like marker
/// behavior; V3 requires a complete, unique marker for every persisted block.
/// New V3 files carry a checksum over the complete serialized document.
pub(crate) fn decode_stored(
    input: &str,
    ids: &dyn ObjectIdSource,
) -> Result<NoteDocument, MarkdownError> {
    let (metadata, body) = split_front_matter(input)?;
    let fields = parse_fields(metadata)?;
    let version = fields
        .get("nagi-notes-version")
        .copied()
        .ok_or(MarkdownError::MissingField("nagi-notes-version"))?;

    let decoder_input = match version {
        "1" => {
            if fields.contains_key("body-byte-length")
                || fields.contains_key("body-sha256")
                || fields.contains_key(DOCUMENT_DIGEST)
            {
                return Err(MarkdownError::BodyIntegrityMismatch);
            }
            input.to_owned()
        }
        "2" => {
            if fields.contains_key(DOCUMENT_DIGEST) {
                // A V3 digest under a V2 header is a version downgrade.
                return Err(MarkdownError::BodyIntegrityMismatch);
            }
            input.to_owned()
        }
        "3" => {
            verify_document_digest(metadata, body, fields.get(DOCUMENT_DIGEST).copied())?;
            as_retained_v2(metadata, body)
        }
        _ => return Err(MarkdownError::UnsupportedVersion),
    };

    let note = crate::retained_markdown::decode_stored(&decoder_input, ids)?;
    if version == "3" {
        validate_stored_block_ids(body, &note)?;
    }
    Ok(note)
}

fn split_front_matter(input: &str) -> Result<(&str, &str), MarkdownError> {
    let rest = input
        .strip_prefix("---\n")
        .ok_or(MarkdownError::MissingFrontMatter)?;
    let end = rest
        .find("\n---\n")
        .ok_or(MarkdownError::InvalidFrontMatter)?;
    let body_start = end + "\n---\n".len();
    Ok((&rest[..end], &rest[body_start..]))
}

fn parse_fields(metadata: &str) -> Result<HashMap<&str, &str>, MarkdownError> {
    let mut fields = HashMap::new();
    for line in metadata.lines() {
        let (key, value) = line
            .split_once(':')
            .ok_or(MarkdownError::InvalidFrontMatter)?;
        if fields.insert(key.trim(), value.trim()).is_some() {
            return Err(MarkdownError::InvalidFrontMatter);
        }
    }
    Ok(fields)
}

fn verify_document_digest(
    metadata: &str,
    body: &str,
    expected: Option<&str>,
) -> Result<(), MarkdownError> {
    let expected = expected.ok_or(MarkdownError::MissingField(DOCUMENT_DIGEST))?;
    // The stored writer emits LF-only front matter. Preserve the body verbatim,
    // including user text that contains carriage returns.
    if metadata.contains('\r') {
        return Err(MarkdownError::InvalidFrontMatter);
    }
    let without_digest = metadata
        .lines()
        .filter(|line| !line.starts_with("document-sha256:"))
        .collect::<Vec<_>>()
        .join("\n");
    let checksum_input = format!("---\n{without_digest}\n---\n{body}");
    let actual = hex(&Sha256::digest(checksum_input.as_bytes()));
    if expected != actual {
        return Err(MarkdownError::BodyIntegrityMismatch);
    }
    Ok(())
}

fn as_retained_v2(metadata: &str, body: &str) -> String {
    let fields = metadata
        .lines()
        .filter(|line| !line.starts_with("document-sha256:"))
        .map(|line| if line == VERSION_3 { VERSION_2 } else { line })
        .collect::<Vec<_>>()
        .join("\n");
    format!("---\n{fields}\n---\n{body}")
}

fn validate_stored_block_ids(body: &str, note: &NoteDocument) -> Result<(), MarkdownError> {
    let mut marker_ids = Vec::new();
    for line in body.lines() {
        let line = line.trim();
        if !line.starts_with("<!--nagi:block-id=") {
            continue;
        }
        let marker = line
            .strip_prefix("<!--nagi:block-id=")
            .and_then(|marker| marker.strip_suffix("-->"))
            .ok_or(MarkdownError::InvalidFrontMatter)?;
        let id = marker
            .split(';')
            .next()
            .ok_or(MarkdownError::InvalidFrontMatter)?;
        if id.len() != 16 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(MarkdownError::InvalidFrontMatter);
        }
        let id = u64::from_str_radix(id, 16).map_err(|_| MarkdownError::InvalidFrontMatter)?;
        marker_ids.push(id);
    }

    let mut unique = marker_ids.clone();
    unique.sort_unstable();
    unique.dedup();
    let block_ids = note
        .blocks
        .iter()
        .map(|block| block.id.0)
        .collect::<Vec<_>>();
    if unique.len() != marker_ids.len() || marker_ids != block_ids {
        return Err(MarkdownError::InvalidFrontMatter);
    }
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(DIGITS[(byte >> 4) as usize] as char);
        output.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    output
}
