//! Reproducible host measurement; wall-clock values are observations, not acceptance thresholds.
use nagi_sheets_core::*;
use std::time::Instant;
fn main() -> Result<(), CellError> {
    let mut book = Workbook::new(ObjectId(1));
    let sheet = book.create_sheet(ObjectId(2), "Sparse")?;
    let start = Instant::now();
    let values = (1..=100_000)
        .map(|row| {
            Ok((
                CellKey::new(sheet, CellAddress::new(row, 1)?),
                CellInput::Value(CellValue::Number(row as f64)),
            ))
        })
        .collect::<Result<Vec<_>, CellError>>()?;
    book.set_values(values)?;
    println!(
        "sparse_populated={} sparse_insert_ms={:.3}",
        book.populated_cell_count(),
        start.elapsed().as_secs_f64() * 1000.
    );
    drop(book);
    let mut book = Workbook::new(ObjectId(3));
    let sheet = book.create_sheet(ObjectId(4), "Chain")?;
    let start = Instant::now();
    let values = (2..=10_000)
        .map(|row| {
            Ok((
                CellKey::new(sheet, CellAddress::new(row, 1)?),
                CellInput::Formula(format!("=A{}+1", row - 1)),
            ))
        })
        .collect::<Result<Vec<_>, CellError>>()?;
    book.set_values(values)?;
    println!(
        "chain_build_ms={:.3}",
        start.elapsed().as_secs_f64() * 1000.
    );
    let start = Instant::now();
    let event = book.set_value(
        CellKey::new(sheet, CellAddress::new(1, 1)?),
        CellValue::Number(1.),
    )?;
    println!(
        "chain_recalculated={} chain_recalc_ms={:.3}",
        event.recalculation.evaluated.len(),
        start.elapsed().as_secs_f64() * 1000.
    );
    let event = book.set_value(
        CellKey::new(sheet, CellAddress::new(1, 2)?),
        CellValue::Number(1.),
    )?;
    println!(
        "unrelated_recalculated={}",
        event.recalculation.evaluated.len()
    );
    Ok(())
}
