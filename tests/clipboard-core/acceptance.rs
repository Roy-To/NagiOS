//! CLIP-01 host Foundation acceptance tests.
//!
//! These exercise the host contract and the in-memory reference only. They do
//! not prove compositor, IPC, target, or first-party clipboard behavior.

use std::cell::RefCell;
use std::rc::Rc;

use nagi_clipboard_core::*;

const SECRET: &str = "correct horse battery staple";

fn notes() -> CallerContext {
    CallerContext::from_trusted_boundary(
        AppId::from_identifier(b"org.nagi.notes"),
        AppSessionId(1),
        ExecutionInstanceId(10),
    )
}

fn files() -> CallerContext {
    CallerContext::from_trusted_boundary(
        AppId::from_identifier(b"org.nagi.files"),
        AppSessionId(2),
        ExecutionInstanceId(20),
    )
}

fn system_app() -> AppId {
    AppId::from_identifier(b"org.nagi.system-settings")
}

fn media(value: &str) -> MediaType {
    MediaType::new(value).expect("valid media type")
}

fn allow_all(_: &AuthorizationRequest<'_>) -> AuthorizationDecision {
    AuthorizationDecision::Allow
}

fn open_clipboard() -> InMemoryClipboard<fn(&AuthorizationRequest<'_>) -> AuthorizationDecision> {
    InMemoryClipboard::new(
        allow_all as fn(&AuthorizationRequest<'_>) -> AuthorizationDecision,
        ClipboardLimits::DEFAULT,
    )
    .expect("default limits")
}

fn text_of(result: &ReadResult) -> &str {
    match &result.payload {
        Payload::Text(text) => text,
        other => panic!("expected text payload, got {other:?}"),
    }
}

fn rich_item() -> ClipboardItem {
    ClipboardItem::new(vec![
        Representation::new(media("text/html"), Payload::Text("<b>hi</b>".into())).unwrap(),
        Representation::plain_text("hi"),
        Representation::new(
            media("image/png"),
            Payload::Binary(vec![0x89, b'P', b'N', b'G']),
        )
        .unwrap(),
    ])
}

const V1: ContractVersion = CLIPBOARD_CONTRACT_VERSION;

#[test]
fn empty_clipboard_reads_are_explicit() {
    let mut clipboard = open_clipboard();
    assert_eq!(
        clipboard.generation(&notes(), V1),
        Ok(ClipboardGeneration::INITIAL)
    );
    let offer = clipboard.formats(&notes(), V1).unwrap();
    assert!(offer.is_empty());
    assert_eq!(offer.verified_origin, None);
    assert_eq!(offer.claimed_origin, ClaimedOrigin::default());
    assert!(offer.metadata.is_empty());
    assert_eq!(
        clipboard.read(&notes(), ReadRequest::new(0, media("text/plain"))),
        Err(ClipboardError::Empty)
    );
}

#[test]
fn write_then_read_returns_the_exact_payload() {
    let mut clipboard = open_clipboard();
    let generation = clipboard
        .write(
            &notes(),
            WriteRequest::replace(ClipboardContent::plain_text("hello")),
        )
        .unwrap();
    assert_eq!(generation, ClipboardGeneration::new(1));
    let result = clipboard
        .read(&files(), ReadRequest::new(0, media("text/plain")))
        .unwrap();
    assert_eq!(text_of(&result), "hello");
    assert_eq!(result.generation, generation);
    assert_eq!(result.media_type, media("text/plain"));
}

#[test]
fn write_replaces_all_previous_items_and_formats() {
    let mut clipboard = open_clipboard();
    clipboard
        .write(
            &notes(),
            WriteRequest::replace(ClipboardContent::new(vec![rich_item(), rich_item()])),
        )
        .unwrap();
    clipboard
        .write(
            &files(),
            WriteRequest::replace(ClipboardContent::plain_text("new")),
        )
        .unwrap();
    let offer = clipboard.formats(&notes(), V1).unwrap();
    assert_eq!(offer.items.len(), 1);
    assert_eq!(offer.items[0].formats.len(), 1);
    assert_eq!(offer.verified_origin.unwrap().app, files().app());
    assert_eq!(
        clipboard.read(&notes(), ReadRequest::new(0, media("image/png"))),
        Err(ClipboardError::FormatNotAvailable)
    );
    assert_eq!(
        clipboard.read(&notes(), ReadRequest::new(1, media("text/plain"))),
        Err(ClipboardError::ItemOutOfRange)
    );
}

#[test]
fn clear_removes_content_and_is_idempotent_when_empty() {
    let mut clipboard = open_clipboard();
    clipboard
        .write(
            &notes(),
            WriteRequest::replace(ClipboardContent::plain_text("x")),
        )
        .unwrap();
    assert_eq!(
        clipboard.clear(&notes(), ClearRequest::unconditional()),
        Ok(ClipboardGeneration::new(2))
    );
    assert!(clipboard.formats(&notes(), V1).unwrap().is_empty());
    assert_eq!(
        clipboard.read(&notes(), ReadRequest::new(0, media("text/plain"))),
        Err(ClipboardError::Empty)
    );
    // Clearing an already-empty clipboard does not advance the generation.
    assert_eq!(
        clipboard.clear(&notes(), ClearRequest::unconditional()),
        Ok(ClipboardGeneration::new(2))
    );
}

#[test]
fn multiple_representations_keep_writer_order_and_are_individually_readable() {
    let mut clipboard = open_clipboard();
    clipboard
        .write(
            &notes(),
            WriteRequest::replace(ClipboardContent::new(vec![rich_item()])),
        )
        .unwrap();
    let offer = clipboard.formats(&files(), V1).unwrap();
    let listed: Vec<_> = offer.items[0]
        .formats
        .iter()
        .map(|format| {
            (
                format.media_type.as_str().to_owned(),
                format.kind,
                format.inline_len,
            )
        })
        .collect();
    assert_eq!(
        listed,
        vec![
            ("text/html".to_owned(), PayloadKind::Text, 9),
            ("text/plain".to_owned(), PayloadKind::Text, 2),
            ("image/png".to_owned(), PayloadKind::Binary, 4),
        ]
    );
    let html = clipboard
        .read(&files(), ReadRequest::new(0, media("text/html")))
        .unwrap();
    assert_eq!(text_of(&html), "<b>hi</b>");
    let png = clipboard
        .read(&files(), ReadRequest::new(0, media("image/png")))
        .unwrap();
    assert_eq!(png.payload, Payload::Binary(vec![0x89, b'P', b'N', b'G']));
}

#[test]
fn multiple_ordered_items_preserve_order() {
    let mut clipboard = open_clipboard();
    let items = ["first", "second", "third"]
        .iter()
        .map(|text| ClipboardItem::new(vec![Representation::plain_text(*text)]))
        .collect();
    clipboard
        .write(
            &notes(),
            WriteRequest::replace(ClipboardContent::new(items)),
        )
        .unwrap();
    assert_eq!(clipboard.formats(&notes(), V1).unwrap().items.len(), 3);
    for (index, expected) in ["first", "second", "third"].iter().enumerate() {
        let result = clipboard
            .read(&notes(), ReadRequest::new(index, media("text/plain")))
            .unwrap();
        assert_eq!(text_of(&result), *expected);
    }
    assert_eq!(
        clipboard.read(&notes(), ReadRequest::new(3, media("text/plain"))),
        Err(ClipboardError::ItemOutOfRange)
    );
}

#[test]
fn unsupported_format_is_reported_not_converted() {
    let mut clipboard = open_clipboard();
    clipboard
        .write(
            &notes(),
            WriteRequest::replace(ClipboardContent::plain_text("x")),
        )
        .unwrap();
    assert_eq!(
        clipboard.read(&notes(), ReadRequest::new(0, media("text/html"))),
        Err(ClipboardError::FormatNotAvailable)
    );
    assert_eq!(
        clipboard.read(
            &notes(),
            ReadRequest::new(0, media("application/vnd.nagi.unknown"))
        ),
        Err(ClipboardError::FormatNotAvailable)
    );
}

#[test]
fn invalid_media_type_identifiers_are_rejected() {
    let cases = [
        ("", MediaTypeError::Empty),
        ("text", MediaTypeError::MissingSubtype),
        ("text/", MediaTypeError::MissingSubtype),
        ("/plain", MediaTypeError::MissingSubtype),
        ("Text/Plain", MediaTypeError::InvalidCharacter),
        (
            "text/plain; charset=utf-8",
            MediaTypeError::ParametersNotAllowed,
        ),
        ("text /plain", MediaTypeError::InvalidCharacter),
        ("text/pla in", MediaTypeError::InvalidCharacter),
        ("text/*", MediaTypeError::WildcardNotAllowed),
        ("*/*", MediaTypeError::WildcardNotAllowed),
        ("text/plain/extra", MediaTypeError::InvalidCharacter),
        ("-text/plain", MediaTypeError::InvalidCharacter),
        ("téxt/plain", MediaTypeError::InvalidCharacter),
    ];
    for (input, expected) in cases {
        assert_eq!(MediaType::new(input), Err(expected), "input {input:?}");
    }
    let long_token = "a".repeat(128);
    assert_eq!(
        MediaType::new(format!("text/{long_token}")),
        Err(MediaTypeError::TooLong)
    );
    assert_eq!(
        MediaType::new("x".repeat(300)),
        Err(MediaTypeError::TooLong)
    );
    for valid in [
        "text/plain",
        "image/svg+xml",
        "application/vnd.nagi.note-block",
        "application/x.nagi.object-ref",
    ] {
        assert_eq!(MediaType::new(valid).unwrap().as_str(), valid);
    }
}

#[test]
fn invalid_content_and_metadata_are_rejected_without_state_change() {
    let mut clipboard = open_clipboard();
    clipboard
        .write(
            &notes(),
            WriteRequest::replace(ClipboardContent::plain_text("keep")),
        )
        .unwrap();
    let key = |value: &str| MetadataKey::new(value).unwrap();
    let invalid = [
        (ClipboardContent::new(Vec::new()), ContentError::NoItems),
        (
            ClipboardContent::new(vec![ClipboardItem::new(Vec::new())]),
            ContentError::EmptyItem { item: 0 },
        ),
        (
            ClipboardContent::new(vec![ClipboardItem::new(vec![
                Representation::plain_text("a"),
                Representation::plain_text("b"),
            ])]),
            ContentError::DuplicateMediaType { item: 0 },
        ),
        (
            ClipboardContent::plain_text("x").with_metadata(key("source.title"), "bad\u{7}"),
            ContentError::InvalidMetadataValue,
        ),
        (
            ClipboardContent::plain_text("x").with_metadata(key("source.title"), "v".repeat(257)),
            ContentError::MetadataValueTooLong,
        ),
        (
            ClipboardContent::plain_text("x").with_claimed_origin(ClaimedOrigin {
                label: Some(String::new()),
                ..ClaimedOrigin::default()
            }),
            ContentError::InvalidOriginLabel,
        ),
        (
            (0..9).fold(ClipboardContent::plain_text("x"), |content, index| {
                content.with_metadata(key(&format!("k{index}")), "v")
            }),
            ContentError::TooManyMetadataEntries,
        ),
        (
            ClipboardContent::new(
                (0..17)
                    .map(|_| ClipboardItem::new(vec![Representation::plain_text("x")]))
                    .collect(),
            ),
            ContentError::TooManyItems,
        ),
    ];
    for (content, expected) in invalid {
        assert_eq!(
            clipboard.write(&notes(), WriteRequest::replace(content)),
            Err(ClipboardError::Invalid(expected))
        );
    }
    for bad_key in ["", "Upper", "has space", ".lead", &"k".repeat(65)] {
        assert_eq!(
            MetadataKey::new(bad_key),
            Err(ContentError::InvalidMetadataKey)
        );
    }
    assert_eq!(
        Representation::new(media("text/plain"), Payload::Binary(vec![1])),
        Err(ContentError::PayloadMediaTypeMismatch)
    );
    assert_eq!(
        Representation::new(media("image/png"), Payload::Text("x".into())),
        Err(ContentError::PayloadMediaTypeMismatch)
    );
    assert_eq!(
        clipboard.generation(&notes(), V1),
        Ok(ClipboardGeneration::new(1))
    );
    let kept = clipboard
        .read(&notes(), ReadRequest::new(0, media("text/plain")))
        .unwrap();
    assert_eq!(text_of(&kept), "keep");
}

#[test]
fn payload_bounds_reject_oversized_content_without_truncation() {
    let limits = ClipboardLimits {
        max_representation_bytes: 16,
        max_total_bytes: 24,
        ..ClipboardLimits::DEFAULT
    };
    let mut clipboard = InMemoryClipboard::new(
        allow_all as fn(&AuthorizationRequest<'_>) -> AuthorizationDecision,
        limits,
    )
    .unwrap();
    let at_bound = "a".repeat(16);
    clipboard
        .write(
            &notes(),
            WriteRequest::replace(ClipboardContent::plain_text(at_bound.clone())),
        )
        .unwrap();
    assert_eq!(
        clipboard.write(
            &notes(),
            WriteRequest::replace(ClipboardContent::plain_text("b".repeat(17)))
        ),
        Err(ClipboardError::Invalid(
            ContentError::RepresentationTooLarge { item: 0 }
        ))
    );
    let two_items = ClipboardContent::new(vec![
        ClipboardItem::new(vec![Representation::plain_text("c".repeat(16))]),
        ClipboardItem::new(vec![Representation::plain_text("d".repeat(9))]),
    ]);
    assert_eq!(
        clipboard.write(&notes(), WriteRequest::replace(two_items)),
        Err(ClipboardError::Invalid(ContentError::TotalTooLarge))
    );
    // The previous content is intact and was never truncated.
    let result = clipboard
        .read(&notes(), ReadRequest::new(0, media("text/plain")))
        .unwrap();
    assert_eq!(text_of(&result), at_bound);
    assert_eq!(
        clipboard.generation(&notes(), V1),
        Ok(ClipboardGeneration::new(1))
    );
    // Object references carry no inline bytes.
    let reference = ClipboardContent::new(vec![ClipboardItem::new(vec![Representation::new(
        media("application/x.nagi.object-ref"),
        Payload::ObjectReference(ObjectId(42)),
    )
    .unwrap()])]);
    assert_eq!(reference.total_inline_bytes(), 0);
    clipboard
        .write(&notes(), WriteRequest::replace(reference))
        .unwrap();
}

#[test]
fn limits_must_be_non_zero_and_within_hard_caps() {
    assert_eq!(ClipboardLimits::DEFAULT.validate(), Ok(()));
    let zero = ClipboardLimits {
        max_items: 0,
        ..ClipboardLimits::DEFAULT
    };
    let over = ClipboardLimits {
        max_representation_bytes: hard_caps::MAX_REPRESENTATION_BYTES + 1,
        ..ClipboardLimits::DEFAULT
    };
    for limits in [zero, over] {
        assert!(InMemoryClipboard::new(DenyAllAuthorizer, limits).is_err());
    }
}

#[test]
fn generation_advances_only_on_successful_mutation_and_never_wraps() {
    let mut clipboard = open_clipboard();
    let mut expected = 0;
    for text in ["a", "b", "c"] {
        expected += 1;
        assert_eq!(
            clipboard.write(
                &notes(),
                WriteRequest::replace(ClipboardContent::plain_text(text))
            ),
            Ok(ClipboardGeneration::new(expected))
        );
    }
    let _ = clipboard.write(
        &notes(),
        WriteRequest::replace(ClipboardContent::new(Vec::new())),
    );
    let _ = clipboard.read(&notes(), ReadRequest::new(0, media("text/plain")));
    assert_eq!(
        clipboard.generation(&notes(), V1),
        Ok(ClipboardGeneration::new(3))
    );

    let mut exhausted = InMemoryClipboard::starting_at(
        allow_all as fn(&AuthorizationRequest<'_>) -> AuthorizationDecision,
        ClipboardLimits::DEFAULT,
        ClipboardGeneration::new(u64::MAX - 1),
    )
    .unwrap();
    assert_eq!(
        exhausted.write(
            &notes(),
            WriteRequest::replace(ClipboardContent::plain_text("last"))
        ),
        Ok(ClipboardGeneration::new(u64::MAX))
    );
    assert_eq!(
        exhausted.write(
            &notes(),
            WriteRequest::replace(ClipboardContent::plain_text("over"))
        ),
        Err(ClipboardError::GenerationExhausted)
    );
    assert_eq!(
        exhausted.clear(&notes(), ClearRequest::unconditional()),
        Err(ClipboardError::GenerationExhausted)
    );
    let kept = exhausted
        .read(&notes(), ReadRequest::new(0, media("text/plain")))
        .unwrap();
    assert_eq!(text_of(&kept), "last");
}

#[test]
fn stale_generation_is_rejected_for_write_clear_and_read() {
    let mut clipboard = open_clipboard();
    let first = clipboard
        .write(
            &notes(),
            WriteRequest::replace(ClipboardContent::plain_text("one")),
        )
        .unwrap();
    let second = clipboard
        .write(
            &files(),
            WriteRequest {
                expected_generation: Some(first),
                ..WriteRequest::replace(ClipboardContent::plain_text("two"))
            },
        )
        .unwrap();
    assert_eq!(
        clipboard.write(
            &notes(),
            WriteRequest {
                expected_generation: Some(first),
                ..WriteRequest::replace(ClipboardContent::plain_text("late"))
            }
        ),
        Err(ClipboardError::StaleGeneration { current: second })
    );
    assert_eq!(
        clipboard.clear(
            &notes(),
            ClearRequest {
                expected_generation: Some(first),
                ..ClearRequest::unconditional()
            }
        ),
        Err(ClipboardError::StaleGeneration { current: second })
    );
    assert_eq!(
        clipboard.read(
            &notes(),
            ReadRequest::new(0, media("text/plain")).at_generation(first)
        ),
        Err(ClipboardError::StaleGeneration { current: second })
    );
    let current = clipboard
        .read(
            &notes(),
            ReadRequest::new(0, media("text/plain")).at_generation(second),
        )
        .unwrap();
    assert_eq!(text_of(&current), "two");
    assert_eq!(clipboard.generation(&notes(), V1), Ok(second));
}

type Log = Rc<RefCell<Vec<String>>>;

/// Policy keyed only on the trusted caller and operation, as a future
/// Capability adapter would be.
fn policy(
    writer: AppId,
    readers: Vec<AppId>,
    restricted: &'static str,
    log: Log,
) -> impl FnMut(&AuthorizationRequest<'_>) -> AuthorizationDecision {
    move |request| {
        let app = request.caller.app();
        log.borrow_mut()
            .push(format!("{:?}:{:?}", app, request.operation));
        let allowed = match request.operation {
            ClipboardOperation::Write | ClipboardOperation::Clear => app == writer,
            ClipboardOperation::ReadGeneration | ClipboardOperation::ReadFormats => {
                readers.contains(&app)
            }
            ClipboardOperation::ReadRepresentation { media_type } => {
                readers.contains(&app) && (media_type.as_str() != restricted || app == writer)
            }
        };
        if allowed {
            AuthorizationDecision::Allow
        } else {
            AuthorizationDecision::Deny
        }
    }
}

#[test]
fn denied_write_read_and_clear_fail_closed_without_state_change() {
    let log: Log = Rc::default();
    let mut clipboard = InMemoryClipboard::new(
        policy(notes().app(), vec![notes().app()], "image/png", log.clone()),
        ClipboardLimits::DEFAULT,
    )
    .unwrap();
    clipboard
        .write(
            &notes(),
            WriteRequest::replace(ClipboardContent::plain_text("owned")),
        )
        .unwrap();

    assert_eq!(
        clipboard.write(
            &files(),
            WriteRequest::replace(ClipboardContent::plain_text("x"))
        ),
        Err(ClipboardError::Denied)
    );
    assert_eq!(
        clipboard.clear(&files(), ClearRequest::unconditional()),
        Err(ClipboardError::Denied)
    );
    assert_eq!(
        clipboard.read(&files(), ReadRequest::new(0, media("text/plain"))),
        Err(ClipboardError::Denied)
    );
    assert_eq!(clipboard.formats(&files(), V1), Err(ClipboardError::Denied));
    assert_eq!(
        clipboard.generation(&files(), V1),
        Err(ClipboardError::Denied)
    );
    // Denied callers learn nothing: an invalid write is still just Denied.
    assert_eq!(
        clipboard.write(
            &files(),
            WriteRequest::replace(ClipboardContent::new(Vec::new()))
        ),
        Err(ClipboardError::Denied)
    );
    assert_eq!(
        clipboard.read(&files(), ReadRequest::new(99, media("text/html"))),
        Err(ClipboardError::Denied)
    );

    assert_eq!(
        clipboard.generation(&notes(), V1),
        Ok(ClipboardGeneration::new(1))
    );
    let kept = clipboard
        .read(&notes(), ReadRequest::new(0, media("text/plain")))
        .unwrap();
    assert_eq!(text_of(&kept), "owned");
}

#[test]
fn default_and_unavailable_authorizers_fail_closed() {
    let mut denied = InMemoryClipboard::new(DenyAllAuthorizer, ClipboardLimits::DEFAULT).unwrap();
    assert_eq!(
        denied.write(
            &notes(),
            WriteRequest::replace(ClipboardContent::plain_text("x"))
        ),
        Err(ClipboardError::Denied)
    );
    assert_eq!(
        denied.clear(&notes(), ClearRequest::unconditional()),
        Err(ClipboardError::Denied)
    );
    assert_eq!(
        denied.read(&notes(), ReadRequest::new(0, media("text/plain"))),
        Err(ClipboardError::Denied)
    );

    let available = Rc::new(RefCell::new(true));
    let switch = available.clone();
    let mut clipboard = InMemoryClipboard::new(
        move |_: &AuthorizationRequest<'_>| {
            if *switch.borrow() {
                AuthorizationDecision::Allow
            } else {
                AuthorizationDecision::Unavailable
            }
        },
        ClipboardLimits::DEFAULT,
    )
    .unwrap();
    clipboard
        .write(
            &notes(),
            WriteRequest::replace(ClipboardContent::plain_text("x")),
        )
        .unwrap();
    *available.borrow_mut() = false;
    assert_eq!(
        clipboard.write(
            &notes(),
            WriteRequest::replace(ClipboardContent::plain_text("y"))
        ),
        Err(ClipboardError::AuthorizationUnavailable)
    );
    assert_eq!(
        clipboard.clear(&notes(), ClearRequest::unconditional()),
        Err(ClipboardError::AuthorizationUnavailable)
    );
    assert_eq!(
        clipboard.read(&notes(), ReadRequest::new(0, media("text/plain"))),
        Err(ClipboardError::AuthorizationUnavailable)
    );
    assert_eq!(
        clipboard.formats(&notes(), V1),
        Err(ClipboardError::AuthorizationUnavailable)
    );
    *available.borrow_mut() = true;
    assert_eq!(
        clipboard.generation(&notes(), V1),
        Ok(ClipboardGeneration::new(1))
    );
}

#[test]
fn restricted_representation_is_neither_listed_nor_readable() {
    let log: Log = Rc::default();
    let mut clipboard = InMemoryClipboard::new(
        policy(
            notes().app(),
            vec![notes().app(), files().app()],
            "image/png",
            log,
        ),
        ClipboardLimits::DEFAULT,
    )
    .unwrap();
    clipboard
        .write(
            &notes(),
            WriteRequest::replace(ClipboardContent::new(vec![rich_item()])),
        )
        .unwrap();
    let reader_offer = clipboard.formats(&files(), V1).unwrap();
    let listed: Vec<_> = reader_offer.items[0]
        .formats
        .iter()
        .map(|format| format.media_type.as_str())
        .collect();
    assert_eq!(listed, vec!["text/html", "text/plain"]);
    assert_eq!(
        clipboard.read(&files(), ReadRequest::new(0, media("image/png"))),
        Err(ClipboardError::Denied)
    );
    assert!(clipboard
        .read(&files(), ReadRequest::new(0, media("text/plain")))
        .is_ok());
    assert_eq!(
        clipboard.formats(&notes(), V1).unwrap().items[0]
            .formats
            .len(),
        3
    );
}

#[test]
fn claimed_origin_is_preserved_but_never_grants_authority() {
    let log: Log = Rc::default();
    let mut clipboard = InMemoryClipboard::new(
        policy(
            system_app(),
            vec![notes().app(), files().app()],
            "",
            log.clone(),
        ),
        ClipboardLimits::DEFAULT,
    )
    .unwrap();
    let claim = ClaimedOrigin {
        app: Some(system_app()),
        app_session: Some(AppSessionId(999)),
        label: Some("Quarterly plan".into()),
    };
    let forged = ClipboardContent::plain_text("forged").with_claimed_origin(claim.clone());
    // Claiming the privileged writer's AppId does not let notes() write.
    assert_eq!(
        clipboard.write(&notes(), WriteRequest::replace(forged)),
        Err(ClipboardError::Denied)
    );
    // The authorizer saw the authenticated caller, never the claim.
    assert!(log.borrow().iter().all(|entry| !entry.contains("999")));
    assert_eq!(log.borrow()[0], format!("{:?}:Write", notes().app()));

    // When the real privileged caller writes, a different claim is preserved
    // verbatim but the verified origin is the authenticated caller.
    let system_caller = CallerContext::from_trusted_boundary(
        system_app(),
        AppSessionId(7),
        ExecutionInstanceId(70),
    );
    let claim_files = ClaimedOrigin {
        app: Some(files().app()),
        ..claim
    };
    clipboard
        .write(
            &system_caller,
            WriteRequest::replace(
                ClipboardContent::plain_text("real").with_claimed_origin(claim_files.clone()),
            ),
        )
        .unwrap();
    let offer = clipboard.formats(&notes(), V1).unwrap();
    assert_eq!(offer.claimed_origin, claim_files);
    assert_eq!(
        offer.verified_origin,
        Some(VerifiedOrigin {
            app: system_app(),
            app_session: AppSessionId(7),
            execution_instance: ExecutionInstanceId(70),
        })
    );
}

#[test]
fn move_intent_is_a_non_destructive_hint() {
    let mut clipboard = open_clipboard();
    clipboard
        .write(
            &files(),
            WriteRequest::replace(
                ClipboardContent::plain_text("cut").with_intent(TransferIntent::Move),
            ),
        )
        .unwrap();
    for _ in 0..3 {
        let result = clipboard
            .read(&notes(), ReadRequest::new(0, media("text/plain")))
            .unwrap();
        assert_eq!(text_of(&result), "cut");
    }
    let offer = clipboard.formats(&notes(), V1).unwrap();
    assert_eq!(offer.intent, TransferIntent::Move);
    assert_eq!(offer.generation, ClipboardGeneration::new(1));
}

#[test]
fn unsupported_contract_version_is_rejected_for_every_operation() {
    let mut clipboard = open_clipboard();
    let v2 = ContractVersion(2);
    assert_eq!(
        clipboard.write(
            &notes(),
            WriteRequest {
                version: v2,
                ..WriteRequest::replace(ClipboardContent::plain_text("x"))
            }
        ),
        Err(ClipboardError::UnsupportedVersion { requested: v2 })
    );
    assert_eq!(
        clipboard.clear(
            &notes(),
            ClearRequest {
                version: v2,
                expected_generation: None
            }
        ),
        Err(ClipboardError::UnsupportedVersion { requested: v2 })
    );
    let mut read = ReadRequest::new(0, media("text/plain"));
    read.version = ContractVersion(0);
    assert_eq!(
        clipboard.read(&notes(), read),
        Err(ClipboardError::UnsupportedVersion {
            requested: ContractVersion(0)
        })
    );
    assert!(clipboard.formats(&notes(), v2).is_err());
    assert!(clipboard.generation(&notes(), v2).is_err());
    assert_eq!(
        clipboard.generation(&notes(), V1),
        Ok(ClipboardGeneration::INITIAL)
    );
}

fn sample_content() -> ClipboardContent {
    ClipboardContent::new(vec![
        rich_item(),
        ClipboardItem::new(vec![Representation::new(
            media("application/x.nagi.object-ref"),
            Payload::ObjectReference(ObjectId(77)),
        )
        .unwrap()]),
    ])
    .with_intent(TransferIntent::Move)
    .with_claimed_origin(ClaimedOrigin {
        app: Some(notes().app()),
        app_session: None,
        label: Some("日本語のタイトル".into()),
    })
    .with_metadata(MetadataKey::new("source.title").unwrap(), "Plan")
    .with_metadata(MetadataKey::new("a.first").unwrap(), "1")
}

#[test]
fn encoding_roundtrips_and_is_deterministic() {
    let limits = ClipboardLimits::DEFAULT;
    let content = sample_content();
    let encoded = encode_content(&content, &limits).unwrap();
    assert_eq!(&encoded[..4], b"NCLP");
    assert_eq!(&encoded[4..6], &1u16.to_le_bytes());
    assert_eq!(decode_content(&encoded, &limits), Ok(content.clone()));
    assert_eq!(encode_content(&sample_content(), &limits).unwrap(), encoded);
    assert_eq!(
        encode_content(&ClipboardContent::new(Vec::new()), &limits),
        Err(ContentError::NoItems)
    );
}

#[test]
fn malformed_or_unsupported_envelopes_are_rejected() {
    let limits = ClipboardLimits::DEFAULT;
    let encoded = encode_content(&sample_content(), &limits).unwrap();

    let mut wrong_version = encoded.clone();
    wrong_version[4..6].copy_from_slice(&2u16.to_le_bytes());
    assert_eq!(
        decode_content(&wrong_version, &limits),
        Err(DecodeError::UnsupportedVersion(2))
    );
    let mut zero_version = encoded.clone();
    zero_version[4..6].copy_from_slice(&0u16.to_le_bytes());
    assert_eq!(
        decode_content(&zero_version, &limits),
        Err(DecodeError::UnsupportedVersion(0))
    );
    let mut bad_magic = encoded.clone();
    bad_magic[0] = b'X';
    assert_eq!(
        decode_content(&bad_magic, &limits),
        Err(DecodeError::BadMagic)
    );
    assert_eq!(decode_content(b"NC", &limits), Err(DecodeError::BadMagic));

    for length in 4..encoded.len() {
        assert!(
            decode_content(&encoded[..length], &limits).is_err(),
            "prefix of {length} bytes was accepted"
        );
    }
    let mut trailing = encoded.clone();
    trailing.push(0);
    assert_eq!(
        decode_content(&trailing, &limits),
        Err(DecodeError::TrailingBytes)
    );

    let mut bad_intent = encoded.clone();
    bad_intent[6] = 9;
    assert_eq!(
        decode_content(&bad_intent, &limits),
        Err(DecodeError::UnknownTag)
    );
    let mut bad_flags = encoded.clone();
    bad_flags[7] |= 0b1000;
    assert_eq!(
        decode_content(&bad_flags, &limits),
        Err(DecodeError::UnknownTag)
    );
}

fn header(flags: u8) -> Vec<u8> {
    let mut bytes = b"NCLP".to_vec();
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.push(0);
    bytes.push(flags);
    bytes
}

fn put(bytes: &mut Vec<u8>, value: &[u8]) {
    bytes.extend_from_slice(&(value.len() as u32).to_le_bytes());
    bytes.extend_from_slice(value);
}

#[test]
fn envelope_bounds_are_checked_before_allocation() {
    let limits = ClipboardLimits::DEFAULT;
    // Declares u32::MAX metadata entries.
    let mut huge_count = header(0);
    huge_count.extend_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        decode_content(&huge_count, &limits),
        Err(DecodeError::BoundExceeded)
    );
    // Declares a 4 GiB text payload in a tiny buffer.
    let mut huge_payload = header(0);
    huge_payload.extend_from_slice(&0u32.to_le_bytes());
    huge_payload.extend_from_slice(&1u32.to_le_bytes());
    huge_payload.extend_from_slice(&1u32.to_le_bytes());
    put(&mut huge_payload, b"text/plain");
    huge_payload.push(0);
    huge_payload.extend_from_slice(&u32::MAX.to_le_bytes());
    assert_eq!(
        decode_content(&huge_payload, &limits),
        Err(DecodeError::BoundExceeded)
    );
}

#[test]
fn envelope_content_is_fully_validated() {
    let limits = ClipboardLimits::DEFAULT;
    let item = |media_type: &[u8], kind: u8, payload: &[u8]| {
        let mut bytes = header(0);
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        bytes.extend_from_slice(&1u32.to_le_bytes());
        put(&mut bytes, media_type);
        bytes.push(kind);
        put(&mut bytes, payload);
        bytes
    };
    assert!(decode_content(&item(b"text/plain", 0, b"ok"), &limits).is_ok());
    assert_eq!(
        decode_content(&item(b"Text/Plain", 0, b"ok"), &limits),
        Err(DecodeError::InvalidMediaType)
    );
    assert_eq!(
        decode_content(&item(b"text/plain", 0, &[0xff, 0xfe]), &limits),
        Err(DecodeError::InvalidUtf8)
    );
    assert_eq!(
        decode_content(&item(b"text/plain", 1, b"ok"), &limits),
        Err(DecodeError::Invalid(ContentError::PayloadMediaTypeMismatch))
    );
    assert_eq!(
        decode_content(&item(b"text/plain", 7, b"ok"), &limits),
        Err(DecodeError::UnknownTag)
    );
    let mut no_items = header(0);
    no_items.extend_from_slice(&0u32.to_le_bytes());
    no_items.extend_from_slice(&0u32.to_le_bytes());
    assert_eq!(
        decode_content(&no_items, &limits),
        Err(DecodeError::Invalid(ContentError::NoItems))
    );
    let mut duplicate_key = header(0);
    duplicate_key.extend_from_slice(&2u32.to_le_bytes());
    for _ in 0..2 {
        put(&mut duplicate_key, b"k");
        put(&mut duplicate_key, b"v");
    }
    assert_eq!(
        decode_content(&duplicate_key, &limits),
        Err(DecodeError::DuplicateMetadataKey)
    );
}

#[derive(Clone, Default)]
struct RecordingSink {
    events: Rc<RefCell<Vec<ClipboardEvent>>>,
    fail: bool,
}

impl ClipboardDiagnosticsSink for RecordingSink {
    fn record(&mut self, event: ClipboardEvent) -> Result<(), SinkError> {
        if self.fail {
            return Err(SinkError);
        }
        self.events.borrow_mut().push(event);
        Ok(())
    }
}

#[test]
fn payloads_never_appear_in_debug_output_or_diagnostics() {
    let content = ClipboardContent::plain_text(SECRET)
        .with_metadata(MetadataKey::new("note").unwrap(), SECRET)
        .with_claimed_origin(ClaimedOrigin {
            label: Some(SECRET.into()),
            ..ClaimedOrigin::default()
        });
    assert!(!format!("{content:?}").contains(SECRET));
    assert!(!format!("{:?}", Payload::Text(SECRET.into())).contains(SECRET));
    let binary = format!("{:?}", Payload::Binary(SECRET.as_bytes().to_vec()));
    assert_eq!(binary, format!("Binary(<redacted {} bytes>)", SECRET.len()));

    let sink = RecordingSink::default();
    let events = sink.events.clone();
    let mut clipboard = open_clipboard().with_diagnostics_sink(Box::new(sink));
    clipboard
        .write(&notes(), WriteRequest::replace(content))
        .unwrap();
    let result = clipboard
        .read(&notes(), ReadRequest::new(0, media("text/plain")))
        .unwrap();
    assert!(!format!("{result:?}").contains(SECRET));
    let _ = clipboard.read(&notes(), ReadRequest::new(0, media("text/html")));
    let _ = clipboard.write(
        &notes(),
        WriteRequest::replace(ClipboardContent::new(Vec::new())),
    );
    clipboard
        .clear(&notes(), ClearRequest::unconditional())
        .unwrap();
    let codes: Vec<_> = events.borrow().iter().map(|event| event.code).collect();
    assert_eq!(
        codes,
        vec![
            ClipboardEventCode::Written,
            ClipboardEventCode::RepresentationRead,
            ClipboardEventCode::Rejected,
            ClipboardEventCode::Cleared,
        ]
    );
    for event in events.borrow().iter() {
        assert!(!format!("{event:?}").contains(SECRET));
    }
}

#[test]
fn diagnostics_sink_failure_never_changes_outcomes() {
    let mut clipboard = open_clipboard().with_diagnostics_sink(Box::new(RecordingSink {
        fail: true,
        ..RecordingSink::default()
    }));
    assert_eq!(
        clipboard.write(
            &notes(),
            WriteRequest::replace(ClipboardContent::plain_text("x"))
        ),
        Ok(ClipboardGeneration::new(1))
    );
    assert!(clipboard
        .read(&notes(), ReadRequest::new(0, media("text/plain")))
        .is_ok());
    assert_eq!(clipboard.dropped_diagnostics(), 2);
}

#[test]
fn operations_map_to_capability_permissions() {
    let png = media("image/png");
    assert_eq!(
        ClipboardOperation::Write.required_permission(),
        "clipboard.write"
    );
    assert_eq!(
        ClipboardOperation::Clear.required_permission(),
        "clipboard.write"
    );
    assert_eq!(
        ClipboardOperation::ReadFormats.required_permission(),
        "clipboard.read"
    );
    assert_eq!(
        ClipboardOperation::ReadGeneration.required_permission(),
        "clipboard.read"
    );
    assert_eq!(
        ClipboardOperation::ReadRepresentation { media_type: &png }.required_permission(),
        "clipboard.read"
    );
}

fn scripted_run() -> (
    Vec<Result<ClipboardGeneration, ClipboardError>>,
    ClipboardOffer,
    Vec<u8>,
) {
    let log: Log = Rc::default();
    let mut clipboard = InMemoryClipboard::new(
        policy(
            notes().app(),
            vec![notes().app(), files().app()],
            "image/png",
            log,
        ),
        ClipboardLimits::DEFAULT,
    )
    .unwrap();
    let outcomes = vec![
        clipboard.write(&notes(), WriteRequest::replace(sample_content())),
        clipboard.write(
            &files(),
            WriteRequest::replace(ClipboardContent::plain_text("no")),
        ),
        clipboard.clear(&files(), ClearRequest::unconditional()),
        clipboard.write(
            &notes(),
            WriteRequest {
                expected_generation: Some(ClipboardGeneration::new(1)),
                ..WriteRequest::replace(sample_content())
            },
        ),
    ];
    let offer = clipboard.formats(&files(), V1).unwrap();
    let encoded = encode_content(&sample_content(), &ClipboardLimits::DEFAULT).unwrap();
    (outcomes, offer, encoded)
}

#[test]
fn repeated_runs_are_deterministic() {
    let first = scripted_run();
    for _ in 0..5 {
        assert_eq!(scripted_run(), first);
    }
    assert_eq!(
        first.0,
        vec![
            Ok(ClipboardGeneration::new(1)),
            Err(ClipboardError::Denied),
            Err(ClipboardError::Denied),
            Ok(ClipboardGeneration::new(2)),
        ]
    );
    let keys: Vec<_> = first.1.metadata.keys().map(MetadataKey::as_str).collect();
    assert_eq!(keys, vec!["a.first", "source.title"]);
}
