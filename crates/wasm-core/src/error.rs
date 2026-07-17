//! Error taxonomy. The spec distinguishes malformed (decode-time),
//! invalid (validation-time), link errors and runtime traps; the runner's
//! verdicts depend on this separation, so it is kept strict here.

use std::fmt;

#[derive(Clone, Debug)]
pub struct DecodeError {
    pub offset: usize,
    pub msg: String,
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} (at offset {:#x})", self.msg, self.offset)
    }
}

#[derive(Clone, Debug)]
pub struct ValidateError {
    pub msg: String,
}

impl fmt::Display for ValidateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.msg)
    }
}

/// Runtime trap with a spec-canonical message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Trap {
    pub msg: String,
}

impl Trap {
    pub fn new(msg: impl Into<String>) -> Trap {
        Trap { msg: msg.into() }
    }
}

impl fmt::Display for Trap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.msg)
    }
}

/// Instantiation failure.
#[derive(Clone, Debug)]
pub enum InstError {
    /// Import resolution / matching failed (assert_unlinkable territory).
    Link(String),
    /// A trap occurred while applying segments or running start
    /// (assert_uninstantiable territory).
    Trap(Trap),
}

impl fmt::Display for InstError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InstError::Link(m) => write!(f, "link error: {m}"),
            InstError::Trap(t) => write!(f, "trap: {t}"),
        }
    }
}

#[derive(Clone, Debug)]
pub enum InvokeError {
    NoSuchExport(String),
    KindMismatch(String),
    ArgMismatch(String),
    Trap(Trap),
}

impl fmt::Display for InvokeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            InvokeError::NoSuchExport(n) => write!(f, "no such export: {n}"),
            InvokeError::KindMismatch(m) => write!(f, "export kind mismatch: {m}"),
            InvokeError::ArgMismatch(m) => write!(f, "argument mismatch: {m}"),
            InvokeError::Trap(t) => write!(f, "trap: {t}"),
        }
    }
}
