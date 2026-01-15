use serde::{Deserialize, Serialize};

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash, Serialize, Deserialize)]
#[repr(i32)]
pub enum ErrorCode {
    Ok = 0,
    InvalidArgument = 1,
    NotFound = 2,
    Unsupported = 3,
    PermissionDenied = 4,
    SystemError = 5,
    Unknown = 999,
}

impl ErrorCode {
    #[must_use]
    pub const fn as_i32(self) -> i32 {
        self as i32
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ForgeFfiError {
    pub code: ErrorCode,
    pub message: String,
}

impl ForgeFfiError {
    #[must_use]
    pub fn invalid_argument<M: Into<String>>(message: M) -> Self {
        Self {
            code: ErrorCode::InvalidArgument,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn not_found<M: Into<String>>(message: M) -> Self {
        Self {
            code: ErrorCode::NotFound,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn unsupported<M: Into<String>>(message: M) -> Self {
        Self {
            code: ErrorCode::Unsupported,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn permission_denied<M: Into<String>>(message: M) -> Self {
        Self {
            code: ErrorCode::PermissionDenied,
            message: message.into(),
        }
    }

    #[must_use]
    pub fn system_error<M: Into<String>>(message: M) -> Self {
        Self {
            code: ErrorCode::SystemError,
            message: message.into(),
        }
    }
}

