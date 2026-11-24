use super::*;

fn container(revision: u64, payload: &[u8]) -> Vec<u8> {
    encode_container(NonZeroU64::new(revision).unwrap(), payload).unwrap()
}

#[test]
fn round_trips_preferences_and_advances_revision() {
    let (first, bytes) = encode_replacement(None, Locale::JaJp, ThemeMode::Dark).unwrap();
    assert_eq!(first.get(), 1);
    assert_eq!(&bytes[..16], b"NAGI-SETTINGS\0\0\x01");
    let decoded = decode(&bytes).unwrap();
    assert_eq!(decoded.revision, first);
    assert_eq!(decoded.preferences.locale, Locale::JaJp);
    assert_eq!(decoded.preferences.theme, ThemeMode::Dark);
    assert!(!decoded.invalid_locale);
    assert!(!decoded.invalid_theme);
    let payload: Value = serde_json::from_slice(decoded.payload).unwrap();
    assert_eq!(
        payload,
        serde_json::json!({"schema": 1, "locale": "ja-JP", "theme": "dark"})
    );

    let (next, bytes) = encode_replacement(Some(&bytes), Locale::EnUs, ThemeMode::Light).unwrap();
    assert_eq!(next.get(), 2);
    assert_eq!(decode(&bytes).unwrap().preferences.theme, ThemeMode::Light);
}

#[test]
fn preserves_unknown_fields_on_replacement() {
    let bytes = container(
        9,
        br#"{"schema":1,"locale":"en-US","theme":"light","future":{"list":[1,null,true]}}"#,
    );
    let (revision, bytes) =
        encode_replacement(Some(&bytes), Locale::JaJp, ThemeMode::Dark).unwrap();
    assert_eq!(revision.get(), 10);
    let decoded = decode(&bytes).unwrap();
    assert_eq!(
        decoded.preferences.unknown["future"],
        serde_json::json!({"list": [1, null, true]})
    );
    assert_eq!(decoded.preferences.locale, Locale::JaJp);
    assert_eq!(decoded.preferences.theme, ThemeMode::Dark);
}

#[test]
fn crc_mismatch_is_corrupt() {
    // The standard CRC32C check value distinguishes it from ordinary CRC32.
    assert_eq!(crc32c(b"123456789"), 0xe306_9283);
    let (_, mut bytes) = encode_replacement(None, Locale::EnUs, ThemeMode::Light).unwrap();
    bytes[HEADER_BYTES] ^= 1;
    assert_eq!(decode(&bytes).unwrap_err(), DecodeError::Corrupt);
}

#[test]
fn newer_schema_cannot_be_replaced() {
    let bytes = container(1, br#"{"schema":2,"locale":"en-US","theme":"light"}"#);
    let expected = DecodeError::UnsupportedSchema(2);
    assert_eq!(decode(&bytes).unwrap_err(), expected);
    assert_eq!(
        encode_replacement(Some(&bytes), Locale::JaJp, ThemeMode::Dark).unwrap_err(),
        expected
    );
}

#[test]
fn invalid_locale_and_theme_fall_back_independently() {
    let bytes = container(1, br#"{"schema":1,"locale":"fr-FR","theme":"dark"}"#);
    let decoded = decode(&bytes).unwrap();
    assert_eq!(decoded.preferences.locale, Locale::EnUs);
    assert_eq!(decoded.preferences.theme, ThemeMode::Dark);
    assert!(decoded.invalid_locale);
    assert!(!decoded.invalid_theme);

    let bytes = container(1, br#"{"schema":1,"locale":"ja-JP","theme":42}"#);
    let decoded = decode(&bytes).unwrap();
    assert_eq!(decoded.preferences.locale, Locale::JaJp);
    assert_eq!(decoded.preferences.theme, ThemeMode::Light);
    assert!(!decoded.invalid_locale);
    assert!(decoded.invalid_theme);
}

#[test]
fn validates_container_lengths() {
    let (_, mut bytes) = encode_replacement(None, Locale::EnUs, ThemeMode::Light).unwrap();
    bytes.push(0);
    assert_eq!(decode(&bytes).unwrap_err(), DecodeError::Corrupt);
    bytes[24..28].copy_from_slice(&(MAX_PAYLOAD_BYTES as u32 + 1).to_le_bytes());
    assert_eq!(decode(&bytes).unwrap_err(), DecodeError::PayloadTooLarge);
    assert_eq!(
        encode_container(NonZeroU64::MIN, &[0; MAX_PAYLOAD_BYTES + 1]).unwrap_err(),
        DecodeError::PayloadTooLarge
    );
}

#[test]
fn rejects_zero_and_does_not_wrap_maximum_revision() {
    let mut bytes = container(
        u64::MAX,
        br#"{"schema":1,"locale":"en-US","theme":"light"}"#,
    );
    assert_eq!(
        encode_replacement(Some(&bytes), Locale::JaJp, ThemeMode::Dark).unwrap_err(),
        DecodeError::RevisionExhausted
    );
    bytes[16..24].fill(0);
    assert_eq!(decode(&bytes).unwrap_err(), DecodeError::Corrupt);
}
