use crate::{CellError, CellValue};

/// Evaluate IFS as a sequence of condition/value decisions. The callback keeps
/// unevaluated AST branches out of the runtime path while preserving AST-based
/// dependency collection in the workbook layer.
pub(crate) fn evaluate<F>(arguments: &[usize], mut scalar: F) -> Result<CellValue, CellError>
where
    F: FnMut(usize) -> Result<CellValue, CellError>,
{
    let pairs = arguments.chunks_exact(2);
    if arguments.len() < 2 || !pairs.remainder().is_empty() {
        return Err(CellError::ValueError);
    }

    for pair in pairs {
        let matched = matches_condition(scalar(pair[0])?)?;
        if matched {
            return scalar(pair[1]);
        }
    }

    Err(CellError::NotAvailable)
}

fn matches_condition(value: CellValue) -> Result<bool, CellError> {
    match value {
        CellValue::Empty => Ok(false),
        CellValue::Boolean(value) => Ok(value),
        CellValue::Number(value) => Ok(value != 0.0),
        CellValue::Error(error) => Err(error),
        CellValue::Text(_)
        | CellValue::Date(_)
        | CellValue::DateTime(_)
        | CellValue::Duration(_) => Err(CellError::ValueError),
    }
}
