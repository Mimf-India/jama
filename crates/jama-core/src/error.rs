//! A small hand-rolled error type. `jama-core` has no `unwrap()` outside
//! tests, so every fallible operation returns a [`JamaError`] describing
//! what happened, where, and — where we can — how to fix it.

use std::fmt;

#[derive(Debug)]
pub enum JamaError {
    /// A parse error in a ledger text file: line number + message.
    Parse { line: usize, message: String },
    /// A transaction (or set of postings) that doesn't balance.
    Imbalance(crate::model::Imbalance),
    /// A `balance` assertion directive that doesn't hold.
    BalanceAssertionFailed { message: String },
    /// An account referenced without an `open` directive (strict mode).
    UndeclaredAccount { account: String },
    /// A problem reading, writing, or migrating the SQLite store.
    Store(String),
    /// A problem with a CSV import file or its rules file.
    Import(String),
    /// Any other I/O failure, with context about which file/operation.
    Io {
        context: String,
        source: std::io::Error,
    },
    /// A user-facing validation error (bad flag, bad path, etc).
    Invalid(String),
}

impl fmt::Display for JamaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JamaError::Parse { line, message } => {
                write!(f, "parse error at line {line}: {message}")
            }
            JamaError::Imbalance(imb) => write!(f, "imbalance: {imb}"),
            JamaError::BalanceAssertionFailed { message } => {
                write!(f, "balance assertion failed: {message}")
            }
            JamaError::UndeclaredAccount { account } => {
                write!(f, "undeclared account: {account}")
            }
            JamaError::Store(msg) => write!(f, "store error: {msg}"),
            JamaError::Import(msg) => write!(f, "import error: {msg}"),
            JamaError::Io { context, source } => write!(f, "{context}: {source}"),
            JamaError::Invalid(msg) => write!(f, "{msg}"),
        }
    }
}

impl std::error::Error for JamaError {}

impl From<rusqlite::Error> for JamaError {
    fn from(e: rusqlite::Error) -> Self {
        JamaError::Store(e.to_string())
    }
}

pub type Result<T> = std::result::Result<T, JamaError>;
