use std::fmt;

/// A user-facing failure. The CLI prints `오류: <message>` and exits 1.
#[derive(Debug)]
pub struct Error(pub String);

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    pub fn msg(message: impl Into<String>) -> Self {
        Error(message.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

macro_rules! from_error {
    ($($source:ty),*) => {$(
        impl From<$source> for Error {
            fn from(err: $source) -> Self {
                Error(err.to_string())
            }
        }
    )*};
}

from_error!(std::io::Error, serde_json::Error, serde_yaml_ng::Error, ureq::Error, regex::Error);

/// Early return with a formatted [`Error`].
#[macro_export]
macro_rules! bail {
    ($($arg:tt)*) => {
        return Err($crate::error::Error(format!($($arg)*)))
    };
}
