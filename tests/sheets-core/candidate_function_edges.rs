use super::{eval, fixture, key, value};
use nagi_sheets_core::*;

fn num(value: f64) -> CellValue {
    CellValue::Number(value)
}

fn err(error: CellError) -> CellValue {
    CellValue::Error(error)
}

#[test]
fn ifs_pair_order_and_condition_failures_are_explicit() {
    let (mut workbook, sheet, _) = fixture();
    workbook
        .set_value(key(sheet, "A1"), err(CellError::NameError))
        .unwrap();
    workbook
        .set_value(key(sheet, "A2"), CellValue::Text("yes".into()))
        .unwrap();

    assert_eq!(
        eval(&mut workbook, sheet, "=IFS(FALSE,1/0,TRUE,7,1/0,9)"),
        num(7.0)
    );
    assert_eq!(
        eval(&mut workbook, sheet, "=IFS(FALSE,A1,TRUE,A9)"),
        CellValue::Empty
    );
    assert_eq!(eval(&mut workbook, sheet, "=IFS(A9,1,TRUE,2)"), num(2.0));
    assert_eq!(
        eval(&mut workbook, sheet, "=IFS(A2,1,TRUE,2)"),
        err(CellError::ValueError)
    );
    assert_eq!(
        eval(&mut workbook, sheet, "=IFS(A1,1,TRUE,2)"),
        err(CellError::NameError)
    );
    assert_eq!(
        eval(&mut workbook, sheet, "=IFS(A1:A2,1,TRUE,2)"),
        err(CellError::ValueError)
    );
    assert_eq!(
        eval(&mut workbook, sheet, "=IFS(FALSE,1,FALSE,2)"),
        err(CellError::NotAvailable)
    );
}

#[test]
fn directed_rounding_covers_carry_coercion_and_error_order() {
    let (mut workbook, sheet, _) = fixture();
    for (address, value) in [
        ("A3", CellValue::Date(5)),
        ("A4", CellValue::DateTime(-1)),
        ("A5", CellValue::Duration(2)),
    ] {
        workbook.set_value(key(sheet, address), value).unwrap();
    }
    for address in ["A3", "A4", "A5"] {
        let rounding_formula = format!("=ROUNDUP({address},0)");
        assert_eq!(
            eval(&mut workbook, sheet, &rounding_formula),
            err(CellError::ValueError),
            "{rounding_formula}"
        );
        let condition_formula = format!("=IFS({address},1,TRUE,2)");
        assert_eq!(
            eval(&mut workbook, sheet, &condition_formula),
            err(CellError::ValueError),
            "{condition_formula}"
        );
    }

    for (formula, expected) in [
        ("=ROUNDUP(9.999,2)", 10.0),
        ("=ROUNDUP(99.99,-1)", 100.0),
        ("=ROUNDDOWN(99.99,-1)", 90.0),
        ("=ROUNDUP(-0.5,0)", -1.0),
        ("=ROUNDDOWN(-0.5,0)", 0.0),
        ("=ROUNDUP(1.5,1)", 1.5),
        ("=ROUNDDOWN(-1.25,2)", -1.25),
        ("=ROUNDUP(1.1,2)", 1.1),
        ("=ROUNDUP(0.1+0.2,1)", 0.4),
        ("=ROUNDDOWN(0.1+0.2,1)", 0.3),
        ("=ROUNDUP(A9,0)", 0.0),
        ("=ROUNDUP(TRUE,0)", 1.0),
        ("=ROUNDDOWN(1.55,TRUE)", 1.5),
        ("=ROUNDUP(1e308,-308)", 1e308),
        ("=ROUNDDOWN(1.01e308,-308)", 1e308),
        ("=ROUNDUP(5e-324,308)", 1e-308),
    ] {
        assert_eq!(
            eval(&mut workbook, sheet, formula),
            num(expected),
            "{formula}"
        );
    }

    for formula in [
        "=ROUNDDOWN(-0.5,0)",
        "=ROUNDDOWN(-4,-1)",
        "=ROUNDDOWN(-1e-300,2)",
    ] {
        let CellValue::Number(result) = eval(&mut workbook, sheet, formula) else {
            panic!("{formula} should return a number");
        };
        assert_eq!(result, 0.0, "{formula}");
        assert!(
            result.is_sign_positive(),
            "{formula} returned negative zero"
        );
    }

    assert_eq!(
        eval(&mut workbook, sheet, r#"=ROUNDUP("x",1/0)"#),
        err(CellError::ValueError)
    );
    assert_eq!(
        eval(&mut workbook, sheet, r#"=ROUNDUP(1/0,"x")"#),
        err(CellError::DivByZero)
    );
    for formula in [
        "=ROUNDUP(1,308.5)",
        "=ROUNDDOWN(1,-308.5)",
        "=ROUNDUP(1,309)",
        "=ROUNDDOWN(1,-309)",
        "=ROUNDUP(1,1e300)",
        "=ROUNDUP(1.01e308,-308)",
    ] {
        assert_eq!(
            eval(&mut workbook, sheet, formula),
            err(CellError::NumericError),
            "{formula}"
        );
    }
}

#[test]
fn function_dependencies_recalculate_and_recover_through_snapshots() {
    let (mut workbook, sheet, _) = fixture();
    workbook.set_value(key(sheet, "A1"), num(1.0)).unwrap();
    workbook.set_value(key(sheet, "A2"), num(1.2)).unwrap();
    workbook.set_value(key(sheet, "A3"), num(8.0)).unwrap();
    workbook
        .set_formula(
            key(sheet, "B1"),
            "=IFS(A1=1,ROUNDUP(A2,0),A1=2,ROUNDDOWN(A3,0))",
        )
        .unwrap();
    workbook.set_formula(key(sheet, "C1"), "=B1+1").unwrap();

    assert_eq!(value(&workbook, sheet, "B1"), num(2.0));
    let dependencies: Vec<_> = workbook.dependencies(key(sheet, "B1")).collect();
    assert_eq!(
        dependencies,
        vec![key(sheet, "A1"), key(sheet, "A2"), key(sheet, "A3")]
    );

    let inactive_edit = workbook
        .set_value(key(sheet, "A3"), err(CellError::DivByZero))
        .unwrap();
    assert!(inactive_edit
        .recalculation
        .evaluated
        .contains(&key(sheet, "B1")));
    assert_eq!(value(&workbook, sheet, "B1"), num(2.0));

    workbook.set_value(key(sheet, "A1"), num(2.0)).unwrap();
    assert_eq!(value(&workbook, sheet, "B1"), err(CellError::DivByZero));
    assert_eq!(value(&workbook, sheet, "C1"), err(CellError::DivByZero));

    workbook.set_value(key(sheet, "A3"), num(3.8)).unwrap();
    assert_eq!(value(&workbook, sheet, "B1"), num(3.0));
    assert_eq!(value(&workbook, sheet, "C1"), num(4.0));

    let restored = Workbook::load(workbook.snapshot()).unwrap();
    for cell in ["B1", "C1"] {
        assert_eq!(
            value(&restored, sheet, cell),
            value(&workbook, sheet, cell),
            "{cell}"
        );
    }
}
