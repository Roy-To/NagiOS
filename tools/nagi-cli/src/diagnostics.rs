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

    pub fn to_json(&self) -> Result<String, DiagnosticError> {
        serde_json::to_string_pretty(&self.safe_view())
            .map_err(|error| DiagnosticError(format!("serialize diagnostic event: {error}")))
    }

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

fn is_secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase().replace('-', "_");
    [
        "password",
        "passwd",
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
    .any(|needle| key.contains(needle))
}

pub(crate) fn escape_terminal_controls(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_control() {
                character.escape_default().to_string()
            } else {
                character.to_string()
            }
        })
        .collect()
}

fn redact_inline_secrets(value: &str) -> String {
    let mut output = String::new();
    let mut cursor = 0;
    let lower = value.to_ascii_lowercase();
    while cursor < value.len() {
        let tail = &lower[cursor..];
        let marker = [
            "authorization:",
            "authorization=",
            "password=",
            "passwd=",
            "token=",
            "secret=",
            "api_key=",
            "api-key=",
            "cookie=",
        ]
        .iter()
        .filter_map(|marker| tail.find(marker).map(|offset| (offset, *marker)))
        .min_by_key(|(offset, _)| *offset);
        let Some((offset, marker)) = marker else {
            output.push_str(&value[cursor..]);
            break;
        };
        let start = cursor + offset;
        output.push_str(&value[cursor..start + marker.len()]);
        let mut value_start = start + marker.len();
        while value[value_start..]
            .chars()
            .next()
            .is_some_and(|character| character.is_whitespace())
        {
            value_start += value[value_start..].chars().next().unwrap().len_utf8();
        }
        if marker.starts_with("authorization") && lower[value_start..].starts_with("bearer ") {
            value_start += "bearer ".len();
        }
        let value_end = value[value_start..]
            .find(|character: char| {
                character.is_whitespace() || character == ',' || character == ';'
            })
            .map(|offset| value_start + offset)
            .unwrap_or(value.len());
        output.push_str(REDACTED);
        cursor = value_end;
    }
    let single_line = escape_terminal_controls(&output);
    bounded_text(&single_line, MAX_MESSAGE_BYTES)
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CrashRecord {
    schema_version: u32,
    timestamp_unix_ms: u64,
    component: String,
    build_id: Option<String>,
    fatal_event: SafeDiagnosticEvent,
    recent_context: Vec<SafeDiagnosticEvent>,
    recovery_hint: Option<String>,
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
        serde_json::to_string_pretty(self)
            .map_err(|error| DiagnosticError(format!("serialize crash record: {error}")))
    }

    pub fn from_json(input: &str) -> Result<Self, DiagnosticError> {
        let record: Self = serde_json::from_str(input)
            .map_err(|error| DiagnosticError(format!("parse crash record: {error}")))?;
        record.validate()?;
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
            "request failed token=top-secret authorization: Bearer bearer-secret",
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
