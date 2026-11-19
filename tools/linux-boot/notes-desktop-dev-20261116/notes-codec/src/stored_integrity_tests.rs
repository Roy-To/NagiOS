use crate::domain::{Block, BlockKind, NoteDocument, Timestamp};
use crate::identity::HostObjectIdSource;
use crate::markdown::{decode_stored, encode_stored, MarkdownError};
use nagi_model::ObjectId;

fn fixture() -> NoteDocument {
    let mut note = NoteDocument::new(ObjectId(0x1234), "integrity title".to_owned(), Timestamp(1));
    note.revision = 1;
    note.blocks.push(Block::new(
        ObjectId(0x1235),
        BlockKind::Paragraph("integrity body".to_owned()),
    ));
    note
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

fn refresh_v3_integrity(stored: &str) -> String {
    use sha2::{Digest, Sha256};

    let rest = stored.strip_prefix("---\n").expect("front matter prefix");
    let (metadata, body) = rest.split_once("\n---\n").expect("front matter end");
    let body_digest = hex(&Sha256::digest(body.as_bytes()));
    let mut fields = metadata
        .lines()
        .filter(|line| !line.starts_with("document-sha256:"))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    for line in &mut fields {
        if line.starts_with("body-byte-length:") {
            *line = format!("body-byte-length: {}", body.len());
        } else if line.starts_with("body-sha256:") {
            *line = format!("body-sha256: {body_digest}");
        }
    }
    let base_metadata = fields.join("\n");
    let base = format!("---\n{base_metadata}\n---\n{body}");
    let digest = hex(&Sha256::digest(base.as_bytes()));
    format!("---\n{base_metadata}\ndocument-sha256: {digest}\n---\n{body}")
}

fn legacy_bytes(stored: &str, version: &str, body: &str) -> String {
    use sha2::{Digest, Sha256};

    let rest = stored.strip_prefix("---\n").expect("front matter prefix");
    let (metadata, _) = rest.split_once("\n---\n").expect("front matter end");
    let body_digest = hex(&Sha256::digest(body.as_bytes()));
    let fields = metadata
        .lines()
        .filter_map(|line| {
            if line.starts_with("document-sha256:")
                || (version == "1"
                    && (line.starts_with("body-byte-length:") || line.starts_with("body-sha256:")))
            {
                None
            } else if line.starts_with("nagi-notes-version:") {
                Some(format!("nagi-notes-version: {version}"))
            } else if version == "2" && line.starts_with("body-byte-length:") {
                Some(format!("body-byte-length: {}", body.len()))
            } else if version == "2" && line.starts_with("body-sha256:") {
                Some(format!("body-sha256: {body_digest}"))
            } else {
                Some(line.to_owned())
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("---\n{fields}\n---\n{body}")
}

fn without_block_ids(note: &NoteDocument) -> Vec<BlockKind> {
    note.blocks.iter().map(|block| block.kind.clone()).collect()
}

#[test]
fn new_stored_revisions_protect_all_metadata_and_reject_version_downgrade() {
    let encoded = encode_stored(&fixture());
    assert!(encoded.starts_with("---\nnagi-notes-version: 3\n"));
    assert!(encoded.contains("document-sha256: "));

    let mut changed_title = encoded.clone();
    changed_title = changed_title.replace("integrity title", "integrity titlE");
    assert!(matches!(
        decode_stored(&changed_title, &HostObjectIdSource::new()),
        Err(MarkdownError::BodyIntegrityMismatch)
    ));

    for downgraded_version in ["2", "1"] {
        let downgraded = encoded.replacen(
            "nagi-notes-version: 3",
            &format!("nagi-notes-version: {downgraded_version}"),
            1,
        );
        assert!(matches!(
            decode_stored(&downgraded, &HostObjectIdSource::new()),
            Err(MarkdownError::BodyIntegrityMismatch | MarkdownError::InvalidFrontMatter)
        ));
    }
}

#[test]
fn stored_decode_rejects_missing_or_duplicate_block_ids_even_with_valid_integrity() {
    let mut note = fixture();
    note.blocks.push(Block::new(
        ObjectId(0x1236),
        BlockKind::Paragraph("second block".to_owned()),
    ));
    let encoded = encode_stored(&note);
    let first_marker = "<!--nagi:block-id=0000000000001235-->\n";
    let missing = encoded.replacen(first_marker, "", 1);
    let missing = refresh_v3_integrity(&missing);
    assert!(matches!(
        decode_stored(&missing, &HostObjectIdSource::new()),
        Err(MarkdownError::InvalidFrontMatter)
    ));

    let second_marker = "<!--nagi:block-id=0000000000001236-->";
    let duplicate = encoded.replacen(second_marker, "<!--nagi:block-id=0000000000001235-->", 1);
    let duplicate = refresh_v3_integrity(&duplicate);
    assert!(matches!(
        decode_stored(&duplicate, &HostObjectIdSource::new()),
        Err(MarkdownError::InvalidFrontMatter)
    ));
}

#[test]
fn legacy_v1_v2_markers_keep_the_retained_reader_behavior() {
    let mut note = fixture();
    note.blocks.push(Block::new(
        ObjectId(0x1236),
        BlockKind::Paragraph("second block".to_owned()),
    ));
    let encoded = encode_stored(&note);
    let first_marker = "<!--nagi:block-id=0000000000001235-->\n";
    let second_marker = "<!--nagi:block-id=0000000000001236-->";
    let (metadata, body) = encoded
        .strip_prefix("---\n")
        .expect("front matter prefix")
        .split_once("\n---\n")
        .expect("front matter end");
    let _ = metadata;
    let missing_marker_body = body.replacen(first_marker, "", 1);
    let duplicate_marker_body =
        body.replacen(second_marker, "<!--nagi:block-id=0000000000001235-->", 1);

    for version in ["1", "2"] {
        for legacy_body in [&missing_marker_body, &duplicate_marker_body] {
            let bytes = legacy_bytes(&encoded, version, legacy_body);
            let old_reader =
                crate::retained_markdown::decode_stored(&bytes, &HostObjectIdSource::new())
                    .expect("retained v1/v2 reader accepted its historical marker behavior");
            let candidate_reader = decode_stored(&bytes, &HostObjectIdSource::new())
                .expect("candidate wrapper preserves the retained legacy behavior");

            assert_eq!(old_reader.blocks.len(), 2);
            assert_eq!(candidate_reader.blocks.len(), 2);
            assert_eq!(
                without_block_ids(&old_reader),
                without_block_ids(&candidate_reader)
            );
            let ids = candidate_reader
                .blocks
                .iter()
                .map(|block| block.id.0)
                .collect::<std::collections::HashSet<_>>();
            assert_eq!(
                ids.len(),
                2,
                "legacy duplicate markers are remapped as before"
            );
        }
    }
}
