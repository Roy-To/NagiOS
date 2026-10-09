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
