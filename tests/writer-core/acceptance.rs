use nagi_history::activity::{ActorId, ActorKind};
use nagi_writer_core::{adapters::*, formats::*, *};
fn actor() -> Actor {
    Actor::new(ActorId(10), ActorKind::User)
}
fn provenance() -> Provenance {
    Provenance {
        actor: actor(),
        source: "host-acceptance".into(),
        time: 123,
    }
}
fn message(text: &str) -> Message {
    Message {
        author: actor(),
        time: 123,
        text: text.into(),
    }
}
fn engine() -> Engine {
    let mut e = Engine::open(
        Document::new(DocumentId(ObjectId(1)), RevisionId(1), "Report 報告".into()).unwrap(),
        Limits::default(),
    )
    .unwrap();
    edit(
        &mut e,
        Operation::InsertSection {
            id: ObjectId(2),
            index: 0,
            title: String::new(),
        },
    );
    e
}
fn edit(e: &mut Engine, op: Operation) {
    e.apply(e.document().revision, &[op], provenance()).unwrap();
}
fn insert(e: &mut Engine, id: u64, kind: BlockKind) {
    let index = e.document().sections[0].blocks.len();
    edit(
        e,
        Operation::InsertBlock {
            section: ObjectId(2),
            index,
            block: Block::new(ObjectId(id), kind),
        },
    );
}
fn paragraph(e: &mut Engine, id: u64, text: &str) {
    insert(e, id, BlockKind::Paragraph(text.into()));
}
fn kinds(doc: &Document) -> Vec<BlockKind> {
    doc.sections
        .iter()
        .flat_map(|s| &s.blocks)
        .map(|b| b.kind.clone())
        .collect()
}
fn import_text(format: Format, text: &str) -> ImportResult {
    let mut id = 1;
    import(
        format,
        text,
        DocumentId(ObjectId(100)),
        || {
            id += 1;
            Ok(ObjectId(id))
        },
        Limits::default(),
        provenance(),
    )
    .unwrap()
}
#[test]
fn h01_create_insert_move_delete_and_read_without_ai() {
    let mut e = engine();
    paragraph(&mut e, 3, "a");
    paragraph(&mut e, 4, "b");
    edit(
        &mut e,
        Operation::MoveObject {
            object: ObjectId(4),
            section: Some(ObjectId(2)),
            index: 0,
        },
    );
    assert_eq!(e.document().sections[0].blocks[0].id, ObjectId(4));
    edit(
        &mut e,
        Operation::DeleteObject {
            object: ObjectId(3),
        },
    );
    assert!(e.document().block(ObjectId(3)).is_none());
    assert_eq!(
        e.document().block(ObjectId(4)).unwrap().kind.text(),
        Some("b")
    );
}
#[test]
fn h02_all_block_types_are_retained() {
    let mut e = engine();
    let reference = Reference {
        resource: ObjectId(900),
        revision: Some(RevisionId(7)),
        object: Some(ObjectId(901)),
        label: "source".into(),
        locator: Some("https://example.invalid/untrusted".into()),
    };
    let blocks = vec![
        BlockKind::Paragraph("p".into()),
        BlockKind::Heading {
            level: 2,
            text: "h".into(),
        },
        BlockKind::List {
            ordered: false,
            items: vec!["item".into()],
        },
        BlockKind::Table(Table::new(vec![vec!["cell".into()]]).unwrap()),
        BlockKind::Quote("q".into()),
        BlockKind::Code {
            language: "rust".into(),
            text: "code".into(),
        },
        BlockKind::Citation(reference.clone()),
        BlockKind::LinkedReference(reference.clone()),
        BlockKind::Figure {
            alt: "image".into(),
            source: Some(reference),
        },
    ];
    for (n, block) in blocks.iter().enumerate() {
        insert(&mut e, 10 + n as u64, block.clone());
    }
    assert_eq!(kinds(e.document()), blocks);
}
#[test]
fn h03_ids_stay_stable_across_changes_and_native_reopen() {
    let mut e = engine();
    paragraph(&mut e, 3, "日本語");
    paragraph(&mut e, 4, "untouched");
    edit(
        &mut e,
        Operation::ReplaceText {
            object: ObjectId(3),
            range: 0..3,
            text: "英".into(),
        },
    );
    edit(
        &mut e,
        Operation::ApplyStyle {
            object: ObjectId(3),
            style: "Title".into(),
        },
    );
    edit(
        &mut e,
        Operation::MoveObject {
            object: ObjectId(3),
            section: Some(ObjectId(2)),
            index: 1,
        },
    );
    let snapshot = e.document().clone();
    let reopened = Engine::open(snapshot.clone(), Limits::default()).unwrap();
    assert_eq!(reopened.document(), &snapshot);
    assert_eq!(
        snapshot.block(ObjectId(4)).unwrap().kind.text(),
        Some("untouched")
    );
    assert_eq!(snapshot.id, DocumentId(ObjectId(1)));
}
#[test]
fn h03_duplicates_tombstones_and_parent_cycles_are_rejected_atomically() {
    let mut e = engine();
    paragraph(&mut e, 3, "a");
    let before = e.document().clone();
    for op in [
        Operation::MoveObject {
            object: ObjectId(2),
            section: Some(ObjectId(2)),
            index: 0,
        },
        Operation::InsertBlock {
            section: ObjectId(3),
            index: 0,
            block: Block::new(ObjectId(4), BlockKind::Paragraph("x".into())),
        },
        Operation::InsertSection {
            id: ObjectId(3),
            index: 0,
            title: "dup".into(),
        },
    ] {
        assert!(e.apply(before.revision, &[op], provenance()).is_err());
        assert_eq!(e.document(), &before);
    }
    edit(
        &mut e,
        Operation::DeleteObject {
            object: ObjectId(3),
        },
    );
    assert_eq!(
        e.apply(
            e.document().revision,
            &[Operation::InsertBlock {
                section: ObjectId(2),
                index: 0,
                block: Block::new(ObjectId(3), BlockKind::Paragraph("reuse".into()))
            }],
            provenance()
        ),
        Err(Error::DuplicateId(ObjectId(3)))
    );
}
#[test]
fn h04_styles_inherit_and_outline_follows_structural_order() {
    let mut e = engine();
    insert(
        &mut e,
        3,
        BlockKind::Heading {
            level: 1,
            text: "一".into(),
        },
    );
    insert(
        &mut e,
        4,
        BlockKind::Heading {
            level: 3,
            text: "三".into(),
        },
    );
    edit(
        &mut e,
        Operation::DefineStyle {
            name: "Body".into(),
            style: Style {
                parent: None,
                formatting: Formatting {
                    bold: Some(true),
                    italic: None,
                    size_points: Some(12),
                },
            },
        },
    );
    edit(
        &mut e,
        Operation::DefineStyle {
            name: "Custom".into(),
            style: Style {
                parent: Some("Body".into()),
                formatting: Formatting {
                    bold: Some(false),
                    italic: Some(true),
                    size_points: None,
                },
            },
        },
    );
    edit(
        &mut e,
        Operation::ApplyStyle {
            object: ObjectId(3),
            style: "Custom".into(),
        },
    );
    assert_eq!(
        e.document().effective_style("Custom").unwrap(),
        Formatting {
            bold: Some(false),
            italic: Some(true),
            size_points: Some(12)
        }
    );
    edit(
        &mut e,
        Operation::MoveObject {
            object: ObjectId(4),
            section: Some(ObjectId(2)),
            index: 0,
        },
    );
    assert_eq!(
        e.document()
            .outline()
            .iter()
            .map(|i| (i.id, i.level))
            .collect::<Vec<_>>(),
        vec![(ObjectId(4), 3), (ObjectId(3), 1)]
    );
    let before = e.document().clone();
    assert_eq!(
        e.apply(
            before.revision,
            &[Operation::DefineStyle {
                name: "Body".into(),
                style: Style {
                    parent: Some("Custom".into()),
                    formatting: Formatting::default()
                }
            }],
            provenance()
        ),
        Err(Error::StyleCycle)
    );
    assert_eq!(e.document(), &before);
}
#[test]
fn h05_immutable_history_conflicts_and_failed_batches() {
    let mut e = engine();
    let old = e.document().clone();
    paragraph(&mut e, 3, "a");
    assert_eq!(e.history()[1].document, old);
    assert!(matches!(
        e.apply(
            old.revision,
            &[Operation::DeleteObject {
                object: ObjectId(3)
            }],
            provenance()
        ),
        Err(Error::Conflict { .. })
    ));
    let before = e.document().clone();
    let count = e.history().len();
    assert!(e
        .apply(
            before.revision,
            &[
                Operation::ReplaceText {
                    object: ObjectId(3),
                    range: 0..1,
                    text: "b".into()
                },
                Operation::DeleteObject {
                    object: ObjectId(999)
                }
            ],
            provenance()
        )
        .is_err());
    assert_eq!(e.document(), &before);
    assert_eq!(e.history().len(), count);
}
#[test]
fn h06_changesets_cover_add_delete_move_format_and_provenance() {
    let mut e = engine();
    paragraph(&mut e, 3, "a");
    edit(
        &mut e,
        Operation::MoveObject {
            object: ObjectId(3),
            section: Some(ObjectId(2)),
            index: 0,
        },
    );
    edit(
        &mut e,
        Operation::ApplyStyle {
            object: ObjectId(3),
            style: "Title".into(),
        },
    );
    edit(
        &mut e,
        Operation::DeleteObject {
            object: ObjectId(3),
        },
    );
    let sets: Vec<_> = e
        .history()
        .iter()
        .filter_map(|r| r.changes.as_ref())
        .collect();
    assert_eq!(
        sets.iter().map(|s| s.changes[0].kind).collect::<Vec<_>>(),
        vec![
            ChangeKind::Add,
            ChangeKind::Add,
            ChangeKind::Move,
            ChangeKind::Format,
            ChangeKind::Delete
        ]
    );
    assert!(sets.iter().all(|s| s.provenance == provenance()
        && s.to.0 == s.from.0 + 1
        && s.document == e.document().id));
}
#[test]
fn h07_comments_reply_resolve_and_deleted_anchor_preservation() {
    let mut e = engine();
    paragraph(&mut e, 3, "a");
    edit(
        &mut e,
        Operation::AddComment {
            id: ObjectId(4),
            object: ObjectId(3),
            message: message("質問"),
        },
    );
    edit(
        &mut e,
        Operation::Reply {
            comment: ObjectId(4),
            message: message("answer"),
        },
    );
    edit(
        &mut e,
        Operation::Resolve {
            comment: ObjectId(4),
            resolved: true,
        },
    );
    edit(
        &mut e,
        Operation::DeleteObject {
            object: ObjectId(3),
        },
    );
    let c = &e.document().comments[0];
    assert_eq!(c.object, ObjectId(3));
    assert_eq!(c.messages.len(), 2);
    assert!(c.resolved);
    assert!(e
        .apply(
            e.document().revision,
            &[Operation::AddComment {
                id: ObjectId(5),
                object: ObjectId(3),
                message: message("orphan")
            }],
            provenance()
        )
        .is_err());
}
#[test]
fn h06_track_changes_preview_accept_reject_and_stale_review() {
    let mut e = engine();
    paragraph(&mut e, 3, "a");
    let operation = Operation::ReplaceText {
        object: ObjectId(3),
        range: 0..1,
        text: "提案".into(),
    };
    edit(
        &mut e,
        Operation::Suggest {
            id: ObjectId(4),
            operations: vec![operation.clone()],
        },
    );
    assert_eq!(
        e.document().block(ObjectId(3)).unwrap().kind.text(),
        Some("a")
    );
    assert_eq!(
        e.preview_suggestion(ObjectId(4))
            .unwrap()
            .block(ObjectId(3))
            .unwrap()
            .kind
            .text(),
        Some("提案")
    );
    edit(
        &mut e,
        Operation::Accept {
            change: ObjectId(4),
        },
    );
    assert_eq!(
        e.document().block(ObjectId(3)).unwrap().kind.text(),
        Some("提案")
    );
    assert_eq!(
        e.document().tracked_changes[0].status,
        ReviewStatus::Accepted
    );
    edit(
        &mut e,
        Operation::Suggest {
            id: ObjectId(5),
            operations: vec![Operation::DeleteObject {
                object: ObjectId(3),
            }],
        },
    );
    edit(
        &mut e,
        Operation::Reject {
            change: ObjectId(5),
        },
    );
    assert_eq!(
        e.document().tracked_changes[1].status,
        ReviewStatus::Rejected
    );
    assert_eq!(
        e.document().block(ObjectId(3)).unwrap().kind.text(),
        Some("提案")
    );
    edit(
        &mut e,
        Operation::Suggest {
            id: ObjectId(6),
            operations: vec![Operation::DeleteObject {
                object: ObjectId(3),
            }],
        },
    );
    paragraph(&mut e, 7, "concurrent");
    assert!(matches!(
        e.apply(
            e.document().revision,
            &[Operation::Accept {
                change: ObjectId(6)
            }],
            provenance()
        ),
        Err(Error::Conflict { .. })
    ));
    assert!(matches!(
        e.preview_suggestion(ObjectId(6)),
        Err(Error::Conflict { .. })
    ));
}
#[test]
fn h06_preview_has_no_effects_and_review_batch_is_rejected() {
    let e = engine();
    let before = e.document().clone();
    let op = Operation::InsertBlock {
        section: ObjectId(2),
        index: 0,
        block: Block::new(ObjectId(3), BlockKind::Paragraph("p".into())),
    };
    let preview = e
        .preview(before.revision, std::slice::from_ref(&op), &provenance())
        .unwrap();
    assert!(preview.document.block(ObjectId(3)).is_some());
    assert_eq!(e.document(), &before);
    assert_eq!(
        e.preview(
            before.revision,
            &[
                op,
                Operation::Suggest {
                    id: ObjectId(4),
                    operations: vec![Operation::DeleteObject {
                        object: ObjectId(3)
                    }]
                }
            ],
            &provenance()
        ),
        Err(Error::InvalidReview)
    );
}
#[test]
fn h08_bilingual_markdown_subset_roundtrips() {
    let imported = import_text(Format::Markdown, include_str!("fixtures/bilingual.md"));
    assert!(imported.warnings.is_empty());
    let output = export(Format::Markdown, &imported.document, Limits::default()).unwrap();
    assert_eq!(
        output.warnings.iter().map(|w| &w.kind).collect::<Vec<_>>(),
        vec![&WarningKind::NativeMetadata]
    );
    let reopened = import_text(Format::Markdown, &output.text);
    assert!(reopened.warnings.is_empty());
    assert_eq!(kinds(&reopened.document), kinds(&imported.document));
}
#[test]
fn h08_plaintext_is_literal_and_normalizes_newlines() {
    let input = "# literal\r\n日本語\r\n\r\n<script> *text*";
    let doc = import_text(Format::PlainText, input);
    assert!(doc.warnings.is_empty());
    let output = export(Format::PlainText, &doc.document, Limits::default()).unwrap();
    assert_eq!(output.text, "# literal\n日本語\n\n<script> *text*");
    assert_eq!(
        kinds(&doc.document),
        kinds(&import_text(Format::PlainText, &output.text).document)
    );
}
#[test]
fn h09_unsupported_markdown_is_preserved_with_line_warnings() {
    let input =
        "![image](https://invalid)\n\n<table>秘密</table>\n\n|a|b|\n|-|-|\n\n**bold** [link](x)";
    let doc = import_text(Format::Markdown, input);
    assert_eq!(doc.warnings.len(), 5);
    assert!(doc.warnings.iter().all(|w| w.line.is_some()));
    let texts: Vec<_> = kinds(&doc.document)
        .iter()
        .map(|k| k.text().unwrap().to_string())
        .collect();
    assert_eq!(texts.join("\n\n"), input);
    let output = export(Format::Markdown, &doc.document, Limits::default()).unwrap();
    assert_eq!(
        kinds(&doc.document),
        kinds(&import_text(Format::Markdown, &output.text).document)
    );
}
#[test]
fn h09_unclosed_fence_keeps_all_content_and_warns() {
    let doc = import_text(Format::Markdown, "```rust\n秘密\nlast");
    assert_eq!(doc.warnings[0].kind, WarningKind::UnclosedFence);
    assert_eq!(
        doc.document.sections[0].blocks[0].kind.text(),
        Some("```rust\n秘密\nlast")
    );
}
#[test]
fn h09_export_table_and_reference_reports_loss_before_write() {
    let mut e = engine();
    insert(
        &mut e,
        3,
        BlockKind::Table(Table::new(vec![vec!["秘密".into(), "other".into()]]).unwrap()),
    );
    insert(
        &mut e,
        4,
        BlockKind::Citation(Reference {
            resource: ObjectId(99),
            revision: Some(RevisionId(7)),
            object: Some(ObjectId(8)),
            label: "source".into(),
            locator: Some("https://invalid".into()),
        }),
    );
    let output = export(Format::Markdown, e.document(), Limits::default()).unwrap();
    assert_eq!(
        output
            .warnings
            .iter()
            .filter(|w| w.kind == WarningKind::UnsupportedBlock)
            .count(),
        2
    );
    assert!(
        output.text.contains("秘密")
            && output.text.contains("source")
            && output.text.contains("99")
    );
}
#[test]
fn h10_unicode_byte_boundaries_and_overflow_ranges_do_not_panic() {
    let mut e = engine();
    paragraph(&mut e, 3, "日本🌊e\u{301}");
    let before = e.document().clone();
    for range in [1..3, 0..usize::MAX, std::ops::Range { start: 8, end: 7 }] {
        assert_eq!(
            e.apply(
                before.revision,
                &[Operation::ReplaceText {
                    object: ObjectId(3),
                    range,
                    text: "x".into()
                }],
                provenance()
            ),
            Err(Error::InvalidTextRange)
        );
        assert_eq!(e.document(), &before);
    }
    edit(
        &mut e,
        Operation::ReplaceText {
            object: ObjectId(3),
            range: 6..10,
            text: "波".into(),
        },
    );
    assert_eq!(
        e.document().block(ObjectId(3)).unwrap().kind.text(),
        Some("日本波e\u{301}")
    );
}
#[test]
fn h10_limits_and_corrupt_native_inputs_are_rejected() {
    let e = engine();
    let mut doc = e.document().clone();
    doc.sections[0].id = doc.id.0;
    assert!(Engine::open(doc, Limits::default()).is_err());
    let limits = Limits {
        max_bytes: 10,
        ..Limits::default()
    };
    assert_eq!(
        import(
            Format::Markdown,
            &"a".repeat(11),
            DocumentId(ObjectId(1)),
            || Ok(ObjectId(2)),
            limits,
            provenance()
        ),
        Err(Error::LimitExceeded)
    );
    let limits = Limits {
        max_revisions: 1,
        ..Limits::default()
    };
    let mut limited = Engine::open(e.document().clone(), limits).unwrap();
    assert_eq!(
        limited.apply(
            limited.document().revision,
            &[Operation::SetMetadata(Metadata::default())],
            provenance()
        ),
        Err(Error::LimitExceeded)
    );
    assert!(Document::new(DocumentId(ObjectId(0)), RevisionId(1), String::new()).is_err());
    assert!(Document::new(DocumentId(ObjectId(1)), RevisionId(u64::MAX), String::new()).is_err());
}
#[test]
fn h02_table_operations_are_transactional_and_stable() {
    let mut e = engine();
    let mut t = Table::new(vec![vec!["a".into(), "b".into()]]).unwrap();
    insert(&mut e, 3, BlockKind::Table(t.clone()));
    t.insert_row(1, vec!["c".into(), "d".into()]).unwrap();
    t.insert_column(1).unwrap();
    t.set_cell(0, 1, "日本語".into()).unwrap();
    t.delete_row(1).unwrap();
    t.delete_column(2).unwrap();
    edit(
        &mut e,
        Operation::ReplaceTable {
            object: ObjectId(3),
            table: t.clone(),
        },
    );
    assert_eq!(
        e.document().block(ObjectId(3)).unwrap().kind,
        BlockKind::Table(t)
    );
    let before = e.document().clone();
    assert_eq!(
        e.apply(
            before.revision,
            &[Operation::ReplaceTable {
                object: ObjectId(3),
                table: Table { rows: vec![vec![]] }
            }],
            provenance()
        ),
        Err(Error::InvalidStructure)
    );
    assert_eq!(e.document(), &before);
}
#[test]
fn h11_absent_adapters_never_claim_persistence_activity_or_restore() {
    let mut e = engine();
    paragraph(&mut e, 3, "a");
    let mut absent = Unavailable;
    assert_eq!(
        absent.open(e.document().id, actor()),
        Err(Error::Unavailable)
    );
    assert_eq!(
        absent.create(e.document(), actor()),
        Err(Error::Unavailable)
    );
    assert_eq!(
        absent.save(RevisionId(1), e.document(), actor()),
        Err(Error::Unavailable)
    );
    assert_eq!(
        absent.checkpoint(e.document(), actor()),
        Err(Error::Unavailable)
    );
    assert_eq!(
        absent.record(&activity_record(
            e.history().last().unwrap().changes.as_ref().unwrap()
        )),
        Err(Error::Unavailable)
    );
}
struct Permission(bool);
impl PermissionBoundary for Permission {
    fn authorize(&self, _: Actor, _: DocumentId, _: Access) -> Result<(), Error> {
        if self.0 {
            Ok(())
        } else {
            Err(Error::PermissionDenied)
        }
    }
}
#[test]
fn h11_search_projection_requires_permission_and_excludes_comments() {
    let mut e = engine();
    paragraph(&mut e, 3, "visible");
    edit(
        &mut e,
        Operation::AddComment {
            id: ObjectId(4),
            object: ObjectId(3),
            message: message("secret comment"),
        },
    );
    assert_eq!(
        index_items(e.document(), actor(), &Permission(false)),
        Err(Error::PermissionDenied)
    );
    let items = index_items(e.document(), actor(), &Permission(true)).unwrap();
    assert_eq!(items.len(), 2);
    assert_eq!(items[1].object, ObjectId(3));
    assert_eq!(items[1].revision, e.document().revision);
    assert!(!items.iter().any(|i| i.text.contains("secret")));
}
#[test]
fn h03_platform_identity_compatibility_fixture() {
    let id: nagi_model::ObjectId = ObjectId(0x1234);
    let document = DocumentId(id);
    let revision: nagi_history::activity::RevisionId = RevisionId(7);
    assert_eq!(document.0, id);
    assert_eq!(revision, RevisionId::new(7).unwrap());
    assert_eq!(CONTRACT_VERSION, 1);
}
#[test]
fn h08_fenced_code_and_literal_punctuation_roundtrip() {
    let mut e = engine();
    paragraph(&mut e, 3, "# literal\n- no list\n[link](x) \\ end");
    insert(
        &mut e,
        4,
        BlockKind::Code {
            language: "rust".into(),
            text: "```\n~ 日本語\n````".into(),
        },
    );
    let output = export(Format::Markdown, e.document(), Limits::default()).unwrap();
    let imported = import_text(Format::Markdown, &output.text);
    assert!(imported.warnings.is_empty());
    assert_eq!(kinds(&imported.document), kinds(e.document()));
}

#[test]
fn h10_allocator_collisions_and_object_operation_limits_are_explicit() {
    assert!(matches!(
        import(
            Format::PlainText,
            "one",
            DocumentId(ObjectId(1)),
            || Ok(ObjectId(2)),
            Limits::default(),
            provenance()
        ),
        Err(Error::DuplicateId(ObjectId(2)))
    ));
    let limits = Limits {
        max_objects: 3,
        ..Limits::default()
    };
    let mut id = 1;
    assert_eq!(
        import(
            Format::Markdown,
            "a\n\nb",
            DocumentId(ObjectId(100)),
            || {
                id += 1;
                Ok(ObjectId(id))
            },
            limits,
            provenance()
        ),
        Err(Error::LimitExceeded)
    );
    let mut e = engine();
    paragraph(&mut e, 3, "a");
    let limits = Limits {
        max_operations: 1,
        ..Limits::default()
    };
    let e = Engine::open(e.document().clone(), limits).unwrap();
    assert_eq!(
        e.preview(
            e.document().revision,
            &[
                Operation::DeleteObject {
                    object: ObjectId(3)
                },
                Operation::DeleteObject {
                    object: ObjectId(2)
                }
            ],
            &provenance()
        ),
        Err(Error::LimitExceeded)
    );
}
#[test]
fn h10_large_bounded_input_and_empty_structural_elements() {
    let input = "日English🌊".repeat(10000);
    let doc = import_text(Format::PlainText, &input);
    assert_eq!(
        doc.document.sections[0].blocks[0].kind.text(),
        Some(input.as_str())
    );
    let mut e = engine();
    let before = e.document().clone();
    let op = Operation::InsertBlock {
        section: ObjectId(2),
        index: 0,
        block: Block::new(
            ObjectId(3),
            BlockKind::List {
                ordered: false,
                items: vec![String::new(); Limits::default().max_objects + 1],
            },
        ),
    };
    assert_eq!(
        e.apply(before.revision, &[op], provenance()),
        Err(Error::LimitExceeded)
    );
    assert_eq!(e.document(), &before);
    let table = Table {
        rows: vec![vec![String::new(); 129]; 129],
    };
    assert_eq!(
        table.validate(Limits::default().max_table_cells),
        Err(Error::LimitExceeded)
    );
}
#[test]
fn h09_plaintext_whitespace_structure_loss_is_reported() {
    let imported = import_text(Format::PlainText, "\nfirst\n\n\nsecond\n");
    assert_eq!(imported.warnings[0].kind, WarningKind::StructuralLoss);
    let plain = export(Format::PlainText, &imported.document, Limits::default()).unwrap();
    assert_eq!(plain.text, "first\n\nsecond");
}
#[test]
fn h10_every_utf8_boundary_is_checked_against_scalar_positions() {
    let text = "日A🌊e\u{301}";
    for start in 0..=text.len() + 1 {
        for end in 0..=text.len() + 1 {
            let mut e = engine();
            paragraph(&mut e, 3, text);
            let before = e.document().clone();
            let result = e.apply(
                before.revision,
                &[Operation::ReplaceText {
                    object: ObjectId(3),
                    range: start..end,
                    text: "置換".into(),
                }],
                provenance(),
            );
            if start <= end && text.is_char_boundary(start) && text.is_char_boundary(end) {
                assert!(result.is_ok());
                let expected = format!("{}置換{}", &text[..start], &text[end..]);
                assert_eq!(
                    e.document().block(ObjectId(3)).unwrap().kind.text(),
                    Some(expected.as_str())
                );
            } else {
                assert_eq!(result, Err(Error::InvalidTextRange));
                assert_eq!(e.document(), &before);
            }
        }
    }
}
#[test]
fn h11_activity_projection_contains_only_identity_and_categories() {
    let mut e = engine();
    paragraph(&mut e, 3, "secret document text");
    let projection = activity_record(e.history().last().unwrap().changes.as_ref().unwrap());
    assert_eq!(
        projection.changes,
        vec![(ChangeKind::Add, Some(ObjectId(3)))]
    );
    assert_eq!(projection.actor, actor());
    assert_eq!(projection.document, e.document().id);
    assert!(!format!("{projection:?}").contains("secret document text"));
}
#[test]
fn h03_sections_move_with_children_and_invalid_positions_leave_snapshot_intact() {
    let mut e = engine();
    paragraph(&mut e, 3, "child");
    edit(
        &mut e,
        Operation::InsertSection {
            id: ObjectId(4),
            index: 1,
            title: "second".into(),
        },
    );
    edit(
        &mut e,
        Operation::MoveObject {
            object: ObjectId(2),
            section: None,
            index: 1,
        },
    );
    assert_eq!(e.document().sections[1].blocks[0].id, ObjectId(3));
    edit(
        &mut e,
        Operation::MoveObject {
            object: ObjectId(3),
            section: Some(ObjectId(4)),
            index: 0,
        },
    );
    assert_eq!(e.document().sections[0].blocks[0].id, ObjectId(3));
    let before = e.document().clone();
    assert_eq!(
        e.apply(
            before.revision,
            &[Operation::MoveObject {
                object: ObjectId(3),
                section: Some(ObjectId(4)),
                index: usize::MAX
            }],
            provenance()
        ),
        Err(Error::InvalidIndex)
    );
    assert_eq!(e.document(), &before);
}
#[test]
fn h06_review_nested_operations_and_duplicate_decisions_fail() {
    let mut e = engine();
    paragraph(&mut e, 3, "a");
    let before = e.document().clone();
    assert_eq!(
        e.apply(
            before.revision,
            &[Operation::Suggest {
                id: ObjectId(4),
                operations: vec![Operation::Accept {
                    change: ObjectId(4)
                }]
            }],
            provenance()
        ),
        Err(Error::InvalidReview)
    );
    assert_eq!(e.document(), &before);
    edit(
        &mut e,
        Operation::Suggest {
            id: ObjectId(4),
            operations: vec![Operation::DeleteObject {
                object: ObjectId(3),
            }],
        },
    );
    edit(
        &mut e,
        Operation::Accept {
            change: ObjectId(4),
        },
    );
    assert!(e.document().block(ObjectId(3)).is_none());
    assert_eq!(
        e.apply(
            e.document().revision,
            &[Operation::Accept {
                change: ObjectId(4)
            }],
            provenance()
        ),
        Err(Error::InvalidReview)
    );
}

#[test]
fn h06_proposal_identity_cannot_collide_with_proposed_object() {
    let mut e = engine();
    let before = e.document().clone();
    let op = Operation::Suggest {
        id: ObjectId(3),
        operations: vec![Operation::InsertBlock {
            section: ObjectId(2),
            index: 0,
            block: Block::new(ObjectId(3), BlockKind::Paragraph("p".into())),
        }],
    };
    assert_eq!(
        e.apply(before.revision, &[op], provenance()),
        Err(Error::DuplicateId(ObjectId(3)))
    );
    assert_eq!(e.document(), &before);
    edit(
        &mut e,
        Operation::Suggest {
            id: ObjectId(4),
            operations: vec![Operation::InsertBlock {
                section: ObjectId(2),
                index: 0,
                block: Block::new(ObjectId(3), BlockKind::Paragraph("p".into())),
            }],
        },
    );
    edit(
        &mut e,
        Operation::Accept {
            change: ObjectId(4),
        },
    );
    assert_eq!(
        e.document().block(ObjectId(3)).unwrap().kind.text(),
        Some("p")
    );
}

#[test]
fn h10_review_and_batch_budgets_include_empty_table_cells_before_preview() {
    let mut e = engine();
    insert(
        &mut e,
        3,
        BlockKind::Table(Table::new(vec![vec![String::new()]]).unwrap()),
    );
    let before = e.document().clone();
    // Each valid table has 16K empty cells. Their structure still consumes
    // budget even though the text payload is zero bytes.
    let table = Table {
        rows: vec![vec![String::new(); 128]; 128],
    };
    let operations = vec![
        Operation::ReplaceTable {
            object: ObjectId(3),
            table
        };
        3
    ];
    assert_eq!(
        e.preview(before.revision, &operations, &provenance()),
        Err(Error::LimitExceeded)
    );
    assert_eq!(
        e.apply(
            before.revision,
            &[Operation::Suggest {
                id: ObjectId(4),
                operations
            }],
            provenance()
        ),
        Err(Error::LimitExceeded)
    );
    assert_eq!(e.document(), &before);
}
fn md_roundtrip(input: &str) -> (ImportResult, String, ImportResult) {
    let imported = import_text(Format::Markdown, input);
    let output = export(Format::Markdown, &imported.document, Limits::default()).unwrap();
    let reopened = import_text(Format::Markdown, &output.text);
    assert_eq!(
        kinds(&reopened.document),
        kinds(&imported.document),
        "import->export->import changed structure for {input:?} via {:?}",
        output.text
    );
    assert!(reopened.warnings.is_empty(), "{:?}", reopened.warnings);
    (imported, output.text, reopened)
}
fn warned_lines(doc: &ImportResult) -> Vec<usize> {
    assert!(doc
        .warnings
        .iter()
        .all(|w| w.kind == WarningKind::UnsupportedSyntax && w.object.is_none()));
    doc.warnings.iter().map(|w| w.line.unwrap()).collect()
}
#[test]
fn h09_ordered_list_non_one_start_is_kept_literally_not_renumbered() {
    // Regression: "9. item" used to import as List{ordered} and export "1. item".
    let (doc, text, reopened) = md_roundtrip("9. item");
    assert_eq!(
        kinds(&doc.document),
        vec![BlockKind::Paragraph("9. item".into())]
    );
    assert_eq!(warned_lines(&doc), vec![1]);
    assert_eq!(text, "9\\. item");
    assert_ne!(text, "1. item");
    assert_eq!(
        kinds(&reopened.document),
        vec![BlockKind::Paragraph("9. item".into())]
    );
    for input in ["0. zero", "01. leading zero", "2. b\n3. c"] {
        let doc = import_text(Format::Markdown, input);
        assert_eq!(
            kinds(&doc.document),
            vec![BlockKind::Paragraph(input.into())]
        );
        assert_eq!(
            warned_lines(&doc),
            (1..=input.lines().count()).collect::<Vec<_>>()
        );
    }
}
#[test]
fn h09_ordered_list_gaps_and_resumed_numbering_are_not_lost() {
    let (doc, text, _) = md_roundtrip("1. a\n2. b\n4. c");
    assert_eq!(
        kinds(&doc.document),
        vec![BlockKind::Paragraph("1. a\n2. b\n4. c".into())]
    );
    assert_eq!(warned_lines(&doc), vec![1, 2, 3]);
    assert!(text.contains("4\\. c"));
    // A run resumed after a blank line would otherwise restart at "1.".
    let (doc, text, _) = md_roundtrip("1. a\n\n2. b\n3. c");
    assert_eq!(
        kinds(&doc.document),
        vec![
            BlockKind::List {
                ordered: true,
                items: vec!["a".into()],
            },
            BlockKind::Paragraph("2. b\n3. c".into()),
        ]
    );
    assert_eq!(warned_lines(&doc), vec![3, 4]);
    assert_eq!(text, "1. a\n\n2\\. b\n3\\. c");
}
#[test]
fn h08_canonical_ordered_lists_still_roundtrip_exactly() {
    let input = (1..=12)
        .map(|n| format!("{n}. item {n}"))
        .collect::<Vec<_>>()
        .join("\n");
    let (doc, text, _) = md_roundtrip(&input);
    assert!(doc.warnings.is_empty());
    assert_eq!(
        kinds(&doc.document),
        vec![BlockKind::List {
            ordered: true,
            items: (1..=12).map(|n| format!("item {n}")).collect(),
        }]
    );
    assert_eq!(text, input);
    // Inline-unsupported items in a canonical list keep exact line numbers.
    let doc = import_text(Format::Markdown, "intro\n\n1. a\n2. **b**");
    assert_eq!(warned_lines(&doc), vec![4]);
}
#[test]
fn h10_huge_ordered_list_numbers_do_not_overflow_or_get_rewritten() {
    let huge = "9".repeat(400);
    for number in [
        u64::MAX.to_string(),
        (u128::from(u64::MAX) + 1).to_string(),
        usize::MAX.to_string(),
        huge,
    ] {
        let input = format!("{number}. 巨大\n1. next");
        let (doc, text, _) = md_roundtrip(&input);
        assert_eq!(
            kinds(&doc.document),
            vec![BlockKind::Paragraph(input.clone())]
        );
        assert_eq!(warned_lines(&doc), vec![1, 2]);
        assert!(text.starts_with(&format!("{number}\\. ")));
    }
}
#[test]
fn h09_mixed_ordered_unordered_runs_keep_every_number() {
    let input = "1. a\n- b\n2. c\n* d\n1. e\n2. f";
    let (doc, text, _) = md_roundtrip(input);
    assert_eq!(
        kinds(&doc.document),
        vec![
            BlockKind::List {
                ordered: true,
                items: vec!["a".into()],
            },
            BlockKind::List {
                ordered: false,
                items: vec!["b".into()],
            },
            BlockKind::Paragraph("2. c".into()),
            BlockKind::List {
                ordered: false,
                items: vec!["d".into()],
            },
            BlockKind::List {
                ordered: true,
                items: vec!["e".into(), "f".into()],
            },
        ]
    );
    assert_eq!(warned_lines(&doc), vec![3]);
    assert_eq!(text, "1. a\n\n- b\n\n2\\. c\n\n- d\n\n1. e\n2. f");
}
#[test]
fn h08_escaped_list_markers_are_text_not_lists() {
    let (doc, text, _) = md_roundtrip("9\\. not a list");
    assert!(doc.warnings.is_empty());
    assert_eq!(
        kinds(&doc.document),
        vec![BlockKind::Paragraph("9. not a list".into())]
    );
    assert_eq!(text, "9\\. not a list");
    let (doc, text, _) = md_roundtrip("- 9\\. inside item\n- \\- dash");
    assert!(doc.warnings.is_empty());
    assert_eq!(
        kinds(&doc.document),
        vec![BlockKind::List {
            ordered: false,
            items: vec!["9. inside item".into(), "- dash".into()],
        }]
    );
    assert_eq!(text, "- 9\\. inside item\n- \\- dash");
}
#[test]
fn h10_japanese_ordered_items_keep_numbers() {
    let (doc, text, _) = md_roundtrip("1. 日本語\n2. 項目🌊");
    assert!(doc.warnings.is_empty());
    assert_eq!(text, "1. 日本語\n2. 項目🌊");
    let (doc, text, _) = md_roundtrip("3. 第三項目\n4. 続き");
    assert_eq!(
        kinds(&doc.document),
        vec![BlockKind::Paragraph("3. 第三項目\n4. 続き".into())]
    );
    assert_eq!(warned_lines(&doc), vec![1, 2]);
    assert_eq!(text, "3\\. 第三項目\n4\\. 続き");
    // Full-width digits are not list markers in the subset.
    let (doc, _, _) = md_roundtrip("１. 全角");
    assert!(doc.warnings.is_empty());
    assert_eq!(
        kinds(&doc.document),
        vec![BlockKind::Paragraph("１. 全角".into())]
    );
}
#[test]
fn h09_import_export_import_never_drops_an_ordered_number() {
    for input in [
        "5. a",
        "1. a\n3. b",
        "intro\n7. x",
        "# 見出し\n\n10. 十\n11. 十一",
        "1. a\n\n1. b",
        "> quote\n\n42. answer **bold**",
    ] {
        let (doc, text, _) = md_roundtrip(input);
        for line in input.lines() {
            let n = line.bytes().take_while(u8::is_ascii_digit).count();
            if n > 0 && line[n..].starts_with(". ") {
                assert!(
                    text.contains(&line[..n]),
                    "number {} lost: {input:?} -> {text:?}",
                    &line[..n]
                );
            }
        }
        assert!(!kinds(&doc.document).is_empty());
    }
}
