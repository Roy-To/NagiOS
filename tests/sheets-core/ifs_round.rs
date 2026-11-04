//! Host tests for IFS, ROUNDUP and ROUNDDOWN (Nagi 0.2 spec 58.6
//! "IF/IFS/AND/OR/NOT", "ROUND variants"; 58.7 structured errors).
use super::{eval, fixture, key, value};
use nagi_sheets_core::*;

fn num(n: f64) -> CellValue {
    CellValue::Number(n)
}
fn err(e: CellError) -> CellValue {
    CellValue::Error(e)
}
fn assert_finite_or_error(v: &CellValue, f: &str) {
    match v {
        CellValue::Number(n) => assert!(n.is_finite(), "{f} produced {n}"),
        CellValue::Error(_) => {}
        other => panic!("{f} produced non-numeric {other:?}"),
    }
}

#[test]
fn ifs_canonical_names_resolve_case_insensitively() {
    for (name, id) in [
        ("ifs", FunctionId::Ifs),
        ("RoundUp", FunctionId::RoundUp),
        ("rounddown", FunctionId::RoundDown),
    ] {
        assert_eq!(FunctionId::resolve(name), Ok(id));
        assert_eq!(FunctionId::resolve(id.canonical_name()), Ok(id));
    }
    assert_eq!(FunctionId::Ifs.canonical_name(), "IFS");
    assert_eq!(FunctionId::RoundUp.canonical_name(), "ROUNDUP");
    assert_eq!(FunctionId::RoundDown.canonical_name(), "ROUNDDOWN");
    assert_eq!(FunctionId::resolve("IFSX"), Err(CellError::NameError));
    assert_eq!(FunctionId::resolve("ROUNDUPP"), Err(CellError::NameError));
}

#[test]
fn ifs_selects_first_true_pair() {
    let (mut w, a, _) = fixture();
    w.set_value(key(a, "A1"), num(75.)).unwrap();
    for (f, v) in [
        (
            "=IFS(A1>=90,\"A\",A1>=70,\"B\",TRUE,\"C\")",
            CellValue::Text("B".into()),
        ),
        ("=IFS(TRUE,1,TRUE,2)", num(1.)),
        ("=IFS(FALSE,1,1,2)", num(2.)),
        ("=IFS(0,1,A9,2,-3,3)", num(3.)),
        ("=IFS(TRUE,A9)", CellValue::Empty),
        ("=ifs(1=1,\"小\")", CellValue::Text("小".into())),
    ] {
        assert_eq!(eval(&mut w, a, f), v, "{f}");
    }
}

#[test]
fn ifs_is_lazy_dead_branches_unevaluated() {
    let (mut w, a, b) = fixture();
    w.set_value(key(a, "A1"), CellValue::Error(CellError::ValueError))
        .unwrap();
    // Reference into a sheet that is deleted below: evaluating it would
    // yield INVALID_REFERENCE, so a clean result proves it was not evaluated.
    w.set_formula(key(a, "C1"), "=IFS(TRUE,7,1/0,8,'日本語 集計'!A1,9,A1,10)")
        .unwrap();
    // Positive control: the same deleted-sheet reference in a live branch.
    w.set_formula(key(a, "C2"), "=IFS(FALSE,1,TRUE,'日本語 集計'!A1)")
        .unwrap();
    assert_eq!(value(&w, a, "C2"), CellValue::Empty);
    let e = w.delete_sheet(b).unwrap();
    assert!(e.recalculation.evaluated.contains(&key(a, "C1")));
    for (f, v) in [
        // Dead values after the first true condition.
        ("=IFS(TRUE,1,TRUE,1/0)", num(1.)),
        ("=IFS(TRUE,1,TRUE,A1)", num(1.)),
        // Dead conditions after the first true condition.
        ("=IFS(TRUE,1,1/0,2)", num(1.)),
        ("=IFS(TRUE,1,\"text\",2)", num(1.)),
        // Value of a false pair is never evaluated.
        ("=IFS(FALSE,1/0,TRUE,5)", num(5.)),
        ("=IFS(FALSE,A1,TRUE,5)", num(5.)),
    ] {
        assert_eq!(eval(&mut w, a, f), v, "{f}");
    }
    assert_eq!(value(&w, a, "C1"), num(7.));
    assert_eq!(value(&w, a, "C2"), err(CellError::InvalidReference));
}

#[test]
fn ifs_errors_are_typed() {
    let (mut w, a, _) = fixture();
    w.set_value(key(a, "A1"), CellValue::Error(CellError::NameError))
        .unwrap();
    w.set_value(key(a, "A2"), CellValue::Text("yes".into()))
        .unwrap();
    for (f, v) in [
        // Arity: zero, odd and single argument counts.
        ("=IFS()", err(CellError::ValueError)),
        ("=IFS(TRUE)", err(CellError::ValueError)),
        ("=IFS(TRUE,1,FALSE)", err(CellError::ValueError)),
        ("=IFS(FALSE,1,FALSE,2,TRUE)", err(CellError::ValueError)),
        // No condition is true.
        ("=IFS(FALSE,1)", err(CellError::NotAvailable)),
        ("=IFS(FALSE,1,0,2,A9,3)", err(CellError::NotAvailable)),
        // Condition errors propagate (same conversion as IF).
        ("=IFS(1/0,1,TRUE,2)", err(CellError::DivByZero)),
        ("=IFS(A1,1,TRUE,2)", err(CellError::NameError)),
        ("=IFS(A2,1,TRUE,2)", err(CellError::ValueError)),
        ("=IFS(A1:A2,1,TRUE,2)", err(CellError::ValueError)),
        // Selected value error propagates.
        ("=IFS(TRUE,1/0)", err(CellError::DivByZero)),
        // IFERROR can wrap the no-match error.
        (
            "=IFERROR(IFS(FALSE,1),\"none\")",
            CellValue::Text("none".into()),
        ),
    ] {
        assert_eq!(eval(&mut w, a, f), v, "{f}");
    }
    let event = w.set_formula(key(a, "B1"), "=IFS(FALSE,1)").unwrap();
    assert!(!event.is_successful());
    assert_eq!(
        event.recalculation.errors,
        vec![(key(a, "B1"), CellError::NotAvailable)]
    );
}

#[test]
fn ifs_incremental_recalculation_tracks_all_branch_refs() {
    let (mut w, a, _) = fixture();
    w.set_value(key(a, "A1"), num(1.)).unwrap();
    w.set_value(key(a, "A2"), num(10.)).unwrap();
    w.set_value(key(a, "A3"), num(20.)).unwrap();
    w.set_value(key(a, "A4"), num(99.)).unwrap();
    w.set_formula(key(a, "B1"), "=IFS(A1=1,A2,A1=2,A3)")
        .unwrap();
    w.set_formula(key(a, "C1"), "=B1*2").unwrap();
    let deps: Vec<_> = w.dependencies(key(a, "B1")).collect();
    assert_eq!(deps, vec![key(a, "A1"), key(a, "A2"), key(a, "A3")]);
    assert_eq!(value(&w, a, "C1"), num(20.));

    // Switch the active pair.
    let e = w.set_value(key(a, "A1"), num(2.)).unwrap();
    assert_eq!(e.recalculation.evaluated, vec![key(a, "B1"), key(a, "C1")]);
    assert_eq!(value(&w, a, "B1"), num(20.));
    assert_eq!(value(&w, a, "C1"), num(40.));

    // Edit inside the now-live branch.
    w.set_value(key(a, "A3"), num(21.)).unwrap();
    assert_eq!(value(&w, a, "C1"), num(42.));

    // Edit inside the dead branch: conservatively recalculated, value stable.
    let e = w
        .set_value(key(a, "A2"), CellValue::Error(CellError::DivByZero))
        .unwrap();
    assert_eq!(e.recalculation.evaluated, vec![key(a, "B1"), key(a, "C1")]);
    assert!(e.is_successful());
    assert_eq!(value(&w, a, "C1"), num(42.));

    // Unrelated cell: nothing recalculated.
    let e = w.set_value(key(a, "A4"), num(0.)).unwrap();
    assert!(e.recalculation.evaluated.is_empty());

    // No pair matches -> NOT_AVAILABLE flows downstream, then recovers.
    w.set_value(key(a, "A1"), num(3.)).unwrap();
    assert_eq!(value(&w, a, "B1"), err(CellError::NotAvailable));
    assert_eq!(value(&w, a, "C1"), err(CellError::NotAvailable));
    w.set_value(key(a, "A1"), num(2.)).unwrap();
    assert_eq!(value(&w, a, "C1"), num(42.));

    // Replacing the formula drops the old branch edges.
    w.set_formula(key(a, "B1"), "=IFS(TRUE,A4)").unwrap();
    let deps: Vec<_> = w.dependencies(key(a, "B1")).collect();
    assert_eq!(deps, vec![key(a, "A4")]);
    let e = w.set_value(key(a, "A3"), num(5.)).unwrap();
    assert!(e.recalculation.evaluated.is_empty());

    // Conservative cycle policy matches IF: a dead-branch self reference cycles.
    let e = w
        .set_formula(key(a, "D1"), "=IFS(TRUE,1,FALSE,D1)")
        .unwrap();
    assert_eq!(e.recalculation.circular, vec![key(a, "D1")]);

    // Snapshot reload recomputes identical values.
    let reloaded = Workbook::load(w.snapshot()).unwrap();
    for cell in ["B1", "C1", "D1"] {
        assert_eq!(value(&reloaded, a, cell), value(&w, a, cell), "{cell}");
    }
}

#[test]
fn roundup_rounddown_directions_and_digits() {
    let (mut w, a, _) = fixture();
    for (f, v) in [
        ("=ROUNDUP(3.2,0)", 4.),
        ("=ROUNDUP(3.0,0)", 3.),
        ("=ROUNDUP(-3.2,0)", -4.),
        ("=ROUNDDOWN(3.7,0)", 3.),
        ("=ROUNDDOWN(-3.7,0)", -3.),
        ("=ROUNDUP(4.56789,3)", 4.568),
        ("=ROUNDDOWN(4.56789,3)", 4.567),
        ("=ROUNDUP(-4.56789,1)", -4.6),
        ("=ROUNDDOWN(-4.56789,1)", -4.5),
        ("=ROUNDUP(0.004,2)", 0.01),
        ("=ROUNDDOWN(0.004,2)", 0.),
        ("=ROUNDUP(9.99,1)", 10.),
        ("=ROUNDUP(-99.91,0)", -100.),
        // Negative digits round left of the decimal point.
        ("=ROUNDUP(1234.5678,-2)", 1300.),
        ("=ROUNDDOWN(1234.5678,-2)", 1200.),
        ("=ROUNDUP(-1234.5678,-2)", -1300.),
        ("=ROUNDDOWN(-1234.5678,-2)", -1200.),
        ("=ROUNDUP(5,-1)", 10.),
        ("=ROUNDDOWN(5,-1)", 0.),
        ("=ROUNDUP(-5,-1)", -10.),
        ("=ROUNDUP(999,-3)", 1000.),
        ("=ROUNDDOWN(999,-3)", 0.),
        ("=ROUNDUP(1000,-3)", 1000.),
        // Binary artefacts do not leak: 1.1*100 = 110.00000000000001 in f64.
        ("=ROUNDUP(1.1,2)", 1.1),
        ("=ROUNDDOWN(4.35,2)", 4.35),
        ("=ROUNDDOWN(0.29,2)", 0.29),
        ("=ROUNDUP(2.675,2)", 2.68),
        ("=ROUNDDOWN(2.675,2)", 2.67),
        // Digits beyond the value's precision leave it unchanged.
        ("=ROUNDUP(0.1,20)", 0.1),
        ("=ROUNDDOWN(-0.1,308)", -0.1),
        // Empty/boolean coerce like ROUND.
        ("=ROUNDUP(A9,0)", 0.),
        ("=ROUNDUP(TRUE,0)", 1.),
        ("=ROUNDDOWN(1.55,TRUE)", 1.5),
        ("=ROUNDUP(0,5)", 0.),
    ] {
        assert_eq!(eval(&mut w, a, f), num(v), "{f}");
    }
    // Inputs are interpreted as their shortest round-trip decimal: 0.1+0.2 is
    // 0.30000000000000004, so ROUNDUP sees a nonzero tail.
    assert_eq!(eval(&mut w, a, "=ROUNDUP(0.1+0.2,1)"), num(0.4));
    assert_eq!(eval(&mut w, a, "=ROUNDDOWN(0.1+0.2,1)"), num(0.3));
}

#[test]
fn roundup_rounddown_negative_zero_is_normalised() {
    let (mut w, a, _) = fixture();
    for f in [
        "=ROUNDDOWN(-0.5,0)",
        "=ROUNDDOWN(-4,-1)",
        "=ROUNDDOWN(-1e-300,2)",
        "=ROUNDUP(-0,2)",
    ] {
        let CellValue::Number(n) = eval(&mut w, a, f) else {
            panic!("{f}")
        };
        assert_eq!(n, 0.0, "{f}");
        assert!(n.is_sign_positive(), "{f} produced -0");
    }
}

#[test]
fn roundup_rounddown_typed_errors() {
    let (mut w, a, _) = fixture();
    w.set_value(key(a, "A1"), CellValue::Text("x".into()))
        .unwrap();
    w.set_value(key(a, "A2"), CellValue::Error(CellError::NotAvailable))
        .unwrap();
    w.set_value(key(a, "A3"), CellValue::Date(5)).unwrap();
    for (f, e) in [
        // Arity.
        ("=ROUNDUP()", CellError::ValueError),
        ("=ROUNDUP(1)", CellError::ValueError),
        ("=ROUNDDOWN(1,2,3)", CellError::ValueError),
        // Non-integer digits follow the existing ROUND convention.
        ("=ROUNDUP(1.5,0.5)", CellError::NumericError),
        ("=ROUNDDOWN(1.5,-0.1)", CellError::NumericError),
        // Digits outside [-308, 308].
        ("=ROUNDUP(1,309)", CellError::NumericError),
        ("=ROUNDDOWN(1,-309)", CellError::NumericError),
        ("=ROUNDUP(1,1e300)", CellError::NumericError),
        // Type and error propagation.
        ("=ROUNDUP(A1,0)", CellError::ValueError),
        ("=ROUNDDOWN(1,A1)", CellError::ValueError),
        ("=ROUNDUP(A2,0)", CellError::NotAvailable),
        ("=ROUNDDOWN(A3,0)", CellError::ValueError),
        ("=ROUNDUP(1/0,0)", CellError::DivByZero),
        ("=ROUNDUP(A1:A2,0)", CellError::ValueError),
    ] {
        assert_eq!(eval(&mut w, a, f), err(e), "{f}");
    }
}

#[test]
fn roundup_rounddown_extremes_never_produce_nonfinite() {
    let (mut w, a, _) = fixture();
    w.set_value(key(a, "A1"), num(f64::MAX)).unwrap();
    w.set_value(key(a, "A2"), num(-f64::MAX)).unwrap();
    w.set_value(key(a, "A3"), num(f64::MIN_POSITIVE)).unwrap();
    w.set_value(key(a, "A4"), num(5e-324)).unwrap();
    // Overflow on rounding away from zero is a typed error.
    assert_eq!(
        eval(&mut w, a, "=ROUNDUP(A1,-308)"),
        err(CellError::NumericError)
    );
    assert_eq!(
        eval(&mut w, a, "=ROUNDUP(A2,-308)"),
        err(CellError::NumericError)
    );
    assert_eq!(eval(&mut w, a, "=ROUNDDOWN(A1,-308)"), num(1e308));
    assert_eq!(eval(&mut w, a, "=ROUNDDOWN(A2,-308)"), num(-1e308));
    assert_eq!(eval(&mut w, a, "=ROUNDUP(A1,0)"), num(f64::MAX));
    assert_eq!(eval(&mut w, a, "=ROUNDDOWN(A1,308)"), num(f64::MAX));
    assert_eq!(eval(&mut w, a, "=ROUNDUP(1e308,-308)"), num(1e308));
    // Tiny values.
    assert_eq!(eval(&mut w, a, "=ROUNDUP(A4,308)"), num(1e-308));
    assert_eq!(eval(&mut w, a, "=ROUNDDOWN(A4,308)"), num(0.));
    assert_eq!(eval(&mut w, a, "=ROUNDUP(A3,308)"), num(3e-308));
    assert_eq!(eval(&mut w, a, "=ROUNDUP(-A4,-308)"), num(-1e308));
    // Sweep: every combination is finite or a typed error.
    let inputs = [
        "A1",
        "A2",
        "A3",
        "A4",
        "-A3",
        "0",
        "1",
        "-1",
        "0.5",
        "123456789.987654321",
        "1e300",
        "-1e300",
        "1e-300",
        "9.999999999999999e307",
    ];
    for x in inputs {
        for d in [-308, -307, -200, -16, -1, 0, 1, 15, 16, 17, 200, 307, 308] {
            for fname in ["ROUNDUP", "ROUNDDOWN"] {
                let f = format!("={fname}({x},{d})");
                let v = eval(&mut w, a, &f);
                assert_finite_or_error(&v, &f);
                if let CellValue::Number(r) = v {
                    let CellValue::Number(src) = eval(&mut w, a, &format!("=+{x}")) else {
                        panic!("{x}")
                    };
                    if fname == "ROUNDUP" {
                        assert!(r.abs() >= src.abs(), "{f}: {r} vs {src}");
                    } else {
                        assert!(r.abs() <= src.abs(), "{f}: {r} vs {src}");
                    }
                    assert!(r == 0.0 || r.signum() == src.signum(), "{f}: sign");
                }
            }
        }
    }
}

#[test]
fn roundup_rounddown_incremental_recalculation() {
    let (mut w, a, _) = fixture();
    w.set_value(key(a, "A1"), num(2.345)).unwrap();
    w.set_value(key(a, "A2"), num(2.)).unwrap();
    w.set_formula(key(a, "B1"), "=ROUNDUP(A1,A2)").unwrap();
    w.set_formula(key(a, "B2"), "=ROUNDDOWN(A1,A2)").unwrap();
    w.set_formula(key(a, "C1"), "=B1-B2").unwrap();
    assert_eq!(value(&w, a, "B1"), num(2.35));
    assert_eq!(value(&w, a, "B2"), num(2.34));
    let e = w.set_value(key(a, "A2"), num(-1.)).unwrap();
    assert_eq!(
        e.recalculation.evaluated,
        vec![key(a, "B1"), key(a, "B2"), key(a, "C1")]
    );
    assert_eq!(value(&w, a, "B1"), num(10.));
    assert_eq!(value(&w, a, "B2"), num(0.));
    assert_eq!(value(&w, a, "C1"), num(10.));
    // Invalid digits become a typed error downstream and then recover.
    w.set_value(key(a, "A2"), num(0.5)).unwrap();
    assert_eq!(value(&w, a, "C1"), err(CellError::NumericError));
    w.set_value(key(a, "A2"), num(1.)).unwrap();
    assert_eq!(value(&w, a, "B1"), num(2.4));
    assert_eq!(value(&w, a, "B2"), num(2.3));
    // Mixed with IFS and reload.
    w.set_formula(
        key(a, "D1"),
        "=IFS(A1>2,ROUNDDOWN(A1,0),TRUE,ROUNDUP(A1,0))",
    )
    .unwrap();
    assert_eq!(value(&w, a, "D1"), num(2.));
    w.set_value(key(a, "A1"), num(1.2)).unwrap();
    assert_eq!(value(&w, a, "D1"), num(2.));
    w.set_value(key(a, "A1"), num(-1.2)).unwrap();
    assert_eq!(value(&w, a, "D1"), num(-2.));
    let reloaded = Workbook::load(w.snapshot()).unwrap();
    for cell in ["B1", "B2", "C1", "D1"] {
        assert_eq!(value(&reloaded, a, cell), value(&w, a, cell), "{cell}");
    }
}
