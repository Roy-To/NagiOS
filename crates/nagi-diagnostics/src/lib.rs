#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::io::{self, Write};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::ser::{Error as _, SerializeStruct};
use serde::{Deserialize, Serialize};

pub const DIAGNOSTIC_EVENT_SCHEMA_VERSION: u32 = 1;
pub const VERIFICATION_REPORT_SCHEMA_VERSION: u32 = 1;
pub const DIAGNOSTIC_BUNDLE_SCHEMA_VERSION: u32 = 1;
pub const MAX_IDENTIFIER_BYTES: usize = 96;
pub const MAX_MESSAGE_BYTES: usize = 2_048;
pub const MAX_FIELD_COUNT: usize = 64;
pub const MAX_FIELD_VALUE_BYTES: usize = 2_048;
pub const MAX_ERROR_CHAIN: usize = 8;
pub const MAX_DIAGNOSTIC_BUFFER_CAPACITY: usize = 4_096;
pub const MAX_SNAPSHOT_EVENTS: usize = 256;
pub const MAX_SNAPSHOT_HEALTH_RECORDS: usize = 128;
pub const MAX_SNAPSHOT_COMPONENTS: usize = 128;
pub const MAX_SNAPSHOT_ENVIRONMENT_FIELDS: usize = 64;
pub const DIAGNOSTIC_SNAPSHOT_SCHEMA_VERSION: u32 = 1;
pub const ERROR_REPORT_SCHEMA_VERSION: u32 = 1;

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
    pub message_id: Option<String>,
    pub correlation_id: Option<String>,
    pub session_id: Option<String>,
    pub component: Option<String>,
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
    #[serde(default)]
    pub message_id: Option<String>,
    #[serde(default)]
    pub correlation_id: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub component: Option<String>,
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

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SafeDiagnosticField {
    pub key: String,
    pub value: String,
    pub privacy: PrivacyClass,
}

impl Serialize for SafeDiagnosticField {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        validate_identifier(&self.key, false, "field key").map_err(S::Error::custom)?;
        if self.value.len() > MAX_FIELD_VALUE_BYTES {
            return Err(S::Error::custom("diagnostic field value exceeds its bound"));
        }
        let mut state = serializer.serialize_struct("SafeDiagnosticField", 3)?;
        state.serialize_field("key", &self.key)?;
        state.serialize_field(
            "value",
            &safe_classified_value(&self.key, &self.value, self.privacy),
        )?;
        state.serialize_field("privacy", &self.privacy)?;
        state.end()
    }
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
        if self.schema_version != DIAGNOSTIC_EVENT_SCHEMA_VERSION {
            return Err(S::Error::custom("unsupported safe event schema version"));
        }
        validate_identifier(&self.subsystem, false, "subsystem").map_err(S::Error::custom)?;
        validate_identifier(&self.event_code, true, "event code").map_err(S::Error::custom)?;
        validate_optional_identifier(self.message_id.as_deref(), "message id")
            .map_err(S::Error::custom)?;
        validate_optional_identifier(self.correlation_id.as_deref(), "correlation id")
            .map_err(S::Error::custom)?;
        validate_optional_identifier(self.session_id.as_deref(), "session id")
            .map_err(S::Error::custom)?;
        validate_optional_identifier(self.component.as_deref(), "component")
            .map_err(S::Error::custom)?;
        validate_optional_identifier(self.operation_id.as_deref(), "operation id")
            .map_err(S::Error::custom)?;
        if self.fields.len() > MAX_FIELD_COUNT || self.error_chain.len() > MAX_ERROR_CHAIN {
            return Err(S::Error::custom(
                "safe event exceeds a bounded collection limit",
            ));
        }
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
        let message_template =
            redact_inline_secrets(&bounded_text(&self.message_template, MAX_MESSAGE_BYTES));
        let recovery_hint = self
            .recovery_hint
            .as_ref()
            .map(|hint| redact_inline_secrets(hint));
        let source = self.source.as_ref().map(|source| SafeSourceLocation {
            file: safe_source_file(&source.file),
            line: source.line,
        });
        let mut state = serializer.serialize_struct("SafeDiagnosticEvent", 16)?;
        state.serialize_field("schema_version", &self.schema_version)?;
        state.serialize_field("timestamp_unix_ms", &self.timestamp_unix_ms)?;
        state.serialize_field("severity", &self.severity)?;
        state.serialize_field("subsystem", &self.subsystem)?;
        state.serialize_field("event_code", &self.event_code)?;
        state.serialize_field("message_template", &message_template)?;
        state.serialize_field("message_id", &self.message_id)?;
        state.serialize_field("correlation_id", &self.correlation_id)?;
        state.serialize_field("session_id", &self.session_id)?;
        state.serialize_field("component", &self.component)?;
        state.serialize_field("operation_id", &self.operation_id)?;
        state.serialize_field("error_class", &self.error_class)?;
        state.serialize_field("source", &source)?;
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
            message_id: None,
            correlation_id: None,
            session_id: None,
            component: None,
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

    /// Stable localization lookup key. It is separate from the event code and
    /// from the English-safe fallback template.
    pub fn with_message_id(
        mut self,
        message_id: impl Into<String>,
    ) -> Result<Self, DiagnosticError> {
        let message_id = message_id.into();
        validate_identifier(&message_id, false, "message id")?;
        self.message_id = Some(message_id);
        Ok(self)
    }

    /// Opaque identifier shared by related operations; callers must never put
    /// user data, paths, or secrets in correlation identifiers.
    pub fn with_correlation_id(
        mut self,
        correlation_id: impl Into<String>,
    ) -> Result<Self, DiagnosticError> {
        self.correlation_id = Some(validated_context_id(
            correlation_id.into(),
            "correlation id",
        )?);
        Ok(self)
    }

    pub fn with_session_id(
        mut self,
        session_id: impl Into<String>,
    ) -> Result<Self, DiagnosticError> {
        self.session_id = Some(validated_context_id(session_id.into(), "session id")?);
        Ok(self)
    }

    pub fn with_component(mut self, component: impl Into<String>) -> Result<Self, DiagnosticError> {
        let component = component.into();
        validate_identifier(&component, false, "component")?;
        self.component = Some(component);
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

    pub fn validate(&self) -> Result<(), DiagnosticError> {
        validate_identifier(&self.subsystem, false, "subsystem")?;
        validate_identifier(&self.event_code, true, "event code")?;
        validate_optional_identifier(self.message_id.as_deref(), "message id")?;
        validate_optional_identifier(self.correlation_id.as_deref(), "correlation id")?;
        validate_optional_identifier(self.session_id.as_deref(), "session id")?;
        validate_optional_identifier(self.component.as_deref(), "component")?;
        validate_optional_identifier(self.operation_id.as_deref(), "operation id")?;
        if self.message_template.len() > MAX_MESSAGE_BYTES
            || self.fields.len() > MAX_FIELD_COUNT
            || self.error_chain.len() > MAX_ERROR_CHAIN
            || self
                .recovery_hint
                .as_ref()
                .is_some_and(|hint| hint.len() > MAX_MESSAGE_BYTES)
        {
            return Err(DiagnosticError(
                "diagnostic event exceeds a bounded field limit".into(),
            ));
        }
        let mut keys = BTreeSet::new();
        for field in &self.fields {
            validate_identifier(&field.key, false, "field key")?;
            if field.value.len() > MAX_FIELD_VALUE_BYTES || !keys.insert(field.key.as_str()) {
                return Err(DiagnosticError(
                    "diagnostic field is oversized or duplicated".into(),
                ));
            }
        }
        for cause in &self.error_chain {
            validate_identifier(&cause.code, true, "error cause code")?;
            if cause.safe_message.len() > MAX_MESSAGE_BYTES {
                return Err(DiagnosticError(
                    "diagnostic error cause is oversized".into(),
                ));
            }
        }
        Ok(())
    }

    pub fn to_json(&self) -> Result<String, DiagnosticError> {
        self.validate()?;
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
        validate_optional_identifier(record.message_id.as_deref(), "message id")?;
        validate_optional_identifier(record.correlation_id.as_deref(), "correlation id")?;
        validate_optional_identifier(record.session_id.as_deref(), "session id")?;
        validate_optional_identifier(record.component.as_deref(), "component")?;
        if record.fields.len() > MAX_FIELD_COUNT || record.error_chain.len() > MAX_ERROR_CHAIN {
            return Err(DiagnosticError(
                "diagnostic event exceeds the bounded field or error-chain limit".into(),
            ));
        }
        if record.message_template.len() > MAX_MESSAGE_BYTES
            || record
                .recovery_hint
                .as_ref()
                .is_some_and(|hint| hint.len() > MAX_MESSAGE_BYTES)
        {
            return Err(DiagnosticError(
                "diagnostic event contains oversized text".into(),
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
            message_id: record.message_id,
            correlation_id: record.correlation_id,
            session_id: record.session_id,
            component: record.component,
            operation_id: record.operation_id,
            error_class: record.error_class,
            source: record.source.map(|source| SourceLocation {
                file: safe_source_file(&source.file),
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
        if let Some(correlation_id) = safe.correlation_id {
            line.push_str(&format!(" correlation={correlation_id}"));
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
            message_id: self.message_id.clone(),
            correlation_id: self.correlation_id.clone(),
            session_id: self.session_id.clone(),
            component: self.component.clone(),
            operation_id: self.operation_id.clone(),
            error_class: self.error_class,
            source: self.source.as_ref().map(|source| SafeSourceLocation {
                file: safe_source_file(&source.file),
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
        message_id: event.message_id.clone(),
        correlation_id: event.correlation_id.clone(),
        session_id: event.session_id.clone(),
        component: event.component.clone(),
        operation_id: event.operation_id.clone(),
        error_class: event.error_class,
        source: event.source.as_ref().map(|source| SafeSourceLocation {
            file: safe_source_file(&source.file),
            line: source.line,
        }),
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

fn diagnostic_event_from_safe(event: &SafeDiagnosticEvent) -> DiagnosticEvent {
    DiagnosticEvent {
        timestamp_unix_ms: event.timestamp_unix_ms,
        severity: event.severity,
        subsystem: event.subsystem.clone(),
        event_code: event.event_code.clone(),
        message_template: event.message_template.clone(),
        message_id: event.message_id.clone(),
        correlation_id: event.correlation_id.clone(),
        session_id: event.session_id.clone(),
        component: event.component.clone(),
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
                value: safe_classified_value(&field.key, &field.value, field.privacy),
                privacy: field.privacy,
            })
            .collect(),
        error_chain: event
            .error_chain
            .iter()
            .map(|cause| ErrorCause {
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
        "path",
        "file_path",
        "document_path",
        "source_path",
        "filename",
    ]
    .iter()
    .any(|needle| key.contains(needle))
}

fn safe_source_file(file: &str) -> String {
    let bytes = file.as_bytes();
    let has_windows_drive_root = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\');
    let is_absolute_path =
        file.starts_with('/') || file.starts_with('\\') || has_windows_drive_root;
    let safe = if is_absolute_path {
        file.trim_end_matches(['/', '\\'])
            .rsplit(['/', '\\'])
            .find(|part| !part.is_empty())
            .unwrap_or("[REDACTED_PATH]")
    } else {
        file
    };
    bounded_text(&redact_inline_secrets(safe), MAX_IDENTIFIER_BYTES * 4)
}

fn validated_context_id(value: String, label: &str) -> Result<String, DiagnosticError> {
    validate_identifier(&value, false, label)?;
    Ok(value)
}

fn validate_optional_identifier(value: Option<&str>, label: &str) -> Result<(), DiagnosticError> {
    if let Some(value) = value {
        validate_identifier(value, false, label)?;
    }
    Ok(())
}

fn redact_inline_secrets(value: &str) -> String {
    let mut output = String::new();
    let mut cursor = 0;
    let mut search_from = 0;
    let lower = value.to_ascii_lowercase();
    while cursor < value.len() {
        let marker = [
            "authorization",
            "password",
            "passwd",
            "token",
            "secret",
            "api_key",
            "api-key",
            "cookie",
        ]
        .iter()
        .filter_map(|marker| {
            lower[search_from..]
                .find(marker)
                .map(|offset| (search_from + offset, *marker))
        })
        .min_by_key(|(offset, _)| *offset);
        let Some((start, marker)) = marker else {
            output.push_str(&value[cursor..]);
            break;
        };
        let Some((prefix_end, value_end)) = inline_secret_bounds(value, &lower, start, marker)
        else {
            search_from = start + marker.len();
            continue;
        };
        output.push_str(&value[cursor..prefix_end]);
        output.push_str(REDACTED);
        cursor = value_end;
        search_from = cursor;
    }
    let single_line = output
        .chars()
        .map(|character| match character {
            '\n' => "\\n".to_owned(),
            '\r' => "\\r".to_owned(),
            '\t' => "\\t".to_owned(),
            control if control.is_control() => "?".to_owned(),
            character => character.to_string(),
        })
        .collect::<String>();
    bounded_text(&single_line, MAX_MESSAGE_BYTES)
}

fn inline_secret_bounds(
    value: &str,
    lower: &str,
    start: usize,
    marker: &str,
) -> Option<(usize, usize)> {
    let mut separator = start + marker.len();
    if matches!(value.as_bytes().get(separator), Some(b'"' | b'\'')) {
        separator += 1;
    }
    separator = skip_whitespace(value, separator);
    if !matches!(value.as_bytes().get(separator), Some(b':' | b'=')) {
        return None;
    }
    let separator_end = separator + 1;
    let mut secret_start = skip_whitespace(value, separator_end);

    // Serialized safe views may be serialized again. Treat our own marker as
    // one complete value so redaction stays idempotent.
    if value[secret_start..].starts_with(REDACTED) {
        return Some((secret_start, secret_start + REDACTED.len()));
    }

    if marker == "authorization"
        && lower[secret_start..].starts_with("bearer")
        && value[secret_start + "bearer".len()..]
            .chars()
            .next()
            .is_some_and(char::is_whitespace)
    {
        secret_start = skip_whitespace(value, secret_start + "bearer".len());
        if let Some(value_end) = unquoted_secret_end(value, secret_start) {
            return Some((separator_end, value_end));
        }
    }

    if let Some(quote @ (b'"' | b'\'')) = value.as_bytes().get(secret_start).copied() {
        let content_start = secret_start + 1;
        let mut escaped = false;
        for (offset, character) in value[content_start..].char_indices() {
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character as u32 == u32::from(quote) {
                return Some((content_start, content_start + offset));
            }
        }
        return Some((content_start, value.len()));
    }

    unquoted_secret_end(value, secret_start).map(|value_end| (secret_start, value_end))
}

fn skip_whitespace(value: &str, mut offset: usize) -> usize {
    while value[offset..]
        .chars()
        .next()
        .is_some_and(char::is_whitespace)
    {
        offset += value[offset..].chars().next().unwrap().len_utf8();
    }
    offset
}

fn unquoted_secret_end(value: &str, start: usize) -> Option<usize> {
    let end = value[start..]
        .find(|character: char| {
            character.is_whitespace() || matches!(character, ',' | ';' | '}' | ']')
        })
        .map(|offset| start + offset)
        .unwrap_or(value.len());
    Some(end)
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
        Self::new(
            id,
            scope,
            title,
            CheckStatus::Pass,
            evidence_kind,
            None,
            summary,
            evidence,
        )
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
        Self::new(
            id,
            scope,
            title,
            CheckStatus::Fail,
            evidence_kind,
            Some(failure_class),
            summary,
            evidence,
        )
    }

    pub fn skipped(
        id: impl Into<String>,
        scope: impl Into<String>,
        title: impl Into<String>,
        evidence_kind: EvidenceKind,
        summary: impl Into<String>,
    ) -> Self {
        Self::new(
            id,
            scope,
            title,
            CheckStatus::Skipped,
            evidence_kind,
            None,
            summary,
            Vec::new(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn new(
        id: impl Into<String>,
        scope: impl Into<String>,
        title: impl Into<String>,
        status: CheckStatus,
        evidence_kind: EvidenceKind,
        failure_class: Option<FailureClass>,
        summary: impl Into<String>,
        evidence: Vec<String>,
    ) -> Self {
        Self {
            id: id.into(),
            scope: scope.into(),
            title: redact_inline_secrets(&bounded_text(&title.into(), MAX_IDENTIFIER_BYTES * 2)),
            status,
            evidence_kind,
            failure_class,
            summary: redact_inline_secrets(&bounded_text(&summary.into(), MAX_MESSAGE_BYTES)),
            evidence: evidence
                .into_iter()
                .take(MAX_FIELD_COUNT)
                .map(|item| redact_inline_secrets(&bounded_text(&item, MAX_FIELD_VALUE_BYTES)))
                .collect(),
        }
    }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrashSinkFailure {
    Rejected,
    Panicked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CrashFlushReport {
    pub persisted: usize,
    pub pending: usize,
    pub failure: Option<CrashSinkFailure>,
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
    recent: VecDeque<SafeDiagnosticEvent>,
    pending: VecDeque<CrashRecord>,
    capacity: usize,
    dropped_records: u64,
}

impl<S: CrashSink> CrashCapture<S> {
    pub fn new(sink: S, capacity: usize) -> Self {
        Self {
            sink,
            recent: VecDeque::new(),
            pending: VecDeque::new(),
            capacity: capacity.clamp(1, MAX_FIELD_COUNT),
            dropped_records: 0,
        }
    }

    pub fn observe(&mut self, event: DiagnosticEvent) {
        if event.severity == Severity::Fatal || event.validate().is_err() {
            return;
        }
        if self.recent.len() == self.capacity {
            self.recent.pop_front();
        }
        self.recent.push_back(event.safe_record());
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
        event
            .validate()
            .map_err(|_| "crash capture received an invalid event".to_owned())?;
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
            recent_context: self.recent.iter().map(sanitized_event).collect(),
            recovery_hint: recovery_hint
                .map(|hint| redact_inline_secrets(&bounded_text(&hint, MAX_MESSAGE_BYTES))),
        };
        if self.pending.len() == self.capacity {
            self.pending.pop_front();
            self.dropped_records = self.dropped_records.saturating_add(1);
        }
        self.pending.push_back(record);
        Ok(())
    }

    pub fn pending_count(&self) -> usize {
        self.pending.len()
    }

    pub fn dropped_records(&self) -> u64 {
        self.dropped_records
    }

    /// Perform persistence away from a critical path. Failed records remain
    /// queued for retry; sink errors and panics are contained.
    pub fn flush_pending(&mut self) -> CrashFlushReport {
        let mut persisted = 0;
        let mut failure = None;
        while let Some(record) = self.pending.front() {
            let result = catch_unwind(AssertUnwindSafe(|| self.sink.persist(record)));
            match result {
                Ok(Ok(())) => {
                    self.pending.pop_front();
                    persisted += 1;
                }
                Ok(Err(_)) => {
                    failure = Some(CrashSinkFailure::Rejected);
                    break;
                }
                Err(_) => {
                    failure = Some(CrashSinkFailure::Panicked);
                    break;
                }
            }
        }
        CrashFlushReport {
            persisted,
            pending: self.pending.len(),
            failure,
        }
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
            self.schema_version, self.host_os, self.host_arch
        )];
        if let Some(commit) = &self.source_commit {
            lines.push(format!("source commit: {commit}"));
        }
        lines.push(self.verification.render_human());
        lines.extend(
            self.events
                .iter()
                .map(|event| diagnostic_event_from_safe(event).render_human()),
        );
        lines.join("\n")
    }
}

/// Bounded in-memory sink intended for tests and volatile recent-event views.
/// It stores the safe event representation, never the caller's raw values.
pub struct MemorySink {
    capacity: usize,
    events: VecDeque<SafeDiagnosticEvent>,
    dropped_events: u64,
}

impl MemorySink {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.min(MAX_DIAGNOSTIC_BUFFER_CAPACITY),
            events: VecDeque::new(),
            dropped_events: 0,
        }
    }

    pub fn events(&self) -> &VecDeque<SafeDiagnosticEvent> {
        &self.events
    }

    pub fn dropped_events(&self) -> u64 {
        self.dropped_events
    }

    pub fn into_events(self) -> Vec<SafeDiagnosticEvent> {
        self.events.into_iter().collect()
    }
}

impl DiagnosticSink for MemorySink {
    fn write_event(&mut self, event: &DiagnosticEvent) -> io::Result<()> {
        if event.validate().is_err() {
            self.dropped_events = self.dropped_events.saturating_add(1);
            return Ok(());
        }
        if self.capacity == 0 {
            self.dropped_events = self.dropped_events.saturating_add(1);
            return Ok(());
        }
        if self.events.len() == self.capacity {
            self.events.pop_front();
            self.dropped_events = self.dropped_events.saturating_add(1);
        }
        self.events.push_back(event.safe_view());
        Ok(())
    }
}

/// Human-readable output for development consoles. Use through `EventBuffer`
/// flushing on a developer/control path, never synchronously on a critical path.
pub type DevelopmentSink<W> = HumanReadableSink<W>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SinkFailure {
    Io(io::ErrorKind),
    Panicked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrainReport {
    pub written: usize,
    pub pending: usize,
    pub failure: Option<SinkFailure>,
}

/// In-memory event queue with a hard upper bound. Recording performs no I/O;
/// draining is explicit so a slow console or unavailable sink cannot hold up
/// the code path that records the event.
pub struct EventBuffer {
    capacity: usize,
    events: VecDeque<SafeDiagnosticEvent>,
    dropped_events: u64,
}

impl EventBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity: capacity.min(MAX_DIAGNOSTIC_BUFFER_CAPACITY),
            events: VecDeque::new(),
            dropped_events: 0,
        }
    }

    pub fn record(&mut self, event: DiagnosticEvent) {
        if event.validate().is_err() {
            self.dropped_events = self.dropped_events.saturating_add(1);
            return;
        }
        if self.capacity == 0 {
            self.dropped_events = self.dropped_events.saturating_add(1);
            return;
        }
        if self.events.len() == self.capacity {
            self.events.pop_front();
            self.dropped_events = self.dropped_events.saturating_add(1);
        }
        self.events.push_back(event.safe_view());
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    pub fn is_empty(&self) -> bool {
        self.events.is_empty()
    }

    pub fn dropped_events(&self) -> u64 {
        self.dropped_events
    }

    pub fn snapshot(&self) -> Vec<SafeDiagnosticEvent> {
        self.events.iter().map(sanitized_event).collect()
    }

    pub fn flush_to(&mut self, sink: &mut impl DiagnosticSink) -> DrainReport {
        let mut written = 0;
        let mut failure = None;
        while let Some(event) = self.events.front() {
            let safe_event = diagnostic_event_from_safe(event);
            let result = catch_unwind(AssertUnwindSafe(|| sink.write_event(&safe_event)));
            match result {
                Ok(Ok(())) => {
                    self.events.pop_front();
                    written += 1;
                }
                Ok(Err(error)) => {
                    failure = Some(SinkFailure::Io(error.kind()));
                    break;
                }
                Err(_) => {
                    failure = Some(SinkFailure::Panicked);
                    break;
                }
            }
        }
        DrainReport {
            written,
            pending: self.events.len(),
            failure,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HealthStatus {
    Healthy,
    Degraded,
    Unavailable,
    Unknown,
}

impl HealthStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "HEALTHY",
            Self::Degraded => "DEGRADED",
            Self::Unavailable => "UNAVAILABLE",
            Self::Unknown => "UNKNOWN",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HealthSnapshot {
    pub subsystem: String,
    pub status: HealthStatus,
    pub reason_code: String,
    pub last_transition_unix_ms: u64,
    pub safe_detail: Option<String>,
}

impl Serialize for HealthSnapshot {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        validate_identifier(&self.subsystem, false, "health subsystem")
            .map_err(S::Error::custom)?;
        validate_identifier(&self.reason_code, true, "health reason code")
            .map_err(S::Error::custom)?;
        let safe_detail = self
            .safe_detail
            .as_ref()
            .map(|detail| redact_inline_secrets(&bounded_text(detail, MAX_MESSAGE_BYTES)));
        let mut state = serializer.serialize_struct("HealthSnapshot", 5)?;
        state.serialize_field("subsystem", &self.subsystem)?;
        state.serialize_field("status", &self.status)?;
        state.serialize_field("reason_code", &self.reason_code)?;
        state.serialize_field("last_transition_unix_ms", &self.last_transition_unix_ms)?;
        state.serialize_field("safe_detail", &safe_detail)?;
        state.end()
    }
}

impl HealthSnapshot {
    fn registered(subsystem: String) -> Self {
        Self {
            subsystem,
            status: HealthStatus::Unknown,
            reason_code: "HEALTH.REGISTERED".into(),
            last_transition_unix_ms: now_unix_ms(),
            safe_detail: None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HealthSummary {
    pub overall: HealthStatus,
    pub healthy: usize,
    pub degraded: usize,
    pub unavailable: usize,
    pub unknown: usize,
}

pub struct HealthRegistration {
    subsystem: String,
    generation: u64,
}

#[derive(Default)]
pub struct HealthRegistry {
    records: BTreeMap<String, HealthSnapshot>,
    generations: BTreeMap<String, u64>,
    next_generation: u64,
}

impl HealthRegistry {
    pub fn register(
        &mut self,
        subsystem: impl Into<String>,
    ) -> Result<HealthRegistration, DiagnosticError> {
        let subsystem = subsystem.into();
        validate_identifier(&subsystem, false, "health subsystem")?;
        if self.records.contains_key(&subsystem) {
            return Err(DiagnosticError(format!(
                "duplicate health subsystem registration `{subsystem}`"
            )));
        }
        if self.records.len() >= MAX_SNAPSHOT_HEALTH_RECORDS {
            return Err(DiagnosticError("health registry capacity exceeded".into()));
        }
        self.next_generation = self
            .next_generation
            .checked_add(1)
            .ok_or_else(|| DiagnosticError("health registration generation exhausted".into()))?;
        let generation = self.next_generation;
        self.records.insert(
            subsystem.clone(),
            HealthSnapshot::registered(subsystem.clone()),
        );
        self.generations.insert(subsystem.clone(), generation);
        Ok(HealthRegistration {
            subsystem,
            generation,
        })
    }

    pub fn transition(
        &mut self,
        registration: &HealthRegistration,
        status: HealthStatus,
        reason_code: impl Into<String>,
        safe_detail: Option<String>,
    ) -> Result<(), DiagnosticError> {
        self.transition_at(
            registration,
            status,
            reason_code,
            safe_detail,
            now_unix_ms(),
        )
    }

    pub fn transition_at(
        &mut self,
        registration: &HealthRegistration,
        status: HealthStatus,
        reason_code: impl Into<String>,
        safe_detail: Option<String>,
        timestamp_unix_ms: u64,
    ) -> Result<(), DiagnosticError> {
        let reason_code = reason_code.into();
        validate_identifier(&reason_code, true, "health reason code")?;
        let current_generation = self.generations.get(&registration.subsystem);
        if current_generation != Some(&registration.generation) {
            return Err(DiagnosticError("stale health registration".into()));
        }
        let record = self
            .records
            .get_mut(&registration.subsystem)
            .ok_or_else(|| DiagnosticError("health subsystem is not registered".into()))?;
        if record.status != status || record.reason_code != reason_code {
            record.last_transition_unix_ms = timestamp_unix_ms;
        }
        record.status = status;
        record.reason_code = reason_code;
        record.safe_detail = safe_detail
            .map(|detail| redact_inline_secrets(&bounded_text(&detail, MAX_MESSAGE_BYTES)));
        Ok(())
    }

    pub fn remove(
        &mut self,
        registration: &HealthRegistration,
    ) -> Result<HealthSnapshot, DiagnosticError> {
        if self.generations.get(&registration.subsystem) != Some(&registration.generation) {
            return Err(DiagnosticError("stale health registration".into()));
        }
        self.generations.remove(&registration.subsystem);
        self.records
            .remove(&registration.subsystem)
            .ok_or_else(|| DiagnosticError("health subsystem is not registered".into()))
    }

    pub fn get(&self, subsystem: &str) -> Option<&HealthSnapshot> {
        self.records.get(subsystem)
    }

    pub fn records(&self) -> Vec<HealthSnapshot> {
        self.records.values().cloned().collect()
    }

    pub fn summary(&self) -> HealthSummary {
        summarize_health(self.records.values().map(|record| record.status))
    }
}

fn summarize_health(statuses: impl IntoIterator<Item = HealthStatus>) -> HealthSummary {
    let mut summary = HealthSummary {
        overall: HealthStatus::Unknown,
        healthy: 0,
        degraded: 0,
        unavailable: 0,
        unknown: 0,
    };
    for status in statuses {
        match status {
            HealthStatus::Healthy => summary.healthy += 1,
            HealthStatus::Degraded => summary.degraded += 1,
            HealthStatus::Unavailable => summary.unavailable += 1,
            HealthStatus::Unknown => summary.unknown += 1,
        }
    }
    summary.overall = if summary.unavailable > 0 {
        HealthStatus::Unavailable
    } else if summary.degraded > 0 {
        HealthStatus::Degraded
    } else if summary.unknown > 0 || summary.healthy + summary.degraded + summary.unavailable == 0 {
        HealthStatus::Unknown
    } else {
        HealthStatus::Healthy
    };
    summary
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BuildMetadata {
    pub os_version: String,
    pub build_id: String,
    pub source_commit: Option<String>,
}

impl Serialize for BuildMetadata {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.validate().map_err(S::Error::custom)?;
        let mut state = serializer.serialize_struct("BuildMetadata", 3)?;
        state.serialize_field("os_version", &self.os_version)?;
        state.serialize_field("build_id", &self.build_id)?;
        state.serialize_field("source_commit", &self.source_commit)?;
        state.end()
    }
}

impl BuildMetadata {
    pub fn new(
        os_version: impl Into<String>,
        build_id: impl Into<String>,
        source_commit: Option<&str>,
    ) -> Result<Self, DiagnosticError> {
        let os_version = os_version.into();
        let build_id = build_id.into();
        validate_identifier(&os_version, false, "OS version")?;
        validate_identifier(&build_id, false, "build id")?;
        let source_commit = source_commit.map(str::to_owned);
        if source_commit.as_ref().is_some_and(|commit| {
            commit.len() != 40 || !commit.bytes().all(|byte| byte.is_ascii_hexdigit())
        }) {
            return Err(DiagnosticError(
                "source commit must be a full hexadecimal SHA-1".into(),
            ));
        }
        Ok(Self {
            os_version,
            build_id,
            source_commit,
        })
    }

    pub fn validate(&self) -> Result<(), DiagnosticError> {
        validate_identifier(&self.os_version, false, "OS version")?;
        validate_identifier(&self.build_id, false, "build id")?;
        if self.source_commit.as_ref().is_some_and(|commit| {
            commit.len() != 40 || !commit.bytes().all(|byte| byte.is_ascii_hexdigit())
        }) {
            return Err(DiagnosticError(
                "source commit must be a full hexadecimal SHA-1".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorReport {
    pub schema_version: u32,
    pub timestamp_unix_ms: u64,
    pub process: String,
    pub component: String,
    pub failure_category: FailureClass,
    pub stable_error_code: String,
    pub correlation_id: Option<String>,
    pub build: Option<BuildMetadata>,
    pub safe_context: Vec<SafeDiagnosticEvent>,
    pub backtrace_reference: Option<String>,
}

impl Serialize for ErrorReport {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.validate().map_err(S::Error::custom)?;
        let mut state = serializer.serialize_struct("ErrorReport", 10)?;
        state.serialize_field("schema_version", &self.schema_version)?;
        state.serialize_field("timestamp_unix_ms", &self.timestamp_unix_ms)?;
        state.serialize_field("process", &self.process)?;
        state.serialize_field("component", &self.component)?;
        state.serialize_field("failure_category", &self.failure_category)?;
        state.serialize_field("stable_error_code", &self.stable_error_code)?;
        state.serialize_field("correlation_id", &self.correlation_id)?;
        state.serialize_field("build", &self.build)?;
        state.serialize_field("safe_context", &self.safe_context)?;
        state.serialize_field("backtrace_reference", &self.backtrace_reference)?;
        state.end()
    }
}

impl ErrorReport {
    pub fn from_event(
        process: impl Into<String>,
        component: impl Into<String>,
        event: DiagnosticEvent,
        build: Option<BuildMetadata>,
        backtrace_reference: Option<&str>,
        context: Vec<DiagnosticEvent>,
    ) -> Result<Self, DiagnosticError> {
        event.validate()?;
        if let Some(build) = &build {
            build.validate()?;
        }
        for item in &context {
            item.validate()?;
        }
        let process = process.into();
        let component = component.into();
        validate_identifier(&process, false, "error process")?;
        validate_identifier(&component, false, "error component")?;
        let backtrace_reference = backtrace_reference.map(str::to_owned);
        validate_optional_identifier(backtrace_reference.as_deref(), "backtrace reference")?;
        let mut safe_context = context
            .iter()
            .rev()
            .take(MAX_ERROR_CONTEXT.saturating_sub(1))
            .rev()
            .map(DiagnosticEvent::safe_record)
            .collect::<Vec<_>>();
        let failure_category = event.error_class.unwrap_or(FailureClass::Unknown);
        let stable_error_code = event.event_code.clone();
        let correlation_id = event.correlation_id.clone();
        let timestamp_unix_ms = event.timestamp_unix_ms;
        safe_context.push(event.safe_record());
        Ok(Self {
            schema_version: ERROR_REPORT_SCHEMA_VERSION,
            timestamp_unix_ms,
            process,
            component,
            failure_category,
            stable_error_code,
            correlation_id,
            build,
            safe_context,
            backtrace_reference,
        })
    }

    pub fn validate(&self) -> Result<(), DiagnosticError> {
        if self.schema_version != ERROR_REPORT_SCHEMA_VERSION {
            return Err(DiagnosticError(
                "unsupported error report schema version".into(),
            ));
        }
        validate_identifier(&self.process, false, "error process")?;
        validate_identifier(&self.component, false, "error component")?;
        validate_identifier(&self.stable_error_code, true, "stable error code")?;
        validate_optional_identifier(self.correlation_id.as_deref(), "correlation id")?;
        validate_optional_identifier(self.backtrace_reference.as_deref(), "backtrace reference")?;
        if let Some(build) = &self.build {
            build.validate()?;
        }
        if self.safe_context.len() > MAX_ERROR_CONTEXT
            || self.safe_context.iter().any(|event| {
                event.schema_version != DIAGNOSTIC_EVENT_SCHEMA_VERSION
                    || event.fields.len() > MAX_FIELD_COUNT
                    || event.error_chain.len() > MAX_ERROR_CHAIN
            })
        {
            return Err(DiagnosticError(
                "error report context is invalid or oversized".into(),
            ));
        }
        Ok(())
    }

    pub fn to_json(&self) -> Result<String, DiagnosticError> {
        self.validate()?;
        serde_json::to_string_pretty(self)
            .map_err(|error| DiagnosticError(format!("serialize error report: {error}")))
    }

    pub fn from_json(input: &str) -> Result<Self, DiagnosticError> {
        let mut report: Self = serde_json::from_str(input)
            .map_err(|error| DiagnosticError(format!("parse error report: {error}")))?;
        report.safe_context = report.safe_context.iter().map(sanitized_event).collect();
        report.validate()?;
        Ok(report)
    }
}

const MAX_ERROR_CONTEXT: usize = 64;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticSnapshot {
    pub schema_version: u32,
    pub generated_at_unix_ms: u64,
    pub build: Option<BuildMetadata>,
    pub health_summary: HealthSummary,
    pub health: Vec<HealthSnapshot>,
    pub enabled_components: Vec<String>,
    pub environment: Vec<SafeDiagnosticField>,
    pub recent_events: Vec<SafeDiagnosticEvent>,
    pub dropped_events: u64,
}

impl Serialize for DiagnosticSnapshot {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        self.validate().map_err(S::Error::custom)?;
        let mut state = serializer.serialize_struct("DiagnosticSnapshot", 9)?;
        state.serialize_field("schema_version", &self.schema_version)?;
        state.serialize_field("generated_at_unix_ms", &self.generated_at_unix_ms)?;
        state.serialize_field("build", &self.build)?;
        state.serialize_field("health_summary", &self.health_summary)?;
        state.serialize_field("health", &self.health)?;
        state.serialize_field("enabled_components", &self.enabled_components)?;
        state.serialize_field("environment", &self.environment)?;
        state.serialize_field("recent_events", &self.recent_events)?;
        state.serialize_field("dropped_events", &self.dropped_events)?;
        state.end()
    }
}

impl Serialize for HealthSummary {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut state = serializer.serialize_struct("HealthSummary", 5)?;
        state.serialize_field("overall", &self.overall)?;
        state.serialize_field("healthy", &self.healthy)?;
        state.serialize_field("degraded", &self.degraded)?;
        state.serialize_field("unavailable", &self.unavailable)?;
        state.serialize_field("unknown", &self.unknown)?;
        state.end()
    }
}

impl<'de> Deserialize<'de> for HealthSummary {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Record {
            overall: HealthStatus,
            healthy: usize,
            degraded: usize,
            unavailable: usize,
            unknown: usize,
        }
        let record = Record::deserialize(deserializer)?;
        Ok(Self {
            overall: record.overall,
            healthy: record.healthy,
            degraded: record.degraded,
            unavailable: record.unavailable,
            unknown: record.unknown,
        })
    }
}

impl DiagnosticSnapshot {
    pub fn new(build: Option<BuildMetadata>) -> Self {
        Self {
            schema_version: DIAGNOSTIC_SNAPSHOT_SCHEMA_VERSION,
            generated_at_unix_ms: now_unix_ms(),
            build,
            health_summary: HealthSummary {
                overall: HealthStatus::Unknown,
                healthy: 0,
                degraded: 0,
                unavailable: 0,
                unknown: 0,
            },
            health: Vec::new(),
            enabled_components: Vec::new(),
            environment: Vec::new(),
            recent_events: Vec::new(),
            dropped_events: 0,
        }
    }

    pub fn include_health(&mut self, registry: &HealthRegistry) {
        self.health = registry.records();
        self.health_summary = registry.summary();
    }

    pub fn add_enabled_component(
        &mut self,
        component: impl Into<String>,
    ) -> Result<(), DiagnosticError> {
        let component = component.into();
        validate_identifier(&component, false, "enabled component")?;
        if self
            .enabled_components
            .iter()
            .any(|existing| existing == &component)
        {
            return Err(DiagnosticError(format!(
                "duplicate enabled component `{component}`"
            )));
        }
        if self.enabled_components.len() >= MAX_SNAPSHOT_COMPONENTS {
            return Err(DiagnosticError("snapshot component limit exceeded".into()));
        }
        self.enabled_components.push(component);
        self.enabled_components.sort();
        Ok(())
    }

    pub fn add_environment_field(
        &mut self,
        key: impl Into<String>,
        value: impl Into<String>,
        privacy: PrivacyClass,
    ) -> Result<(), DiagnosticError> {
        if self.environment.len() >= MAX_SNAPSHOT_ENVIRONMENT_FIELDS {
            return Err(DiagnosticError(
                "snapshot environment field limit exceeded".into(),
            ));
        }
        let key = key.into();
        validate_identifier(&key, false, "environment key")?;
        if self.environment.iter().any(|field| field.key == key) {
            return Err(DiagnosticError(format!(
                "duplicate environment field `{key}`"
            )));
        }
        let value = value.into();
        self.environment.push(SafeDiagnosticField {
            key: key.clone(),
            value: safe_classified_value(&key, &value, privacy),
            privacy,
        });
        self.environment
            .sort_by(|left, right| left.key.cmp(&right.key));
        Ok(())
    }

    pub fn add_event(&mut self, event: DiagnosticEvent) {
        if self.recent_events.len() == MAX_SNAPSHOT_EVENTS {
            self.recent_events.remove(0);
            self.dropped_events = self.dropped_events.saturating_add(1);
        }
        self.recent_events.push(event.safe_view());
    }

    pub fn validate(&self) -> Result<(), DiagnosticError> {
        if self.schema_version != DIAGNOSTIC_SNAPSHOT_SCHEMA_VERSION {
            return Err(DiagnosticError(
                "unsupported diagnostic snapshot schema version".into(),
            ));
        }
        if self.health.len() > MAX_SNAPSHOT_HEALTH_RECORDS
            || self.enabled_components.len() > MAX_SNAPSHOT_COMPONENTS
            || self.environment.len() > MAX_SNAPSHOT_ENVIRONMENT_FIELDS
            || self.recent_events.len() > MAX_SNAPSHOT_EVENTS
        {
            return Err(DiagnosticError(
                "diagnostic snapshot exceeds a bounded collection limit".into(),
            ));
        }
        if let Some(build) = &self.build {
            build.validate()?;
        }
        let mut components = BTreeSet::new();
        for component in &self.enabled_components {
            validate_identifier(component, false, "enabled component")?;
            if !components.insert(component.as_str()) {
                return Err(DiagnosticError(
                    "diagnostic snapshot has duplicate components".into(),
                ));
            }
        }
        let mut subsystems = BTreeSet::new();
        for health in &self.health {
            validate_identifier(&health.subsystem, false, "health subsystem")?;
            validate_identifier(&health.reason_code, true, "health reason code")?;
            if !subsystems.insert(health.subsystem.as_str()) {
                return Err(DiagnosticError(
                    "diagnostic snapshot has duplicate health records".into(),
                ));
            }
        }
        if self.health_summary != summarize_health(self.health.iter().map(|health| health.status)) {
            return Err(DiagnosticError(
                "diagnostic snapshot health summary does not match its records".into(),
            ));
        }
        let mut keys = BTreeSet::new();
        for field in &self.environment {
            validate_identifier(&field.key, false, "environment key")?;
            if !keys.insert(field.key.as_str()) {
                return Err(DiagnosticError(
                    "diagnostic snapshot has duplicate environment keys".into(),
                ));
            }
        }
        for event in &self.recent_events {
            if event.schema_version != DIAGNOSTIC_EVENT_SCHEMA_VERSION
                || event.fields.len() > MAX_FIELD_COUNT
                || event.error_chain.len() > MAX_ERROR_CHAIN
            {
                return Err(DiagnosticError(
                    "diagnostic snapshot has an unsupported event version".into(),
                ));
            }
        }
        Ok(())
    }

    pub fn to_json(&self) -> Result<String, DiagnosticError> {
        self.validate()?;
        serde_json::to_string_pretty(self)
            .map_err(|error| DiagnosticError(format!("serialize diagnostic snapshot: {error}")))
    }

    pub fn from_json(input: &str) -> Result<Self, DiagnosticError> {
        let mut snapshot: Self = serde_json::from_str(input)
            .map_err(|error| DiagnosticError(format!("parse diagnostic snapshot: {error}")))?;
        snapshot.recent_events = snapshot.recent_events.iter().map(sanitized_event).collect();
        for field in &mut snapshot.environment {
            field.value = safe_classified_value(&field.key, &field.value, field.privacy);
        }
        snapshot.validate()?;
        Ok(snapshot)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "SCREAMING_SNAKE_CASE", deny_unknown_fields)]
pub enum SemanticActivity {
    AiChangedFile {
        action_id: String,
        correlation_id: String,
        object_id: String,
    },
    AppLaunched {
        action_id: String,
        correlation_id: String,
        app_id: String,
    },
    WaybackRestored {
        action_id: String,
        correlation_id: String,
        checkpoint_id: String,
        object_id: String,
    },
}

impl SemanticActivity {
    fn validate(&self) -> Result<(), DiagnosticError> {
        let mut ids = Vec::new();
        match self {
            Self::AiChangedFile {
                action_id,
                correlation_id,
                object_id,
            } => {
                ids.extend([
                    ("action id", action_id),
                    ("correlation id", correlation_id),
                    ("object id", object_id),
                ]);
            }
            Self::AppLaunched {
                action_id,
                correlation_id,
                app_id,
            } => {
                ids.extend([
                    ("action id", action_id),
                    ("correlation id", correlation_id),
                    ("app id", app_id),
                ]);
            }
            Self::WaybackRestored {
                action_id,
                correlation_id,
                checkpoint_id,
                object_id,
            } => {
                ids.extend([
                    ("action id", action_id),
                    ("correlation id", correlation_id),
                    ("checkpoint id", checkpoint_id),
                    ("object id", object_id),
                ]);
            }
        }
        for (label, value) in ids {
            validate_identifier(value, false, label)?;
        }
        Ok(())
    }
}

pub trait ActivityLedgerSink {
    fn promote(&mut self, activity: &SemanticActivity) -> Result<(), String>;
}

/// Explicit adapter for meaningful actions. Raw diagnostic events have no
/// conversion path into this adapter and are never promoted automatically.
pub struct ActivityBridge<S: ActivityLedgerSink> {
    sink: S,
    promoted_count: u64,
}

impl<S: ActivityLedgerSink> ActivityBridge<S> {
    pub fn new(sink: S) -> Self {
        Self {
            sink,
            promoted_count: 0,
        }
    }

    pub fn promote(&mut self, activity: SemanticActivity) -> Result<(), DiagnosticError> {
        activity.validate()?;
        self.sink
            .promote(&activity)
            .map_err(|_| DiagnosticError("Activity Ledger promotion failed".into()))?;
        self.promoted_count = self.promoted_count.saturating_add(1);
        Ok(())
    }

    pub fn promoted_count(&self) -> u64 {
        self.promoted_count
    }

    pub fn sink(&self) -> &S {
        &self.sink
    }

    pub fn into_sink(self) -> S {
        self.sink
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

        let legacy_v1 = r#"{"schema_version":1,"timestamp_unix_ms":1,"severity":"INFO","subsystem":"test","event_code":"TEST.OK","message_template":"ok","operation_id":null,"error_class":null,"source":null,"fields":[],"error_chain":[],"recovery_hint":null}"#;
        let legacy = DiagnosticEvent::from_json(legacy_v1).unwrap();
        assert_eq!(legacy.correlation_id, None);
        assert!(legacy
            .to_json()
            .unwrap()
            .contains("\"correlation_id\": null"));

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
    fn inline_redaction_covers_json_and_colon_delimited_credentials() {
        let event = DiagnosticEvent::new(
            Severity::Warn,
            "network",
            "NETWORK.AUTH_FAILED",
            r#"request {"password":"json secret", "nested":{"token": "json-token"}} password: plain-secret api-key = api-secret authorization: Bearer auth-secret safe=value"#,
        )
        .unwrap();

        let rendered = event.render_human();
        assert!(rendered.contains(r#"{"password":"[REDACTED]", "nested":{"token": "[REDACTED]"}}"#));
        assert!(rendered.contains("password: [REDACTED]"));
        assert!(rendered.contains("api-key = [REDACTED]"));
        assert!(rendered.contains("authorization:[REDACTED]"));
        assert!(rendered.contains("safe=value"));
        for secret in [
            "json secret",
            "json-token",
            "plain-secret",
            "api-secret",
            "auth-secret",
        ] {
            assert!(
                !rendered.contains(secret),
                "leaked `{secret}` in {rendered}"
            );
        }
    }

    #[test]
    fn path_values_and_absolute_source_paths_are_redacted_by_default() {
        for source_path in [
            "/Users/alice/private/documents/notes.txt",
            r"C:\Users\alice\private\documents\notes.txt",
            r"\\server\users\alice\private\documents\notes.txt",
        ] {
            let event = DiagnosticEvent::new(
                Severity::Info,
                "filesystem",
                "FS.OPEN_FAILED",
                "open failed",
            )
            .unwrap()
            .with_source(source_path, 9)
            .with_field(
                "document_path",
                "/Users/alice/private/notes.txt",
                PrivacyClass::Public,
            )
            .unwrap();
            let json = event.to_json().unwrap();
            assert!(json.contains("notes.txt"));
            assert!(json.contains(REDACTED));
            assert!(!json.contains("alice"), "leaked path in {json}");
            assert!(!json.contains("private/documents"));
            assert!(!json.contains(r"private\documents"));
        }
    }

    #[test]
    fn sinks_share_the_safe_event_contract() {
        let event = DiagnosticEvent::new(Severity::Info, "test", "TEST.EVENT", "ok")
            .unwrap()
            .with_field("password", "dont-print", PrivacyClass::Public)
            .unwrap();
        let mut human = DevelopmentSink::new(Vec::new());
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
    fn safe_event_serializer_rejects_malformed_machine_identity() {
        let event = DiagnosticEvent::new(Severity::Info, "test", "TEST.EVENT", "safe").unwrap();
        let mut safe = event.safe_view();
        safe.message_id = Some("token=private".into());
        assert!(serde_json::to_string(&safe).is_err());
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
                    .with_timestamp(index)
                    .with_field("password", "raw-secret", PrivacyClass::Public)
                    .unwrap(),
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
        assert_eq!(capture.pending_count(), 1);
        let flush = capture.flush_pending();
        assert_eq!(flush.persisted, 1);
        assert_eq!(flush.pending, 0);
        let record_json = handle.lock().unwrap()[0].to_json().unwrap();
        assert!(!record_json.contains("raw-secret"));
        assert!(record_json.contains(REDACTED));
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
    fn crash_persistence_failure_is_contained_and_record_remains_retryable() {
        struct FailingCrashSink;
        impl CrashSink for FailingCrashSink {
            fn persist(&mut self, _: &CrashRecord) -> Result<(), String> {
                Err("storage failed token=private".into())
            }
        }

        let mut capture = CrashCapture::new(FailingCrashSink, 2);
        capture
            .capture_fatal(
                "kernel",
                None,
                DiagnosticEvent::new(Severity::Fatal, "kernel", "KERNEL.PANIC", "panic").unwrap(),
                None,
            )
            .unwrap();
        let report = capture.flush_pending();
        assert_eq!(report.failure, Some(CrashSinkFailure::Rejected));
        assert_eq!(report.pending, 1);
    }

    #[test]
    fn malformed_ids_are_rejected() {
        assert!(DiagnosticEvent::new(Severity::Info, "bad subsystem", "TEST.OK", "ok").is_err());
        assert!(DiagnosticEvent::new(Severity::Info, "test", "test.ok", "ok").is_err());
    }

    #[test]
    fn event_keeps_correlation_session_component_and_localization_identity_separate() {
        let event = DiagnosticEvent::new(
            Severity::Error,
            "model-runtime",
            "MODEL.RUNTIME.LOAD_FAILED",
            "Model load failed",
        )
        .unwrap()
        .with_message_id("diagnostics.model.load_failed")
        .unwrap()
        .with_component("granite-provider")
        .unwrap()
        .with_correlation_id("corr-7")
        .unwrap()
        .with_session_id("session-4")
        .unwrap()
        .with_operation_id("load-2")
        .unwrap();

        let json = event.to_json().unwrap();
        assert!(json.contains("MODEL.RUNTIME.LOAD_FAILED"));
        assert!(json.contains("diagnostics.model.load_failed"));
        assert!(json.contains("corr-7"));
        assert!(json.contains("session-4"));
        assert!(json.contains("granite-provider"));
        assert_eq!(DiagnosticEvent::from_json(&json).unwrap(), event);
    }

    #[test]
    fn memory_sink_is_bounded_and_keeps_only_sanitized_events() {
        let mut sink = MemorySink::new(2);
        for timestamp in 0..3 {
            let event = DiagnosticEvent::new(Severity::Info, "test", "TEST.EVENT", "ok")
                .unwrap()
                .with_timestamp(timestamp)
                .with_field("password", "must-not-remain", PrivacyClass::Public)
                .unwrap();
            sink.write_event(&event).unwrap();
        }

        assert_eq!(sink.events().len(), 2);
        assert_eq!(sink.events()[0].timestamp_unix_ms, 1);
        assert_eq!(sink.dropped_events(), 1);
        assert_eq!(sink.events()[1].fields[0].value, REDACTED);
    }

    #[test]
    fn bounded_buffer_drops_oldest_and_sink_failure_is_contained() {
        struct FailingSink;
        impl DiagnosticSink for FailingSink {
            fn write_event(&mut self, _: &DiagnosticEvent) -> io::Result<()> {
                Err(io::Error::other("sink contains token=private"))
            }
        }

        let mut buffer = EventBuffer::new(1);
        for timestamp in 1..=2 {
            buffer.record(
                DiagnosticEvent::new(Severity::Info, "test", "TEST.EVENT", "ok")
                    .unwrap()
                    .with_timestamp(timestamp),
            );
        }
        assert_eq!(buffer.dropped_events(), 1);
        assert_eq!(buffer.snapshot()[0].timestamp_unix_ms, 2);

        let report = buffer.flush_to(&mut FailingSink);
        assert_eq!(report.written, 0);
        assert_eq!(report.pending, 1);
        assert!(matches!(report.failure, Some(SinkFailure::Io(_))));
        assert_eq!(buffer.len(), 1);
    }

    #[test]
    fn diagnostics_buffer_contains_sink_panics_without_losing_the_event() {
        struct PanickingSink;
        impl DiagnosticSink for PanickingSink {
            fn write_event(&mut self, _: &DiagnosticEvent) -> io::Result<()> {
                panic!("sink panic")
            }
        }

        let mut buffer = EventBuffer::new(2);
        buffer.record(
            DiagnosticEvent::new(Severity::Error, "runtime", "RUNTIME.FAILED", "failed").unwrap(),
        );
        let report = buffer.flush_to(&mut PanickingSink);
        assert!(matches!(report.failure, Some(SinkFailure::Panicked)));
        assert_eq!(report.pending, 1);
    }

    #[test]
    fn health_registry_registers_and_transitions_multiple_subsystems() {
        let mut registry = HealthRegistry::default();
        let kernel = registry.register("kernel").unwrap();
        let model = registry.register("model-runtime").unwrap();
        registry
            .transition_at(&kernel, HealthStatus::Healthy, "KERNEL.READY", None, 10)
            .unwrap();
        registry
            .transition_at(
                &model,
                HealthStatus::Degraded,
                "MODEL.RUNTIME.FALLBACK",
                Some("using reduced mode token=private".into()),
                20,
            )
            .unwrap();
        registry
            .transition_at(
                &kernel,
                HealthStatus::Healthy,
                "KERNEL.READY",
                Some("still ready".into()),
                99,
            )
            .unwrap();

        assert_eq!(registry.records().len(), 2);
        assert_eq!(
            registry.get("kernel").unwrap().status,
            HealthStatus::Healthy
        );
        assert_eq!(registry.get("kernel").unwrap().last_transition_unix_ms, 10);
        assert_eq!(registry.summary().overall, HealthStatus::Degraded);
        assert_eq!(
            registry
                .get("model-runtime")
                .unwrap()
                .last_transition_unix_ms,
            20
        );
        let json = serde_json::to_string(&registry.records()).unwrap();
        assert!(json.contains(REDACTED));
        assert!(!json.contains("private"));
    }

    #[test]
    fn health_registry_rejects_duplicate_registration_and_stale_handles() {
        let mut registry = HealthRegistry::default();
        let old = registry.register("filesystem").unwrap();
        assert!(registry.register("filesystem").is_err());
        assert_eq!(registry.remove(&old).unwrap().subsystem, "filesystem");
        let current = registry.register("filesystem").unwrap();
        assert!(registry
            .transition(&old, HealthStatus::Unavailable, "FS.OFFLINE", None)
            .is_err());
        registry
            .transition(&current, HealthStatus::Unknown, "FS.PROVIDER_MISSING", None)
            .unwrap();
        assert_eq!(
            registry.get("filesystem").unwrap().reason_code,
            "FS.PROVIDER_MISSING"
        );
    }

    #[test]
    fn unavailable_provider_is_representable_without_localized_identity() {
        let mut registry = HealthRegistry::default();
        let provider = registry.register("model-runtime").unwrap();
        registry
            .transition(
                &provider,
                HealthStatus::Unavailable,
                "MODEL.RUNTIME.PROVIDER_UNAVAILABLE",
                Some("provider not registered".into()),
            )
            .unwrap();
        let status = registry.get("model-runtime").unwrap();
        assert_eq!(status.status, HealthStatus::Unavailable);
        assert_eq!(status.reason_code, "MODEL.RUNTIME.PROVIDER_UNAVAILABLE");
    }

    #[test]
    fn error_report_contains_stable_safe_context_and_build_metadata() {
        let event = DiagnosticEvent::new(
            Severity::Fatal,
            "kernel",
            "KERNEL.PANIC",
            "fatal token=private",
        )
        .unwrap()
        .with_error_class(FailureClass::Runtime)
        .with_correlation_id("corr-12")
        .unwrap();
        let report = ErrorReport::from_event(
            "nagi-init",
            "kernel",
            event,
            Some(
                BuildMetadata::new(
                    "0.2.0",
                    "build-19",
                    Some("0123456789abcdef0123456789abcdef01234567"),
                )
                .unwrap(),
            ),
            Some("trace-4"),
            Vec::new(),
        )
        .unwrap();
        let json = report.to_json().unwrap();
        assert!(json.contains("KERNEL.PANIC"));
        assert!(json.contains("RUNTIME"));
        assert!(json.contains("nagi-init"));
        assert!(json.contains("trace-4"));
        assert!(json.contains(REDACTED));
        assert!(!json.contains("private"));
        assert_eq!(ErrorReport::from_json(&json).unwrap(), report);
    }

    #[test]
    fn diagnostic_snapshot_sanitizes_environment_and_includes_health_components_and_events() {
        let mut registry = HealthRegistry::default();
        let runtime = registry.register("model-runtime").unwrap();
        registry
            .transition(&runtime, HealthStatus::Healthy, "MODEL.RUNTIME.READY", None)
            .unwrap();
        let mut snapshot =
            DiagnosticSnapshot::new(Some(BuildMetadata::new("0.2.0", "build-19", None).unwrap()));
        snapshot.include_health(&registry);
        snapshot.add_enabled_component("granite-provider").unwrap();
        snapshot
            .add_environment_field("HOME", "/Users/private-user", PrivacyClass::Sensitive)
            .unwrap();
        snapshot.add_event(
            DiagnosticEvent::new(
                Severity::Info,
                "model-runtime",
                "MODEL.RUNTIME.READY",
                "ready",
            )
            .unwrap(),
        );

        let json = snapshot.to_json().unwrap();
        assert!(json.contains("granite-provider"));
        assert!(json.contains("MODEL.RUNTIME.READY"));
        assert!(json.contains("HEALTHY"));
        assert!(json.contains(REDACTED));
        assert!(!json.contains("private-user"));
        assert_eq!(DiagnosticSnapshot::from_json(&json).unwrap(), snapshot);
    }

    #[test]
    fn malformed_snapshot_and_event_payloads_are_rejected() {
        let mut event = DiagnosticEvent::new(Severity::Info, "test", "TEST.EVENT", "ok").unwrap();
        event.fields.extend([
            DiagnosticField {
                key: "duplicate".into(),
                value: "one".into(),
                privacy: PrivacyClass::Public,
            },
            DiagnosticField {
                key: "duplicate".into(),
                value: "two".into(),
                privacy: PrivacyClass::Public,
            },
        ]);
        assert!(event.to_json().is_err());

        let malformed = r#"{"schema_version":1,"generated_at_unix_ms":1,"build":null,"health_summary":{"overall":"HEALTHY","healthy":1,"degraded":0,"unavailable":0,"unknown":0},"health":[],"enabled_components":[],"environment":[],"recent_events":[],"dropped_events":0}"#;
        assert!(DiagnosticSnapshot::from_json(malformed)
            .unwrap_err()
            .to_string()
            .contains("summary"));
    }

    #[test]
    fn activity_bridge_only_accepts_typed_explicit_semantic_promotions() {
        #[derive(Default)]
        struct Ledger(Vec<SemanticActivity>);
        impl ActivityLedgerSink for Ledger {
            fn promote(&mut self, activity: &SemanticActivity) -> Result<(), String> {
                self.0.push(activity.clone());
                Ok(())
            }
        }

        let candidate = SemanticActivity::AiChangedFile {
            action_id: "ai-edit-3".into(),
            correlation_id: "corr-8".into(),
            object_id: "object-12".into(),
        };
        let mut bridge = ActivityBridge::new(Ledger::default());
        bridge.promote(candidate.clone()).unwrap();
        assert_eq!(bridge.sink().0, vec![candidate]);
        assert_eq!(bridge.promoted_count(), 1);
    }

    #[test]
    fn failed_activity_promotion_is_reported_without_changing_event_storage() {
        struct FailingLedger;
        impl ActivityLedgerSink for FailingLedger {
            fn promote(&mut self, _: &SemanticActivity) -> Result<(), String> {
                Err("ledger unavailable".into())
            }
        }
        let mut bridge = ActivityBridge::new(FailingLedger);
        let activity = SemanticActivity::AppLaunched {
            action_id: "launch-5".into(),
            correlation_id: "corr-8".into(),
            app_id: "notes".into(),
        };
        assert!(bridge.promote(activity).is_err());
        assert_eq!(bridge.promoted_count(), 0);
    }
}
