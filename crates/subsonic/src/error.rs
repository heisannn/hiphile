/// Error code from a `status: "failed"` response.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServerErrorCode(u32);

impl ServerErrorCode {
    pub const GENERIC: Self = Self(0);
    pub const MISSING_PARAMETER: Self = Self(10);
    pub const CLIENT_TOO_OLD: Self = Self(20);
    pub const SERVER_TOO_OLD: Self = Self(30);
    pub const WRONG_CREDENTIALS: Self = Self(40);
    pub const TOKEN_AUTH_UNSUPPORTED: Self = Self(41);
    pub const AUTH_MECHANISM_UNSUPPORTED: Self = Self(42);
    pub const CONFLICTING_AUTH: Self = Self(43);
    pub const INVALID_API_KEY: Self = Self(44);
    pub const NOT_AUTHORIZED: Self = Self(50);
    pub const NOT_FOUND: Self = Self(70);

    pub fn new(code: u32) -> Self {
        Self(code)
    }

    pub fn value(self) -> u32 {
        self.0
    }

    /// Whether asking the user for different credentials can fix the error.
    pub fn is_authentication(self) -> bool {
        matches!(self.0, 40..=44)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("the server rejected the request ({}): {message}", code.value())]
    Server {
        code: ServerErrorCode,
        message: String,
    },
    #[error("the response is not a valid OpenSubsonic response: {0}")]
    InvalidResponse(#[from] serde_json::Error),
}
