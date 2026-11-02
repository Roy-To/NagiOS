use std::collections::BTreeSet;
use std::io::{self, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize};

pub const DIAGNOSTIC_EVENT_SCHEMA_VERSION: u32 = 1;
pub const VERIFICATION_REPORT_SCHEMA_VERSION: u32 = 1;
pub const DIAGNOSTIC_BUNDLE_SCHEMA_VERSION: u32 = 1;
pub const MAX_IDENTIFIER_BYTES: usize = 96;
pub const MAX_MESSAGE_BYTES: usize = 2_048;
pub const MAX_FIELD_COUNT: usize = 64;
pub const MAX_FIELD_VALUE_BYTES: usize = 2_048;
pub const MAX_ERROR_CHAIN: usize = 8;

const REDACTED: &str = "[REDACTED]";
const TRUNCATED: &str = "[TRUNCATED]";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Severity {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
    Fatal,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Trace => "TRACE",
            Self::Debug => "DEBUG",
            Self::Info => "INFO",
            Self::Warn => "WARN",
            Self::Error => "ERROR",
            Self::Fatal => "FATAL",
        }
    }
}

/// Failure classes are aligned with the accepted DF-01 state schema so that
/// diagnostics can be consumed by `nagi dev verify` without translation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FailureClass {
    #[serde(rename = "SOURCE")]
    Source,
    #[serde(rename = "BUILD")]
    Build,
    #[serde(rename = "LINK")]
    Link,
    #[serde(rename = "ABI")]
    Abi,
    #[serde(rename = "RUNTIME")]
    Runtime,
    #[serde(rename = "BOOT")]
    Boot,
    #[serde(rename = "DEVICE")]
    Device,
    #[serde(rename = "STORAGE")]
    Storage,
    #[serde(rename = "GRAPHICS")]
    Graphics,
    #[serde(rename = "NETWORK")]
    Network,
    #[serde(rename = "MODEL")]
    Model,
    #[serde(rename = "PERMISSION")]
    Permission,
    #[serde(rename = "ACCEPTANCE")]
    Acceptance,
    #[serde(rename = "CI_INFRA")]
    CiInfra,
    #[serde(rename = "HOST_ENV")]
    HostEnv,
    #[serde(rename = "UNKNOWN")]
    Unknown,
}

impl FailureClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Source => "SOURCE",
            Self::Build => "BUILD",
            Self::Link => "LINK",
            Self::Abi => "ABI",
            Self::Runtime => "RUNTIME",
            Self::Boot => "BOOT",
            Self::Device => "DEVICE",
            Self::Storage => "STORAGE",
            Self::Graphics => "GRAPHICS",
            Self::Network => "NETWORK",
            Self::Model => "MODEL",
            Self::Permission => "PERMISSION",
            Self::Acceptance => "ACCEPTANCE",
            Self::CiInfra => "CI_INFRA",
            Self::HostEnv => "HOST_ENV",
            Self::Unknown => "UNKNOWN",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PrivacyClass {
    Public,
    Sensitive,
    Secret,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagnosticField {
    pub key: String,
    pub value: String,
    pub privacy: PrivacyClass,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceLocation {
    pub file: String,
    pub line: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErrorCause {
    pub class: FailureClass,
    pub code: String,
    pub safe_message: String,
}

/// In-memory event. Serialization is deliberately only exposed through
/// `to_json`, which applies redaction and size limits before emitting data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiagnosticEvent {
    pub timestamp_unix_ms: u64,
    pub severity: Severity,
    pub subsystem: String,
    pub event_code: String,
    pub message_template: String,
    pub operation_id: Option<String>,
    pub error_class: Option<FailureClass>,
    pub source: Option<SourceLocation>,
    pub fields: Vec<DiagnosticField>,
    pub error_chain: Vec<ErrorCause>,
    pub recovery_hint: Option<String>,
}

/// A structured event view whose supported text fields have been redacted.
/// This is not a sanitizer for arbitrary raw logs, HAR, URL userinfo, pair
/// arrays, bare bearer tokens, command-line flags, or bracketed key paths.
/// Reprocessing ambiguous free text may redact more content; this is not a
/// byte-canonicalizer for arbitrary strings.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SafeDiagnosticEvent {
    pub schema_version: u32,
    pub timestamp_unix_ms: u64,
    pub severity: Severity,
    pub subsystem: String,
    pub event_code: String,
    pub message_template: String,
    pub operation_id: Option<String>,
    pub error_class: Option<FailureClass>,
    pub source: Option<SafeSourceLocation>,
    pub fields: Vec<SafeDiagnosticField>,
    pub error_chain: Vec<SafeErrorCause>,
    pub recovery_hint: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SafeSourceLocation {
    pub file: String,
    pub line: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SafeDiagnosticField {
    pub key: String,
    pub value: String,
    pub privacy: PrivacyClass,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SafeErrorCause {
    pub class: FailureClass,
    pub code: String,
    pub safe_message: String,
}

impl Serialize for SafeDiagnosticEvent {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let fields = self
            .fields
            .iter()
            .map(|field| SafeDiagnosticField {
                key: field.key.clone(),
                value: safe_classified_value(&field.key, &field.value, field.privacy),
                privacy: field.privacy,
            })
            .collect::<Vec<_>>();
        let error_chain = self
            .error_chain
            .iter()
            .map(|cause| SafeErrorCause {
                class: cause.class,
                code: cause.code.clone(),
                safe_message: redact_inline_secrets(&cause.safe_message),
            })
            .collect::<Vec<_>>();
        let message_template = redact_inline_secrets(&self.message_template);
        let recovery_hint = self
            .recovery_hint
            .as_ref()
            .map(|hint| redact_inline_secrets(hint));
        let mut state = serializer.serialize_struct("SafeDiagnosticEvent", 12)?;
        state.serialize_field("schema_version", &self.schema_version)?;
        state.serialize_field("timestamp_unix_ms", &self.timestamp_unix_ms)?;
        state.serialize_field("severity", &self.severity)?;
        state.serialize_field("subsystem", &self.subsystem)?;
        state.serialize_field("event_code", &self.event_code)?;
        state.serialize_field("message_template", &message_template)?;
        state.serialize_field("operation_id", &self.operation_id)?;
        state.serialize_field("error_class", &self.error_class)?;
        state.serialize_field("source", &self.source)?;
        state.serialize_field("fields", &fields)?;
        state.serialize_field("error_chain", &error_chain)?;
        state.serialize_field("recovery_hint", &recovery_hint)?;
        state.end()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticError(pub String);

impl std::fmt::Display for DiagnosticError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for DiagnosticError {}

impl DiagnosticEvent {
    pub fn new(
        severity: Severity,
        subsystem: impl Into<String>,
        event_code: impl Into<String>,
        message_template: impl Into<String>,
    ) -> Result<Self, DiagnosticError> {
        let subsystem = subsystem.into();
        let event_code = event_code.into();
        validate_identifier(&subsystem, false, "subsystem")?;
        validate_identifier(&event_code, true, "event code")?;

        Ok(Self {
            timestamp_unix_ms: now_unix_ms(),
            severity,
            subsystem,
            event_code,
            message_template: bounded_text(&message_template.into(), MAX_MESSAGE_BYTES),
            operation_id: None,
            error_class: None,
            source: None,
            fields: Vec::new(),
            error_chain: Vec::new(),
            recovery_hint: None,
        })
    }

    pub fn with_timestamp(mut self, timestamp_unix_ms: u64) -> Self {
        self.timestamp_unix_ms = timestamp_unix_ms;
        self
    }

    pub fn with_operation_id(
        mut self,
        operation_id: impl Into<String>,
    ) -> Result<Self, DiagnosticError> {
        let operation_id = operation_id.into();
        validate_identifier(&operation_id, false, "operation id")?;
        self.operation_id = Some(operation_id);
        Ok(self)
    }

    pub fn with_error_class(mut self, error_class: FailureClass) -> Self {
        self.error_class = Some(error_class);
        self
    }

    pub fn with_source(mut self, file: impl Into<String>, line: u32) -> Self {
        self.source = Some(SourceLocation {
            file: bounded_text(&file.into(), MAX_IDENTIFIER_BYTES * 4),
            line,
        });
        self
    }

    pub fn with_field(
        mut self,
        key: impl Into<String>,
        value: impl Into<String>,
        privacy: PrivacyClass,
    ) -> Result<Self, DiagnosticError> {
        if self.fields.len() >= MAX_FIELD_COUNT {
            return Err(DiagnosticError(format!(
                "diagnostic field count exceeds {MAX_FIELD_COUNT}"
            )));
        }
        let key = key.into();
        validate_identifier(&key, false, "field key")?;
        if self.fields.iter().any(|field| field.key == key) {
            return Err(DiagnosticError(format!(
                "duplicate diagnostic field key `{key}`"
            )));
        }
        self.fields.push(DiagnosticField {
            key,
            value: bounded_text(&value.into(), MAX_FIELD_VALUE_BYTES),
            privacy,
        });
        Ok(self)
    }

    pub fn with_error_cause(mut self, cause: ErrorCause) -> Result<Self, DiagnosticError> {
        if self.error_chain.len() < MAX_ERROR_CHAIN {
            validate_identifier(&cause.code, true, "error cause code")?;
            self.error_chain.push(ErrorCause {
                class: cause.class,
                code: bounded_text(&cause.code, MAX_IDENTIFIER_BYTES),
                safe_message: bounded_text(&cause.safe_message, MAX_MESSAGE_BYTES),
            });
        }
        Ok(self)
    }

    pub fn with_recovery_hint(mut self, hint: impl Into<String>) -> Self {
        self.recovery_hint = Some(bounded_text(&hint.into(), MAX_MESSAGE_BYTES));
        self
    }

    /// Serializes this structured event after applying privacy classes and the
    /// diagnostic text scrubber. The scrubber recognizes JSON object members
    /// (including JSON `\\uXXXX` key escapes), HTTP-style headers, and key/value
    /// fields. It does not certify arbitrary raw logs, HAR, URL userinfo, pair
    /// arrays, bare bearer tokens, command-line flags, or bracketed key paths.
    pub fn to_json(&self) -> Result<String, DiagnosticError> {
        serde_json::to_string_pretty(&self.safe_view())
            .map_err(|error| DiagnosticError(format!("serialize diagnostic event: {error}")))
    }

    /// Produces a redacted view for a structured event. Text matching covers
    /// JSON object members (including JSON `\\uXXXX` key escapes), HTTP-style
    /// headers, and key/value fields; it does not certify arbitrary raw logs,
    /// HAR, URL userinfo, pair arrays, bare bearer tokens, command-line flags,
    /// or bracketed key paths.
    pub fn safe_view(&self) -> SafeDiagnosticEvent {
        self.safe_record()
    }

    pub fn from_json(input: &str) -> Result<Self, DiagnosticError> {
        let record: SafeDiagnosticEvent = serde_json::from_str(input)
            .map_err(|error| DiagnosticError(format!("parse diagnostic event: {error}")))?;
        if record.schema_version != DIAGNOSTIC_EVENT_SCHEMA_VERSION {
            return Err(DiagnosticError(format!(
                "unsupported diagnostic event schema version {}",
                record.schema_version
            )));
        }
        validate_identifier(&record.subsystem, false, "subsystem")?;
        validate_identifier(&record.event_code, true, "event code")?;
        if let Some(operation_id) = &record.operation_id {
            validate_identifier(operation_id, false, "operation id")?;
        }
        if record.fields.len() > MAX_FIELD_COUNT || record.error_chain.len() > MAX_ERROR_CHAIN {
            return Err(DiagnosticError(
                "diagnostic event exceeds the bounded field or error-chain limit".into(),
            ));
        }

        let mut field_keys = BTreeSet::new();
        for field in &record.fields {
            validate_identifier(&field.key, false, "field key")?;
            if !field_keys.insert(field.key.as_str()) {
                return Err(DiagnosticError(format!(
                    "duplicate diagnostic field key `{}`",
                    field.key
                )));
            }
        }
        for cause in &record.error_chain {
            validate_identifier(&cause.code, true, "error cause code")?;
        }

        Ok(Self {
            timestamp_unix_ms: record.timestamp_unix_ms,
            severity: record.severity,
            subsystem: record.subsystem,
            event_code: record.event_code,
            message_template: redact_inline_secrets(&bounded_text(
                &record.message_template,
                MAX_MESSAGE_BYTES,
            )),
            operation_id: record.operation_id,
            error_class: record.error_class,
            source: record.source.map(|source| SourceLocation {
                file: bounded_text(&source.file, MAX_IDENTIFIER_BYTES * 4),
                line: source.line,
            }),
            fields: record
                .fields
                .into_iter()
                .map(|field| DiagnosticField {
                    key: bounded_text(&field.key, MAX_IDENTIFIER_BYTES),
                    value: safe_classified_value(
                        &field.key,
                        &bounded_text(&field.value, MAX_FIELD_VALUE_BYTES),
                        field.privacy,
                    ),
                    privacy: field.privacy,
                })
                .collect(),
            error_chain: record
                .error_chain
                .into_iter()
                .map(|cause| ErrorCause {
                    class: cause.class,
                    code: bounded_text(&cause.code, MAX_IDENTIFIER_BYTES),
                    safe_message: redact_inline_secrets(&bounded_text(
                        &cause.safe_message,
                        MAX_MESSAGE_BYTES,
                    )),
                })
                .collect(),
            recovery_hint: record
                .recovery_hint
                .map(|hint| redact_inline_secrets(&bounded_text(&hint, MAX_MESSAGE_BYTES))),
        })
    }

    pub fn render_human(&self) -> String {
        let safe = self.safe_record();
        let mut line = format!(
            "{} {} {}/{}: {}",
            safe.timestamp_unix_ms,
            safe.severity.as_str(),
            safe.subsystem,
            safe.event_code,
            safe.message_template
        );
        if let Some(class) = safe.error_class {
            line.push_str(&format!(" [{}]", class.as_str()));
        }
        if let Some(operation_id) = safe.operation_id {
            line.push_str(&format!(" operation={operation_id}"));
        }
        for field in safe.fields {
            line.push_str(&format!(" {}={}", field.key, field.value));
        }
        if let Some(hint) = safe.recovery_hint {
            line.push_str(&format!(" recovery={hint}"));
        }
        line
    }

    fn safe_record(&self) -> SafeDiagnosticEvent {
        SafeDiagnosticEvent {
            schema_version: DIAGNOSTIC_EVENT_SCHEMA_VERSION,
            timestamp_unix_ms: self.timestamp_unix_ms,
            severity: self.severity,
            subsystem: self.subsystem.clone(),
            event_code: self.event_code.clone(),
            message_template: redact_inline_secrets(&self.message_template),
            operation_id: self.operation_id.clone(),
            error_class: self.error_class,
            source: self.source.as_ref().map(|source| SafeSourceLocation {
                file: bounded_text(&source.file, MAX_IDENTIFIER_BYTES * 4),
                line: source.line,
            }),
            fields: self
                .fields
                .iter()
                .map(|field| SafeDiagnosticField {
                    key: field.key.clone(),
                    value: safe_field_value(field),
                    privacy: field.privacy,
                })
                .collect(),
            error_chain: self
                .error_chain
                .iter()
                .map(|cause| SafeErrorCause {
                    class: cause.class,
                    code: cause.code.clone(),
                    safe_message: redact_inline_secrets(&cause.safe_message),
                })
                .collect(),
            recovery_hint: self
                .recovery_hint
                .as_ref()
                .map(|hint| redact_inline_secrets(hint)),
        }
    }
}

fn safe_field_value(field: &DiagnosticField) -> String {
    safe_classified_value(&field.key, &field.value, field.privacy)
}

fn sanitized_event(event: &SafeDiagnosticEvent) -> SafeDiagnosticEvent {
    SafeDiagnosticEvent {
        schema_version: event.schema_version,
        timestamp_unix_ms: event.timestamp_unix_ms,
        severity: event.severity,
        subsystem: event.subsystem.clone(),
        event_code: event.event_code.clone(),
        message_template: redact_inline_secrets(&event.message_template),
        operation_id: event.operation_id.clone(),
        error_class: event.error_class,
        source: event.source.clone(),
        fields: event
            .fields
            .iter()
            .map(|field| SafeDiagnosticField {
                key: field.key.clone(),
                value: safe_classified_value(&field.key, &field.value, field.privacy),
                privacy: field.privacy,
            })
            .collect(),
        error_chain: event
            .error_chain
            .iter()
            .map(|cause| SafeErrorCause {
                class: cause.class,
                code: cause.code.clone(),
                safe_message: redact_inline_secrets(&cause.safe_message),
            })
            .collect(),
        recovery_hint: event
            .recovery_hint
            .as_ref()
            .map(|hint| redact_inline_secrets(hint)),
    }
}

fn safe_classified_value(key: &str, value: &str, privacy: PrivacyClass) -> String {
    if privacy != PrivacyClass::Public || is_secret_key(key) {
        REDACTED.to_owned()
    } else {
        redact_inline_secrets(value)
    }
}

fn normalized_secret_key(key: &str) -> String {
    let characters = key.chars().collect::<Vec<_>>();
    let mut normalized = String::with_capacity(key.len() + 8);
    for (index, character) in characters.iter().copied().enumerate() {
        if character == '-' {
            if !normalized.ends_with('_') {
                normalized.push('_');
            }
            continue;
        }
        if character.is_ascii_uppercase() {
            let previous = index.checked_sub(1).map(|previous| characters[previous]);
            let next = characters.get(index + 1).copied();
            let camel_boundary = previous.is_some_and(|previous| {
                previous.is_ascii_lowercase()
                    || previous.is_ascii_digit()
                    || (previous.is_ascii_uppercase()
                        && next.is_some_and(|next| next.is_ascii_lowercase()))
            });
            if camel_boundary && !normalized.ends_with('_') {
                normalized.push('_');
            }
            normalized.push(character.to_ascii_lowercase());
        } else if character.is_ascii_alphanumeric() || character == '_' {
            normalized.push(character.to_ascii_lowercase());
        } else {
            normalized.push(character);
        }
    }
    normalized
}

fn is_secret_key(key: &str) -> bool {
    let simple = key.to_ascii_lowercase().replace('-', "_");
    let camel = normalized_secret_key(key);
    let contains_sensitive_marker = [
        "password",
        "passwd",
        "passphrase",
        "token",
        "secret",
        "cookie",
        "credential",
        "authorization",
        "api_key",
        "private_key",
        "access_key",
    ]
    .iter()
    .any(|needle| simple.contains(needle) || camel.contains(needle));
    let short_alias = ["pwd", "pass", "auth", "session", "apikey", "privatekey"]
        .iter()
        .any(|alias| {
            [simple.as_str(), camel.as_str()].iter().any(|key| {
                key.split(|character: char| !character.is_ascii_alphanumeric())
                    .any(|component| component == *alias)
            })
        });
    contains_sensitive_marker || short_alias || simple.contains("api.key")
}

pub(crate) fn escape_terminal_controls(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() || matches!(character, '\u{2028}' | '\u{2029}') {
                character.escape_default().to_string()
            } else {
                character.to_string()
            }
        })
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum QuoteStyle {
    Plain,
    Escaped(usize),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FieldValueKind {
    Header,
    Inline,
}

#[derive(Clone, Copy)]
struct FieldMarker {
    separator_end: usize,
    value_start: usize,
    kind: FieldValueKind,
}

struct JsonSecretMember {
    prefix_end: usize,
    value_start: usize,
    quote_style: QuoteStyle,
}

const MAX_JSON_DEPTH: usize = 32;

fn redact_inline_secrets(value: &str) -> String {
    // This text scrubber recognizes JSON members (including JSON Unicode
    // escapes in keys), HTTP-style header lines, and key/value fields. It does
    // not parse arbitrary raw logs, HAR, URL userinfo, pair arrays, bare bearer
    // tokens, command-line flags, or bracketed key paths. Callers must not
    // label those formats safe without first converting them to structured
    // fields.
    // Replace oversized input before parsing. This bounds work and prevents a
    // truncation from splitting a credential marker or quoted value.
    if value.len() > MAX_MESSAGE_BYTES {
        return format!("{REDACTED}{TRUNCATED}");
    }

    let mut output = String::new();
    let mut cursor = 0;
    while cursor < value.len() {
        if let Some(member) = json_secret_member_at(value, cursor) {
            output.push_str(&value[cursor..member.prefix_end]);
            append_json_redaction(&mut output, member.quote_style);
            cursor = json_value_end(value, member.value_start, member.quote_style)
                .unwrap_or(value.len());
            continue;
        }

        if let Some(marker) = field_marker_at(value, cursor) {
            output.push_str(&value[cursor..marker.separator_end]);
            if marker.kind == FieldValueKind::Header
                || value_starts_with_credential_scheme(value, marker.value_start)
            {
                // Authorization, auth/token/cookie fields, and other secret
                // headers are opaque through the line ending. This includes
                // Digest, AWS, Negotiate, and schemes unknown to this CLI.
                output.push_str(REDACTED);
                cursor = header_value_end(value, marker.value_start);
                append_escaped_control_boundary_space(
                    value,
                    marker.value_start,
                    cursor,
                    &mut output,
                );
                continue;
            }

            if inline_value_is_empty(value, marker) {
                output.push_str(REDACTED);
                if field_marker_at(value, marker.value_start).is_some() {
                    output.push(' ');
                }
                cursor = marker.value_start;
                append_escaped_control_boundary_space(
                    value,
                    marker.value_start,
                    cursor,
                    &mut output,
                );
                continue;
            }

            if let Some((opening_end, closing_start, after_closing)) =
                inline_quoted_value(value, marker.value_start)
            {
                if !quoted_value_suffix_is_delimited(value, after_closing) {
                    output.push_str(REDACTED);
                    cursor = header_value_end(value, marker.value_start);
                    append_escaped_control_boundary_space(
                        value,
                        marker.value_start,
                        cursor,
                        &mut output,
                    );
                    continue;
                }
                output.push_str(&value[marker.value_start..opening_end]);
                output.push_str(REDACTED);
                output.push_str(&value[closing_start..after_closing]);
                cursor = after_closing;
                continue;
            }

            if starts_with_quote(value, marker.value_start) {
                output.push_str(REDACTED);
                cursor = header_value_end(value, marker.value_start);
                append_escaped_control_boundary_space(
                    value,
                    marker.value_start,
                    cursor,
                    &mut output,
                );
                continue;
            }

            output.push_str(REDACTED);
            cursor = inline_value_end(value, marker.value_start);
            append_escaped_control_boundary_space(value, marker.value_start, cursor, &mut output);
            continue;
        }

        let character = value[cursor..].chars().next().unwrap();
        output.push(character);
        cursor += character.len_utf8();
    }
    let escaped_controls = escape_terminal_controls(&output);
    bounded_text(&escaped_controls, MAX_MESSAGE_BYTES)
}

fn skip_json_whitespace(value: &str, mut offset: usize) -> usize {
    while offset < value.len() {
        if let Some(length) = line_ending_len(value, offset) {
            offset += length;
            continue;
        }
        let byte = value.as_bytes()[offset];
        if !matches!(byte, b' ' | b'\t') {
            break;
        }
        offset += 1;
    }
    offset
}

fn skip_horizontal_whitespace(value: &str, mut offset: usize) -> usize {
    while matches!(value.as_bytes().get(offset), Some(b' ' | b'\t')) {
        offset += 1;
    }
    offset
}

fn json_secret_member_at(value: &str, start: usize) -> Option<JsonSecretMember> {
    let (style, content_start) = opening_quote_at(value, start)?;
    let (closing_start, after_closing) = delimited_string_end(value, content_start, style)?;
    let mut key = decode_json_string(&value[content_start..closing_start]);
    if matches!(style, QuoteStyle::Escaped(_)) {
        for _ in 0..2 {
            let Some(decoded) = key.as_deref().and_then(decode_json_string) else {
                break;
            };
            if key.as_deref() == Some(decoded.as_str()) {
                break;
            }
            key = Some(decoded);
        }
    }
    let colon = skip_json_whitespace(value, after_closing);
    if value.as_bytes().get(colon) != Some(&b':') {
        return None;
    }
    let value_start = skip_json_whitespace(value, colon + 1);
    if key.as_deref().is_some_and(|key| !is_secret_key(key)) {
        return None;
    }
    Some(JsonSecretMember {
        prefix_end: colon + 1,
        value_start,
        quote_style: style,
    })
}

fn opening_quote_at(value: &str, start: usize) -> Option<(QuoteStyle, usize)> {
    let bytes = value.as_bytes();
    if bytes.get(start) == Some(&b'"') {
        return Some((QuoteStyle::Plain, start + 1));
    }
    if bytes.get(start) != Some(&b'\\') {
        return None;
    }
    let mut quote_index = start;
    while bytes.get(quote_index) == Some(&b'\\') {
        quote_index += 1;
    }
    let slash_count = quote_index - start;
    if !slash_count.is_multiple_of(2) && bytes.get(quote_index) == Some(&b'"') {
        Some((QuoteStyle::Escaped(slash_count), quote_index + 1))
    } else {
        None
    }
}

fn delimited_string_end(
    value: &str,
    content_start: usize,
    style: QuoteStyle,
) -> Option<(usize, usize)> {
    for (offset, character) in value[content_start..].char_indices() {
        if matches!(
            character,
            '\r' | '\n' | '\u{0085}' | '\u{2028}' | '\u{2029}'
        ) {
            return None;
        }
        if character != '"' {
            continue;
        }
        let quote_index = content_start + offset;
        let slash_count = backslashes_before(value, quote_index);
        match style {
            QuoteStyle::Plain if slash_count.is_multiple_of(2) => {
                return Some((quote_index, quote_index + 1));
            }
            QuoteStyle::Escaped(expected) if slash_count == expected => {
                return Some((quote_index - slash_count, quote_index + 1));
            }
            _ => {}
        }
    }
    None
}

fn decode_json_string(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut output = String::with_capacity(value.len());
    let mut cursor = 0;
    while cursor < bytes.len() {
        if bytes[cursor] != b'\\' {
            let character = value[cursor..].chars().next()?;
            output.push(character);
            cursor += character.len_utf8();
            continue;
        }
        cursor += 1;
        let escape = *bytes.get(cursor)?;
        cursor += 1;
        match escape {
            b'"' => output.push('"'),
            b'\\' => output.push('\\'),
            b'/' => output.push('/'),
            b'b' => output.push('\u{0008}'),
            b'f' => output.push('\u{000c}'),
            b'n' => output.push('\n'),
            b'r' => output.push('\r'),
            b't' => output.push('\t'),
            b'u' => {
                let first = parse_json_hex4(bytes, cursor)?;
                cursor += 4;
                let codepoint = if (0xd800..=0xdbff).contains(&first) {
                    if bytes.get(cursor..cursor + 2) != Some(b"\\u") {
                        return None;
                    }
                    cursor += 2;
                    let second = parse_json_hex4(bytes, cursor)?;
                    cursor += 4;
                    if !(0xdc00..=0xdfff).contains(&second) {
                        return None;
                    }
                    0x1_0000 + ((u32::from(first) - 0xd800) << 10) + (u32::from(second) - 0xdc00)
                } else if (0xdc00..=0xdfff).contains(&first) {
                    return None;
                } else {
                    u32::from(first)
                };
                output.push(char::from_u32(codepoint)?);
            }
            _ => return None,
        }
    }
    Some(output)
}

fn parse_json_hex4(bytes: &[u8], start: usize) -> Option<u16> {
    let slice = bytes.get(start..start + 4)?;
    let mut value = 0u16;
    for byte in slice {
        value = value.checked_mul(16)?
            + match byte {
                b'0'..=b'9' => u16::from(byte - b'0'),
                b'a'..=b'f' => u16::from(byte - b'a' + 10),
                b'A'..=b'F' => u16::from(byte - b'A' + 10),
                _ => return None,
            };
    }
    Some(value)
}

fn append_json_redaction(output: &mut String, style: QuoteStyle) {
    let slash_count = match style {
        QuoteStyle::Plain => 0,
        QuoteStyle::Escaped(slash_count) => slash_count,
    };
    output.extend(std::iter::repeat_n('\\', slash_count));
    output.push('"');
    output.push_str(REDACTED);
    output.extend(std::iter::repeat_n('\\', slash_count));
    output.push('"');
}

fn json_value_end(value: &str, start: usize, style: QuoteStyle) -> Option<usize> {
    if start >= value.len() {
        return None;
    }
    let end = if let Some((actual_style, content_start)) = opening_quote_at(value, start) {
        if actual_style != style {
            return None;
        }
        delimited_string_end(value, content_start, style).map(|(_, after)| after)?
    } else {
        match value.as_bytes()[start] {
            b'{' | b'[' => json_container_end(value, start, style),
            _ => json_scalar_end(value, start),
        }?
    };
    let delimiter = skip_json_whitespace(value, end);
    if delimiter == value.len()
        || matches!(value.as_bytes().get(delimiter), Some(b',' | b'}' | b']'))
    {
        Some(end)
    } else {
        None
    }
}

fn json_container_end(value: &str, start: usize, style: QuoteStyle) -> Option<usize> {
    let first = *value.as_bytes().get(start)?;
    let mut stack = Vec::with_capacity(MAX_JSON_DEPTH);
    stack.push(if first == b'{' { b'}' } else { b']' });
    let mut cursor = start + 1;
    while cursor < value.len() {
        let byte = value.as_bytes()[cursor];
        if let Some(length) = line_ending_len(value, cursor) {
            cursor += length;
            continue;
        }
        if matches!(byte, b' ' | b'\t') {
            cursor += 1;
            continue;
        }

        if byte == b'"' || byte == b'\\' {
            if let Some((actual_style, content_start)) = opening_quote_at(value, cursor) {
                if actual_style == style {
                    let (_, after) = delimited_string_end(value, content_start, style)?;
                    cursor = after;
                    continue;
                }
                if style != QuoteStyle::Plain {
                    let mut quote_index = cursor;
                    while value.as_bytes().get(quote_index) == Some(&b'\\') {
                        quote_index += 1;
                    }
                    if value.as_bytes().get(quote_index) == Some(&b'"') {
                        cursor = quote_index + 1;
                        continue;
                    }
                }
                return None;
            }
            if byte == b'\\' {
                return None;
            }
            if style != QuoteStyle::Plain {
                return None;
            }
            let (_, after) = delimited_string_end(value, cursor + 1, QuoteStyle::Plain)?;
            cursor = after;
            continue;
        }

        match byte {
            b'{' | b'[' => {
                if stack.len() >= MAX_JSON_DEPTH {
                    return None;
                }
                stack.push(if byte == b'{' { b'}' } else { b']' });
                cursor += 1;
            }
            b'}' | b']' => {
                if stack.pop() != Some(byte) {
                    return None;
                }
                cursor += 1;
                if stack.is_empty() {
                    return Some(cursor);
                }
            }
            _ => {
                let character = value[cursor..].chars().next()?;
                cursor += character.len_utf8();
            }
        }
    }
    None
}

fn json_scalar_end(value: &str, start: usize) -> Option<usize> {
    let mut cursor = start;
    while cursor < value.len() {
        let character = value[cursor..].chars().next()?;
        if line_ending_len(value, cursor).is_some()
            || character.is_ascii_whitespace()
            || matches!(character, ',' | '}' | ']')
        {
            break;
        }
        cursor += character.len_utf8();
    }
    (cursor > start).then_some(cursor)
}

fn field_marker_at(value: &str, start: usize) -> Option<FieldMarker> {
    if start >= value.len() {
        return None;
    }
    if start > 0 {
        let previous = value.as_bytes()[start - 1];
        if is_field_name_byte(previous) && !escaped_line_ending_before(value, start) {
            return None;
        }
    }
    let bytes = value.as_bytes();
    let (key, key_end, quoted_key) = match bytes[start] {
        b'\'' | b'"' => {
            let quote = bytes[start];
            let content_start = start + 1;
            let mut quote_index = content_start;
            let content_end = loop {
                if quote_index >= value.len() || raw_line_ending_len(value, quote_index).is_some() {
                    return None;
                }
                let character = value[quote_index..].chars().next()?;
                if character == char::from(quote)
                    && backslashes_before(value, quote_index).is_multiple_of(2)
                {
                    break quote_index;
                }
                quote_index += character.len_utf8();
            };
            let raw_key = &value[content_start..content_end];
            let key = if quote == b'"' {
                decode_json_string(raw_key).unwrap_or_else(|| raw_key.to_owned())
            } else {
                raw_key.to_owned()
            };
            (key, content_end + 1, true)
        }
        byte if byte.is_ascii_alphanumeric() || byte == b'_' => {
            let mut key_end = start;
            while bytes
                .get(key_end)
                .is_some_and(|byte| is_field_name_byte(*byte))
            {
                key_end += 1;
            }
            (value[start..key_end].to_owned(), key_end, false)
        }
        _ => return None,
    };
    let separator = skip_horizontal_whitespace(value, key_end);
    let separator_byte = *bytes.get(separator)?;
    if !matches!(separator_byte, b':' | b'=') || !is_secret_key(&key) {
        return None;
    }
    let separator_end = if separator_byte == b'=' && bytes.get(separator + 1) == Some(&b'>') {
        separator + 2
    } else {
        separator + 1
    };
    let normalized = normalized_secret_key(&key);
    let kind = if (!quoted_key && separator_byte == b':')
        || normalized.contains("auth")
        || normalized.contains("token")
        || normalized.contains("cookie")
        || normalized.contains("session")
    {
        FieldValueKind::Header
    } else {
        FieldValueKind::Inline
    };
    Some(FieldMarker {
        separator_end,
        value_start: skip_horizontal_whitespace(value, separator_end),
        kind,
    })
}

fn is_field_name_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')
}

fn inline_value_is_empty(value: &str, marker: FieldMarker) -> bool {
    let start = marker.value_start;
    if start >= value.len()
        || value[start..]
            .chars()
            .next()
            .is_some_and(char::is_whitespace)
    {
        return true;
    }
    field_marker_at(value, start).is_some()
}

fn value_starts_with_credential_scheme(value: &str, start: usize) -> bool {
    let bytes = value.as_bytes();
    let mut scheme_end = start;
    while bytes
        .get(scheme_end)
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
    {
        scheme_end += 1;
    }
    if scheme_end == start || !matches!(bytes.get(scheme_end), Some(b' ' | b'\t')) {
        return false;
    }
    let scheme = &value[start..scheme_end];
    [
        "basic",
        "bearer",
        "digest",
        "negotiate",
        "ntlm",
        "aws4-hmac-sha256",
        "scram-sha-1",
        "scram-sha-256",
        "oauth",
    ]
    .iter()
    .any(|known| scheme.eq_ignore_ascii_case(known))
}

fn inline_quoted_value(value: &str, start: usize) -> Option<(usize, usize, usize)> {
    if value.as_bytes().get(start) == Some(&b'\'') {
        for (offset, character) in value[start + 1..].char_indices() {
            if character == '\'' {
                let quote_index = start + 1 + offset;
                if backslashes_before(value, quote_index).is_multiple_of(2) {
                    return Some((start + 1, quote_index, quote_index + 1));
                }
            }
            if matches!(
                character,
                '\r' | '\n' | '\u{0085}' | '\u{2028}' | '\u{2029}'
            ) {
                return None;
            }
        }
        return None;
    }
    let (style, content_start) = opening_quote_at(value, start)?;
    let (closing_start, after) = delimited_string_end(value, content_start, style)?;
    Some((content_start, closing_start, after))
}

fn quoted_value_suffix_is_delimited(value: &str, after_closing: usize) -> bool {
    if after_closing >= value.len() || line_ending_len(value, after_closing).is_some() {
        return true;
    }
    value[after_closing..]
        .chars()
        .next()
        .is_some_and(char::is_whitespace)
}

fn starts_with_quote(value: &str, start: usize) -> bool {
    matches!(value.as_bytes().get(start), Some(b'"' | b'\'' | b'\\'))
}

fn inline_value_end(value: &str, start: usize) -> usize {
    // In unquoted key/value syntax whitespace ends the value, and a semicolon
    // followed by a field-shaped key/value pair begins the next field. Quotes
    // and commas after the value start are data; callers must quote values
    // that contain spaces. Encoded line endings are ambiguous, so fail closed.
    let mut offset = start;
    while offset < value.len() {
        if value.as_bytes()[offset] == b';' && has_key_value_after_semicolon(value, offset) {
            return offset;
        }
        if line_ending_len(value, offset).is_some() {
            if raw_line_ending_len(value, offset).is_some() {
                return offset;
            }
            // A literal escape such as "\\n" inside an unquoted credential
            // is ambiguous: it may represent a new log line. Hide the rest of
            // the input instead of exposing text that could be its suffix.
            return value.len();
        }
        let character = value[offset..].chars().next().unwrap();
        if character.is_whitespace() {
            return offset;
        }
        offset += character.len_utf8();
    }
    value.len()
}

fn has_key_value_after_semicolon(value: &str, semicolon: usize) -> bool {
    let bytes = value.as_bytes();
    let Some(first_after) = bytes.get(semicolon + 1) else {
        return false;
    };
    if !matches!(first_after, b' ' | b'\t') {
        return false;
    }
    let field_start = skip_horizontal_whitespace(value, semicolon + 1);
    let mut field_end = field_start;
    while bytes
        .get(field_end)
        .is_some_and(|byte| is_field_name_byte(*byte))
    {
        field_end += 1;
    }
    if field_end == field_start {
        return false;
    }
    let separator = skip_horizontal_whitespace(value, field_end);
    matches!(bytes.get(separator), Some(b'=' | b':'))
}

fn line_ending_len(value: &str, offset: usize) -> Option<usize> {
    let tail = value.get(offset..)?;
    for sequence in [
        r"\r\n",
        r"\r",
        r"\n",
        r"\u{85}",
        r"\u{0085}",
        r"\u{2028}",
        r"\u{2029}",
    ] {
        if tail.starts_with(sequence) {
            return Some(sequence.len());
        }
    }
    let character = tail.chars().next()?;
    matches!(
        character,
        '\r' | '\n' | '\u{0085}' | '\u{2028}' | '\u{2029}'
    )
    .then_some(character.len_utf8())
}

fn raw_line_ending_len(value: &str, offset: usize) -> Option<usize> {
    let character = value.get(offset..)?.chars().next()?;
    matches!(
        character,
        '\r' | '\n' | '\u{0085}' | '\u{2028}' | '\u{2029}'
    )
    .then_some(character.len_utf8())
}

fn append_escaped_control_boundary_space(
    value: &str,
    value_start: usize,
    cursor: usize,
    output: &mut String,
) {
    let already_sanitized_boundary = value[value_start..cursor]
        .strip_suffix(' ')
        .is_some_and(|prior| prior == REDACTED);
    if already_sanitized_boundary
        || value[cursor..].chars().next().is_some_and(|character| {
            (character.is_control() && character.is_whitespace())
                || matches!(character, '\u{2028}' | '\u{2029}')
        })
    {
        // escape_terminal_controls turns raw control whitespace into printable
        // sequences. This space keeps the next field boundary stable on a
        // second sanitizer pass.
        output.push(' ');
    }
}

fn escaped_line_ending_before(value: &str, offset: usize) -> bool {
    let prefix = &value[..offset];
    [
        r"\t",
        r"\r\n",
        r"\r",
        r"\n",
        r"\u{9}",
        r"\u{85}",
        r"\u{0085}",
        r"\u{2028}",
        r"\u{2029}",
    ]
    .iter()
    .any(|sequence| prefix.ends_with(sequence))
}

fn header_value_end(value: &str, start: usize) -> usize {
    let mut offset = start;
    while offset < value.len() {
        if line_ending_len(value, offset).is_some() {
            if raw_line_ending_len(value, offset).is_some() {
                return offset;
            }
            if value[start..offset]
                .strip_suffix(' ')
                .is_some_and(|prior| prior == REDACTED)
            {
                return offset;
            }
            return value.len();
        }
        offset += value[offset..].chars().next().unwrap().len_utf8();
    }
    value.len()
}

fn backslashes_before(value: &str, quote_index: usize) -> usize {
    let bytes = value.as_bytes();
    let mut cursor = quote_index;
    while cursor > 0 && bytes[cursor - 1] == b'\\' {
        cursor -= 1;
    }
    quote_index - cursor
}

fn validate_identifier(value: &str, uppercase: bool, label: &str) -> Result<(), DiagnosticError> {
    let valid_chars = value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'));
    let valid_first = value
        .bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_alphanumeric());
    let valid_case = if uppercase {
        value.bytes().any(|byte| byte.is_ascii_uppercase())
    } else {
        true
    };
    if value.is_empty()
        || value.len() > MAX_IDENTIFIER_BYTES
        || !valid_chars
        || !valid_first
        || !valid_case
    {
        return Err(DiagnosticError(format!("invalid {label} identifier")));
    }
    Ok(())
}

fn bounded_text(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_owned();
    }
    let suffix = TRUNCATED;
    let target = max_bytes.saturating_sub(suffix.len());
    let mut end = target.min(value.len());
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}{}", &value[..end], suffix)
}

pub fn now_unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or_default()
}

pub trait DiagnosticSink {
    fn write_event(&mut self, event: &DiagnosticEvent) -> io::Result<()>;
}

pub struct HumanReadableSink<W: Write> {
    writer: W,
}

impl<W: Write> HumanReadableSink<W> {
    pub fn new(writer: W) -> Self {
        Self { writer }
    }

    pub fn into_inner(self) -> W {
        self.writer
    }
}

impl<W: Write> DiagnosticSink for HumanReadableSink<W> {
    fn write_event(&mut self, event: &DiagnosticEvent) -> io::Result<()> {
        writeln!(self.writer, "{}", event.render_human())
    }
}

pub struct JsonLinesSink<W: Write> {
    writer: W,
}

impl<W: Write> JsonLinesSink<W> {
    pub fn new(writer: W) -> Self {
        Self { writer }
    }

    pub fn into_inner(self) -> W {
        self.writer
    }
}

impl<W: Write> DiagnosticSink for JsonLinesSink<W> {
    fn write_event(&mut self, event: &DiagnosticEvent) -> io::Result<()> {
        let json = event.to_json().map_err(io::Error::other)?;
        writeln!(self.writer, "{json}")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EvidenceKind {
    Host,
    Target,
    Vm,
    Mocked,
}

impl EvidenceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Host => "HOST",
            Self::Target => "TARGET",
            Self::Vm => "VM",
            Self::Mocked => "MOCKED",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum CheckStatus {
    Pass,
    Fail,
    Skipped,
}

impl CheckStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
            Self::Skipped => "SKIPPED",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OverallStatus {
    Pass,
    Fail,
    NotRun,
}

impl OverallStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
            Self::NotRun => "NOT_RUN",
        }
    }
}

/// Verification text fields use the limited JSON/header/key-value scrubber;
/// this type does not certify arbitrary raw logs or unsupported formats.
/// Reprocessing ambiguous free text may redact more content.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationCheckResult {
    pub id: String,
    pub scope: String,
    pub title: String,
    pub status: CheckStatus,
    pub evidence_kind: EvidenceKind,
    pub failure_class: Option<FailureClass>,
    pub summary: String,
    pub evidence: Vec<String>,
}

impl Serialize for VerificationCheckResult {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let title = redact_inline_secrets(&self.title);
        let summary = redact_inline_secrets(&self.summary);
        let evidence = self
            .evidence
            .iter()
            .map(|item| redact_inline_secrets(item))
            .collect::<Vec<_>>();
        let mut state = serializer.serialize_struct("VerificationCheckResult", 8)?;
        state.serialize_field("id", &self.id)?;
        state.serialize_field("scope", &self.scope)?;
        state.serialize_field("title", &title)?;
        state.serialize_field("status", &self.status)?;
        state.serialize_field("evidence_kind", &self.evidence_kind)?;
        state.serialize_field("failure_class", &self.failure_class)?;
        state.serialize_field("summary", &summary)?;
        state.serialize_field("evidence", &evidence)?;
        state.end()
    }
}

impl VerificationCheckResult {
    pub fn pass(
        id: impl Into<String>,
        scope: impl Into<String>,
        title: impl Into<String>,
        evidence_kind: EvidenceKind,
        summary: impl Into<String>,
        evidence: Vec<String>,
    ) -> Self {
        Self::new(CheckResultDraft {
            id: id.into(),
            scope: scope.into(),
            title: title.into(),
            status: CheckStatus::Pass,
            evidence_kind,
            failure_class: None,
            summary: summary.into(),
            evidence,
        })
    }

    pub fn fail(
        id: impl Into<String>,
        scope: impl Into<String>,
        title: impl Into<String>,
        evidence_kind: EvidenceKind,
        failure_class: FailureClass,
        summary: impl Into<String>,
        evidence: Vec<String>,
    ) -> Self {
        Self::new(CheckResultDraft {
            id: id.into(),
            scope: scope.into(),
            title: title.into(),
            status: CheckStatus::Fail,
            evidence_kind,
            failure_class: Some(failure_class),
            summary: summary.into(),
            evidence,
        })
    }

    pub fn skipped(
        id: impl Into<String>,
        scope: impl Into<String>,
        title: impl Into<String>,
        evidence_kind: EvidenceKind,
        summary: impl Into<String>,
    ) -> Self {
        Self::new(CheckResultDraft {
            id: id.into(),
            scope: scope.into(),
            title: title.into(),
            status: CheckStatus::Skipped,
            evidence_kind,
            failure_class: None,
            summary: summary.into(),
            evidence: Vec::new(),
        })
    }

    fn new(draft: CheckResultDraft) -> Self {
        Self {
            id: draft.id,
            scope: draft.scope,
            title: redact_inline_secrets(&bounded_text(&draft.title, MAX_IDENTIFIER_BYTES * 2)),
            status: draft.status,
            evidence_kind: draft.evidence_kind,
            failure_class: draft.failure_class,
            summary: redact_inline_secrets(&bounded_text(&draft.summary, MAX_MESSAGE_BYTES)),
            evidence: draft
                .evidence
                .into_iter()
                .take(MAX_FIELD_COUNT)
                .map(|item| redact_inline_secrets(&bounded_text(&item, MAX_FIELD_VALUE_BYTES)))
                .collect(),
        }
    }
}

struct CheckResultDraft {
    id: String,
    scope: String,
    title: String,
    status: CheckStatus,
    evidence_kind: EvidenceKind,
    failure_class: Option<FailureClass>,
    summary: String,
    evidence: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationReport {
    pub schema_version: u32,
    pub generated_at_unix_ms: u64,
    pub source_commit: Option<String>,
    pub requested_scope: Option<String>,
    pub outcome: OverallStatus,
    pub checks: Vec<VerificationCheckResult>,
}

impl VerificationReport {
    pub fn new(
        checks: Vec<VerificationCheckResult>,
        requested_scope: Option<String>,
        source_commit: Option<String>,
        generated_at_unix_ms: u64,
    ) -> Self {
        let mut checks = checks;
        checks.sort_by(|left, right| left.id.cmp(&right.id));
        let outcome = overall_status(&checks);
        Self {
            schema_version: VERIFICATION_REPORT_SCHEMA_VERSION,
            generated_at_unix_ms,
            source_commit,
            requested_scope,
            outcome,
            checks,
        }
    }

    pub fn validate(&self) -> Result<(), DiagnosticError> {
        if self.schema_version != VERIFICATION_REPORT_SCHEMA_VERSION {
            return Err(DiagnosticError(format!(
                "unsupported verification report schema version {}",
                self.schema_version
            )));
        }
        if self.source_commit.as_ref().is_some_and(|commit| {
            commit.len() != 40 || !commit.bytes().all(|byte| byte.is_ascii_hexdigit())
        }) {
            return Err(DiagnosticError(
                "verification source commit must be a full hexadecimal SHA-1".into(),
            ));
        }
        if let Some(scope) = &self.requested_scope {
            validate_identifier(scope, false, "requested scope")?;
        }
        let mut ids = BTreeSet::new();
        for check in &self.checks {
            validate_identifier(&check.id, false, "check id")?;
            validate_identifier(&check.scope, false, "check scope")?;
            if !ids.insert(check.id.as_str()) {
                return Err(DiagnosticError(format!(
                    "duplicate verification check id `{}`",
                    check.id
                )));
            }
            if check.status == CheckStatus::Fail && check.failure_class.is_none() {
                return Err(DiagnosticError(format!(
                    "failed check `{}` has no failure class",
                    check.id
                )));
            }
            if check.status != CheckStatus::Fail && check.failure_class.is_some() {
                return Err(DiagnosticError(format!(
                    "non-failing check `{}` carries a failure class",
                    check.id
                )));
            }
        }
        let expected = overall_status(&self.checks);
        if self.outcome != expected {
            return Err(DiagnosticError(format!(
                "verification outcome {} does not match check evidence {}",
                self.outcome.as_str(),
                expected.as_str()
            )));
        }
        Ok(())
    }

    pub fn to_json(&self) -> Result<String, DiagnosticError> {
        self.validate()?;
        serde_json::to_string_pretty(self)
            .map_err(|error| DiagnosticError(format!("serialize verification report: {error}")))
    }

    pub fn from_json(input: &str) -> Result<Self, DiagnosticError> {
        let report: Self = serde_json::from_str(input)
            .map_err(|error| DiagnosticError(format!("parse verification report: {error}")))?;
        report.validate()?;
        Ok(report)
    }

    pub fn render_human(&self) -> String {
        let passed = self
            .checks
            .iter()
            .filter(|check| check.status == CheckStatus::Pass)
            .count();
        let failed = self
            .checks
            .iter()
            .filter(|check| check.status == CheckStatus::Fail)
            .count();
        let skipped = self
            .checks
            .iter()
            .filter(|check| check.status == CheckStatus::Skipped)
            .count();
        let mut lines = vec![format!(
            "{} verification: {} pass, {} fail, {} skipped",
            self.outcome.as_str(),
            passed,
            failed,
            skipped
        )];
        for check in &self.checks {
            let class = check
                .failure_class
                .map(|class| format!(" [{}]", class.as_str()))
                .unwrap_or_default();
            lines.push(format!(
                "{} {} [{}] {}: {}{}",
                check.status.as_str(),
                check.id,
                check.evidence_kind.as_str(),
                redact_inline_secrets(&check.title),
                redact_inline_secrets(&check.summary),
                class
            ));
        }
        lines.join("\n")
    }
}

fn overall_status(checks: &[VerificationCheckResult]) -> OverallStatus {
    if checks.is_empty()
        || checks
            .iter()
            .all(|check| check.status == CheckStatus::Skipped)
    {
        OverallStatus::NotRun
    } else if checks.iter().any(|check| check.status == CheckStatus::Fail) {
        OverallStatus::Fail
    } else {
        OverallStatus::Pass
    }
}

pub struct VerificationContext {
    pub repository_root: PathBuf,
}

pub trait HealthCheck: Send + Sync {
    fn id(&self) -> &str;
    fn scope(&self) -> &str;
    fn run(&self, context: &VerificationContext) -> Vec<VerificationCheckResult>;
}

pub struct ClosureHealthCheck<F> {
    id: String,
    scope: String,
    run: F,
}

impl<F> ClosureHealthCheck<F> {
    pub fn new(id: impl Into<String>, scope: impl Into<String>, run: F) -> Self {
        Self {
            id: id.into(),
            scope: scope.into(),
            run,
        }
    }
}

impl<F> HealthCheck for ClosureHealthCheck<F>
where
    F: for<'context> Fn(&'context VerificationContext) -> Vec<VerificationCheckResult>
        + Send
        + Sync,
{
    fn id(&self) -> &str {
        &self.id
    }

    fn scope(&self) -> &str {
        &self.scope
    }

    fn run(&self, context: &VerificationContext) -> Vec<VerificationCheckResult> {
        (self.run)(context)
    }
}

#[derive(Default)]
pub struct HealthCheckRegistry {
    checks: Vec<Box<dyn HealthCheck>>,
}

impl HealthCheckRegistry {
    pub fn register(&mut self, check: impl HealthCheck + 'static) -> Result<(), DiagnosticError> {
        validate_identifier(check.id(), false, "health check id")?;
        validate_identifier(check.scope(), false, "health check scope")?;
        if self
            .checks
            .iter()
            .any(|existing| existing.id() == check.id())
        {
            return Err(DiagnosticError(format!(
                "duplicate health check id `{}`",
                check.id()
            )));
        }
        self.checks.push(Box::new(check));
        Ok(())
    }

    pub fn execute(
        &self,
        repository_root: impl AsRef<Path>,
        requested_scope: Option<String>,
        source_commit: Option<String>,
    ) -> VerificationReport {
        let context = VerificationContext {
            repository_root: repository_root.as_ref().to_path_buf(),
        };
        let mut results = Vec::new();
        for check in &self.checks {
            if requested_scope
                .as_deref()
                .is_some_and(|scope| scope != check.scope() && scope != check.id())
            {
                continue;
            }
            match catch_unwind(AssertUnwindSafe(|| check.run(&context))) {
                Ok(mut check_results) => results.append(&mut check_results),
                Err(_) => results.push(VerificationCheckResult::fail(
                    check.id(),
                    check.scope(),
                    check.id(),
                    EvidenceKind::Host,
                    FailureClass::Unknown,
                    "health check panicked before producing evidence",
                    Vec::new(),
                )),
            }
        }
        VerificationReport::new(results, requested_scope, source_commit, now_unix_ms())
    }
}

pub trait CrashSink {
    fn persist(&mut self, record: &CrashRecord) -> Result<(), String>;
}

/// Crash recovery hints use the limited diagnostic text scrubber. Arbitrary
/// raw logs and unsupported formats are not certified as secret-free.
/// Reprocessing ambiguous free text may redact more content.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrashRecord {
    schema_version: u32,
    timestamp_unix_ms: u64,
    component: String,
    build_id: Option<String>,
    fatal_event: SafeDiagnosticEvent,
    recent_context: Vec<SafeDiagnosticEvent>,
    #[serde(serialize_with = "serialize_sanitized_recovery_hint")]
    recovery_hint: Option<String>,
}

fn serialize_sanitized_recovery_hint<S>(
    value: &Option<String>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    value
        .as_deref()
        .map(redact_inline_secrets)
        .serialize(serializer)
}

impl CrashRecord {
    pub fn validate(&self) -> Result<(), DiagnosticError> {
        if self.schema_version != DIAGNOSTIC_BUNDLE_SCHEMA_VERSION {
            return Err(DiagnosticError(format!(
                "unsupported crash record schema version {}",
                self.schema_version
            )));
        }
        validate_identifier(&self.component, false, "crash component")?;
        if let Some(build_id) = &self.build_id {
            validate_identifier(build_id, false, "crash build id")?;
        }
        if self.fatal_event.severity != Severity::Fatal
            || self.fatal_event.schema_version != DIAGNOSTIC_EVENT_SCHEMA_VERSION
            || self.recent_context.len() > MAX_FIELD_COUNT
            || self
                .recent_context
                .iter()
                .any(|event| event.schema_version != DIAGNOSTIC_EVENT_SCHEMA_VERSION)
        {
            return Err(DiagnosticError(
                "crash record has invalid fatal event or context".into(),
            ));
        }
        Ok(())
    }

    pub fn to_json(&self) -> Result<String, DiagnosticError> {
        self.validate()?;
        let mut safe = self.clone();
        safe.recovery_hint = safe
            .recovery_hint
            .take()
            .map(|hint| redact_inline_secrets(&hint));
        serde_json::to_string_pretty(&safe)
            .map_err(|error| DiagnosticError(format!("serialize crash record: {error}")))
    }

    pub fn from_json(input: &str) -> Result<Self, DiagnosticError> {
        let mut record: Self = serde_json::from_str(input)
            .map_err(|error| DiagnosticError(format!("parse crash record: {error}")))?;
        record.validate()?;
        record.recovery_hint = record
            .recovery_hint
            .take()
            .map(|hint| redact_inline_secrets(&hint));
        Ok(record)
    }

    pub fn component(&self) -> &str {
        &self.component
    }

    pub fn timestamp_unix_ms(&self) -> u64 {
        self.timestamp_unix_ms
    }

    pub fn fatal_event(&self) -> SafeDiagnosticEvent {
        sanitized_event(&self.fatal_event)
    }

    pub fn recent_context(&self) -> Vec<SafeDiagnosticEvent> {
        self.recent_context.iter().map(sanitized_event).collect()
    }
}

/// A portable, bounded fatal-event contract. The caller supplies persistence;
/// Nagi target/kernel persistence is not implied by this host-side type.
pub struct CrashCapture<S: CrashSink> {
    sink: S,
    recent: Vec<DiagnosticEvent>,
    capacity: usize,
}

impl<S: CrashSink> CrashCapture<S> {
    pub fn new(sink: S, capacity: usize) -> Self {
        Self {
            sink,
            recent: Vec::new(),
            capacity: capacity.clamp(1, MAX_FIELD_COUNT),
        }
    }

    pub fn observe(&mut self, event: DiagnosticEvent) {
        if event.severity == Severity::Fatal {
            return;
        }
        if self.recent.len() == self.capacity {
            self.recent.remove(0);
        }
        self.recent.push(event);
    }

    pub fn capture_fatal(
        &mut self,
        component: impl Into<String>,
        build_id: Option<String>,
        event: DiagnosticEvent,
        recovery_hint: Option<String>,
    ) -> Result<(), String> {
        if event.severity != Severity::Fatal {
            return Err("crash capture requires a FATAL diagnostic event".into());
        }
        let component = component.into();
        validate_identifier(&component, false, "crash component")
            .map_err(|error| error.to_string())?;
        if let Some(build_id) = &build_id {
            validate_identifier(build_id, false, "crash build id")
                .map_err(|error| error.to_string())?;
        }
        let record = CrashRecord {
            schema_version: DIAGNOSTIC_BUNDLE_SCHEMA_VERSION,
            timestamp_unix_ms: event.timestamp_unix_ms,
            component,
            build_id,
            fatal_event: event.safe_record(),
            recent_context: self
                .recent
                .iter()
                .map(DiagnosticEvent::safe_record)
                .collect(),
            recovery_hint: recovery_hint
                .map(|hint| redact_inline_secrets(&bounded_text(&hint, MAX_MESSAGE_BYTES))),
        };
        self.sink.persist(&record)
    }

    pub fn into_sink(self) -> S {
        self.sink
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticsBundle {
    pub schema_version: u32,
    pub generated_at_unix_ms: u64,
    pub source_commit: Option<String>,
    pub host_os: String,
    pub host_arch: String,
    pub verification: VerificationReport,
    pub events: Vec<SafeDiagnosticEvent>,
}

impl DiagnosticsBundle {
    pub fn validate(&self) -> Result<(), DiagnosticError> {
        if self.schema_version != DIAGNOSTIC_BUNDLE_SCHEMA_VERSION {
            return Err(DiagnosticError(format!(
                "unsupported diagnostics bundle schema version {}",
                self.schema_version
            )));
        }
        self.verification.validate()?;
        for event in &self.events {
            if event.schema_version != DIAGNOSTIC_EVENT_SCHEMA_VERSION
                || event.fields.len() > MAX_FIELD_COUNT
                || event.error_chain.len() > MAX_ERROR_CHAIN
            {
                return Err(DiagnosticError(
                    "diagnostics bundle contains an invalid or oversized event".into(),
                ));
            }
            validate_identifier(&event.subsystem, false, "subsystem")?;
            validate_identifier(&event.event_code, true, "event code")?;
        }
        Ok(())
    }

    pub fn to_json(&self) -> Result<String, DiagnosticError> {
        self.validate()?;
        serde_json::to_string_pretty(self)
            .map_err(|error| DiagnosticError(format!("serialize diagnostics bundle: {error}")))
    }

    pub fn from_json(input: &str) -> Result<Self, DiagnosticError> {
        let mut bundle: Self = serde_json::from_str(input)
            .map_err(|error| DiagnosticError(format!("parse diagnostics bundle: {error}")))?;
        bundle.events = bundle.events.iter().map(sanitized_event).collect();
        bundle.validate()?;
        Ok(bundle)
    }

    pub fn render_human(&self) -> String {
        let mut lines = vec![format!(
            "Nagi diagnostics bundle v{} ({}/{})",
            self.schema_version,
            escape_terminal_controls(&self.host_os),
            escape_terminal_controls(&self.host_arch)
        )];
        if let Some(commit) = &self.source_commit {
            lines.push(format!(
                "source commit: {}",
                escape_terminal_controls(commit)
            ));
        }
        lines.push(self.verification.render_human());
        lines.extend(self.events.iter().map(|event| {
            DiagnosticEvent {
                timestamp_unix_ms: event.timestamp_unix_ms,
                severity: event.severity,
                subsystem: event.subsystem.clone(),
                event_code: event.event_code.clone(),
                message_template: event.message_template.clone(),
                operation_id: event.operation_id.clone(),
                error_class: event.error_class,
                source: event.source.as_ref().map(|source| SourceLocation {
                    file: source.file.clone(),
                    line: source.line,
                }),
                fields: event
                    .fields
                    .iter()
                    .map(|field| DiagnosticField {
                        key: field.key.clone(),
                        value: field.value.clone(),
                        privacy: field.privacy,
                    })
                    .collect(),
                error_chain: event
                    .error_chain
                    .iter()
                    .map(|cause| ErrorCause {
                        class: cause.class,
                        code: cause.code.clone(),
                        safe_message: cause.safe_message.clone(),
                    })
                    .collect(),
                recovery_hint: event.recovery_hint.clone(),
            }
            .render_human()
        }));
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[test]
    fn event_json_round_trip_is_bounded_and_redacts_secret_fields() {
        let event = DiagnosticEvent::new(
            Severity::Error,
            "nagi-cli",
            "VERIFY.CHECK_FAILED",
            "check failed",
        )
        .unwrap()
        .with_timestamp(123)
        .with_error_class(FailureClass::HostEnv)
        .with_operation_id("operation-7")
        .unwrap()
        .with_source("tools/nagi-cli/src/main.rs", 17)
        .with_error_cause(ErrorCause {
            class: FailureClass::HostEnv,
            code: "TOOL.NOT_FOUND".into(),
            safe_message: "required host tool is missing".into(),
        })
        .unwrap()
        .with_field("tool", "qemu", PrivacyClass::Public)
        .unwrap()
        .with_field("api_token", "secret-value", PrivacyClass::Secret)
        .unwrap();

        let json = event.to_json().unwrap();
        assert!(json.contains("\"HOST_ENV\""));
        assert!(json.contains(REDACTED));
        assert!(!json.contains("secret-value"));

        let parsed = DiagnosticEvent::from_json(&json).unwrap();
        assert_eq!(parsed.timestamp_unix_ms, 123);
        assert_eq!(parsed.fields[1].value, REDACTED);
        assert_eq!(parsed.to_json().unwrap(), json);
    }

    #[test]
    fn event_json_rejects_unsupported_versions_and_excess_fields() {
        let unsupported = r#"{"schema_version":2,"timestamp_unix_ms":1,"severity":"INFO","subsystem":"test","event_code":"TEST.OK","message_template":"ok","operation_id":null,"error_class":null,"source":null,"fields":[],"error_chain":[],"recovery_hint":null}"#;
        assert!(DiagnosticEvent::from_json(unsupported)
            .unwrap_err()
            .to_string()
            .contains("unsupported"));

        let mut event = DiagnosticEvent::new(Severity::Info, "test", "TEST.OK", "ok").unwrap();
        for index in 0..MAX_FIELD_COUNT {
            event = event
                .with_field(format!("field-{index}"), "value", PrivacyClass::Public)
                .unwrap();
        }
        assert!(event
            .with_field("too-many", "value", PrivacyClass::Public)
            .is_err());
    }

    #[test]
    fn event_rendering_redacts_sensitive_fields_and_inline_credentials() {
        let event = DiagnosticEvent::new(
            Severity::Warn,
            "network",
            "NETWORK.AUTH_FAILED",
            "request failed token=top-secret\nauthorization: Bearer bearer-secret",
        )
        .unwrap()
        .with_field("request_id", "r-7", PrivacyClass::Public)
        .unwrap()
        .with_field("user_note", "private text", PrivacyClass::Sensitive)
        .unwrap();

        let rendered = event.render_human();
        assert!(rendered.contains("request failed token=[REDACTED]"));
        assert!(rendered.contains("authorization:[REDACTED]"));
        assert!(rendered.contains("request_id=r-7"));
        assert!(rendered.contains("user_note=[REDACTED]"));
        assert!(!rendered.contains("top-secret"));
        assert!(!rendered.contains("bearer-secret"));
        assert!(!rendered.contains("private text"));
    }

    #[test]
    fn inline_authorization_credentials_are_redacted_without_crossing_header_boundaries() {
        let cases = [
            ("Authorization: Basic dGVzdDpzZWNyZXQ=", "dGVzdDpzZWNyZXQ="),
            (
                "aUtHoRiZaTiOn:\u{2003}bAsIc\tdW5pY29kZS1zZWNyZXQ=",
                "dW5pY29kZS1zZWNyZXQ=",
            ),
            (
                "Proxy-Authorization: Basic cHJveHktc2VjcmV0",
                "cHJveHktc2VjcmV0",
            ),
            (
                "Authorization: bEaReR bearer-secret-case",
                "bearer-secret-case",
            ),
            (
                "pRoXy-AuThOrIzAtIoN: BeArEr proxy-bearer-secret",
                "proxy-bearer-secret",
            ),
            (
                "Authorization: BearerP bare-scheme-secret",
                "bare-scheme-secret",
            ),
            (
                "Authorization: Digest username=\"safe-user\", response=\"digest-secret\"",
                "digest-secret",
            ),
            (
                "Authorization: AWS4-HMAC-SHA256 Credential=id SignedHeaders=host Signature=aws-secret",
                "aws-secret",
            ),
            (
                "Authorization: Negotiate negotiate-secret",
                "negotiate-secret",
            ),
            (
                "Authorization: Custom unknown-scheme-secret",
                "unknown-scheme-secret",
            ),
        ];

        for (input, credential) in cases {
            let output = redact_inline_secrets(input);
            assert!(
                !output.contains(credential),
                "credential leaked for {input:?}: {output:?}"
            );
            assert!(output.contains(REDACTED), "missing redaction for {input:?}");
        }

        let quoted_json = redact_inline_secrets(
            r#"{"Authorization": "Basic json-synthetic-secret", "note": "safe"}"#,
        );
        assert_eq!(
            quoted_json,
            r#"{"Authorization":"[REDACTED]", "note": "safe"}"#
        );
        assert!(!quoted_json.contains("json-synthetic-secret"));
        assert!(quoted_json.contains("\"note\": \"safe\""));
        serde_json::from_str::<serde_json::Value>(&quoted_json)
            .expect("redacted quoted JSON remains valid JSON");

        let spaced_json = redact_inline_secrets(
            r#"{"Authorization" : "Basic spaced-json-secret", "note":"safe"}"#,
        );
        assert!(!spaced_json.contains("spaced-json-secret"));
        serde_json::from_str::<serde_json::Value>(&spaced_json)
            .expect("JSON key spacing remains valid");

        let digest_json = redact_inline_secrets(
            r#"{"authorization":"Digest username=\"safe\", response=\"digest-json-secret\""}"#,
        );
        assert!(!digest_json.contains("digest-json-secret"));
        serde_json::from_str::<serde_json::Value>(&digest_json)
            .expect("Digest parameters do not break JSON quoting");

        let escaped_json = r#"{\"authorization\":\"Basic escaped-json-secret\"}"#;
        let escaped_safe = redact_inline_secrets(escaped_json);
        assert_eq!(escaped_safe, r#"{\"authorization\":\"[REDACTED]\"}"#);
        assert!(!escaped_safe.contains("escaped-json-secret"));
        assert_eq!(redact_inline_secrets(&escaped_safe), escaped_safe);

        let escaped_spaced = r#"{\"authorization\" : \"Basic escaped-spaced-secret\"}"#;
        let escaped_spaced_safe = redact_inline_secrets(escaped_spaced);
        assert!(!escaped_spaced_safe.contains("escaped-spaced-secret"));
        assert_eq!(
            redact_inline_secrets(&escaped_spaced_safe),
            escaped_spaced_safe
        );

        let escaped_digest = r#"{\"authorization\":\"Digest username=\\\"safe\\\", response=\\\"escaped-digest-secret\\\"\"}"#;
        let escaped_digest_safe = redact_inline_secrets(escaped_digest);
        assert!(!escaped_digest_safe.contains("escaped-digest-secret"));
        assert_eq!(
            redact_inline_secrets(&escaped_digest_safe),
            escaped_digest_safe
        );

        let adjacent_headers = redact_inline_secrets(
            "Authorization: Basic\r\nX-Trace: safe\r\nProxy-Authorization: Basic adjacent-secret",
        );
        assert!(!adjacent_headers.contains("adjacent-secret"));
        assert!(adjacent_headers.contains("X-Trace: safe"));
        assert!(adjacent_headers.contains("\\r\\n"));
        assert!(!adjacent_headers.contains('\r'));
        assert!(!adjacent_headers.contains('\n'));

        let semicolon =
            redact_inline_secrets("Authorization: Basic semicolon-secret; next=auth-tail-secret");
        assert!(!semicolon.contains("semicolon-secret"));
        assert!(!semicolon.contains("auth-tail-secret"));
        let digest_tail =
            redact_inline_secrets("Authorization: Digest response=\"digest-secret\", safe=1");
        assert!(!digest_tail.contains("digest-secret"));
        assert!(!digest_tail.contains("safe=1"));

        let inline_semicolon = redact_inline_secrets("token=inline-secret; next=visible");
        assert!(!inline_semicolon.contains("inline-secret"));
        assert!(!inline_semicolon.contains("next=visible"));

        let semicolon_secret =
            redact_inline_secrets("token=first-secret; sessionId=second-secret; next=visible");
        assert!(!semicolon_secret.contains("first-secret"));
        assert!(!semicolon_secret.contains("second-secret"));
        assert!(!semicolon_secret.contains("next=visible"));

        let cookie_field = redact_inline_secrets("cookie=sid=first-secret; theme=second-secret");
        assert!(!cookie_field.contains("first-secret"));
        assert!(!cookie_field.contains("second-secret"));

        let long_credential = format!(
            "Authorization: Basic bounded-secret\r\npublic={}",
            "y".repeat(MAX_MESSAGE_BYTES * 2)
        );
        let bounded = redact_inline_secrets(&long_credential);
        assert!(bounded.len() <= MAX_MESSAGE_BYTES);
        assert!(bounded.ends_with(TRUNCATED));
    }

    #[test]
    fn secret_json_members_cover_normalized_keys_and_complete_values() {
        let input = r#"{"access_token":"access-synthetic","client_secret":"client-synthetic","refresh_token":"refresh-synthetic","apiKey":"camel-synthetic","X-Api-Key":"header-synthetic","X-Amz-Security-Token":"aws-synthetic","api\u004bey":"unicode-key-synthetic","token":["first","array-tail-synthetic"],"credential":{"user":"safe","pass":"object-tail-synthetic"},"note":"visible"}"#;
        let safe = redact_inline_secrets(input);
        for secret in [
            "access-synthetic",
            "client-synthetic",
            "refresh-synthetic",
            "camel-synthetic",
            "header-synthetic",
            "aws-synthetic",
            "unicode-key-synthetic",
            "first",
            "array-tail-synthetic",
            "object-tail-synthetic",
        ] {
            assert!(!safe.contains(secret), "{secret:?} leaked in {safe:?}");
        }
        assert!(safe.contains("\"note\":\"visible\""), "{safe:?}");
        assert_eq!(redact_inline_secrets(&safe), safe);
        serde_json::from_str::<serde_json::Value>(&safe)
            .expect("redacted object and array values remain valid JSON");

        let escaped_key = r#"{\"api\\u004bey\":\"escaped-key-synthetic\"}"#;
        let escaped_key_safe = redact_inline_secrets(escaped_key);
        assert!(!escaped_key_safe.contains("escaped-key-synthetic"));
        assert_eq!(redact_inline_secrets(&escaped_key_safe), escaped_key_safe);

        let newline =
            redact_inline_secrets("{\"token\":\n  \"newline-synthetic\",\n\"note\":\"visible\"}");
        assert!(!newline.contains("newline-synthetic"));
        assert!(newline.contains("\"note\":\"visible\""));

        for primitive in ["42", "null", "false"] {
            let input = format!("{{\"token\":{primitive},\"note\":\"visible\"}}");
            let once = redact_inline_secrets(&input);
            assert_eq!(redact_inline_secrets(&once), once, "{once:?}");
            serde_json::from_str::<serde_json::Value>(&once)
                .expect("all JSON value kinds become quoted sentinels");
        }
    }

    #[test]
    fn inline_markers_fail_closed_and_auth_fields_consume_the_line() {
        for (input, secrets) in [
            (
                "password= Authorization: Basic empty-auth-synthetic",
                &["empty-auth-synthetic"][..],
            ),
            (
                "token=\tCookie:sid=empty-cookie-synthetic",
                &["empty-cookie-synthetic"][..],
            ),
            (
                "password=Authorization: Basic empty-auth-no-space-synthetic",
                &["empty-auth-no-space-synthetic"][..],
            ),
            (
                "token=Cookie:sid=empty-cookie-no-space-synthetic",
                &["empty-cookie-no-space-synthetic"][..],
            ),
            ("password = spaced-synthetic", &["spaced-synthetic"][..]),
            (
                "token='single quote synthetic' next=visible",
                &["single quote synthetic"][..],
            ),
            (
                "Authorization : Digest response=\"inner-synthetic, safe=1",
                &["inner-synthetic", "safe=1"][..],
            ),
        ] {
            let safe = redact_inline_secrets(input);
            for secret in secrets {
                assert!(!safe.contains(secret), "{secret:?} leaked in {safe:?}");
            }
            assert_eq!(redact_inline_secrets(&safe), safe, "{safe:?}");
        }
        assert!(
            !redact_inline_secrets("token='single quote synthetic' next=visible")
                .contains("next=visible")
        );
    }

    #[test]
    fn authentication_and_cookie_values_are_opaque_through_the_line() {
        for (input, secrets) in [
            (
                "auth=Bearer bearer-scheme-synthetic",
                &["bearer-scheme-synthetic"][..],
            ),
            (
                "token=Basic basic-scheme-synthetic",
                &["basic-scheme-synthetic"][..],
            ),
            (
                "password=Bearer password-scheme-synthetic",
                &["password-scheme-synthetic"][..],
            ),
            (
                "cookie=sid=cookie-synthetic; csrf=csrf-synthetic",
                &["cookie-synthetic", "csrf-synthetic"][..],
            ),
            (
                "'Authorization'=Bearer quoted-auth-synthetic",
                &["quoted-auth-synthetic"][..],
            ),
        ] {
            let safe = redact_inline_secrets(input);
            for secret in secrets {
                assert!(!safe.contains(secret), "{secret:?} leaked in {safe:?}");
            }
            assert!(safe.contains(REDACTED), "{input:?} -> {safe:?}");
            assert_eq!(redact_inline_secrets(&safe), safe, "{safe:?}");
        }
    }

    #[test]
    fn quoted_secret_suffixes_fail_closed_after_the_closing_quote() {
        for (input, secret) in [
            (r#"password="ab"P"#, "P"),
            (r#"password="ab""P"#, "P"),
            ("password='ab''P'", "P"),
            (r#"password="P"'more'"#, "more"),
        ] {
            let safe = redact_inline_secrets(input);
            assert!(!safe.contains(secret), "{input:?} -> {safe:?}");
            assert!(safe.contains(REDACTED), "{input:?} -> {safe:?}");
            assert_eq!(redact_inline_secrets(&safe), safe, "{safe:?}");
        }
    }

    #[test]
    fn leading_comma_and_semicolon_values_are_not_treated_as_empty() {
        for (input, secret) in [
            ("password=;semicolon-synthetic", "semicolon-synthetic"),
            ("password=,comma-synthetic", "comma-synthetic"),
        ] {
            let safe = redact_inline_secrets(input);
            assert!(!safe.contains(secret), "{input:?} -> {safe:?}");
            assert!(safe.contains(REDACTED), "{input:?} -> {safe:?}");
            assert_eq!(redact_inline_secrets(&safe), safe, "{safe:?}");
        }
    }

    #[test]
    fn inline_secrets_consume_quote_comma_and_escaped_suffixes() {
        for (input, secret) in [
            (r#"password=ab"quote-synthetic"#, "quote-synthetic"),
            ("password=ab'single-synthetic", "single-synthetic"),
            ("password=ab,comma-synthetic", "comma-synthetic"),
            (
                r#"password=ab\\"escaped-quote-synthetic"#,
                "escaped-quote-synthetic",
            ),
            (
                r"password=secret-synthetic\\nencoded-suffix-synthetic",
                "encoded-suffix-synthetic",
            ),
        ] {
            let safe = redact_inline_secrets(input);
            assert!(!safe.contains(secret), "{input:?} -> {safe:?}");
            assert!(safe.contains(REDACTED), "{input:?} -> {safe:?}");
            assert_eq!(redact_inline_secrets(&safe), safe, "{safe:?}");
        }
    }

    #[test]
    fn terminal_escape_reprocessing_preserves_tab_delimited_fields() {
        let tabbed = redact_inline_secrets("password=tab-secret-synthetic\tvisible=ok");
        assert!(!tabbed.contains("tab-secret-synthetic"));
        assert!(tabbed.contains("visible=ok"), "{tabbed:?}");
        assert_eq!(redact_inline_secrets(&tabbed), tabbed, "{tabbed:?}");

        let tabbed_sensitive = redact_inline_secrets(
            "password=first-tab-secret-synthetic\tapi.key=second-tab-secret-synthetic",
        );
        assert!(!tabbed_sensitive.contains("first-tab-secret-synthetic"));
        assert!(!tabbed_sensitive.contains("second-tab-secret-synthetic"));
        assert_eq!(
            redact_inline_secrets(&tabbed_sensitive),
            tabbed_sensitive,
            "{tabbed_sensitive:?}"
        );
    }

    #[test]
    fn nested_quoted_digest_header_is_redacted_through_the_line() {
        let nested_digest = redact_inline_secrets(
            r#"msg="Authorization: Digest username="bob", response="nested-digest-synthetic" end"#,
        );
        assert!(
            !nested_digest.contains("nested-digest-synthetic"),
            "{nested_digest:?}"
        );
        assert_eq!(redact_inline_secrets(&nested_digest), nested_digest);
    }

    #[test]
    fn encoded_header_boundary_is_fail_closed_and_idempotent() {
        let encoded_header = redact_inline_secrets(
            r"Authorization: Basic header-secret-synthetic\r\nX-Api-Key: next-header-secret-synthetic",
        );
        assert!(!encoded_header.contains("header-secret-synthetic"));
        assert!(!encoded_header.contains("next-header-secret-synthetic"));
        assert_eq!(redact_inline_secrets(&encoded_header), encoded_header);
    }

    #[test]
    fn quoted_field_keys_arrows_and_secret_aliases_are_redacted() {
        let cases = [
            (
                "{'password': 'python-repr-synthetic', 'safe': 'bob'}",
                "python-repr-synthetic",
            ),
            (
                r#"{"password"=double-key-synthetic}"#,
                "double-key-synthetic",
            ),
            (
                r#"{"password"=>arrow-key-synthetic}"#,
                "arrow-key-synthetic",
            ),
            (
                "password=> spaced-arrow-synthetic",
                "spaced-arrow-synthetic",
            ),
            ("pwd=pwd-synthetic", "pwd-synthetic"),
            ("pass=pass-synthetic", "pass-synthetic"),
            ("auth=auth-synthetic", "auth-synthetic"),
            ("session_id=session-synthetic", "session-synthetic"),
            (
                "sessionId=camel-session-synthetic",
                "camel-session-synthetic",
            ),
            ("PRIVATEKEY=private-synthetic", "private-synthetic"),
            ("api.key=dot-synthetic", "dot-synthetic"),
            ("apikey=api-synthetic", "api-synthetic"),
        ];

        for (input, secret) in cases {
            let safe = redact_inline_secrets(input);
            assert!(!safe.contains(secret), "{input:?} -> {safe:?}");
            assert!(safe.contains(REDACTED), "{input:?} -> {safe:?}");
            assert_eq!(redact_inline_secrets(&safe), safe, "{safe:?}");
        }

        for alias in [
            "pwd",
            "pass",
            "auth",
            "session",
            "PRIVATEKEY",
            "api.key",
            "apikey",
        ] {
            assert!(is_secret_key(alias), "alias not classified: {alias}");
        }
        assert!(is_secret_key("API_KEY"));
        assert!(is_secret_key("X-Api-Key"));
    }

    #[test]
    fn oversized_inputs_are_redacted_before_any_truncation() {
        for size in [36 * 1024, 144 * 1024] {
            let input = format!("public={} token=oversized-synthetic-tail", "x".repeat(size));
            let safe = redact_inline_secrets(&input);
            assert!(!safe.contains("oversized-synthetic-tail"));
            assert_eq!(safe, format!("{REDACTED}{TRUNCATED}"));
        }
    }

    #[test]
    fn malformed_json_secret_values_fail_closed_through_the_remaining_input() {
        for input in [
            r#"{"token":["first","unclosed-synthetic""#,
            r#"{"token":"value-synthetic" trailing-synthetic,"note":"visible"}"#,
            r#"{"token":{"nested":["value-synthetic"]}tail-synthetic}"#,
        ] {
            let safe = redact_inline_secrets(input);
            assert!(!safe.contains("synthetic"), "{safe:?}");
            assert!(safe.contains(REDACTED), "{safe:?}");
            assert_eq!(redact_inline_secrets(&safe), safe, "{safe:?}");
        }
    }

    #[test]
    fn json_container_line_breaks_and_quotes_never_panic_or_expose_secret_values() {
        let separators = [
            "\r\n",
            "\n",
            "\u{0085}",
            "\u{2028}",
            "\u{2029}",
            r"\u0085",
            r"\u2028",
            r"\u2029",
            r"\u{0085}",
            r"\u{2028}",
            r"\u{2029}",
        ];

        for separator in separators {
            let inputs = [
                format!(r#"{{"token":[1,{separator}2,"array-secret-synthetic"],"user":"bob"}}"#),
                format!(
                    r#"{{"token":[1,{separator}{{"label":"quoted \" value-secret-synthetic"}},2],"user":"bob"}}"#
                ),
                format!(
                    r#"{{"token":[1,"escaped \\u2028 and \" quote-secret-synthetic"{separator}2],"user":"bob"}}"#
                ),
            ];

            for input in inputs {
                let safe = std::panic::catch_unwind(|| redact_inline_secrets(&input))
                    .unwrap_or_else(|_| panic!("panic for separator {separator:?} in {input:?}"));
                assert!(safe.contains(REDACTED), "{input:?} -> {safe:?}");
                assert!(!safe.contains("secret-synthetic"), "{safe:?}");
                assert_eq!(redact_inline_secrets(&safe), safe, "{safe:?}");
            }
        }

        let incomplete = r#"{"token":[1,{"label":"unterminated-synthetic""#;
        let incomplete_safe = std::panic::catch_unwind(|| redact_inline_secrets(incomplete))
            .expect("incomplete JSON must not panic");
        assert!(!incomplete_safe.contains("unterminated-synthetic"));

        let too_deep = format!(
            r#"{{"token":{}depth-secret-synthetic{}}}"#,
            "[".repeat(MAX_JSON_DEPTH + 1),
            "]".repeat(MAX_JSON_DEPTH + 1)
        );
        let deep_safe = std::panic::catch_unwind(|| redact_inline_secrets(&too_deep))
            .expect("deep JSON must not panic");
        assert!(!deep_safe.contains("depth-secret-synthetic"));

        let truncated = format!(
            r#"{{"token":"{}truncated-secret-synthetic"}}"#,
            "x".repeat(MAX_MESSAGE_BYTES)
        );
        assert_eq!(
            redact_inline_secrets(&truncated),
            format!("{REDACTED}{TRUNCATED}")
        );
    }

    #[test]
    fn empty_headers_and_unicode_line_boundaries_do_not_swallow_next_fields() {
        for separator in ["\r\n", "\u{0085}", "\u{2028}", "\u{2029}"] {
            let input = format!(
                "Authorization:{separator}Cookie:a=1;sid=cookie-synthetic{separator}X-Api-Token:api-synthetic{separator}X-Trace: safe"
            );
            let safe = redact_inline_secrets(&input);
            assert!(!safe.contains("cookie-synthetic"), "{safe:?}");
            assert!(!safe.contains("api-synthetic"), "{safe:?}");
            assert!(safe.contains("Cookie:[REDACTED]"), "{safe:?}");
            assert!(safe.contains("X-Api-Token:[REDACTED]"), "{safe:?}");
            assert!(safe.contains("X-Trace: safe"), "{safe:?}");
            assert!(!safe.contains('\u{2028}'), "{safe:?}");
            assert!(!safe.contains('\u{2029}'), "{safe:?}");
            assert_eq!(redact_inline_secrets(&safe), safe, "{safe:?}");
        }

        let adjacent_authorization =
            redact_inline_secrets("Authorization:\r\nAuthorization: Bearer next-auth-synthetic");
        assert!(!adjacent_authorization.contains("next-auth-synthetic"));
        assert!(adjacent_authorization.contains("Authorization:[REDACTED]"));
    }

    #[test]
    fn inline_redaction_handles_secret_key_names_and_redacted_prefixes() {
        for (input, secret) in [
            ("secret_key=secret-key-synthetic", "secret-key-synthetic"),
            ("private_key=private-key-synthetic", "private-key-synthetic"),
            ("access_key=access-key-synthetic", "access-key-synthetic"),
        ] {
            let safe = redact_inline_secrets(input);
            assert!(!safe.contains(secret), "{safe:?}");
            assert!(safe.contains(REDACTED), "{safe:?}");
        }

        let suffix = redact_inline_secrets("token=[REDACTED]suffix-synthetic");
        assert!(!suffix.contains("suffix-synthetic"));
        assert_eq!(suffix, "token=[REDACTED]");
        assert_eq!(redact_inline_secrets(&suffix), suffix);

        let cookie = redact_inline_secrets("Cookie:a=1;sid=cookie-synthetic\r\nX-Trace: safe");
        assert!(!cookie.contains("cookie-synthetic"));
        assert!(cookie.contains("X-Trace: safe"));
    }

    #[test]
    fn redaction_preserves_terminal_escaping_and_2048_byte_boundaries() {
        let input = "Authorization: Basic terminal-synthetic\r\nX-Trace: \u{1b}[2J";
        let safe = redact_inline_secrets(input);
        assert!(!safe.contains("terminal-synthetic"));
        assert!(!safe.contains('\u{1b}'));
        assert!(safe.contains("\\u{1b}[2J"));
        assert_eq!(redact_inline_secrets(&safe), safe);

        let unicode_separators = redact_inline_secrets("visible\u{2028}next\u{2029}field");
        assert!(!unicode_separators.contains('\u{2028}'));
        assert!(!unicode_separators.contains('\u{2029}'));
        assert!(unicode_separators.contains("\\u{2028}"));
        assert!(unicode_separators.contains("\\u{2029}"));

        let below = redact_inline_secrets(&"x".repeat(MAX_MESSAGE_BYTES - 1));
        let exact = redact_inline_secrets(&"x".repeat(MAX_MESSAGE_BYTES));
        let over = redact_inline_secrets(&format!("{}é", "x".repeat(MAX_MESSAGE_BYTES - 1)));
        assert_eq!(below.len(), MAX_MESSAGE_BYTES - 1);
        assert_eq!(exact.len(), MAX_MESSAGE_BYTES);
        assert!(over.len() <= MAX_MESSAGE_BYTES);
        assert!(over.ends_with(TRUNCATED));
    }

    #[test]
    fn safe_views_and_json_round_trip_keep_basic_credentials_redacted() {
        let credential = "round-trip-synthetic-secret";
        let event = DiagnosticEvent::new(
            Severity::Error,
            "network",
            "NETWORK.AUTH_FAILED",
            format!("request failed Proxy-Authorization: Basic {credential}"),
        )
        .unwrap();

        let safe_json = serde_json::to_string(&event.safe_view()).unwrap();
        assert!(safe_json.contains(REDACTED));
        assert!(!safe_json.contains(credential));

        let json = event.to_json().unwrap();
        assert!(json.contains(REDACTED));
        assert!(!json.contains(credential));
        let parsed = DiagnosticEvent::from_json(&json).unwrap();
        assert_eq!(parsed.to_json().unwrap(), json);
        assert!(!parsed.render_human().contains(credential));
    }

    #[test]
    fn sanitizer_is_idempotent_across_event_bundle_and_tampered_safe_views() {
        let event_secret = "event-synthetic-secret";
        let field_secret = "field-synthetic-secret";
        let event = DiagnosticEvent::new(
            Severity::Error,
            "network",
            "NETWORK.AUTH_FAILED",
            format!("Authorization: Custom {event_secret}\r\nCookie:a=1;sid={field_secret}"),
        )
        .unwrap()
        .with_field(
            "details",
            format!("token=[REDACTED]{field_secret}"),
            PrivacyClass::Public,
        )
        .unwrap();

        let human = event.render_human();
        assert!(!human.contains(event_secret));
        assert!(!human.contains(field_secret));
        let json = event.to_json().unwrap();
        assert!(!json.contains(event_secret));
        assert!(!json.contains(field_secret));
        let parsed = DiagnosticEvent::from_json(&json).unwrap();
        assert_eq!(parsed.to_json().unwrap(), json);
        assert!(!parsed.render_human().contains(event_secret));

        let mut tampered = event.safe_view();
        tampered.message_template =
            format!("Proxy-Authorization: Bearer {event_secret}\r\nCookie:a=1;sid={field_secret}");
        tampered.fields[0].value = format!("Cookie:a=1;sid={field_secret}");
        let tampered_json = serde_json::to_string(&tampered).unwrap();
        assert!(!tampered_json.contains(event_secret));
        assert!(!tampered_json.contains(field_secret));

        let bundle = DiagnosticsBundle {
            schema_version: DIAGNOSTIC_BUNDLE_SCHEMA_VERSION,
            generated_at_unix_ms: 1,
            source_commit: None,
            host_os: "macos".into(),
            host_arch: "arm64".into(),
            verification: VerificationReport::new(Vec::new(), None, None, 1),
            events: vec![tampered],
        };
        let bundle_json = bundle.to_json().unwrap();
        assert!(!bundle_json.contains(event_secret));
        assert!(!bundle_json.contains(field_secret));
        let parsed_bundle = DiagnosticsBundle::from_json(&bundle_json).unwrap();
        let rendered_bundle = parsed_bundle.render_human();
        assert!(!rendered_bundle.contains(event_secret));
        assert!(!rendered_bundle.contains(field_secret));
    }

    #[test]
    fn verification_and_crash_records_redact_header_values() {
        let verification_secret = "verification-synthetic-secret";
        let check = VerificationCheckResult::fail(
            "auth-check",
            "diagnostics",
            "Authorization: Digest response=\"verification-synthetic-secret\"",
            EvidenceKind::Host,
            FailureClass::Network,
            "X-Api-Token:verification-synthetic-secret",
            vec!["Cookie:a=1;sid=verification-synthetic-secret".into()],
        );
        let report = VerificationReport::new(vec![check], None, None, 1);
        let report_json = report.to_json().unwrap();
        let report_human = report.render_human();
        assert!(!report_json.contains(verification_secret));
        assert!(!report_human.contains(verification_secret));
        assert_eq!(
            VerificationReport::from_json(&report_json)
                .unwrap()
                .to_json()
                .unwrap(),
            report_json
        );

        #[derive(Clone, Default)]
        struct Store(Arc<Mutex<Vec<CrashRecord>>>);
        impl CrashSink for Store {
            fn persist(&mut self, record: &CrashRecord) -> Result<(), String> {
                self.0.lock().unwrap().push(record.clone());
                Ok(())
            }
        }

        let crash_secret = "crash-synthetic-secret";
        let store = Store::default();
        let handle = store.0.clone();
        let mut capture = CrashCapture::new(store, 1);
        capture.observe(
            DiagnosticEvent::new(
                Severity::Error,
                "kernel",
                "KERNEL.ERROR",
                format!("Cookie:a=1;sid={crash_secret}"),
            )
            .unwrap(),
        );
        let fatal = DiagnosticEvent::new(
            Severity::Fatal,
            "kernel",
            "KERNEL.PANIC",
            format!("Authorization: Negotiate {crash_secret}"),
        )
        .unwrap();
        capture
            .capture_fatal(
                "kernel",
                None,
                fatal,
                Some(format!("private_key={crash_secret}")),
            )
            .unwrap();
        let records = handle.lock().unwrap();
        let crash_json = records[0].to_json().unwrap();
        assert!(!crash_json.contains(crash_secret));
        let parsed = CrashRecord::from_json(&crash_json).unwrap();
        assert!(!parsed.fatal_event().message_template.contains(crash_secret));
        assert!(parsed
            .recent_context()
            .iter()
            .all(|event| !event.message_template.contains(crash_secret)));
        assert_eq!(parsed.to_json().unwrap(), crash_json);

        let mut tampered: serde_json::Value = serde_json::from_str(&crash_json).unwrap();
        tampered["recovery_hint"] =
            serde_json::Value::String("Authorization: Basic crash-recovery-synthetic".into());
        let tampered_json = serde_json::to_string(&tampered).unwrap();
        let parsed_tampered = CrashRecord::from_json(&tampered_json).unwrap();
        let sanitized_json = parsed_tampered.to_json().unwrap();
        assert!(!sanitized_json.contains("crash-recovery-synthetic"));
        assert!(!parsed_tampered
            .recovery_hint
            .as_deref()
            .unwrap_or_default()
            .contains("crash-recovery-synthetic"));

        // Direct serde deserialization is also bounded by the public serializer.
        let directly_deserialized: CrashRecord = serde_json::from_str(&tampered_json).unwrap();
        let sanitized_direct = directly_deserialized.to_json().unwrap();
        assert!(!sanitized_direct.contains("crash-recovery-synthetic"));
        let serde_json = serde_json::to_string(&directly_deserialized).unwrap();
        assert!(!serde_json.contains("crash-recovery-synthetic"));
    }

    #[test]
    fn diagnostics_bundle_human_header_escapes_terminal_controls() {
        let bundle = DiagnosticsBundle {
            schema_version: DIAGNOSTIC_BUNDLE_SCHEMA_VERSION,
            generated_at_unix_ms: 1,
            source_commit: Some("unknown\rcommit".into()),
            host_os: "linux\u{1b}[2J".into(),
            host_arch: "x86_64\ninjected".into(),
            verification: VerificationReport::new(Vec::new(), None, None, 1),
            events: Vec::new(),
        };

        let rendered = bundle.render_human();

        assert!(!rendered.contains('\u{1b}'));
        assert!(!rendered.contains('\r'));
        assert_eq!(rendered.matches('\n').count(), 2, "{rendered:?}");
        assert!(rendered.contains("linux\\u{1b}[2J"));
        assert!(rendered.contains("x86_64\\ninjected"));
        assert!(rendered.contains("unknown\\rcommit"));
    }

    #[test]
    fn sinks_share_the_safe_event_contract() {
        let event = DiagnosticEvent::new(Severity::Info, "test", "TEST.EVENT", "ok")
            .unwrap()
            .with_field("password", "dont-print", PrivacyClass::Public)
            .unwrap();
        let mut human = HumanReadableSink::new(Vec::new());
        human.write_event(&event).unwrap();
        let human = String::from_utf8(human.into_inner()).unwrap();
        assert!(human.contains("password=[REDACTED]"));
        assert!(!human.contains("dont-print"));

        let mut json = JsonLinesSink::new(Vec::new());
        json.write_event(&event).unwrap();
        let json = String::from_utf8(json.into_inner()).unwrap();
        assert!(json.contains(REDACTED));
        assert!(!json.contains("dont-print"));
    }

    #[test]
    fn public_safe_event_serializer_redacts_tampered_classified_values() {
        let event = DiagnosticEvent::new(Severity::Info, "test", "TEST.EVENT", "safe")
            .unwrap()
            .with_field("api_token", "original", PrivacyClass::Secret)
            .unwrap();
        let mut safe_view = event.safe_view();
        safe_view.fields[0].value = "injected-secret".into();
        let json = serde_json::to_string(&safe_view).unwrap();
        assert!(json.contains(REDACTED));
        assert!(!json.contains("injected-secret"));
    }

    #[test]
    fn failed_check_does_not_hide_later_successful_checks() {
        let mut registry = HealthCheckRegistry::default();
        registry
            .register(ClosureHealthCheck::new(
                "first",
                "diagnostics",
                |_: &VerificationContext| {
                    vec![VerificationCheckResult::fail(
                        "first-result",
                        "diagnostics",
                        "first check",
                        EvidenceKind::Host,
                        FailureClass::Build,
                        "failed for test",
                        vec!["host test".into()],
                    )]
                },
            ))
            .unwrap();
        registry
            .register(ClosureHealthCheck::new(
                "second",
                "diagnostics",
                |_: &VerificationContext| {
                    vec![VerificationCheckResult::pass(
                        "second-result",
                        "diagnostics",
                        "second check",
                        EvidenceKind::Mocked,
                        "still executed",
                        vec!["mocked test".into()],
                    )]
                },
            ))
            .unwrap();

        let report = registry.execute(".", None, None);
        assert_eq!(report.outcome, OverallStatus::Fail);
        assert_eq!(report.checks.len(), 2);
        assert_eq!(report.checks[0].status, CheckStatus::Fail);
        assert_eq!(report.checks[1].status, CheckStatus::Pass);
        assert!(report.render_human().contains("second-result"));
    }

    #[test]
    fn focused_scope_executes_only_matching_checks() {
        let mut registry = HealthCheckRegistry::default();
        for (id, scope) in [("repo", "repository"), ("diag", "diagnostics")] {
            registry
                .register(ClosureHealthCheck::new(
                    id,
                    scope,
                    move |_: &VerificationContext| {
                        vec![VerificationCheckResult::pass(
                            id,
                            scope,
                            id,
                            EvidenceKind::Host,
                            "ok",
                            Vec::new(),
                        )]
                    },
                ))
                .unwrap();
        }
        let report = registry.execute(".", Some("diagnostics".into()), None);
        assert_eq!(report.checks.len(), 1);
        assert_eq!(report.checks[0].id, "diag");
    }

    #[test]
    fn report_outcome_is_derived_and_corrupt_pass_is_rejected() {
        let failing = VerificationCheckResult::fail(
            "qemu",
            "host",
            "QEMU",
            EvidenceKind::Host,
            FailureClass::HostEnv,
            "not found",
            vec!["doctor check".into()],
        );
        let report = VerificationReport::new(vec![failing], None, None, 456);
        assert_eq!(report.outcome, OverallStatus::Fail);
        let json = report.to_json().unwrap();
        assert_eq!(VerificationReport::from_json(&json).unwrap(), report);

        let mut corrupted_value: serde_json::Value = serde_json::from_str(&json).unwrap();
        corrupted_value["outcome"] = serde_json::Value::String("PASS".into());
        let corrupted = serde_json::to_string(&corrupted_value).unwrap();
        assert!(VerificationReport::from_json(&corrupted)
            .unwrap_err()
            .to_string()
            .contains("does not match"));
    }

    #[test]
    fn deterministic_report_serialization_preserves_host_and_vm_distinction() {
        let checks = vec![
            VerificationCheckResult::pass(
                "vm-boot",
                "m1",
                "QEMU boot",
                EvidenceKind::Vm,
                "guest marker observed",
                vec!["Nagi M1 acceptance PASS".into()],
            ),
            VerificationCheckResult::pass(
                "host-toolchain",
                "host",
                "Host toolchain",
                EvidenceKind::Host,
                "all tools found",
                Vec::new(),
            ),
        ];
        let commit = "0123456789abcdef0123456789abcdef01234567";
        let first = VerificationReport::new(checks.clone(), None, Some(commit.into()), 789);
        let second = VerificationReport::new(checks, None, Some(commit.into()), 789);
        assert_eq!(first.to_json().unwrap(), second.to_json().unwrap());
        assert!(first.render_human().contains("[HOST]"));
        assert!(first.render_human().contains("[VM]"));
    }

    #[test]
    fn crash_capture_keeps_only_bounded_recent_context_and_requires_fatal_event() {
        #[derive(Clone, Default)]
        struct Store(Arc<Mutex<Vec<CrashRecord>>>);
        impl CrashSink for Store {
            fn persist(&mut self, record: &CrashRecord) -> Result<(), String> {
                self.0.lock().unwrap().push(record.clone());
                Ok(())
            }
        }

        let store = Store::default();
        let handle = store.0.clone();
        let mut capture = CrashCapture::new(store, 2);
        for index in 0..3 {
            capture.observe(
                DiagnosticEvent::new(Severity::Info, "kernel", "BOOT.STAGE", "stage")
                    .unwrap()
                    .with_timestamp(index),
            );
        }
        let nonfatal =
            DiagnosticEvent::new(Severity::Error, "kernel", "KERNEL.ERROR", "no").unwrap();
        assert!(capture
            .capture_fatal("kernel", None, nonfatal, None)
            .is_err());

        let fatal = DiagnosticEvent::new(Severity::Fatal, "kernel", "KERNEL.PANIC", "panic")
            .unwrap()
            .with_timestamp(9);
        capture
            .capture_fatal(
                "kernel",
                Some("build-7".into()),
                fatal,
                Some("reboot".into()),
            )
            .unwrap();
        let record_json = handle.lock().unwrap()[0].to_json().unwrap();
        assert_eq!(
            CrashRecord::from_json(&record_json).unwrap().component(),
            "kernel"
        );
        let records = handle.lock().unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].recent_context.len(), 2);
        assert_eq!(records[0].recent_context[0].timestamp_unix_ms, 1);
        assert_eq!(records[0].recent_context[1].timestamp_unix_ms, 2);
        assert_eq!(records[0].schema_version, DIAGNOSTIC_BUNDLE_SCHEMA_VERSION);
    }

    #[test]
    fn malformed_ids_are_rejected() {
        assert!(DiagnosticEvent::new(Severity::Info, "bad subsystem", "TEST.OK", "ok").is_err());
        assert!(DiagnosticEvent::new(Severity::Info, "test", "test.ok", "ok").is_err());
    }
}
