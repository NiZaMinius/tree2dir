//! Centralised error types for the tree2dir library core.
//!
//! All internal errors implement [`std::error::Error`] via the
//! [`thiserror::Error`] derive macro and are gathered under the top-level
//! [`AppError`] enum.  Application code should propagate these errors through
//! [`anyhow::Result`] boundaries where appropriate.

use std::path::PathBuf;

use thiserror::Error;

/// All recoverable and fatal errors produced by tree2dir.
#[derive(Debug, Error)]
pub enum AppError {
    // ── I/O ─────────────────────────────────────────────────────────────────
    /// A filesystem operation (read, write, create) failed.
    #[error("I/O error at path `{path}`: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    // ── Input ────────────────────────────────────────────────────────────────
    /// The provided input produced an empty parse result (no nodes).
    #[error("The input tree was empty or contained no recognisable entries.")]
    EmptyInput,

    /// A path that would be created escapes the output root via `..` segments.
    #[error("Unsafe path detected (path traversal): `{0}`")]
    UnsafePath(PathBuf),

    /// A path that would be written resolves outside the output root through
    /// a symbolic link (or junction), even though its text stays inside.
    #[error(
        "Refusing to write `{0}`: it leads outside the output directory through a symbolic link."
    )]
    SymlinkEscape(PathBuf),

    // ── Filesystem conflicts ─────────────────────────────────────────────────
    /// A target file already exists and `--force` was not supplied.
    ///
    /// This error is **non-fatal** by default: the builder logs a warning and
    /// continues rather than aborting the entire run.
    #[error("File already exists, skipping (use --force to overwrite): `{0}`")]
    FileExists(PathBuf),

    // ── Clipboard ────────────────────────────────────────────────────────────
    /// The clipboard could not be accessed.
    #[error("Clipboard error: {0}")]
    Clipboard(String),

    // ── TUI ─────────────────────────────────────────────────────────────────
    /// A terminal / TUI operation failed due to an I/O error.
    #[error("Terminal I/O error: {source}")]
    TerminalIo {
        #[source]
        source: std::io::Error,
    },

    /// A terminal / TUI operation failed (non I/O).
    #[error("Terminal error: {0}")]
    Terminal(String),
}

impl AppError {
    /// Convenience constructor for [`AppError::Io`].
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Self::Io {
            path: path.into(),
            source,
        }
    }

    pub fn terminal_io(source: std::io::Error) -> Self {
        Self::TerminalIo { source }
    }

    pub fn terminal(msg: impl Into<String>) -> Self {
        Self::Terminal(msg.into())
    }
}
