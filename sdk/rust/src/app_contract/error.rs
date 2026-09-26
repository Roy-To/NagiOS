use super::CorrelationId;

/// Stable, English machine-readable contract error codes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u16)]
pub enum ErrorCode {
    InvalidManifest = 1,
    IncompatibleContractVersion = 2,
    InvalidLifecycleTransition = 3,
    StateUnavailable = 4,
    StateCorrupt = 5,
    StateMigrationFailed = 6,
    StateVersionIncompatible = 7,
    UnsupportedIntent = 8,
    InvalidRoute = 9,
    IpcProtocolMismatch = 10,
    InvalidIpcMessage = 11,
    PermissionDenied = 12,
    RuntimeUnavailable = 13,
    BufferTooSmall = 14,
    AppIdentityCollision = 15,
    AppAlreadyRegistered = 16,
    RegistrationCapacityExceeded = 17,
}

impl ErrorCode {
    pub const fn identifier(self) -> &'static str {
        match self {
            Self::InvalidManifest => "APP_INVALID_MANIFEST",
            Self::IncompatibleContractVersion => "APP_INCOMPATIBLE_CONTRACT_VERSION",
            Self::InvalidLifecycleTransition => "APP_INVALID_LIFECYCLE_TRANSITION",
            Self::StateUnavailable => "APP_STATE_UNAVAILABLE",
            Self::StateCorrupt => "APP_STATE_CORRUPT",
            Self::StateMigrationFailed => "APP_STATE_MIGRATION_FAILED",
            Self::StateVersionIncompatible => "APP_STATE_VERSION_INCOMPATIBLE",
            Self::UnsupportedIntent => "APP_UNSUPPORTED_INTENT",
            Self::InvalidRoute => "APP_INVALID_ROUTE",
            Self::IpcProtocolMismatch => "APP_IPC_PROTOCOL_MISMATCH",
            Self::InvalidIpcMessage => "APP_INVALID_IPC_MESSAGE",
            Self::PermissionDenied => "APP_PERMISSION_DENIED",
            Self::RuntimeUnavailable => "APP_RUNTIME_UNAVAILABLE",
            Self::BufferTooSmall => "APP_BUFFER_TOO_SMALL",
            Self::AppIdentityCollision => "APP_IDENTITY_COLLISION",
            Self::AppAlreadyRegistered => "APP_ALREADY_REGISTERED",
            Self::RegistrationCapacityExceeded => "APP_REGISTRATION_CAPACITY_EXCEEDED",
        }
    }

    pub const fn from_wire(value: u16) -> Option<Self> {
        match value {
            1 => Some(Self::InvalidManifest),
            2 => Some(Self::IncompatibleContractVersion),
            3 => Some(Self::InvalidLifecycleTransition),
            4 => Some(Self::StateUnavailable),
            5 => Some(Self::StateCorrupt),
            6 => Some(Self::StateMigrationFailed),
            7 => Some(Self::StateVersionIncompatible),
            8 => Some(Self::UnsupportedIntent),
            9 => Some(Self::InvalidRoute),
            10 => Some(Self::IpcProtocolMismatch),
            11 => Some(Self::InvalidIpcMessage),
            12 => Some(Self::PermissionDenied),
            13 => Some(Self::RuntimeUnavailable),
            14 => Some(Self::BufferTooSmall),
            15 => Some(Self::AppIdentityCollision),
            16 => Some(Self::AppAlreadyRegistered),
            17 => Some(Self::RegistrationCapacityExceeded),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppError {
    pub code: ErrorCode,
}

impl AppError {
    pub const fn new(code: ErrorCode) -> Self {
        Self { code }
    }
}

/// A transport-ready error payload. Human-readable text is resolved through
/// localization using `message_key`; this envelope contains no localized text.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ErrorEnvelope<'a> {
    pub correlation_id: CorrelationId,
    pub code: ErrorCode,
    pub retryable: bool,
    pub message_key: Option<&'a str>,
}

impl ErrorEnvelope<'_> {
    /// Encode as a bounded binary payload for an IPC `Error` message.
    pub fn encode(&self, output: &mut [u8]) -> Result<usize, AppError> {
        let key = self.message_key.unwrap_or("").as_bytes();
        if key.len() > u16::MAX as usize || core::str::from_utf8(key).is_err() {
            return Err(AppError::new(ErrorCode::InvalidIpcMessage));
        }
        let required = 16 + 2 + 1 + 2 + key.len();
        if output.len() < required {
            return Err(AppError::new(ErrorCode::BufferTooSmall));
        }
        output[..16].copy_from_slice(&self.correlation_id.0);
        output[16..18].copy_from_slice(&(self.code as u16).to_le_bytes());
        output[18] = u8::from(self.retryable);
        output[19..21].copy_from_slice(&(key.len() as u16).to_le_bytes());
        output[21..required].copy_from_slice(key);
        Ok(required)
    }

    pub fn decode(input: &[u8]) -> Result<ErrorEnvelope<'_>, AppError> {
        if input.len() < 21 {
            return Err(AppError::new(ErrorCode::InvalidIpcMessage));
        }
        let correlation_id = CorrelationId(input[..16].try_into().expect("fixed-width slice"));
        let code = ErrorCode::from_wire(u16::from_le_bytes([input[16], input[17]]))
            .ok_or(AppError::new(ErrorCode::InvalidIpcMessage))?;
        let retryable = match input[18] {
            0 => false,
            1 => true,
            _ => return Err(AppError::new(ErrorCode::InvalidIpcMessage)),
        };
        let key_len = usize::from(u16::from_le_bytes([input[19], input[20]]));
        if input.len() != 21 + key_len {
            return Err(AppError::new(ErrorCode::InvalidIpcMessage));
        }
        let key = core::str::from_utf8(&input[21..])
            .map_err(|_| AppError::new(ErrorCode::InvalidIpcMessage))?;
        Ok(ErrorEnvelope {
            correlation_id,
            code,
            retryable,
            message_key: if key.is_empty() { None } else { Some(key) },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{AppError, ErrorCode, ErrorEnvelope};
    use crate::app_contract::CorrelationId;

    #[test]
    fn error_envelope_roundtrips_stable_code_and_localization_key() {
        let original = ErrorEnvelope {
            correlation_id: CorrelationId([7; 16]),
            code: ErrorCode::PermissionDenied,
            retryable: false,
            message_key: Some("errors.permission_denied"),
        };
        let mut bytes = [0; 128];
        let length = original.encode(&mut bytes).unwrap();
        assert_eq!(ErrorEnvelope::decode(&bytes[..length]).unwrap(), original);
        assert_eq!(original.code.identifier(), "APP_PERMISSION_DENIED");
        assert_eq!(ErrorCode::from_wire(999), None);
    }

    #[test]
    fn malformed_error_envelope_is_rejected() {
        let error = ErrorEnvelope::decode(&[0; 20]).unwrap_err();
        assert_eq!(error, AppError::new(ErrorCode::InvalidIpcMessage));
    }
}
