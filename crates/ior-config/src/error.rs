//! Parse diagnostics with 1-based positions (dict-tools message style:
//! `line {n}: ...`, plus a column for this parser).

use std::error::Error;
use std::fmt;

/// A TOML-subset syntax error. `line` and `col` are 1-based; `col` counts
/// characters (not bytes) so it points at the right glyph for CJK text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// 1-based line number.
    pub line: usize,
    /// 1-based column number (in characters).
    pub col: usize,
    /// Human-readable description of what went wrong.
    pub message: String,
}

impl ParseError {
    /// Builds an error pointing at `line`/`col`.
    pub fn new(line: usize, col: usize, message: impl Into<String>) -> Self {
        ParseError {
            line,
            col,
            message: message.into(),
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "line {}, column {}: {}",
            self.line, self.col, self.message
        )
    }
}

impl Error for ParseError {}
