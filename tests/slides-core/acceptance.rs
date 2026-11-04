use nagi_slides_core::*;
use std::collections::BTreeMap;

fn object(id: u64) -> Object {
    Object {
        id: ObjectId(id),
        geometry: Geometry {
            x: 1000,
            y: 2000,
            size: LogicalSize {
                width: 100_000,
                height: 50_000,
            },
            rotation_millidegrees: 0,
            z_order: 3,
        },
        content: Content::Text("日本語 / English 🐈 e\u{301}".into()),
        theme_token: Some("body".into()),
        source: None,
        target: None,
    }
}
fn slide(id: u64, objects: Vec<Object>) -> Slide {
    Slide {
        id: ObjectId(id),
        layout: ObjectId(3),
        notes: "発表ノート\nSpeaker notes".into(),
        objects,
    }
}
fn presentation() -> Presentation {
    Presentation::new(
        ObjectId(1),
        Theme {
            id: ObjectId(2),
            slide_size: LogicalSize {
                width: 960_000,
                height: 540_000,
            },
            fonts: BTreeMap::from([("body".into(), "Noto Sans".into())]),
            colors: BTreeMap::from([("accent".into(), 0x123456ff)]),
        },
        vec![Layout {
            id: ObjectId(3),
            kind: LayoutKind::TitleContent,
            name: "Title + 内容".into(),
        }],
        Limits::default(),
    )
    .unwrap()
}
fn apply(p: &mut Presentation, edits: &[Edit]) -> Result<RevisionId> {
    p.apply(p.revision(), edits, Limits::default())
}
fn populated() -> Presentation {
    let mut p = presentation();
    apply(
        &mut p,
        &[
            Edit::AddSlide {
                index: 0,
                slide: slide(10, vec![object(11)]),
            },
            Edit::AddSlide {
                index: 1,
                slide: slide(20, vec![object(21)]),
            },
        ],
    )
    .unwrap();
    p
}
fn unchanged(p: &mut Presentation, edits: &[Edit], error: Error) {
    let before = p.clone();
    assert_eq!(apply(p, edits), Err(error));
    assert_eq!(*p, before);
}

#[test]
fn canonical_ids_survive_edit_reorder_and_notes() {
    let mut p = populated();
    let mut edited = p.slides()[0].objects[0].clone();
    edited.content = Content::Text("改訂版".into());
    apply(
        &mut p,
        &[
            Edit::SetObject(edited),
            Edit::ReorderSlide {
                slide: ObjectId(10),
                index: 1,
            },
            Edit::SetNotes {
                slide: ObjectId(10),
                notes: "ノート更新".into(),
            },
        ],
    )
    .unwrap();
    assert_eq!(p.id(), ObjectId(1));
    assert_eq!(p.revision(), RevisionId(3));
    assert_eq!(p.slides()[1].id, ObjectId(10));
    assert_eq!(p.slides()[1].objects[0].id, ObjectId(11));
    assert_eq!(p.slides()[1].notes, "ノート更新");
}
#[test]
fn duplicate_remaps_internal_targets_but_preserves_external_source() {
    let mut p = populated();
    let mut o = p.slides()[0].objects[0].clone();
    o.target = Some(ObjectId(10));
    o.source = Some(reference());
    apply(
        &mut p,
        &[
            Edit::SetObject(o),
            Edit::DuplicateSlide {
                slide: ObjectId(10),
                index: 1,
                id: ObjectId(30),
                object_ids: vec![ObjectId(31)],
            },
        ],
    )
    .unwrap();
    assert_eq!(p.slides()[1].id, ObjectId(30));
    assert_eq!(p.slides()[1].objects[0].id, ObjectId(31));
    assert_eq!(p.slides()[1].objects[0].target, Some(ObjectId(30)));
    assert_eq!(p.slides()[1].objects[0].source, Some(reference()));
    assert_eq!(p.slides()[0].objects[0].target, Some(ObjectId(10)));
}
#[test]
fn duplicate_remaps_copied_object_target_and_keeps_other_slide_target() {
    let mut p = populated();
    let mut o = object(12);
    o.target = Some(ObjectId(11));
    let mut original = object(11);
    original.target = Some(ObjectId(20));
    apply(
        &mut p,
        &[
            Edit::SetObject(original),
            Edit::AddObject {
                slide: ObjectId(10),
                index: 1,
                object: o,
            },
            Edit::DuplicateSlide {
                slide: ObjectId(10),
                index: 2,
                id: ObjectId(30),
                object_ids: vec![ObjectId(31), ObjectId(32)],
            },
        ],
    )
    .unwrap();
    assert_eq!(p.slides()[2].objects[1].target, Some(ObjectId(31)));
    assert_eq!(p.slides()[2].objects[0].target, Some(ObjectId(20)));
}
#[test]
fn references_follow_identity_and_deletion_requires_atomic_repair() {
    let mut p = populated();
    let mut o = object(11);
    o.target = Some(ObjectId(20));
    apply(
        &mut p,
        &[
            Edit::SetObject(o),
            Edit::ReorderSlide {
                slide: ObjectId(20),
                index: 0,
            },
        ],
    )
    .unwrap();
    unchanged(
        &mut p,
        &[Edit::DeleteSlide(ObjectId(20))],
        Error::InvalidReference,
    );
    apply(
        &mut p,
        &[Edit::DeleteSlide(ObjectId(20)), Edit::SetObject(object(11))],
    )
    .unwrap();
    assert_eq!(p.slides().len(), 1);
    unchanged(
        &mut p,
        &[Edit::AddSlide {
            index: 1,
            slide: slide(20, vec![]),
        }],
        Error::DuplicateId(ObjectId(20)),
    );
}
#[test]
fn object_delete_and_add_are_transactional_and_ids_never_reused() {
    let mut p = populated();
    apply(&mut p, &[Edit::DeleteObject(ObjectId(11))]).unwrap();
    unchanged(
        &mut p,
        &[Edit::AddObject {
            slide: ObjectId(10),
            index: 0,
            object: object(11),
        }],
        Error::DuplicateId(ObjectId(11)),
    );
    apply(
        &mut p,
        &[Edit::AddObject {
            slide: ObjectId(10),
            index: 0,
            object: object(12),
        }],
    )
    .unwrap();
    assert_eq!(p.slides()[0].objects[0].id, ObjectId(12));
}
#[test]
fn failed_batch_does_not_consume_ids_or_mutate_earlier_edits() {
    let mut p = populated();
    unchanged(
        &mut p,
        &[
            Edit::AddSlide {
                index: 2,
                slide: slide(30, vec![object(31)]),
            },
            Edit::DeleteSlide(ObjectId(404)),
        ],
        Error::MissingObject(ObjectId(404)),
    );
    apply(
        &mut p,
        &[Edit::AddSlide {
            index: 2,
            slide: slide(30, vec![object(31)]),
        }],
    )
    .unwrap();
}
#[test]
fn stale_revision_rejects_everything() {
    let mut p = populated();
    let before = p.clone();
    assert_eq!(
        p.apply(
            RevisionId(1),
            &[Edit::DeleteSlide(ObjectId(10))],
            Limits::default()
        ),
        Err(Error::Conflict {
            expected: RevisionId(1),
            actual: RevisionId(2)
        })
    );
    assert_eq!(p, before);
}
#[test]
fn invalid_geometry_never_partially_mutates() {
    let mut p = populated();
    for g in [
        Geometry {
            x: i64::MAX,
            ..object(11).geometry
        },
        Geometry {
            x: MAX_LOGICAL,
            ..object(11).geometry
        },
        Geometry {
            rotation_millidegrees: 360_000,
            ..object(11).geometry
        },
        Geometry {
            size: LogicalSize {
                width: 0,
                height: 1,
            },
            ..object(11).geometry
        },
        Geometry {
            size: LogicalSize {
                width: -1,
                height: 1,
            },
            ..object(11).geometry
        },
    ] {
        let mut o = object(11);
        o.geometry = g;
        unchanged(
            &mut p,
            &[
                Edit::SetTitle("Should roll back".into()),
                Edit::SetObject(o),
            ],
            Error::InvalidGeometry,
        );
    }
}
#[test]
fn valid_negative_geometry_is_logical_and_roundtrips() {
    let mut p = populated();
    let mut o = object(11);
    o.geometry.x = -1000;
    o.geometry.rotation_millidegrees = 359_999;
    o.geometry.z_order = i32::MIN;
    apply(&mut p, &[Edit::SetObject(o)]).unwrap();
    assert_eq!(
        native::decode(
            &native::encode(&p, Limits::default()).unwrap(),
            Limits::default()
        )
        .unwrap(),
        p
    );
}
#[test]
fn malformed_ids_indices_and_duplicate_mapping_leave_state_unchanged() {
    let mut p = populated();
    for id in [0, u64::MAX] {
        unchanged(
            &mut p,
            &[Edit::AddSlide {
                index: 2,
                slide: slide(id, vec![]),
            }],
            Error::InvalidId,
        );
    }
    unchanged(
        &mut p,
        &[Edit::ReorderSlide {
            slide: ObjectId(10),
            index: 2,
        }],
        Error::InvalidIndex,
    );
    unchanged(
        &mut p,
        &[Edit::DuplicateSlide {
            slide: ObjectId(10),
            index: 0,
            id: ObjectId(30),
            object_ids: vec![],
        }],
        Error::InvalidReference,
    );
    unchanged(
        &mut p,
        &[Edit::DuplicateSlide {
            slide: ObjectId(10),
            index: 0,
            id: ObjectId(30),
            object_ids: vec![ObjectId(30)],
        }],
        Error::DuplicateId(ObjectId(30)),
    );
}
#[test]
fn theme_layout_references_validate_atomically() {
    let mut p = populated();
    unchanged(
        &mut p,
        &[Edit::SetLayout {
            slide: ObjectId(10),
            layout: ObjectId(404),
        }],
        Error::InvalidReference,
    );
    let mut theme = p.theme().clone();
    theme.fonts.clear();
    unchanged(&mut p, &[Edit::SetTheme(theme)], Error::InvalidTheme);
    let mut theme = p.theme().clone();
    theme.id = ObjectId(40);
    unchanged(&mut p, &[Edit::SetTheme(theme)], Error::InvalidReference);
    let mut o = object(11);
    o.theme_token = Some("missing".into());
    unchanged(&mut p, &[Edit::SetObject(o)], Error::InvalidTheme);
}
#[test]
fn count_byte_and_text_bounds_preserve_snapshot() {
    let mut p = populated();
    let before = p.clone();
    let limits = Limits {
        max_slides: 2,
        ..Limits::default()
    };
    assert_eq!(
        p.apply(
            p.revision(),
            &[Edit::AddSlide {
                index: 2,
                slide: slide(30, vec![])
            }],
            limits
        ),
        Err(Error::LimitExceeded)
    );
    let limits = Limits {
        max_ids: 7,
        ..Limits::default()
    };
    assert_eq!(
        p.apply(
            p.revision(),
            &[Edit::AddSlide {
                index: 2,
                slide: slide(30, vec![])
            }],
            limits
        ),
        Err(Error::LimitExceeded)
    );
    let wire = native::encode(&p, Limits::default()).unwrap();
    let limits = Limits {
        max_bytes: wire.len(),
        max_text_bytes: 100,
        ..Limits::default()
    };
    assert_eq!(
        p.apply(p.revision(), &[Edit::SetTitle("x".repeat(100))], limits),
        Err(Error::LimitExceeded)
    );
    unchanged(
        &mut p,
        &[Edit::SetTitle("x".repeat(65_537))],
        Error::LimitExceeded,
    );
    assert_eq!(p, before);
}
#[test]
fn operation_and_per_slide_limits_reject_before_clone() {
    let mut p = populated();
    let before = p.clone();
    let limits = Limits {
        max_operations: 1,
        ..Limits::default()
    };
    assert_eq!(
        p.apply(
            p.revision(),
            &[Edit::SetTitle("a".into()), Edit::SetTitle("b".into())],
            limits
        ),
        Err(Error::LimitExceeded)
    );
    let limits = Limits {
        max_objects_per_slide: 1,
        ..Limits::default()
    };
    assert_eq!(
        p.apply(
            p.revision(),
            &[Edit::AddObject {
                slide: ObjectId(10),
                index: 1,
                object: object(12)
            }],
            limits
        ),
        Err(Error::LimitExceeded)
    );
    assert_eq!(p, before);
}
#[test]
fn unicode_and_empty_notes_are_preserved_and_nul_rejected() {
    let mut p = populated();
    apply(
        &mut p,
        &[Edit::SetNotes {
            slide: ObjectId(10),
            notes: String::new(),
        }],
    )
    .unwrap();
    let bytes = native::encode(&p, Limits::default()).unwrap();
    assert_eq!(native::decode(&bytes, Limits::default()).unwrap(), p);
    unchanged(
        &mut p,
        &[Edit::SetTitle("nul\0".into())],
        Error::InvalidText,
    );
}
#[test]
fn native_is_deterministic_versioned_and_preserves_tombstones() {
    let mut p = populated();
    apply(&mut p, &[Edit::DeleteSlide(ObjectId(20))]).unwrap();
    let bytes = native::encode(&p, Limits::default()).unwrap();
    let mut restored = native::decode(&bytes, Limits::default()).unwrap();
    assert_eq!(restored, p);
    assert_eq!(native::encode(&restored, Limits::default()).unwrap(), bytes);
    unchanged(
        &mut restored,
        &[Edit::AddSlide {
            index: 1,
            slide: slide(20, vec![]),
        }],
        Error::DuplicateId(ObjectId(20)),
    );
    let mut future = bytes.clone();
    future[8..10].copy_from_slice(&2u16.to_le_bytes());
    assert_eq!(
        native::decode(&future, Limits::default()),
        Err(Error::UnknownVersion(2))
    );
}
#[test]
fn every_truncated_native_prefix_and_trailing_bytes_rejected() {
    let bytes = native::encode(&populated(), Limits::default()).unwrap();
    for len in 0..bytes.len() {
        assert!(
            native::decode(&bytes[..len], Limits::default()).is_err(),
            "prefix {len}"
        );
    }
    let mut trailing = bytes;
    trailing.push(0);
    assert_eq!(
        native::decode(&trailing, Limits::default()),
        Err(Error::MalformedNative)
    );
}
#[test]
fn hostile_native_lengths_and_invalid_utf8_rejected() {
    let mut p = populated();
    apply(&mut p, &[Edit::SetTitle("a".into())]).unwrap();
    let bytes = native::encode(&p, Limits::default()).unwrap();
    let mut hostile = bytes.clone();
    hostile[26..30].copy_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        native::decode(&hostile, Limits::default()),
        Err(Error::LimitExceeded)
    );
    let mut invalid = bytes;
    invalid[30] = 255;
    assert_eq!(
        native::decode(&invalid, Limits::default()),
        Err(Error::MalformedNative)
    );
}
#[test]
fn invalid_native_revision_and_revision_overflow_are_rejected() {
    let bytes = native::encode(&populated(), Limits::default()).unwrap();
    for revision in [0, u64::MAX] {
        let mut invalid = bytes.clone();
        invalid[18..26].copy_from_slice(&revision.to_le_bytes());
        assert_eq!(
            native::decode(&invalid, Limits::default()),
            Err(Error::InvalidRevision)
        );
    }
    let mut last = bytes;
    last[18..26].copy_from_slice(&(u64::MAX - 1).to_le_bytes());
    let mut p = native::decode(&last, Limits::default()).unwrap();
    unchanged(
        &mut p,
        &[Edit::SetTitle("overflow".into())],
        Error::InvalidRevision,
    );
}
fn reference() -> SourceReference {
    SourceReference {
        resource: ObjectId(11),
        object: Some(ObjectId(1001)),
        revision: RevisionId(5),
        kind: SourceKind::SheetsChart,
        label: "売上 / Sales".into(),
        locator: Some("sheet:売上!A1".into()),
    }
}
struct RevisionAdapter(RevisionId);
impl SourceRevisions for RevisionAdapter {
    fn revision(&self, _: ObjectId) -> Result<RevisionId> {
        Ok(self.0)
    }
}
#[test]
fn source_revision_detection_is_pure_and_fail_closed() {
    let reference = reference();
    assert_eq!(
        reference.status(&RevisionAdapter(RevisionId(5))),
        Ok(SourceStatus::Current)
    );
    assert_eq!(
        reference.status(&RevisionAdapter(RevisionId(6))),
        Ok(SourceStatus::Changed {
            baseline: RevisionId(5),
            observed: RevisionId(6)
        })
    );
    assert_eq!(
        reference.status(&MissingSourceAdapter),
        Err(Error::AdapterUnavailable)
    );
    assert_eq!(
        reference.status(&RevisionAdapter(RevisionId(0))),
        Err(Error::InvalidRevision)
    );
}
#[test]
fn external_formats_require_real_adapters() {
    let p = populated();
    for format in [OutputFormat::Pdf, OutputFormat::Pptx] {
        assert_eq!(
            p.export(format, Limits::default()),
            Err(Error::AdapterUnavailable)
        );
    }
    assert_eq!(
        p.export(OutputFormat::Native, Limits::default()),
        native::encode(&p, Limits::default())
    );
}
#[test]
fn embedded_reference_roundtrips_and_missing_metadata_rejects() {
    let mut p = populated();
    let mut o = object(11);
    o.content = Content::EmbeddedReference;
    unchanged(
        &mut p,
        &[Edit::SetObject(o.clone())],
        Error::InvalidReference,
    );
    o.source = Some(reference());
    apply(&mut p, &[Edit::SetObject(o)]).unwrap();
    assert_eq!(
        native::decode(
            &native::encode(&p, Limits::default()).unwrap(),
            Limits::default()
        )
        .unwrap(),
        p
    );
}

#[test]
fn duplicate_cannot_exceed_intermediate_bytes_even_if_deleted_in_same_batch() {
    let mut p = populated();
    let before = p.clone();
    let base_bytes = native::encode(&p, Limits::default()).unwrap().len();
    // The final state would fit exactly: only two 8-byte ID tombstones remain.
    // Its intermediate live duplicate must be rejected before copying content.
    let limits = Limits {
        max_bytes: base_bytes + 16,
        max_text_bytes: 128,
        ..Limits::default()
    };
    assert_eq!(
        p.apply(
            p.revision(),
            &[
                Edit::DuplicateSlide {
                    slide: ObjectId(10),
                    index: 2,
                    id: ObjectId(30),
                    object_ids: vec![ObjectId(31)]
                },
                Edit::DeleteSlide(ObjectId(30)),
            ],
            limits
        ),
        Err(Error::LimitExceeded)
    );
    assert_eq!(p, before);
    // Failed accounting must not consume supplied IDs.
    apply(
        &mut p,
        &[Edit::DuplicateSlide {
            slide: ObjectId(10),
            index: 2,
            id: ObjectId(30),
            object_ids: vec![ObjectId(31)],
        }],
    )
    .unwrap();
}

#[test]
fn repeated_duplicate_delete_charges_cumulative_copied_bytes() {
    let mut p = populated();
    let before = p.clone();
    let mut once = p.clone();
    apply(
        &mut once,
        &[Edit::DuplicateSlide {
            slide: ObjectId(10),
            index: 2,
            id: ObjectId(30),
            object_ids: vec![ObjectId(31)],
        }],
    )
    .unwrap();
    let once_bytes = native::encode(&once, Limits::default()).unwrap().len();
    // Every intermediate snapshot can fit, including the eight pairs of
    // tombstones. Repeated copying still exhausts the separate batch budget.
    let limits = Limits {
        max_bytes: once_bytes + 7 * 16,
        max_text_bytes: 128,
        ..Limits::default()
    };
    assert!(limits.max_bytes < 2048);
    let mut edits = vec![];
    for i in 0..8 {
        let id = 30 + 2 * i;
        edits.push(Edit::DuplicateSlide {
            slide: ObjectId(10),
            index: 2,
            id: ObjectId(id),
            object_ids: vec![ObjectId(id + 1)],
        });
        edits.push(Edit::DeleteSlide(ObjectId(id)));
    }
    assert_eq!(
        p.apply(p.revision(), &edits, limits),
        Err(Error::LimitExceeded)
    );
    assert_eq!(p, before);
}

#[test]
fn duplicate_accepts_exact_wire_budget_and_rejects_one_byte_less() {
    let p = populated();
    let edit = Edit::DuplicateSlide {
        slide: ObjectId(10),
        index: 2,
        id: ObjectId(30),
        object_ids: vec![ObjectId(31)],
    };
    let mut expected = p.clone();
    apply(&mut expected, std::slice::from_ref(&edit)).unwrap();
    let exact = native::encode(&expected, Limits::default()).unwrap().len();
    let mut failed = p.clone();
    let limits = Limits {
        max_bytes: exact - 1,
        max_text_bytes: 128,
        ..Limits::default()
    };
    assert_eq!(
        failed.apply(failed.revision(), std::slice::from_ref(&edit), limits),
        Err(Error::LimitExceeded)
    );
    assert_eq!(failed, p);
    let mut accepted = p;
    accepted
        .apply(
            accepted.revision(),
            &[edit],
            Limits {
                max_bytes: exact,
                ..limits
            },
        )
        .unwrap();
    assert_eq!(accepted, expected);
}

#[test]
fn duplicate_uses_source_created_and_edited_earlier_in_batch() {
    let mut p = populated();
    apply(
        &mut p,
        &[
            Edit::AddSlide {
                index: 2,
                slide: slide(30, vec![object(31)]),
            },
            Edit::SetNotes {
                slide: ObjectId(30),
                notes: "最新ノート".repeat(8),
            },
            Edit::DuplicateSlide {
                slide: ObjectId(30),
                index: 3,
                id: ObjectId(40),
                object_ids: vec![ObjectId(41)],
            },
        ],
    )
    .unwrap();
    assert_eq!(p.slides()[3].notes, p.slides()[2].notes);
    assert_eq!(p.slides()[3].objects[0].id, ObjectId(41));
}

// PR #55 supplemental coverage. Native mutants start from bytes that decode,
// and every limit/duplicate rejection is paired with an exact accepting case.
fn le(ids: &[u64]) -> Vec<u8> {
    ids.iter().flat_map(|id| id.to_le_bytes()).collect()
}
fn wire_text(s: &str) -> Vec<u8> {
    let mut bytes = (s.len() as u32).to_le_bytes().to_vec();
    bytes.extend_from_slice(s.as_bytes());
    bytes
}
fn find_unique(bytes: &[u8], needle: &[u8]) -> usize {
    let hits: Vec<_> = bytes
        .windows(needle.len())
        .enumerate()
        .filter(|(_, window)| *window == needle)
        .map(|(at, _)| at)
        .collect();
    assert_eq!(hits.len(), 1, "fixture needle must occur exactly once");
    hits[0]
}
fn object_head(id: u64) -> Vec<u8> {
    [
        le(&[id]),
        1000i64.to_le_bytes().to_vec(),
        2000i64.to_le_bytes().to_vec(),
    ]
    .concat()
}
fn fonts_wire() -> Vec<u8> {
    [
        wire_text("body"),
        wire_text("Noto Sans"),
        wire_text("head"),
        wire_text("Noto Sans"),
    ]
    .concat()
}
fn colors_wire() -> Vec<u8> {
    [
        wire_text("accent"),
        0x1234_56ffu32.to_le_bytes().to_vec(),
        wire_text("border"),
        0xabcd_ef80u32.to_le_bytes().to_vec(),
    ]
    .concat()
}
/// The untouched bytes must decode; only the single mutation may be rejected.
fn rejects_mutant(bytes: &[u8], mutate: impl FnOnce(&mut Vec<u8>), error: Error) {
    native::decode(bytes, Limits::default()).unwrap();
    let mut hostile = bytes.to_vec();
    mutate(&mut hostile);
    assert_ne!(hostile, bytes);
    assert_eq!(native::decode(&hostile, Limits::default()), Err(error));
}
/// Two fonts/colors, source + embedded reference, negative geometry, an
/// internal target and tombstones 30/31 from an in-batch add/delete.
fn rich() -> Presentation {
    let mut p = populated();
    let mut theme = p.theme().clone();
    theme.fonts.insert("head".into(), "Noto Sans".into());
    theme.colors.insert("border".into(), 0xabcd_ef80);
    let mut linked = object(11);
    linked.source = Some(reference());
    let mut embedded = object(12);
    embedded.geometry.x = -5000;
    embedded.geometry.rotation_millidegrees = 359_999;
    embedded.content = Content::EmbeddedReference;
    embedded.theme_token = None;
    embedded.source = Some(SourceReference {
        resource: ObjectId(900),
        object: None,
        revision: RevisionId(1),
        kind: SourceKind::Other,
        label: String::new(),
        locator: None,
    });
    embedded.target = Some(ObjectId(11));
    apply(
        &mut p,
        &[
            Edit::SetTitle("題名 / Title".into()),
            Edit::SetTheme(theme),
            Edit::SetObject(linked),
            Edit::AddObject {
                slide: ObjectId(20),
                index: 1,
                object: embedded,
            },
            Edit::AddSlide {
                index: 2,
                slide: slide(30, vec![object(31)]),
            },
            Edit::DeleteSlide(ObjectId(30)),
        ],
    )
    .unwrap();
    p
}

#[test]
fn rich_native_save_load_save_is_byte_stable_and_reopened_state_edits_identically() {
    let mut original = rich();
    let bytes = native::encode(&original, Limits::default()).unwrap();
    let mut reopened = native::decode(&bytes, Limits::default()).unwrap();
    assert_eq!(reopened, original);
    assert_eq!(native::encode(&reopened, Limits::default()).unwrap(), bytes);
    let next = [Edit::AddSlide {
        index: 2,
        slide: slide(40, vec![object(41)]),
    }];
    assert_eq!(apply(&mut original, &next), Ok(RevisionId(4)));
    assert_eq!(apply(&mut reopened, &next), Ok(RevisionId(4)));
    assert_eq!(reopened, original);
    unchanged(
        &mut reopened,
        &[Edit::AddSlide {
            index: 0,
            slide: slide(31, vec![]),
        }],
        Error::DuplicateId(ObjectId(31)),
    );
}

#[test]
fn native_rejects_noncanonical_issued_font_and_color_order() {
    let plain = native::encode(&populated(), Limits::default()).unwrap();
    rejects_mutant(
        &plain,
        |b| {
            let at = find_unique(b, &le(&[10, 11]));
            b[at..at + 16].copy_from_slice(&le(&[11, 10]));
        },
        Error::MalformedNative,
    );
    let rich = native::encode(&rich(), Limits::default()).unwrap();
    rejects_mutant(
        &rich,
        |b| {
            let at = find_unique(b, &fonts_wire());
            b[at + 4..at + 8].copy_from_slice(b"head");
            b[at + 25..at + 29].copy_from_slice(b"body");
        },
        Error::MalformedNative,
    );
    rejects_mutant(
        &rich,
        |b| {
            let at = find_unique(b, &colors_wire());
            b[at + 4..at + 10].copy_from_slice(b"border");
            b[at + 18..at + 24].copy_from_slice(b"accent");
        },
        Error::MalformedNative,
    );
}

#[test]
fn native_rejects_duplicate_issued_font_and_color_entries() {
    let plain = native::encode(&populated(), Limits::default()).unwrap();
    // Without the duplicate check, set dedup would surface InvalidReference.
    rejects_mutant(
        &plain,
        |b| {
            let at = find_unique(b, &le(&[10, 11]));
            b[at + 8..at + 16].copy_from_slice(&le(&[10]));
        },
        Error::MalformedNative,
    );
    let rich = native::encode(&rich(), Limits::default()).unwrap();
    rejects_mutant(
        &rich,
        |b| {
            let at = find_unique(b, &fonts_wire());
            b[at + 25..at + 29].copy_from_slice(b"body");
        },
        Error::MalformedNative,
    );
    rejects_mutant(
        &rich,
        |b| {
            let at = find_unique(b, &colors_wire());
            b[at + 18..at + 24].copy_from_slice(b"accent");
        },
        Error::MalformedNative,
    );
}

#[test]
fn native_rejects_unknown_enum_and_presence_tags() {
    let plain = native::encode(&populated(), Limits::default()).unwrap();
    rejects_mutant(
        &plain,
        |b| {
            let layout = [le(&[3]), vec![0], wire_text("Title + 内容")].concat();
            let at = find_unique(b, &layout);
            b[at + 8] = 5;
        },
        Error::MalformedNative,
    );
    rejects_mutant(
        &plain,
        |b| {
            let at = find_unique(b, &object_head(21));
            b[at + 48] = 3;
        },
        Error::MalformedNative,
    );
    // The final byte is object 21's absent-target flag.
    rejects_mutant(
        &plain,
        |b| *b.last_mut().unwrap() = 2,
        Error::MalformedNative,
    );
    let rich = native::encode(&rich(), Limits::default()).unwrap();
    rejects_mutant(
        &rich,
        |b| {
            let source = [vec![1], le(&[1001, 5])].concat();
            let at = find_unique(b, &source);
            b[at + 17] = 6;
        },
        Error::MalformedNative,
    );
}

#[test]
fn native_rejects_missing_references_and_unissued_or_duplicate_live_ids() {
    let plain = native::encode(&populated(), Limits::default()).unwrap();
    rejects_mutant(
        &plain,
        |b| {
            let at = find_unique(b, &le(&[10, 3]));
            b[at + 8..at + 16].copy_from_slice(&le(&[404]));
        },
        Error::InvalidReference,
    );
    rejects_mutant(
        &plain,
        |b| {
            let at = find_unique(b, &le(&[20, 21]));
            b[at + 8..at + 16].copy_from_slice(&le(&[22]));
        },
        Error::InvalidReference,
    );
    rejects_mutant(
        &plain,
        |b| {
            let at = find_unique(b, &object_head(21));
            b[at..at + 8].copy_from_slice(&le(&[11]));
        },
        Error::DuplicateId(ObjectId(11)),
    );
    // Slide 20/object 21 become tombstones; object 11's target flag is last.
    let mut p = populated();
    apply(&mut p, &[Edit::DeleteSlide(ObjectId(20))]).unwrap();
    let bytes = native::encode(&p, Limits::default()).unwrap();
    let with_target = |target: u64| {
        let mut b = bytes.clone();
        assert_eq!(b.pop(), Some(0));
        b.push(1);
        b.extend_from_slice(&le(&[target]));
        b
    };
    let linked = native::decode(&with_target(10), Limits::default()).unwrap();
    assert_eq!(linked.slides()[0].objects[0].target, Some(ObjectId(10)));
    assert_eq!(
        native::encode(&linked, Limits::default()).unwrap(),
        with_target(10)
    );
    for dangling in [20, 21, 3, 404] {
        assert_eq!(
            native::decode(&with_target(dangling), Limits::default()),
            Err(Error::InvalidReference),
            "target {dangling}"
        );
    }
}

#[test]
fn native_rejects_a_complete_document_appended_after_a_valid_one() {
    let bytes = native::encode(&populated(), Limits::default()).unwrap();
    native::decode(&bytes, Limits::default()).unwrap();
    assert_eq!(
        native::decode(&[bytes.clone(), bytes].concat(), Limits::default()),
        Err(Error::MalformedNative)
    );
}

#[test]
fn native_limits_accept_exact_bytes_ids_slides_and_reject_one_outside() {
    let p = populated();
    let bytes = native::encode(&p, Limits::default()).unwrap();
    // max_text_bytes must stay <= max_bytes, or Limits::validate rejects first.
    let exact = Limits {
        max_bytes: bytes.len(),
        max_ids: 7,
        max_slides: 2,
        max_text_bytes: 64,
        ..Limits::default()
    };
    assert_eq!(native::encode(&p, exact).unwrap(), bytes);
    assert_eq!(native::decode(&bytes, exact).unwrap(), p);
    for outside in [
        Limits {
            max_bytes: bytes.len() - 1,
            ..exact
        },
        Limits {
            max_ids: 6,
            ..exact
        },
        Limits {
            max_slides: 1,
            ..exact
        },
    ] {
        assert_eq!(native::encode(&p, outside), Err(Error::LimitExceeded));
        assert_eq!(native::decode(&bytes, outside), Err(Error::LimitExceeded));
    }
}

#[test]
fn apply_accepts_exact_id_and_slide_limits_and_rejects_mid_slide_overflow() {
    let base = populated();
    let add = [Edit::AddSlide {
        index: 2,
        slide: slide(30, vec![object(31)]),
    }];
    // max_ids 8 admits slide 30 and then fails on object 31 inside one edit.
    for (limits, accepted) in [
        (
            Limits {
                max_ids: 9,
                ..Limits::default()
            },
            true,
        ),
        (
            Limits {
                max_ids: 8,
                ..Limits::default()
            },
            false,
        ),
        (
            Limits {
                max_slides: 3,
                ..Limits::default()
            },
            true,
        ),
        (
            Limits {
                max_slides: 2,
                ..Limits::default()
            },
            false,
        ),
    ] {
        let mut p = base.clone();
        let result = p.apply(p.revision(), &add, limits);
        if accepted {
            assert_eq!(result, Ok(RevisionId(3)));
            assert_eq!(p.slides()[2].objects[0].id, ObjectId(31));
        } else {
            assert_eq!(result, Err(Error::LimitExceeded));
            assert_eq!(p, base);
        }
    }
}

#[test]
fn duplicate_exhausting_id_budget_midway_leaks_no_ids_or_revision() {
    let mut p = populated();
    apply(
        &mut p,
        &[Edit::AddObject {
            slide: ObjectId(10),
            index: 1,
            object: object(12),
        }],
    )
    .unwrap();
    // Eight IDs are issued; this duplicate needs 30, 31 and 32.
    let before = p.clone();
    let duplicate = Edit::DuplicateSlide {
        slide: ObjectId(10),
        index: 2,
        id: ObjectId(30),
        object_ids: vec![ObjectId(31), ObjectId(32)],
    };
    let batch = [Edit::SetTitle("先行編集".into()), duplicate];
    let ids = |max_ids| Limits {
        max_ids,
        ..Limits::default()
    };
    assert_eq!(
        p.apply(p.revision(), &batch, ids(10)),
        Err(Error::LimitExceeded)
    );
    assert_eq!(p, before);
    assert_eq!(p.revision(), RevisionId(3));
    let mut exact = p.clone();
    assert_eq!(
        exact.apply(exact.revision(), &batch, ids(11)),
        Ok(RevisionId(4))
    );
    assert_eq!(exact.slides()[2].objects[0].id, ObjectId(31));
    assert_eq!(exact.slides()[2].objects[1].id, ObjectId(32));
    // IDs touched by the failed batch remain free for unrelated roles.
    assert_eq!(
        apply(
            &mut p,
            &[Edit::AddSlide {
                index: 2,
                slide: slide(32, vec![object(30), object(31)]),
            }],
        ),
        Ok(RevisionId(4))
    );
}

#[test]
fn second_duplicate_over_capacity_rolls_back_first_duplicate() {
    let mut p = populated();
    let batch = [
        Edit::DuplicateSlide {
            slide: ObjectId(10),
            index: 2,
            id: ObjectId(30),
            object_ids: vec![ObjectId(31)],
        },
        Edit::DuplicateSlide {
            slide: ObjectId(30),
            index: 3,
            id: ObjectId(40),
            object_ids: vec![ObjectId(41)],
        },
    ];
    let mut twice = p.clone();
    apply(&mut twice, &batch).unwrap();
    let twice_bytes = native::encode(&twice, Limits::default()).unwrap().len();
    let bytes = |max_bytes| Limits {
        max_bytes,
        max_text_bytes: 128,
        ..Limits::default()
    };
    let before = p.clone();
    assert_eq!(
        p.apply(p.revision(), &batch, bytes(twice_bytes - 1)),
        Err(Error::LimitExceeded)
    );
    assert_eq!(p, before);
    let mut exact = p.clone();
    assert_eq!(
        exact.apply(exact.revision(), &batch, bytes(twice_bytes)),
        Ok(RevisionId(3))
    );
    assert_eq!(exact, twice);
    assert_eq!(
        apply(
            &mut p,
            &[Edit::AddSlide {
                index: 2,
                slide: slide(41, vec![object(40), object(31), object(30)]),
            }],
        ),
        Ok(RevisionId(3))
    );
}

#[test]
fn duplicate_over_slide_limit_after_earlier_add_rolls_back_both() {
    let mut p = populated();
    let before = p.clone();
    let batch = [
        Edit::AddSlide {
            index: 2,
            slide: slide(30, vec![object(31)]),
        },
        Edit::DuplicateSlide {
            slide: ObjectId(30),
            index: 3,
            id: ObjectId(40),
            object_ids: vec![ObjectId(41)],
        },
    ];
    let slides = |max_slides| Limits {
        max_slides,
        ..Limits::default()
    };
    assert_eq!(
        p.apply(p.revision(), &batch, slides(3)),
        Err(Error::LimitExceeded)
    );
    assert_eq!(p, before);
    let mut exact = p.clone();
    assert_eq!(
        exact.apply(exact.revision(), &batch, slides(4)),
        Ok(RevisionId(3))
    );
    assert_eq!(exact.slides()[3].objects[0].id, ObjectId(41));
    assert_eq!(
        apply(
            &mut p,
            &[Edit::AddSlide {
                index: 2,
                slide: slide(41, vec![object(40), object(31), object(30)]),
            }],
        ),
        Ok(RevisionId(3))
    );
}

fn duplicate_delete_cycles(count: usize) -> Vec<Edit> {
    (0..count as u64)
        .flat_map(|i| {
            let id = 30 + 2 * i;
            [
                Edit::DuplicateSlide {
                    slide: ObjectId(10),
                    index: 2,
                    id: ObjectId(id),
                    object_ids: vec![ObjectId(id + 1)],
                },
                Edit::DeleteSlide(ObjectId(id)),
            ]
        })
        .collect()
}

#[test]
fn copy_budget_accepts_last_fitting_cycle_and_rejects_next_without_id_leak() {
    let base = populated();
    let base_bytes = native::encode(&base, Limits::default()).unwrap().len();
    let mut once = base.clone();
    apply(&mut once, &duplicate_delete_cycles(1)[..1]).unwrap();
    // Copied slide size, excluding its two 8-byte issued-ID tombstones.
    let slide_bytes = native::encode(&once, Limits::default()).unwrap().len() - base_bytes - 16;
    // Smallest n whose cumulative copies exceed base + 16n + slide, which is
    // also the n-th cycle's peak snapshot; n - 1 cycles then fit both budgets.
    const MAX_BOUNDARY_CYCLES: usize = 32;
    let mut n = 2usize;
    while (n - 1) * slide_bytes <= base_bytes + 16 * n {
        assert!(
            n < MAX_BOUNDARY_CYCLES,
            "copy-budget boundary not found within {MAX_BOUNDARY_CYCLES} cycles"
        );
        n += 1;
    }
    let limits = Limits {
        max_bytes: base_bytes + 16 * n + slide_bytes,
        max_text_bytes: 128,
        ..Limits::default()
    };
    let mut p = base.clone();
    assert_eq!(
        p.apply(p.revision(), &duplicate_delete_cycles(n), limits),
        Err(Error::LimitExceeded)
    );
    assert_eq!(p, base);
    assert_eq!(
        p.apply(p.revision(), &duplicate_delete_cycles(n - 1), limits),
        Ok(RevisionId(3))
    );
    assert_eq!(p.slides(), base.slides());
    let last = 30 + 2 * (n as u64 - 1);
    assert_eq!(
        apply(
            &mut p,
            &[Edit::AddSlide {
                index: 2,
                slide: slide(last, vec![object(last + 1)]),
            }],
        ),
        Ok(RevisionId(4))
    );
}
