use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};

use crate::Row;
use crate::fingerprint::Fingerprint;

/// An addition has no base rows and a deletion no head rows. Rows with one
/// start keep the order `analyze` gives them, the outer row first.
#[derive(Clone, Copy, Debug)]
pub struct ChangedFile<'r> {
    pub base: &'r [Row],
    pub head: &'r [Row],
}

/// A row by its file's position in the classified slice and its own in that side.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct RowId {
    pub file: usize,
    pub row: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Body,
    Name,
    Renamed,
    Moved,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ancestry {
    Paired {
        base: RowId,
        stage: Stage,
    },
    New,
    /// Unpaired base rows of the same stem in the same file pair, in line order.
    Ambiguous {
        candidates: Vec<RowId>,
    },
}

#[derive(Debug, PartialEq, Eq)]
pub struct Matching {
    /// Indexed by file, then by head row.
    pub heads: Vec<Vec<Ancestry>>,
    /// Unpaired base rows, by file then source order.
    pub removed: Vec<RowId>,
}

impl Matching {
    #[must_use]
    pub fn ancestry(
        &self,
        head: RowId,
    ) -> &Ancestry {
        &self.heads[head.file][head.row]
    }
}

/// The name without a terminal `#` and ASCII digits.
pub(crate) fn stem(name: &str) -> &str {
    match name.rsplit_once('#') {
        Some((head, digits))
            if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) =>
        {
            head
        },
        _ => name,
    }
}

#[must_use]
pub fn classify(files: &[ChangedFile<'_>]) -> Matching {
    let mut state = State::new(files);
    for file in 0..files.len() {
        state.by_body(file);
        state.by_name(file);
        state.by_unique_fingerprint(&[file], Stage::Renamed);
    }
    let all: Vec<usize> = (0..files.len()).collect();
    state.by_unique_fingerprint(&all, Stage::Moved);
    state.finish()
}

struct State<'f, 'r> {
    files: &'f [ChangedFile<'r>],
    /// Each file's base and head indices in source order.
    order: Vec<(Vec<usize>, Vec<usize>)>,
    head_partner: Vec<Vec<Option<(RowId, Stage)>>>,
    base_paired: Vec<Vec<bool>>,
}

impl<'f, 'r> State<'f, 'r> {
    fn new(files: &'f [ChangedFile<'r>]) -> Self {
        let source_order = |rows: &[Row]| {
            let mut order: Vec<usize> = (0..rows.len()).collect();
            order.sort_by_key(|&i| rows[i].span.start);
            order
        };
        Self {
            files,
            order: files
                .iter()
                .map(|f| (source_order(f.base), source_order(f.head)))
                .collect(),
            head_partner: files.iter().map(|f| vec![None; f.head.len()]).collect(),
            base_paired: files.iter().map(|f| vec![false; f.base.len()]).collect(),
        }
    }

    fn pair(
        &mut self,
        base: RowId,
        head: RowId,
        stage: Stage,
    ) {
        self.base_paired[base.file][base.row] = true;
        self.head_partner[head.file][head.row] = Some((base, stage));
    }

    fn open_heads(
        &self,
        file: usize,
    ) -> impl Iterator<Item = (usize, &'r Row)> + '_ {
        let rows = self.files[file].head;
        self.order[file]
            .1
            .iter()
            .filter(move |&&i| self.head_partner[file][i].is_none())
            .map(move |&i| (i, &rows[i]))
    }

    fn open_bases(
        &self,
        file: usize,
    ) -> impl Iterator<Item = (usize, &'r Row)> + '_ {
        let rows = self.files[file].base;
        self.order[file]
            .0
            .iter()
            .filter(move |&&i| !self.base_paired[file][i])
            .map(move |&i| (i, &rows[i]))
    }

    /// Stems with more than one row on either original side of `file`.
    fn repeated_stems(
        &self,
        file: usize,
    ) -> HashSet<&'r str> {
        let count = |rows: &'r [Row]| {
            let mut counts: HashMap<&'r str, usize> = HashMap::new();
            for row in rows {
                *counts.entry(stem(&row.name)).or_default() += 1;
            }
            counts
        };
        let file = self.files[file];
        count(file.base)
            .into_iter()
            .chain(count(file.head))
            .filter(|&(_, n)| n > 1)
            .map(|(stem, _)| stem)
            .collect()
    }

    fn by_body(
        &mut self,
        file: usize,
    ) {
        let repeated = self.repeated_stems(file);
        let mut bases: BTreeMap<(&str, Fingerprint), VecDeque<usize>> = BTreeMap::new();
        for (i, row) in self.open_bases(file) {
            let key = stem(&row.name);
            if repeated.contains(key) {
                bases
                    .entry((key, row.fingerprint))
                    .or_default()
                    .push_back(i);
            }
        }
        let mut pairs = Vec::new();
        for (i, row) in self.open_heads(file) {
            if let Some(base) = bases
                .get_mut(&(stem(&row.name), row.fingerprint))
                .and_then(VecDeque::pop_front)
            {
                pairs.push((base, i));
            }
        }
        for (base, head) in pairs {
            self.pair(
                RowId { file, row: base },
                RowId { file, row: head },
                Stage::Body,
            );
        }
    }

    fn by_name(
        &mut self,
        file: usize,
    ) {
        let repeated = self.repeated_stems(file);
        let mut open: HashMap<&str, (usize, usize)> = HashMap::new();
        for (_, row) in self.open_bases(file) {
            open.entry(stem(&row.name)).or_default().0 += 1;
        }
        for (_, row) in self.open_heads(file) {
            open.entry(stem(&row.name)).or_default().1 += 1;
        }
        let bases: HashMap<&str, usize> = self
            .open_bases(file)
            .map(|(i, row)| (row.name.as_str(), i))
            .collect();
        let pairs: Vec<_> = self
            .open_heads(file)
            .filter_map(|(i, row)| {
                let key = stem(&row.name);
                let single = !repeated.contains(key) || open[key] == (1, 1);
                bases
                    .get(row.name.as_str())
                    .filter(|_| single)
                    .map(|&b| (b, i))
            })
            .collect();
        for (base, head) in pairs {
            self.pair(
                RowId { file, row: base },
                RowId { file, row: head },
                Stage::Name,
            );
        }
    }

    /// Pairs each fingerprint that occurs exactly once among the open base
    /// rows and once among the open head rows of `files`.
    fn by_unique_fingerprint(
        &mut self,
        files: &[usize],
        stage: Stage,
    ) {
        let mut seen: HashMap<Fingerprint, (Vec<RowId>, Vec<RowId>)> = HashMap::new();
        for &file in files {
            for (row, base) in self.open_bases(file) {
                seen.entry(base.fingerprint)
                    .or_default()
                    .0
                    .push(RowId { file, row });
            }
            for (row, head) in self.open_heads(file) {
                seen.entry(head.fingerprint)
                    .or_default()
                    .1
                    .push(RowId { file, row });
            }
        }
        for (bases, heads) in seen.into_values() {
            if let ([base], [head]) = (&bases[..], &heads[..]) {
                self.pair(*base, *head, stage);
            }
        }
    }

    fn finish(self) -> Matching {
        let heads = (0..self.files.len())
            .map(|file| {
                let mut open: HashMap<&str, Vec<RowId>> = HashMap::new();
                for (row, base) in self.open_bases(file) {
                    open.entry(stem(&base.name))
                        .or_default()
                        .push(RowId { file, row });
                }
                self.files[file]
                    .head
                    .iter()
                    .zip(&self.head_partner[file])
                    .map(|(head, partner)| match *partner {
                        Some((base, stage)) => Ancestry::Paired { base, stage },
                        None => open
                            .get(stem(&head.name))
                            .map_or(Ancestry::New, |candidates| Ancestry::Ambiguous {
                                candidates: candidates.clone(),
                            }),
                    })
                    .collect()
            })
            .collect();
        let removed = (0..self.files.len())
            .flat_map(|file| {
                self.open_bases(file)
                    .map(move |(row, _)| RowId { file, row })
            })
            .collect();
        Matching { heads, removed }
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::{Mode, Side, analyze};

    fn rows(
        source: &str,
        side: Side,
    ) -> Vec<Row> {
        analyze(Path::new("m.ts"), source.as_bytes(), Mode::Strict(side))
            .unwrap()
            .rows
    }

    fn sides(files: &[(&str, &str)]) -> Vec<(Vec<Row>, Vec<Row>)> {
        files
            .iter()
            .map(|(base, head)| (rows(base, Side::Base), rows(head, Side::Head)))
            .collect()
    }

    fn run(files: &[(Vec<Row>, Vec<Row>)]) -> Matching {
        let changed: Vec<_> = files
            .iter()
            .map(|(base, head)| ChangedFile { base, head })
            .collect();
        classify(&changed)
    }

    fn paired(
        file: usize,
        row: usize,
        stage: Stage,
    ) -> Ancestry {
        Ancestry::Paired {
            base: RowId { file, row },
            stage,
        }
    }

    #[test]
    fn a_stem_strips_only_a_terminal_hash_and_ascii_digits() {
        for (name, expected) in [
            ("hook()#2", "hook()"),
            ("hook()#12", "hook()"),
            ("hook()", "hook()"),
            ("hook()#2::inner", "hook()#2::inner"),
            ("a#2#3", "a#2"),
            ("a#", "a#"),
            ("a#x2", "a#x2"),
            ("a#2b", "a#2b"),
        ] {
            assert_eq!(stem(name), expected, "{name}");
        }
    }

    #[test]
    fn a_rename_pairs_by_its_unique_fingerprint_within_the_file_pair() {
        let files = sides(&[(
            "function edge(x) { return x }\nfunction cold() { return 1 }",
            "function edgeCase(x) { return x }\nfunction cold() { return 1 }",
        )]);
        let matching = run(&files);
        assert_eq!(
            matching.heads[0],
            [paired(0, 0, Stage::Renamed), paired(0, 1, Stage::Name)]
        );
        assert_eq!(matching.removed, []);
    }

    #[test]
    fn a_move_into_another_file_pairs_by_its_unique_fingerprint_across_files() {
        let files = sides(&[
            (
                "function edge(x) { return x }\nfunction cold() { return 1 }",
                "function cold() { return 1 }",
            ),
            ("", "function edge(x) { return x }"),
        ]);
        let matching = run(&files);
        assert_eq!(matching.heads[0], [paired(0, 1, Stage::Name)]);
        assert_eq!(matching.heads[1], [paired(0, 0, Stage::Moved)]);
        assert_eq!(matching.removed, []);
    }

    #[test]
    fn the_renamed_stage_counts_only_its_own_file_pair() {
        let files = sides(&[
            ("function a(x) { return x }", "function b(x) { return x }"),
            ("", "function c(x) { return x }"),
        ]);
        let matching = run(&files);
        assert_eq!(matching.heads[0], [paired(0, 0, Stage::Renamed)]);
        assert_eq!(matching.heads[1], [Ancestry::New]);
    }

    #[test]
    fn a_fingerprint_repeated_across_the_pool_moves_nothing() {
        let files = sides(&[
            ("function one() { return 1 }", ""),
            ("", "function uno() { return 1 }"),
            ("", "function eins() { return 1 }"),
        ]);
        let matching = run(&files);
        assert_eq!(matching.heads[1], [Ancestry::New]);
        assert_eq!(matching.heads[2], [Ancestry::New]);
        assert_eq!(matching.removed, [RowId { file: 0, row: 0 }]);
    }

    #[test]
    fn a_repeated_stem_pairs_by_name_only_when_one_row_remains_on_each_side() {
        let base = "useEffect(() => { a() })\nuseEffect(() => { b() })";
        let kept = sides(&[(base, "useEffect(() => { a() })\nuseEffect(() => { e() })")]);
        assert_eq!(
            run(&kept).heads[0],
            [paired(0, 0, Stage::Body), paired(0, 1, Stage::Name)]
        );
        let both = sides(&[(base, "useEffect(() => { c() })\nuseEffect(() => { d() })")]);
        let matching = run(&both);
        let candidates = vec![RowId { file: 0, row: 0 }, RowId { file: 0, row: 1 }];
        let ambiguous = Ancestry::Ambiguous { candidates };
        assert_eq!(matching.heads[0], [ambiguous.clone(), ambiguous]);
        assert_eq!(matching.removed.len(), 2);
    }

    #[test]
    fn identical_bodies_of_a_repeated_stem_pair_in_source_order() {
        let base = "useEffect(() => { a() })\nuseEffect(() => { a() })";
        let head = "useEffect(() => { a() })\nuseEffect(() => { a() })\nuseEffect(() => { a() })";
        let matching = run(&sides(&[(base, head)]));
        assert_eq!(
            matching.heads[0],
            [
                paired(0, 0, Stage::Body),
                paired(0, 1, Stage::Body),
                Ancestry::New
            ]
        );
    }

    #[test]
    fn rows_pair_and_list_in_source_order_whatever_the_slice_order() {
        let base =
            "useEffect(() => { a() })\nuseEffect(() => { a() })\nfunction f() { if (x) y() }";
        let head = "useEffect(() => { a() })\nuseEffect(() => { a() })\nuseEffect(() => { b() })";
        let mut files = sides(&[(base, head)]);
        files[0].0.reverse();
        let matching = run(&files);
        assert_eq!(
            matching.heads[0],
            [
                paired(0, 2, Stage::Body),
                paired(0, 1, Stage::Body),
                Ancestry::New
            ]
        );
        assert_eq!(matching.removed, [RowId { file: 0, row: 0 }]);

        let base = "useEffect(() => { a() })\nuseEffect(() => { b() })";
        let mut files = sides(&[(base, "useEffect(() => { c() })")]);
        files[0].0.reverse();
        let matching = run(&files);
        let candidates = vec![RowId { file: 0, row: 1 }, RowId { file: 0, row: 0 }];
        assert_eq!(matching.heads[0], [Ancestry::Ambiguous { candidates }]);
        assert_eq!(
            matching.removed,
            [RowId { file: 0, row: 1 }, RowId { file: 0, row: 0 }]
        );
    }

    #[test]
    fn a_unique_stem_with_an_edited_body_pairs_by_name() {
        let matching = run(&sides(&[(
            "function f() { return 1 }",
            "function f() { return 2 }",
        )]));
        assert_eq!(matching.heads[0], [paired(0, 0, Stage::Name)]);
        assert_eq!(
            matching.ancestry(RowId { file: 0, row: 0 }),
            &paired(0, 0, Stage::Name)
        );
    }
}
