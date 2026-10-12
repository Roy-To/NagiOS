use nagi_sheets_core::*;
#[path = "ifs_round.rs"]
mod ifs_round;
fn fixture() -> (Workbook, SheetId, SheetId) {
    let mut w = Workbook::new(ObjectId(100));
    let a = w.create_sheet(ObjectId(101), "Sheet1").unwrap();
    let b = w.create_sheet(ObjectId(102), "日本語 集計").unwrap();
    (w, a, b)
}
fn key(s: SheetId, a: &str) -> CellKey {
    CellKey::new(s, CellAddress::from_a1(a).unwrap())
}
fn value(w: &Workbook, s: SheetId, a: &str) -> CellValue {
    w.get_cell(key(s, a)).unwrap().clone()
}
fn eval(w: &mut Workbook, s: SheetId, formula: &str) -> CellValue {
    w.set_formula(key(s, "Z100"), formula).unwrap();
    value(w, s, "Z100")
}
#[test]
fn sheets_h01_identity_and_rename() {
    let (mut w, a, b) = fixture();
    w.set_value(key(a, "A1"), CellValue::Number(1.)).unwrap();
    w.set_value(key(b, "A1"), CellValue::Number(2.)).unwrap();
    w.set_formula(key(a, "B1"), "='日本語 集計'!A1+1").unwrap();
    w.rename_sheet(b, "新しい名前").unwrap();
    assert_eq!(b.object_id(), ObjectId(102));
    assert_eq!(w.sheet_by_name("新しい名前"), Some(b));
    assert_eq!(w.sheet_by_name("日本語 集計"), None);
    w.set_value(key(b, "A1"), CellValue::Number(3.)).unwrap();
    assert_eq!(value(&w, a, "B1"), CellValue::Number(4.));
    assert_eq!(value(&w, a, "A1"), CellValue::Number(1.));
    assert_eq!(
        w.create_sheet(ObjectId(101), "x"),
        Err(CellError::DuplicateIdentity)
    );
    assert_eq!(w.rename_sheet(b, "Sheet1"), Err(CellError::DuplicateName));
}
#[test]
fn sheets_h02_typed_sparse_values() {
    let (mut w, a, _) = fixture();
    let values = [
        CellValue::Empty,
        CellValue::Number(0.),
        CellValue::Boolean(false),
        CellValue::Text("".into()),
        CellValue::Text("こんにちは".into()),
        CellValue::Date(0),
        CellValue::DateTime(-1),
        CellValue::Duration(3),
        CellValue::Error(CellError::NotAvailable),
    ];
    for (i, v) in values.iter().enumerate() {
        let k = CellKey::new(a, CellAddress::new(i as u32 + 1, 1).unwrap());
        w.set_value(k, v.clone()).unwrap();
        assert_eq!(w.get_cell(k).unwrap(), v);
    }
    assert_eq!(w.populated_cell_count(), 8);
    assert_eq!(value(&w, a, "XFD1048576"), CellValue::Empty);
    assert_eq!(
        w.set_value(key(a, "A1"), CellValue::Number(f64::NAN)),
        Err(CellError::NumericError)
    );
}
#[test]
fn sheets_h03_parser_and_arithmetic() {
    let (mut w, a, _) = fixture();
    w.set_value(key(a, "A1"), CellValue::Number(7.)).unwrap();
    for (f, n) in [
        ("=1+2*3", 7.),
        ("=(1+2)*3", 9.),
        ("=10-3-2", 5.),
        ("=-A1 + +$A$1/2", -3.5),
        ("=1e2/4", 25.),
    ] {
        assert_eq!(eval(&mut w, a, f), CellValue::Number(n));
    }
    let parsed = parse_formula("=$A1+A$2+$B$3").unwrap();
    let refs: Vec<_> = parsed
        .nodes()
        .iter()
        .filter_map(|n| match n {
            Expr::Reference(r) => Some((r.absolute_column, r.absolute_row)),
            _ => None,
        })
        .collect();
    assert_eq!(refs, vec![(true, false), (false, true), (true, true)]);
    assert_eq!(eval(&mut w, a, "=A1>=7"), CellValue::Boolean(true));
    assert_eq!(eval(&mut w, a, "=A1<>7"), CellValue::Boolean(false));
    assert_eq!(eval(&mut w, a, "=\"a\"<\"b\""), CellValue::Boolean(true));
}
#[test]
fn sheets_h04_functions_ranges_and_lazy_branches() {
    let (mut w, a, _) = fixture();
    for (i, v) in [
        CellValue::Number(2.),
        CellValue::Number(4.),
        CellValue::Boolean(false),
        CellValue::Text("".into()),
    ]
    .into_iter()
    .enumerate()
    {
        w.set_value(
            CellKey::new(a, CellAddress::new(i as u32 + 1, 1).unwrap()),
            v,
        )
        .unwrap();
    }
    for (f, v) in [
        ("=SUM(A1:A5)", CellValue::Number(6.)),
        ("=AVERAGE(A1:A5)", CellValue::Number(3.)),
        ("=MIN(A1:A5)", CellValue::Number(2.)),
        ("=MAX(A1:A5)", CellValue::Number(4.)),
        ("=COUNT(A1:A5)", CellValue::Number(2.)),
        ("=COUNTA(A1:A5)", CellValue::Number(4.)),
        ("=IF(FALSE,1/0,42)", CellValue::Number(42.)),
        ("=IF(TRUE,42,1/0)", CellValue::Number(42.)),
        ("=AND(FALSE,1/0)", CellValue::Boolean(false)),
        ("=OR(TRUE,1/0)", CellValue::Boolean(true)),
        ("=NOT(0)", CellValue::Boolean(true)),
        ("=ROUND(-1.25,1)", CellValue::Number(-1.3)),
        ("=ROUND(125,-1)", CellValue::Number(130.)),
        ("=IFERROR(1/0,8)", CellValue::Number(8.)),
        ("=SUM(A5:A6)", CellValue::Number(0.)),
        ("=AVERAGE(A5:A6)", CellValue::Error(CellError::DivByZero)),
    ] {
        assert_eq!(eval(&mut w, a, f), v, "{f}");
    }
}
#[test]
fn sheets_h05_transitive_dependents_only() {
    let (mut w, a, b) = fixture();
    w.set_formula(key(a, "B1"), "=A1+1").unwrap();
    w.set_formula(key(a, "C1"), "=B1*2").unwrap();
    w.set_formula(key(b, "A1"), "=Sheet1!C1+3").unwrap();
    w.set_formula(key(a, "D1"), "=99").unwrap();
    let e = w.set_value(key(a, "A1"), CellValue::Number(4.)).unwrap();
    assert_eq!(
        e.recalculation.evaluated,
        vec![key(a, "B1"), key(a, "C1"), key(b, "A1")]
    );
    assert_eq!(value(&w, b, "A1"), CellValue::Number(13.));
    assert_eq!(
        w.set_value(key(a, "Z1"), CellValue::Number(8.))
            .unwrap()
            .recalculation
            .evaluated
            .len(),
        0
    );
}
#[test]
fn sheets_h06_cycles_and_recovery() {
    let (mut w, a, b) = fixture();
    let e = w.set_formula(key(a, "A1"), "=A1").unwrap();
    assert_eq!(e.recalculation.circular, vec![key(a, "A1")]);
    w.set_formula(key(a, "B1"), "=A1+1").unwrap();
    w.set_formula(key(a, "A1"), "=B1").unwrap();
    assert_eq!(
        value(&w, a, "B1"),
        CellValue::Error(CellError::CircularReference)
    );
    w.set_value(key(a, "A1"), CellValue::Number(5.)).unwrap();
    assert_eq!(value(&w, a, "B1"), CellValue::Number(6.));
    w.set_formula(key(a, "A1"), "='日本語 集計'!A1").unwrap();
    let e = w.set_formula(key(b, "A1"), "=Sheet1!A1").unwrap();
    assert_eq!(e.recalculation.circular.len(), 3);
    assert_eq!(
        value(&w, b, "A1"),
        CellValue::Error(CellError::CircularReference)
    );
}
#[test]
fn sheets_h07_errors_and_atomicity() {
    let (mut w, a, b) = fixture();
    assert_eq!(
        eval(&mut w, a, "=1/0"),
        CellValue::Error(CellError::DivByZero)
    );
    assert_eq!(
        eval(&mut w, a, "=1e308*10"),
        CellValue::Error(CellError::NumericError)
    );
    assert_eq!(
        eval(&mut w, a, "=\"日本語\"+1"),
        CellValue::Error(CellError::ValueError)
    );
    for (f, e) in [
        ("=SUM(", CellError::InvalidSyntax),
        ("=UNKNOWN(1)", CellError::NameError),
        ("=A0", CellError::InvalidReference),
        ("=Missing!A1", CellError::InvalidReference),
    ] {
        assert_eq!(w.set_formula(key(a, "A1"), f), Err(e));
        assert!(w.cell(key(a, "A1")).is_none());
    }
    let e = w.set_values(vec![
        (key(a, "A1"), CellInput::Value(CellValue::Number(3.))),
        (key(a, "B1"), CellInput::Formula("=A0".into())),
    ]);
    assert_eq!(e, Err(CellError::InvalidReference));
    assert_eq!(value(&w, a, "A1"), CellValue::Empty);
    w.set_formula(key(a, "A1"), "='日本語 集計'!A1").unwrap();
    let e = w.delete_sheet(b).unwrap();
    assert_eq!(e.recalculation.evaluated, vec![key(a, "A1")]);
    assert_eq!(
        value(&w, a, "A1"),
        CellValue::Error(CellError::InvalidReference)
    );
    assert_eq!(CellError::DivByZero.code(), "DIV_BY_ZERO");
}
#[test]
fn sheets_h08_replacement_removes_edges() {
    let (mut w, a, _) = fixture();
    w.set_formula(key(a, "B1"), "=A1+1").unwrap();
    w.set_formula(key(a, "B1"), "=C1+2").unwrap();
    assert_eq!(
        w.dependencies(key(a, "B1")).collect::<Vec<_>>(),
        vec![key(a, "C1")]
    );
    assert_eq!(
        w.set_value(key(a, "A1"), CellValue::Number(5.))
            .unwrap()
            .recalculation
            .evaluated
            .len(),
        0
    );
    assert_eq!(
        w.set_value(key(a, "C1"), CellValue::Number(8.))
            .unwrap()
            .recalculation
            .evaluated
            .len(),
        1
    );
    w.set_value(key(a, "B1"), CellValue::Empty).unwrap();
    assert_eq!(w.dependency_edge_count(), 0);
    assert_eq!(
        w.set_value(key(a, "C1"), CellValue::Empty)
            .unwrap()
            .recalculation
            .evaluated
            .len(),
        0
    );
}
#[test]
fn sheets_h09_unicode_and_canonical_names() {
    let (mut w, a, b) = fixture();
    w.set_value(key(b, "A1"), CellValue::Text("日本語😀".into()))
        .unwrap();
    assert_eq!(
        eval(&mut w, a, "=LEFT('日本語 集計'!A1,3)"),
        CellValue::Text("日本語".into())
    );
    for (f, v) in [
        ("=LEN(\"日本語😀\")", CellValue::Number(4.)),
        ("=RIGHT(\"日本語😀\",2)", CellValue::Text("語😀".into())),
        ("=MID(\"日本語😀\",2,2)", CellValue::Text("本語".into())),
        (
            "=TRIM(\"  日本語   abc  \")",
            CellValue::Text("日本語 abc".into()),
        ),
        (
            "=CONCAT(\"日\",\"本\",1,TRUE)",
            CellValue::Text("日本1TRUE".into()),
        ),
    ] {
        assert_eq!(eval(&mut w, a, f), v);
    }
    let f = parse_formula("=sum(1,2)").unwrap();
    assert!(matches!(
        f.nodes().last(),
        Some(Expr::Call(FunctionId::Sum, _))
    ));
    assert_eq!(FunctionId::Sum.canonical_name(), "SUM");
    assert_eq!(parse_formula("=合計(1,2)"), Err(CellError::NameError));
}
#[test]
fn sheets_h10_sparse_100k_and_long_chain() {
    let (mut w, a, _) = fixture();
    let input = (1..=100_000)
        .map(|r| {
            (
                CellKey::new(a, CellAddress::new(r, 1).unwrap()),
                CellInput::Value(CellValue::Number(r as f64)),
            )
        })
        .collect();
    w.set_values(input).unwrap();
    assert_eq!(w.populated_cell_count(), 100_000);
    assert_eq!(value(&w, a, "XFD1048576"), CellValue::Empty);
    assert_eq!(
        w.set_value(key(a, "B1"), CellValue::Number(1.)),
        Err(CellError::LimitExceeded)
    );
    let (mut w, a, _) = fixture();
    let n = 10_000;
    let input = (2..=n)
        .map(|r| {
            (
                CellKey::new(a, CellAddress::new(r, 1).unwrap()),
                CellInput::Formula(format!("=A{}+1", r - 1)),
            )
        })
        .collect();
    let e = w.set_values(input).unwrap();
    assert_eq!(e.recalculation.evaluated.len(), n as usize - 1);
    let e = w.set_value(key(a, "A1"), CellValue::Number(1.)).unwrap();
    assert_eq!(e.recalculation.evaluated.len(), n as usize - 1);
    assert_eq!(value(&w, a, "A10000"), CellValue::Number(10_000.));
    let e = w.set_formula(key(a, "A1"), "=A10000").unwrap();
    assert_eq!(e.recalculation.circular.len(), n as usize);
    w.set_value(key(a, "A1"), CellValue::Number(1.)).unwrap();
    assert_eq!(value(&w, a, "A10000"), CellValue::Number(10_000.));
}
#[test]
fn untrusted_formula_and_range_bounds() {
    for f in [
        format!("={}1{}", "(".repeat(500), ")".repeat(500)),
        format!("={}1", "-".repeat(500)),
        format!("=1{}", "+1".repeat(500)),
        format!("={}1", " ".repeat(MAX_FORMULA_BYTES)),
    ] {
        assert_eq!(parse_formula(&f), Err(CellError::LimitExceeded));
    }
    assert_eq!(
        parse_formula("=SUM(A1:XFD1048576)"),
        Err(CellError::LimitExceeded)
    );
    let (mut w, a, _) = fixture();
    assert_eq!(
        w.set_formula(key(a, "A1"), "=SUM(B1:B100000,B1:B100000)"),
        Err(CellError::LimitExceeded)
    );
    assert_eq!(
        w.get_range(
            a,
            CellAddress::from_a1("B2").unwrap(),
            CellAddress::from_a1("A1").unwrap()
        ),
        Err(CellError::InvalidReference)
    );
}
#[test]
fn snapshot_roundtrip_rename_cycles_and_validation() {
    let (mut w, a, b) = fixture();
    w.set_value(key(b, "A1"), CellValue::Number(9.)).unwrap();
    w.set_formula(key(a, "A1"), "='日本語 集計'!A1+1").unwrap();
    w.rename_sheet(b, "改名").unwrap();
    let mut loaded = Workbook::load(w.snapshot()).unwrap();
    assert_eq!(value(&loaded, a, "A1"), CellValue::Number(10.));
    loaded
        .set_value(key(b, "A1"), CellValue::Number(11.))
        .unwrap();
    assert_eq!(value(&loaded, a, "A1"), CellValue::Number(12.));
    let mut snap = w.snapshot();
    snap.version += 1;
    assert_eq!(
        Workbook::load(snap).unwrap_err(),
        CellError::UnsupportedVersion
    );
    let mut snap = w.snapshot();
    let duplicate = snap.sheets[0].cells[0].clone();
    snap.sheets[0].cells.push(duplicate);
    assert_eq!(
        Workbook::load(snap).unwrap_err(),
        CellError::DuplicateIdentity
    );
    loaded.delete_sheet(b).unwrap();
    let loaded = Workbook::load(loaded.snapshot()).unwrap();
    assert_eq!(
        value(&loaded, a, "A1"),
        CellValue::Error(CellError::InvalidReference)
    );
}
#[test]
fn batch_topology_diamond_and_error_recovery() {
    let (mut w, a, _) = fixture();
    let e = w
        .set_values(vec![
            (key(a, "D1"), CellInput::Formula("=B1+C1".into())),
            (key(a, "C1"), CellInput::Formula("=A1+2".into())),
            (key(a, "B1"), CellInput::Formula("=A1+1".into())),
            (key(a, "A1"), CellInput::Value(CellValue::Number(3.))),
        ])
        .unwrap();
    assert_eq!(e.recalculation.evaluated.len(), 3);
    assert_eq!(value(&w, a, "D1"), CellValue::Number(9.));
    w.set_formula(key(a, "A1"), "=1/0").unwrap();
    assert_eq!(value(&w, a, "D1"), CellValue::Error(CellError::DivByZero));
    w.set_value(key(a, "A1"), CellValue::Number(3.)).unwrap();
    assert_eq!(value(&w, a, "D1"), CellValue::Number(9.));
}

#[test]
fn sheet_identity_cannot_be_recycled_even_after_snapshot() {
    let (mut w, a, b) = fixture();
    w.delete_sheet(b).unwrap();
    assert_eq!(
        w.create_sheet(b.object_id(), "reused"),
        Err(CellError::DuplicateIdentity)
    );
    assert_eq!(
        w.set_value(key(b, "A1"), CellValue::Number(1.)),
        Err(CellError::InvalidReference)
    );
    let mut loaded = Workbook::load(w.snapshot()).unwrap();
    assert_eq!(
        loaded.create_sheet(b.object_id(), "reused"),
        Err(CellError::DuplicateIdentity)
    );
    assert_eq!(value(&loaded, a, "A1"), CellValue::Empty);
}
#[test]
fn formula_errors_are_reported_as_unsuccessful_events() {
    let (mut w, a, _) = fixture();
    let event = w.set_formula(key(a, "A1"), "=1/0").unwrap();
    assert!(!event.is_successful());
    assert_eq!(
        event.recalculation.errors,
        vec![(key(a, "A1"), CellError::DivByZero)]
    );
    let event = w.set_formula(key(a, "A1"), "=A1").unwrap();
    assert!(!event.is_successful());
    let event = w.set_formula(key(a, "A1"), "=2").unwrap();
    assert!(event.is_successful());
}
#[test]
fn graph_limits_are_atomic_and_ranges_include_empty_cells() {
    let (mut w, a, _) = fixture();
    w.set_formula(key(a, "B1"), "=SUM(A1:A100000)").unwrap();
    let event = w
        .set_value(key(a, "A99999"), CellValue::Number(4.))
        .unwrap();
    assert_eq!(event.recalculation.evaluated, vec![key(a, "B1")]);
    assert_eq!(value(&w, a, "B1"), CellValue::Number(4.));
    let inputs = (1..=10)
        .map(|r| {
            (
                CellKey::new(a, CellAddress::new(r, 3).unwrap()),
                CellInput::Formula("=SUM(A1:A100000)".into()),
            )
        })
        .collect();
    assert_eq!(w.set_values(inputs), Err(CellError::LimitExceeded));
    assert_eq!(w.dependency_edge_count(), 100_000);
    assert_eq!(value(&w, a, "C1"), CellValue::Empty);
    w.set_formula(key(a, "B1"), "=SUM(A1:A2)").unwrap();
    assert_eq!(w.dependency_edge_count(), 2);
}
#[test]
fn invalid_snapshots_reject_missing_extra_and_duplicate_bindings() {
    let (mut w, a, b) = fixture();
    w.set_formula(key(a, "A1"), "=B1").unwrap();
    for mode in 0..4 {
        let mut snapshot = w.snapshot();
        let SnapshotInput::Formula(f) = &mut snapshot.sheets[0].cells[0].1 else {
            panic!("formula");
        };
        match mode {
            0 => f.bindings.clear(),
            1 => f.bindings[0].1 = b.object_id(),
            2 => f.bindings[0].0 = 999,
            _ => f.bindings.push(f.bindings[0]),
        };
        assert!(Workbook::load(snapshot).is_err());
    }
    let mut snapshot = w.snapshot();
    snapshot.sheets[0].cells = vec![
        (
            CellAddress::from_a1("A1").unwrap(),
            SnapshotInput::Value(CellValue::Empty)
        );
        2
    ];
    assert_eq!(
        Workbook::load(snapshot).unwrap_err(),
        CellError::DuplicateIdentity
    );
}
#[test]
fn incremental_matches_rebuilt_graph_for_many_edits() {
    let (mut w, a, _) = fixture();
    for formula in [
        ("B1", "=A1+1"),
        ("C1", "=B1*2"),
        ("D1", "=IF(A1>0,C1,0)"),
        ("E1", "=SUM(B1:D1)"),
    ] {
        w.set_formula(key(a, formula.0), formula.1).unwrap();
    }
    for i in -20..20 {
        w.set_value(key(a, "A1"), CellValue::Number(i as f64))
            .unwrap();
        let rebuilt = Workbook::load(w.snapshot()).unwrap();
        for cell in ["A1", "B1", "C1", "D1", "E1"] {
            assert_eq!(value(&w, a, cell), value(&rebuilt, a, cell));
        }
    }
}
#[test]
fn parser_malformed_corpus_on_small_stack() {
    std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(|| {
            for f in [
                "",
                "=",
                "=1+",
                "=SUM(,)",
                "=A1:",
                "='broken!A1",
                "=\"unclosed",
                "=1..2",
                "=XFE1",
                "=A1048577",
                "=A1:B0",
                "=SUM(A2:A1)",
                "=IF(TRUE,1,2))",
            ] {
                assert!(parse_formula(f).is_err(), "{f}");
            }
            let alphabet = [
                '=', '(', ')', ',', '日', '"', '\'', '$', 'A', '0', '+', ':', '!', '\0',
            ];
            let mut seed = 7u32;
            for _ in 0..2000 {
                let mut f = String::from("=");
                for _ in 0..60 {
                    seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                    f.push(alphabet[(seed as usize) % alphabet.len()]);
                }
                let _ = parse_formula(&f);
            }
        })
        .unwrap()
        .join()
        .unwrap();
}
