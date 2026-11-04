use crate::engine::{Binding, Formula};
use crate::parser::range_size;
use crate::*;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// Opaque identity supplied by a caller, using the existing model ObjectId contract.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SheetId(u64);
impl SheetId {
    pub const fn object_id(self) -> ObjectId {
        ObjectId(self.0)
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CellKey {
    pub sheet: SheetId,
    pub address: CellAddress,
}
impl CellKey {
    pub const fn new(sheet: SheetId, address: CellAddress) -> Self {
        Self { sheet, address }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub enum CellInput {
    Value(CellValue),
    Formula(String),
}
#[derive(Clone, Debug)]
pub struct Cell {
    input: CellInput,
    value: CellValue,
    formula: Option<Formula>,
}
impl Cell {
    pub fn input(&self) -> &CellInput {
        &self.input
    }
    pub fn value(&self) -> &CellValue {
        &self.value
    }
}
#[derive(Clone, Debug)]
pub struct Sheet {
    id: SheetId,
    name: String,
}
impl Sheet {
    pub const fn id(&self) -> SheetId {
        self.id
    }
    pub fn name(&self) -> &str {
        &self.name
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Recalculation {
    pub evaluated: Vec<CellKey>,
    pub circular: Vec<CellKey>,
    pub errors: Vec<(CellKey, CellError)>,
}
/// In-memory event seam, with no Activity, Wayback or permission service claims.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ChangeEvent {
    pub workbook: ObjectId,
    pub changed: Vec<CellKey>,
    pub recalculation: Recalculation,
}
#[derive(Clone, Debug)]
pub struct Workbook {
    id: ObjectId,
    sheets: BTreeMap<SheetId, Sheet>,
    used_sheet_ids: BTreeSet<SheetId>,
    names: BTreeMap<String, SheetId>,
    cells: BTreeMap<CellKey, Cell>,
    dependencies: BTreeMap<CellKey, BTreeSet<CellKey>>,
    dependents: BTreeMap<CellKey, BTreeSet<CellKey>>,
    edge_count: usize,
}
impl ChangeEvent {
    /// False when a committed formula produced an error; never claims formula success.
    pub fn is_successful(&self) -> bool {
        self.recalculation.errors.is_empty()
    }
}
const EMPTY: CellValue = CellValue::Empty;
impl Workbook {
    pub fn new(id: ObjectId) -> Self {
        Self {
            id,
            sheets: BTreeMap::new(),
            used_sheet_ids: BTreeSet::new(),
            names: BTreeMap::new(),
            cells: BTreeMap::new(),
            dependencies: BTreeMap::new(),
            dependents: BTreeMap::new(),
            edge_count: 0,
        }
    }
    pub const fn id(&self) -> ObjectId {
        self.id
    }
    pub fn sheet(&self, id: SheetId) -> Option<&Sheet> {
        self.sheets.get(&id)
    }
    pub fn sheet_by_name(&self, name: &str) -> Option<SheetId> {
        self.names.get(name).copied()
    }
    pub fn sheets(&self) -> impl Iterator<Item = &Sheet> {
        self.sheets.values()
    }
    pub fn populated_cell_count(&self) -> usize {
        self.cells.len()
    }
    pub const fn dependency_edge_count(&self) -> usize {
        self.edge_count
    }
    pub fn dependencies(&self, cell: CellKey) -> impl Iterator<Item = CellKey> + '_ {
        self.dependencies
            .get(&cell)
            .into_iter()
            .flat_map(|s| s.iter().copied())
    }
    fn name_available(&self, name: &str, except: Option<SheetId>) -> Result<(), CellError> {
        if name.trim().is_empty() || name.len() > 255 {
            return Err(CellError::ValueError);
        }
        if self.names.get(name).is_some_and(|id| Some(*id) != except) {
            return Err(CellError::DuplicateName);
        }
        Ok(())
    }
    pub fn create_sheet(&mut self, id: ObjectId, name: &str) -> Result<SheetId, CellError> {
        let id = SheetId(id.0);
        if id.0 == self.id.0 || self.used_sheet_ids.contains(&id) {
            return Err(CellError::DuplicateIdentity);
        }
        self.name_available(name, None)?;
        if self.used_sheet_ids.len() >= MAX_SHEETS {
            return Err(CellError::LimitExceeded);
        }
        self.used_sheet_ids.insert(id);
        self.sheets.insert(
            id,
            Sheet {
                id,
                name: name.into(),
            },
        );
        self.names.insert(name.into(), id);
        Ok(id)
    }
    pub fn rename_sheet(&mut self, id: SheetId, name: &str) -> Result<(), CellError> {
        if !self.sheets.contains_key(&id) {
            return Err(CellError::InvalidReference);
        }
        self.name_available(name, Some(id))?;
        let sheet = self.sheets.get_mut(&id).expect("validated sheet");
        self.names.remove(&sheet.name);
        sheet.name = name.into();
        self.names.insert(name.into(), id);
        Ok(())
    }
    pub fn get_cell(&self, key: CellKey) -> Result<&CellValue, CellError> {
        if !self.sheets.contains_key(&key.sheet) {
            return Err(CellError::InvalidReference);
        }
        Ok(self.cells.get(&key).map_or(&EMPTY, |c| &c.value))
    }
    pub fn cell(&self, key: CellKey) -> Option<&Cell> {
        self.cells.get(&key)
    }
    pub fn set_value(&mut self, key: CellKey, value: CellValue) -> Result<ChangeEvent, CellError> {
        self.set_values(vec![(key, CellInput::Value(value))])
    }
    pub fn set_formula(&mut self, key: CellKey, formula: &str) -> Result<ChangeEvent, CellError> {
        self.set_values(vec![(key, CellInput::Formula(formula.into()))])
    }
    fn compile(
        &self,
        current: SheetId,
        ast: ParsedFormula,
        edge_budget: usize,
    ) -> Result<(Formula, BTreeSet<CellKey>), CellError> {
        let mut bindings = BTreeMap::new();
        let mut refs = BTreeSet::new();
        let mut visits = 0;
        for (i, node) in ast.nodes.iter().enumerate() {
            let (start, end) = match node {
                Expr::Reference(r) => (r, r),
                Expr::Range(a, b) => (a, b),
                _ => continue,
            };
            let sheet = match &start.sheet {
                Some(name) => self
                    .sheet_by_name(name)
                    .ok_or(CellError::InvalidReference)?,
                None => current,
            };
            visits += range_size(start.address, end.address)?;
            if visits > MAX_RANGE_CELLS {
                return Err(CellError::LimitExceeded);
            }
            for row in start.address.row()..=end.address.row() {
                for column in start.address.column()..=end.address.column() {
                    let key = CellKey::new(sheet, CellAddress::new(row, column)?);
                    if refs.len() == edge_budget && !refs.contains(&key) {
                        return Err(CellError::LimitExceeded);
                    }
                    refs.insert(key);
                }
            }
            bindings.insert(
                i,
                Binding {
                    sheet,
                    start: start.address,
                    end: end.address,
                },
            );
        }
        Ok((Formula { ast, bindings }, refs))
    }
    fn remove_edges(&mut self, key: CellKey) {
        if let Some(refs) = self.dependencies.remove(&key) {
            self.edge_count -= refs.len();
            for r in refs {
                if let Some(set) = self.dependents.get_mut(&r) {
                    set.remove(&key);
                    if set.is_empty() {
                        self.dependents.remove(&r);
                    }
                }
            }
        }
    }
    /// Transactional validation; invalid input changes neither cells nor graph.
    /// Evaluation errors are committed as typed cell errors and reported in the event.
    pub fn set_values(
        &mut self,
        values: Vec<(CellKey, CellInput)>,
    ) -> Result<ChangeEvent, CellError> {
        if values.len() > MAX_POPULATED_CELLS {
            return Err(CellError::LimitExceeded);
        }
        let mut seen = BTreeSet::new();
        let mut cell_count = self.cells.len();
        let mut edges = self.edge_count;
        // Credit every removal before preparing replacements, so input order
        // cannot reject a batch whose final graph and populated cells fit.
        for (key, input) in &values {
            if !self.sheets.contains_key(&key.sheet) {
                return Err(CellError::InvalidReference);
            }
            if !seen.insert(*key) {
                return Err(CellError::DuplicateIdentity);
            }
            if self.cells.contains_key(key) {
                cell_count -= 1;
            }
            if *input != CellInput::Value(CellValue::Empty) {
                cell_count += 1;
            }
            edges -= self.dependencies.get(key).map_or(0, BTreeSet::len);
        }
        if cell_count > MAX_POPULATED_CELLS {
            return Err(CellError::LimitExceeded);
        }
        let mut prepared = Vec::new();
        for (key, input) in values {
            let (formula, refs, value) = match &input {
                CellInput::Value(v) => {
                    v.validate()?;
                    (None, BTreeSet::new(), v.clone())
                }
                CellInput::Formula(s) => {
                    let (f, refs) =
                        self.compile(key.sheet, parse_formula(s)?, MAX_DEPENDENCY_EDGES - edges)?;
                    (Some(f), refs, CellValue::Empty)
                }
            };
            let empty = input == CellInput::Value(CellValue::Empty);
            edges += refs.len();
            prepared.push((
                key,
                Cell {
                    input,
                    value,
                    formula,
                },
                refs,
                empty,
            ));
        }
        let changed: Vec<_> = seen.into_iter().collect();
        // Preparation is complete. Detach all old edges before installing new
        // ones to keep the graph within the limit during replacement too.
        for key in &changed {
            self.remove_edges(*key);
        }
        for (key, cell, refs, empty) in prepared {
            if empty {
                self.cells.remove(&key);
            } else {
                self.cells.insert(key, cell);
            }
            if !refs.is_empty() {
                for r in &refs {
                    self.dependents.entry(*r).or_default().insert(key);
                }
                self.edge_count += refs.len();
                self.dependencies.insert(key, refs);
            }
        }
        let recalculation = self.recalculate(&changed);
        Ok(ChangeEvent {
            workbook: self.id,
            changed,
            recalculation,
        })
    }
    fn recalculate(&mut self, changed: &[CellKey]) -> Recalculation {
        let mut affected = BTreeSet::new();
        let mut queue: VecDeque<_> = changed.iter().copied().collect();
        while let Some(key) = queue.pop_front() {
            if !affected.insert(key) {
                continue;
            }
            if let Some(refs) = self.dependents.get(&key) {
                queue.extend(refs.iter().copied());
            }
        }
        let formulas: BTreeSet<_> = affected
            .into_iter()
            .filter(|k| self.cells.get(k).is_some_and(|c| c.formula.is_some()))
            .collect();
        let mut indegree = BTreeMap::new();
        let mut ready = BTreeSet::new();
        for key in &formulas {
            let n = self.dependencies.get(key).map_or(0, |refs| {
                refs.iter().filter(|r| formulas.contains(r)).count()
            });
            indegree.insert(*key, n);
            if n == 0 {
                ready.insert(*key);
            }
        }
        let mut result = Recalculation::default();
        while let Some(key) = ready.pop_first() {
            let value = self.cells[&key]
                .formula
                .as_ref()
                .expect("formula set")
                .evaluate(self);
            if let CellValue::Error(error) = value {
                result.errors.push((key, error));
            }
            self.cells.get_mut(&key).expect("formula cell").value = value;
            result.evaluated.push(key);
            if let Some(refs) = self.dependents.get(&key) {
                for r in refs {
                    if let Some(n) = indegree.get_mut(r) {
                        *n -= 1;
                        if *n == 0 {
                            ready.insert(*r);
                        }
                    }
                }
            }
            indegree.remove(&key);
        }
        for key in indegree.keys() {
            self.cells.get_mut(key).expect("remaining formula").value =
                CellValue::Error(CellError::CircularReference);
            result.evaluated.push(*key);
            result.circular.push(*key);
            result.errors.push((*key, CellError::CircularReference));
        }
        result
    }
    pub fn delete_sheet(&mut self, id: SheetId) -> Result<ChangeEvent, CellError> {
        let sheet = self.sheets.remove(&id).ok_or(CellError::InvalidReference)?;
        self.names.remove(&sheet.name);
        let keys: Vec<_> = self
            .cells
            .keys()
            .chain(self.dependents.keys())
            .filter(|k| k.sheet == id)
            .copied()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        for key in &keys {
            self.remove_edges(*key);
            self.cells.remove(key);
        }
        let recalculation = self.recalculate(&keys);
        Ok(ChangeEvent {
            workbook: self.id,
            changed: keys,
            recalculation,
        })
    }
    pub fn get_range(
        &self,
        sheet: SheetId,
        start: CellAddress,
        end: CellAddress,
    ) -> Result<Vec<CellValue>, CellError> {
        let n = range_size(start, end)?;
        if !self.sheets.contains_key(&sheet) {
            return Err(CellError::InvalidReference);
        }
        let mut out = Vec::with_capacity(n);
        for row in start.row()..=end.row() {
            for col in start.column()..=end.column() {
                out.push(
                    self.get_cell(CellKey::new(sheet, CellAddress::new(row, col)?))?
                        .clone(),
                );
            }
        }
        Ok(out)
    }
}

pub const SNAPSHOT_VERSION: u32 = 1;
/// Typed in-memory snapshot, not a durable storage format or a permission token.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub version: u32,
    pub workbook: ObjectId,
    pub retired_sheet_ids: Vec<ObjectId>,
    pub sheets: Vec<SheetSnapshot>,
}
#[derive(Clone, Debug)]
pub struct SheetSnapshot {
    pub id: ObjectId,
    pub name: String,
    pub cells: Vec<(CellAddress, SnapshotInput)>,
}
#[derive(Clone, Debug)]
pub enum SnapshotInput {
    Value(CellValue),
    Formula(FormulaSnapshot),
}
/// Preserves resolved sheet IDs across rename. AST is reparsed and binding coordinates
/// are checked on load; caches and dependency edges are never trusted from the snapshot.
#[derive(Clone, Debug)]
pub struct FormulaSnapshot {
    pub source: String,
    pub bindings: Vec<(usize, ObjectId)>,
}
impl Workbook {
    pub fn snapshot(&self) -> Snapshot {
        Snapshot {
            version: SNAPSHOT_VERSION,
            workbook: self.id,
            retired_sheet_ids: self
                .used_sheet_ids
                .iter()
                .filter(|id| !self.sheets.contains_key(id))
                .map(|id| id.object_id())
                .collect(),
            sheets: self
                .sheets
                .values()
                .map(|sheet| {
                    let cells = self
                        .cells
                        .range(
                            CellKey::new(sheet.id, CellAddress::new(1, 1).expect("minimum address"))
                                ..=CellKey::new(
                                    sheet.id,
                                    CellAddress::new(MAX_ROWS, MAX_COLUMNS)
                                        .expect("maximum address"),
                                ),
                        )
                        .map(|(k, c)| {
                            let input = match &c.input {
                                CellInput::Value(v) => SnapshotInput::Value(v.clone()),
                                CellInput::Formula(s) => SnapshotInput::Formula(FormulaSnapshot {
                                    source: s.clone(),
                                    bindings: c
                                        .formula
                                        .as_ref()
                                        .expect("formula")
                                        .bindings
                                        .iter()
                                        .map(|(i, b)| (*i, b.sheet.object_id()))
                                        .collect(),
                                }),
                            };
                            (k.address, input)
                        })
                        .collect();
                    SheetSnapshot {
                        id: sheet.id.object_id(),
                        name: sheet.name.clone(),
                        cells,
                    }
                })
                .collect(),
        }
    }
    pub fn load(snapshot: Snapshot) -> Result<Self, CellError> {
        if snapshot.version != SNAPSHOT_VERSION {
            return Err(CellError::UnsupportedVersion);
        }
        if snapshot
            .sheets
            .len()
            .saturating_add(snapshot.retired_sheet_ids.len())
            > MAX_SHEETS
        {
            return Err(CellError::LimitExceeded);
        }
        let mut book = Self::new(snapshot.workbook);
        for s in &snapshot.sheets {
            book.create_sheet(s.id, &s.name)?;
        }
        for id in snapshot.retired_sheet_ids {
            if id == book.id || !book.used_sheet_ids.insert(SheetId(id.0)) {
                return Err(CellError::DuplicateIdentity);
            }
        }
        let total: usize = snapshot.sheets.iter().map(|s| s.cells.len()).sum();
        if total > MAX_POPULATED_CELLS {
            return Err(CellError::LimitExceeded);
        }
        let mut seen = BTreeSet::new();
        for s in snapshot.sheets {
            for (address, input) in s.cells {
                let key = CellKey::new(SheetId(s.id.0), address);
                if !seen.insert(key) {
                    return Err(CellError::DuplicateIdentity);
                }
                let (input, formula, value, refs) = match input {
                    SnapshotInput::Value(v) => {
                        v.validate()?;
                        (CellInput::Value(v.clone()), None, v, BTreeSet::new())
                    }
                    SnapshotInput::Formula(f) => {
                        let ast = parse_formula(&f.source)?;
                        let mut bindings = BTreeMap::new();
                        let mut refs = BTreeSet::new();
                        let mut visits = 0;
                        if f.bindings.len() > MAX_AST_NODES {
                            return Err(CellError::LimitExceeded);
                        }
                        for (i, id) in f.bindings {
                            let (a, b) = match ast.nodes.get(i) {
                                Some(Expr::Reference(a)) => (a, a),
                                Some(Expr::Range(a, b)) => (a, b),
                                _ => return Err(CellError::InvalidReference),
                            };
                            if !book.used_sheet_ids.contains(&SheetId(id.0))
                                || (a.sheet.is_none() && id != s.id)
                            {
                                return Err(CellError::InvalidReference);
                            }
                            visits += range_size(a.address, b.address)?;
                            if visits > MAX_RANGE_CELLS {
                                return Err(CellError::LimitExceeded);
                            }
                            if bindings
                                .insert(
                                    i,
                                    Binding {
                                        sheet: SheetId(id.0),
                                        start: a.address,
                                        end: b.address,
                                    },
                                )
                                .is_some()
                            {
                                return Err(CellError::DuplicateIdentity);
                            }
                            for row in a.address.row()..=b.address.row() {
                                for col in a.address.column()..=b.address.column() {
                                    refs.insert(CellKey::new(
                                        SheetId(id.0),
                                        CellAddress::new(row, col)?,
                                    ));
                                }
                            }
                        }
                        if bindings.len()
                            != ast
                                .nodes
                                .iter()
                                .filter(|n| matches!(n, Expr::Reference(_) | Expr::Range(_, _)))
                                .count()
                        {
                            return Err(CellError::InvalidReference);
                        }
                        (
                            CellInput::Formula(f.source),
                            Some(Formula { ast, bindings }),
                            CellValue::Empty,
                            refs,
                        )
                    }
                };
                book.edge_count += refs.len();
                if book.edge_count > MAX_DEPENDENCY_EDGES {
                    return Err(CellError::LimitExceeded);
                }
                if input != CellInput::Value(CellValue::Empty) {
                    book.cells.insert(
                        key,
                        Cell {
                            input,
                            value,
                            formula,
                        },
                    );
                }
                if !refs.is_empty() {
                    for r in &refs {
                        book.dependents.entry(*r).or_default().insert(key);
                    }
                    book.dependencies.insert(key, refs);
                }
            }
        }
        let changed: Vec<_> = book.cells.keys().copied().collect();
        book.recalculate(&changed);
        Ok(book)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(sheet: SheetId, row: u32, column: u32) -> CellKey {
        CellKey::new(sheet, CellAddress::new(row, column).unwrap())
    }

    fn full_graph() -> (Workbook, SheetId) {
        let mut book = Workbook::new(ObjectId(1));
        let sheet = book.create_sheet(ObjectId(2), "Data").unwrap();
        book.set_value(key(sheet, 1, 1), CellValue::Number(2.))
            .unwrap();
        let values = (1..=10)
            .map(|row| {
                (
                    key(sheet, row, 2),
                    CellInput::Formula("=SUM(A1:A100000)".into()),
                )
            })
            .collect();
        assert!(book.set_values(values).unwrap().is_successful());
        assert_eq!(book.edge_count, MAX_DEPENDENCY_EDGES);
        (book, sheet)
    }

    fn assert_same_workbook(actual: &Workbook, expected: &Workbook) {
        // Snapshots include identities, names, inputs and resolved bindings.
        assert_eq!(
            format!("{:?}", actual.snapshot()),
            format!("{:?}", expected.snapshot())
        );
        assert_eq!(actual.cells.len(), expected.cells.len());
        for (key, cell) in &actual.cells {
            let expected = &expected.cells[key];
            assert_eq!(cell.value, expected.value);
            assert_eq!(
                format!("{:?}", cell.formula),
                format!("{:?}", expected.formula)
            );
        }
        assert_eq!(actual.dependencies, expected.dependencies);
        assert_eq!(actual.dependents, expected.dependents);
        assert_eq!(actual.edge_count, expected.edge_count);
    }

    #[test]
    fn batch_preparation_stops_at_edge_budget() {
        let mut book = Workbook::new(ObjectId(1));
        let sheet = book.create_sheet(ObjectId(2), "Data").unwrap();
        let mut values: Vec<_> = (1..=11)
            .map(|row| {
                (
                    key(sheet, row, 2),
                    CellInput::Formula("=SUM(A1:A100000)".into()),
                )
            })
            .collect();
        // Once the edge budget is exhausted, do not compile later formulas.
        values.push((key(sheet, 12, 2), CellInput::Formula("=SUM(".into())));
        assert_eq!(book.set_values(values), Err(CellError::LimitExceeded));
        assert!(book.cells.is_empty());
        assert!(book.dependencies.is_empty());
        assert!(book.dependents.is_empty());
        assert_eq!(book.edge_count, 0);
    }

    #[test]
    fn compile_budget_counts_unique_edges_and_keeps_visit_limit() {
        let mut book = Workbook::new(ObjectId(1));
        let sheet = book.create_sheet(ObjectId(2), "Data").unwrap();
        for (source, budget, expected) in [
            ("=1", 0, Ok(0)),
            ("=A1", 0, Err(CellError::LimitExceeded)),
            ("=SUM(A1,A1)", 1, Ok(1)),
            ("=SUM(A1,A2)", 1, Err(CellError::LimitExceeded)),
            ("=SUM(A1:A50000,A1:A50000)", 50_000, Ok(50_000)),
            (
                "=SUM(A1:A50000,A1:A50000)",
                49_999,
                Err(CellError::LimitExceeded),
            ),
            (
                "=SUM(A1:A50001,A1:A50001)",
                50_001,
                Err(CellError::LimitExceeded),
            ),
        ] {
            assert_eq!(
                book.compile(sheet, parse_formula(source).unwrap(), budget)
                    .map(|(_, refs)| refs.len()),
                expected,
                "{source}, budget {budget}"
            );
        }
    }

    #[test]
    fn capped_graph_deletions_and_replacements_are_order_independent() {
        let (mut forward, sheet) = full_graph();
        let mut reverse = forward.clone();
        let mut values = vec![
            (
                key(sheet, 11, 2),
                CellInput::Formula("=SUM(A1:A100000)".into()),
            ),
            (
                key(sheet, 12, 2),
                CellInput::Formula("=SUM(A1:A50000)".into()),
            ),
            (
                key(sheet, 1, 2),
                CellInput::Formula("=SUM(A1:A50000)".into()),
            ),
            (key(sheet, 10, 2), CellInput::Value(CellValue::Empty)),
        ];
        let forward_event = forward.set_values(values.clone()).unwrap();
        values.reverse();
        let reverse_event = reverse.set_values(values).unwrap();
        assert!(forward_event.is_successful());
        assert_eq!(forward_event, reverse_event);
        assert_eq!(forward.edge_count, MAX_DEPENDENCY_EDGES);
        assert_same_workbook(&forward, &reverse);
        assert_eq!(forward.get_cell(key(sheet, 10, 2)), Ok(&CellValue::Empty));
        assert_eq!(
            forward.get_cell(key(sheet, 11, 2)),
            Ok(&CellValue::Number(2.))
        );
        let event = forward
            .set_value(key(sheet, 99_999, 1), CellValue::Number(3.))
            .unwrap();
        let evaluated: Vec<_> = (2..=9).chain([11]).map(|row| key(sheet, row, 2)).collect();
        assert_eq!(event.recalculation.evaluated, evaluated);
        assert_eq!(
            forward.get_cell(key(sheet, 11, 2)),
            Ok(&CellValue::Number(5.))
        );
        assert_eq!(
            forward.get_cell(key(sheet, 1, 2)),
            Ok(&CellValue::Number(2.))
        );
        assert_eq!(
            forward.get_cell(key(sheet, 12, 2)),
            Ok(&CellValue::Number(2.))
        );
    }

    #[test]
    fn failed_preparation_preserves_cells_and_both_graph_directions() {
        let (mut book, sheet) = full_graph();
        let before = book.clone();
        let removal = (key(sheet, 1, 2), CellInput::Value(CellValue::Empty));
        for (values, error) in [
            (
                vec![
                    removal.clone(),
                    (
                        key(sheet, 11, 2),
                        CellInput::Formula("=SUM(A1:A100000)".into()),
                    ),
                    (key(sheet, 12, 2), CellInput::Formula("=A100001".into())),
                ],
                CellError::LimitExceeded,
            ),
            (
                vec![
                    removal.clone(),
                    (key(sheet, 11, 2), CellInput::Formula("=SUM(".into())),
                ],
                CellError::InvalidSyntax,
            ),
            (
                vec![removal.clone(), removal.clone()],
                CellError::DuplicateIdentity,
            ),
            (
                vec![
                    removal,
                    (
                        key(sheet, 11, 2),
                        CellInput::Value(CellValue::Number(f64::NAN)),
                    ),
                ],
                CellError::NumericError,
            ),
        ] {
            assert_eq!(book.set_values(values), Err(error));
            assert_same_workbook(&book, &before);
        }
        let event = book
            .set_value(key(sheet, 1, 1), CellValue::Number(7.))
            .unwrap();
        let expected: Vec<_> = (1..=10).map(|row| key(sheet, row, 2)).collect();
        assert_eq!(event.recalculation.evaluated, expected);
        for key in expected {
            assert_eq!(book.get_cell(key), Ok(&CellValue::Number(7.)));
        }
    }

    #[test]
    fn populated_cell_replacement_at_capacity_is_order_independent() {
        let mut forward = Workbook::new(ObjectId(1));
        let sheet = forward.create_sheet(ObjectId(2), "Data").unwrap();
        forward
            .set_values(
                (1..=MAX_POPULATED_CELLS as u32)
                    .map(|row| (key(sheet, row, 1), CellInput::Value(CellValue::Number(1.))))
                    .collect(),
            )
            .unwrap();
        let mut reverse = forward.clone();
        let added = key(sheet, MAX_POPULATED_CELLS as u32 + 1, 1);
        let mut values = vec![
            (added, CellInput::Value(CellValue::Number(2.))),
            (key(sheet, 1, 1), CellInput::Value(CellValue::Empty)),
        ];
        let event = forward.set_values(values.clone()).unwrap();
        values.reverse();
        assert_eq!(event, reverse.set_values(values).unwrap());
        assert_same_workbook(&forward, &reverse);
        assert_eq!(forward.populated_cell_count(), MAX_POPULATED_CELLS);
        assert_eq!(forward.get_cell(added), Ok(&CellValue::Number(2.)));
        assert_eq!(forward.get_cell(key(sheet, 1, 1)), Ok(&CellValue::Empty));
    }
}
