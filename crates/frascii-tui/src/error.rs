//! This crate's error type.

/// Something went wrong driving the terminal.
///
/// Self-contained on purpose: it carries no `#[from]` into another frascii
/// crate's error type, so every crossing of the boundary has to be spelled out
/// at the call site. `frascii-core` has no error type today — a kernel cannot
/// fail — and if it gains one, the compiler should make someone decide which
/// side of the boundary the new failure sits on rather than silently wrapping
/// it here.
#[derive(Debug, thiserror::Error)]
pub enum TuiError {
    /// The terminal could not be set up, driven, or restored.
    #[error("terminal I/O failed: {0}")]
    Terminal(#[from] std::io::Error),
}

/// `Result` with this crate's error type.
pub type Result<T> = std::result::Result<T, TuiError>;
