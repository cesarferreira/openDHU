use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("invalid arguments: {0}")]
    InvalidArgs(String),
    #[error("backend unavailable: {0}")]
    BackendUnavailable(String),
    #[error("device disconnected: {0}")]
    DeviceDisconnected(String),
    #[error("operation unsupported: {0}")]
    Unsupported(String),
    #[error("timed out: {0}")]
    Timeout(String),
    #[error("session conflict: {0}")]
    SessionConflict(String),
    #[error("{0}")]
    Other(String),
}

impl Error {
    /// Rebuild an error that crossed a process boundary as (exit code, message).
    pub fn from_code(code: u8, message: String) -> Self {
        match code {
            2 => Self::InvalidArgs(message),
            3 => Self::BackendUnavailable(message),
            4 => Self::DeviceDisconnected(message),
            5 => Self::Unsupported(message),
            6 => Self::Timeout(message),
            8 => Self::SessionConflict(message),
            _ => Self::Other(message),
        }
    }

    /// The message without the category prefix that `Display` adds.
    pub fn message(&self) -> &str {
        match self {
            Self::InvalidArgs(m)
            | Self::BackendUnavailable(m)
            | Self::DeviceDisconnected(m)
            | Self::Unsupported(m)
            | Self::Timeout(m)
            | Self::SessionConflict(m)
            | Self::Other(m) => m,
        }
    }

    /// Deterministic CLI exit codes, per PRD section 8.2.
    pub fn exit_code(&self) -> u8 {
        match self {
            Self::Other(_) => 1,
            Self::InvalidArgs(_) => 2,
            Self::BackendUnavailable(_) => 3,
            Self::DeviceDisconnected(_) => 4,
            Self::Unsupported(_) => 5,
            Self::Timeout(_) => 6,
            Self::SessionConflict(_) => 8,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_has_no_category_prefix() {
        let e = Error::BackendUnavailable("boom".into());
        assert_eq!(e.message(), "boom");
        assert_eq!(e.to_string(), "backend unavailable: boom");
    }

    #[test]
    fn from_code_round_trips_every_code() {
        for code in [1u8, 2, 3, 4, 5, 6, 8] {
            assert_eq!(Error::from_code(code, "m".into()).exit_code(), code);
        }
        assert_eq!(Error::from_code(99, "m".into()).exit_code(), 1);
    }

    #[test]
    fn exit_codes_match_prd_table() {
        assert_eq!(Error::Other("x".into()).exit_code(), 1);
        assert_eq!(Error::InvalidArgs("x".into()).exit_code(), 2);
        assert_eq!(Error::BackendUnavailable("x".into()).exit_code(), 3);
        assert_eq!(Error::DeviceDisconnected("x".into()).exit_code(), 4);
        assert_eq!(Error::Unsupported("x".into()).exit_code(), 5);
        assert_eq!(Error::Timeout("x".into()).exit_code(), 6);
        assert_eq!(Error::SessionConflict("x".into()).exit_code(), 8);
    }
}
