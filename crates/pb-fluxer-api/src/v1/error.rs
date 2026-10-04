//! Errors.

/// What kind of failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// Fluxer could not be reached (retried by the caller's policy).
    Network,
    /// The token is not (or no longer) valid.
    Unauthorized,
    /// Missing permission, role hierarchy, a person who does not take direct messages …
    Forbidden,
    NotFound,
    /// The request was refused as given (see `code`).
    BadRequest,
    /// Fluxer had a problem.
    Server,
    /// The gateway is not connected right now.
    NotConnected,
    /// A newer request of the same kind replaced this one before it was sent.
    Superseded,
}

/// A failed Fluxer call. `code` is Fluxer's error code (`USER_NOT_IN_VOICE`, `CANNOT_SEND_MESSAGES_TO_USER`,
/// `MISSING_PERMISSIONS`, `TWO_FACTOR_REQUIRED` …) when it sent one.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{}{}: {message}", status.map(|s| format!("HTTP {s} ")).unwrap_or_default(), code.as_deref().unwrap_or(kind_name(*kind)))]
pub struct FluxerError {
    pub kind: ErrorKind,
    pub status: Option<u16>,
    pub code: Option<String>,
    pub message: String,
}

fn kind_name(k: ErrorKind) -> &'static str {
    match k {
        ErrorKind::Network => "network",
        ErrorKind::Unauthorized => "unauthorized",
        ErrorKind::Forbidden => "forbidden",
        ErrorKind::NotFound => "not found",
        ErrorKind::BadRequest => "bad request",
        ErrorKind::Server => "server error",
        ErrorKind::NotConnected => "not connected",
        ErrorKind::Superseded => "superseded",
    }
}

impl FluxerError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> FluxerError {
        FluxerError {
            kind,
            status: None,
            code: None,
            message: message.into(),
        }
    }

    pub fn is_code(&self, code: &str) -> bool {
        self.code.as_deref() == Some(code)
    }
}

/// Why logging in failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LoginError {
    #[error("Fluxer rejected the bot token")]
    TokenRejected,
    #[error("the instance address is wrong: {0}")]
    BadInstance(String),
    /// Worth trying again later.
    #[error("Fluxer cannot be reached: {0}")]
    Unreachable(String),
    #[error("the gateway refused the login: {0}")]
    Refused(String),
}
