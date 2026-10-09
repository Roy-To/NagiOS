//! Bounded, deterministic, host-only Sheets calculation foundation.
//! No I/O, permission grants, clock, UI or production service adapters.
mod engine;
mod parser;
mod workbook;
pub use nagi_model::ObjectId;
pub use parser::{parse_formula, BinaryOp, Expr, FunctionId, ParsedFormula, Reference, UnaryOp};
pub use workbook::*;

pub const MAX_ROWS: u32 = 1_048_576;
pub const MAX_COLUMNS: u32 = 16_384;
pub const MAX_FORMULA_BYTES: usize = 8192;
pub const MAX_AST_NODES: usize = 256;
pub const MAX_AST_DEPTH: usize = 64;
pub const MAX_RANGE_CELLS: usize = 100_000;
pub const MAX_POPULATED_CELLS: usize = 100_000;
pub const MAX_DEPENDENCY_EDGES: usize = 1_000_000;
pub const MAX_SHEETS: usize = 256;
pub const MAX_TEXT_BYTES: usize = 65_536;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellError {
    DivByZero,
    InvalidReference,
    ValueError,
    NameError,
    NotAvailable,
    CircularReference,
    InvalidSyntax,
    LimitExceeded,
    NumericError,
    DuplicateIdentity,
    DuplicateName,
    UnsupportedVersion,
}
impl CellError {
    pub const fn code(self) -> &'static str {
        match self {
            Self::DivByZero => "DIV_BY_ZERO",
            Self::InvalidReference => "INVALID_REFERENCE",
            Self::ValueError => "VALUE_ERROR",
            Self::NameError => "NAME_ERROR",
            Self::NotAvailable => "NOT_AVAILABLE",
            Self::CircularReference => "CIRCULAR_REFERENCE",
            Self::InvalidSyntax => "INVALID_SYNTAX",
            Self::LimitExceeded => "LIMIT_EXCEEDED",
            Self::NumericError => "NUMERIC_ERROR",
            Self::DuplicateIdentity => "DUPLICATE_IDENTITY",
            Self::DuplicateName => "DUPLICATE_NAME",
            Self::UnsupportedVersion => "UNSUPPORTED_VERSION",
        }
    }
}
impl std::fmt::Display for CellError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.code())
    }
}
impl std::error::Error for CellError {}

#[derive(Clone, Debug, PartialEq)]
pub enum CellValue {
    Empty,
    Text(String),
    Number(f64),
    Boolean(bool),
    /// Days since Unix epoch, never interpreted as Excel serials.
    Date(i32),
    /// UTC milliseconds since Unix epoch. No implicit timezone conversion.
    DateTime(i64),
    /// Signed milliseconds.
    Duration(i64),
    Error(CellError),
}
impl CellValue {
    pub(crate) fn validate(&self) -> Result<(), CellError> {
        match self {
            Self::Number(n) if !n.is_finite() => Err(CellError::NumericError),
            Self::Text(s) if s.len() > MAX_TEXT_BYTES => Err(CellError::LimitExceeded),
            _ => Ok(()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CellAddress {
    row: u32,
    column: u32,
}
impl CellAddress {
    /// One-based coordinates.
    pub fn new(row: u32, column: u32) -> Result<Self, CellError> {
        if row == 0 || column == 0 || row > MAX_ROWS || column > MAX_COLUMNS {
            Err(CellError::InvalidReference)
        } else {
            Ok(Self { row, column })
        }
    }
    pub const fn row(self) -> u32 {
        self.row
    }
    pub const fn column(self) -> u32 {
        self.column
    }
    pub fn from_a1(input: &str) -> Result<Self, CellError> {
        Ok(Reference::parse(None, input)?.address)
    }
}
