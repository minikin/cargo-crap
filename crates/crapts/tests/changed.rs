mod support;

use std::fmt::Write as _;
use std::path::Path;

use crapts::changed::matching::{Ancestry, ChangedFile, Matching, RowId, Stage, classify};
use crapts::{Mode, Row, Side, analyze};
use proptest::prelude::*;

/// `fn name(k)`: CC `k`, one statement per line.
fn func(
    name: &str,
    k: usize,
) -> String {
    let mut text = format!("export function {name}(x: number) {{\n");
    for i in 1..k {
        writeln!(text, "  if (x === {i}) return {i}").unwrap();
    }
    text.push_str("  return 0\n}\n");
    text
}

/// A `useEffect` callback of CC `k` whose statements call `tag`.
fn callback(
    k: usize,
    tag: &str,
) -> String {
    let mut text = String::from("useEffect(() => {\n");
    for i in 1..k {
        writeln!(text, "  if (x === {i}) {tag}({i})").unwrap();
    }
    writeln!(text, "  {tag}(0)\n}})").unwrap();
    text
}

fn analyzed(
    path: &str,
    source: &str,
    side: Side,
) -> Vec<Row> {
    analyze(Path::new(path), source.as_bytes(), Mode::Strict(side))
        .unwrap_or_else(|e| panic!("{e}"))
        .rows
}

struct Pair {
    base: Vec<Row>,
    head: Vec<Row>,
}

impl Pair {
    fn new(
        path: &str,
        base: &str,
        head: &str,
    ) -> Self {
        Self {
            base: analyzed(path, base, Side::Base),
            head: analyzed(path, head, Side::Head),
        }
    }

    fn classify(&self) -> Matching {
        classify(&[ChangedFile {
            base: &self.base,
            head: &self.head,
        }])
    }

    fn ancestry<'m>(
        &self,
        matching: &'m Matching,
        name: &str,
    ) -> &'m Ancestry {
        let row = self
            .head
            .iter()
            .position(|row| row.name == name)
            .unwrap_or_else(|| panic!("no head row {name}"));
        matching.ancestry(RowId { file: 0, row })
    }

    fn partner(
        &self,
        matching: &Matching,
        name: &str,
    ) -> (&str, Stage) {
        match self.ancestry(matching, name) {
            Ancestry::Paired { base, stage } => (self.base[base.row].name.as_str(), *stage),
            other => panic!("{name} is {other:?}"),
        }
    }

    fn base_names(
        &self,
        ids: &[RowId],
    ) -> Vec<&str> {
        ids.iter()
            .map(|id| self.base[id.row].name.as_str())
            .collect()
    }

    fn candidates(
        &self,
        matching: &Matching,
        name: &str,
    ) -> Vec<(&str, u32, u32)> {
        let Ancestry::Ambiguous { candidates } = self.ancestry(matching, name) else {
            panic!("{name} is not ambiguous")
        };
        candidates
            .iter()
            .map(|id| {
                let row = &self.base[id.row];
                (row.name.as_str(), row.span.start.line, row.cyclomatic)
            })
            .collect()
    }
}

fn repository() -> String {
    [func("hot", 6), func("edge", 2), func("cold", 1)].join("\n")
}

#[test]
fn a_staged_removal_with_a_working_tree_replacement_keeps_its_ancestry() {
    let original = repository();
    let edited =
        original
            .replacen("return 1\n", "return 11\n", 1)
            .replacen("return 2\n", "return 12\n", 1);
    assert_ne!(edited, original);
    for head in [original.clone(), edited] {
        // Given the merge-base side and the untracked side of src/app.ts as one file pair
        let pair = Pair::new("src/app.ts", &original, &head);
        // When the matcher classifies it
        let matching = pair.classify();
        // Then hot pairs by name instead of being new beside a removed base row
        assert_eq!(pair.partner(&matching, "hot").0, "hot");
        for name in ["edge", "cold"] {
            assert_eq!(pair.partner(&matching, name).0, name);
        }
        assert_eq!(matching.removed, []);
    }
}

#[test]
fn inserting_a_sibling_callback_shifts_ordinals_without_touching_the_old_ones() {
    // Given two callbacks at B and a working tree that inserts a new first one
    let base = "useEffect(() => { if (a) load() })\nuseEffect(() => { if (b) save() })\n";
    let head = format!("useEffect(() => {{ track() }})\n{base}");
    let pair = Pair::new("src/hooks.ts", base, &head);
    // When the matcher classifies the file pair
    let matching = pair.classify();
    // Then the inserted callback is new and stage 1 pairs the shifted originals
    assert_eq!(pair.ancestry(&matching, "useEffect()"), &Ancestry::New);
    assert_eq!(
        pair.partner(&matching, "useEffect()#2"),
        ("useEffect()", Stage::Body)
    );
    assert_eq!(
        pair.partner(&matching, "useEffect()#3"),
        ("useEffect()#2", Stage::Body)
    );
    // And removed is empty
    assert_eq!(matching.removed, []);
}

#[test]
fn deleting_the_first_of_two_callbacks_and_appending_another_leaves_the_new_one_ambiguous() {
    // Given a first callback of CC 5 and a second of CC 30
    let base = format!("{}{}", callback(5, "first"), callback(30, "second"));
    // And a working tree that deletes the first and appends a different one of CC 2
    let head = format!("{}{}", callback(30, "second"), callback(2, "third"));
    let pair = Pair::new("src/hooks.ts", &base, &head);
    // When the matcher classifies the file pair
    let matching = pair.classify();
    // Then the appended callback is ambiguous with the deleted one as its candidate
    assert_eq!(
        pair.candidates(&matching, "useEffect()#2"),
        [("useEffect()", 1, 5)]
    );
    // And the CC 30 callback, now named useEffect(), pairs with its body
    assert_eq!(
        pair.partner(&matching, "useEffect()"),
        ("useEffect()#2", Stage::Body)
    );
    // And removed lists the CC 5 callback
    assert_eq!(pair.base_names(&matching.removed), ["useEffect()"]);
}

#[test]
fn an_edited_function_with_two_same_stem_base_candidates_fails_as_ambiguous() {
    // Given useEffect() of CC 5 and useEffect()#2 of CC 15
    let base = format!("{}{}", callback(5, "first"), callback(15, "second"));
    // And a working tree that replaces both with one callback of CC 12
    let head = callback(12, "third");
    let pair = Pair::new("src/hooks.ts", &base, &head);
    // When the matcher classifies the file pair
    let matching = pair.classify();
    // Then the callback is ambiguous with both base callbacks in line order
    assert_eq!(
        pair.candidates(&matching, "useEffect()"),
        [("useEffect()", 1, 5), ("useEffect()#2", 8, 15)]
    );
    // And removed lists both base callbacks
    assert_eq!(
        pair.base_names(&matching.removed),
        ["useEffect()", "useEffect()#2"]
    );
}

#[test]
fn a_copy_is_new_and_the_original_keeps_its_partner() {
    // Given a working tree that adds edgeCopy, with edge's body, after cold
    let base = repository();
    let head = format!("{base}\n{}", func("edgeCopy", 2));
    let pair = Pair::new("src/app.ts", &base, &head);
    // When the matcher classifies the file pair
    let matching = pair.classify();
    // Then edgeCopy is new and edge keeps its partner
    assert_eq!(pair.ancestry(&matching, "edgeCopy"), &Ancestry::New);
    assert_eq!(pair.partner(&matching, "edge"), ("edge", Stage::Name));
    // And removed is empty
    assert_eq!(matching.removed, []);
}

#[test]
fn a_rename_with_an_edit_is_new_and_its_base_row_is_removed() {
    // Given a working tree that renames edge to edgeCase and adds one branch to it
    let base = repository();
    let head = base.replace(&func("edge", 2), &func("edgeCase", 3));
    let pair = Pair::new("src/app.ts", &base, &head);
    // When the matcher classifies the file pair
    let matching = pair.classify();
    // Then edgeCase is new
    assert_eq!(pair.ancestry(&matching, "edgeCase"), &Ancestry::New);
    // And removed is edge
    assert_eq!(pair.base_names(&matching.removed), ["edge"]);
}

#[test]
fn identical_trivial_functions_renamed_together_are_new() {
    // Given fn one(1) and fn uno(1), whose fingerprints are equal
    let base = format!("{}\n{}", func("one", 1), func("uno", 1));
    // And a working tree that renames them to first and primero
    let head = format!("{}\n{}", func("first", 1), func("primero", 1));
    let pair = Pair::new("src/t.ts", &base, &head);
    assert_eq!(pair.base[0].fingerprint, pair.base[1].fingerprint);
    // When the matcher classifies the file pair
    let matching = pair.classify();
    // Then first and primero are new
    for name in ["first", "primero"] {
        assert_eq!(pair.ancestry(&matching, name), &Ancestry::New);
    }
    // And removed lists one and uno
    assert_eq!(pair.base_names(&matching.removed), ["one", "uno"]);
}

#[test]
fn a_split_keeps_the_surviving_name_and_judges_the_extracted_function_on_its_own() {
    // Given fn big(12) and a working tree that moves five branches into a new fn part(6)
    let base = func("big", 12);
    let mut head = String::from("export function big(x: number) {\n  if (part(x)) return 1\n");
    for i in 7..12 {
        writeln!(head, "  if (x === {i}) return {i}").unwrap();
    }
    head.push_str("  return 0\n}\n\n");
    head.push_str(&func("part", 6));
    let pair = Pair::new("src/s.ts", &base, &head);
    assert_eq!(
        pair.head.iter().map(|r| r.cyclomatic).collect::<Vec<_>>(),
        [7, 6]
    );
    // When the matcher classifies the file pair
    let matching = pair.classify();
    // Then big keeps its name and part is new
    assert_eq!(pair.partner(&matching, "big"), ("big", Stage::Name));
    assert_eq!(pair.ancestry(&matching, "part"), &Ancestry::New);
    assert_eq!(matching.removed, []);
}

/// One generated row: a stem from a small set, so names repeat, and a body
/// from a small set, so fingerprints repeat across names and files.
fn item() -> impl Strategy<Value = String> {
    let name = prop::sample::select(vec!["a", "b", "c"]);
    let body = prop::sample::select(vec!["return 1", "return x", "if (x) return 2; return 3"]);
    prop_oneof![
        (name.clone(), body.clone()).prop_map(|(n, b)| format!("function {n}(x) {{ {b} }}")),
        body.clone()
            .prop_map(|b| format!("useEffect(() => {{ {b} }})")),
        (name, body.clone(), body).prop_map(|(n, outer, inner)| format!(
            "function {n}(x) {{ function inner() {{ {inner} }} {outer} }}"
        )),
    ]
}

fn side() -> impl Strategy<Value = String> {
    prop::collection::vec(item(), 0..5).prop_map(|items| items.join("\n"))
}

fn tree() -> impl Strategy<Value = Vec<(String, String)>> {
    prop::collection::vec((side(), side()), 1..=4)
}

struct Classified {
    files: Vec<(Vec<Row>, Vec<Row>)>,
}

impl Classified {
    fn new(sources: &[(String, String)]) -> Self {
        let files = sources
            .iter()
            .enumerate()
            .map(|(i, (base, head))| {
                let path = format!("src/f{i}.ts");
                (
                    analyzed(&path, base, Side::Base),
                    analyzed(&path, head, Side::Head),
                )
            })
            .collect();
        Self { files }
    }

    fn classify(
        &self,
        order: &[usize],
    ) -> Matching {
        let files: Vec<_> = order
            .iter()
            .map(|&i| ChangedFile {
                base: &self.files[i].0,
                head: &self.files[i].1,
            })
            .collect();
        classify(&files)
    }
}

proptest! {
    #![proptest_config(support::proptest_config())]

    #[test]
    fn matching_pairs_each_row_at_most_once_and_accounts_for_every_row(sources in tree()) {
        let classified = Classified::new(&sources);
        let order: Vec<usize> = (0..sources.len()).collect();
        let matching = classified.classify(&order);
        let mut used = std::collections::HashSet::new();
        let mut pairs = 0;
        let (mut heads, mut bases) = (0, 0);
        for (file, (base, head)) in classified.files.iter().enumerate() {
            heads += head.len();
            bases += base.len();
            prop_assert_eq!(matching.heads[file].len(), head.len());
            for (row, ancestry) in matching.heads[file].iter().enumerate() {
                let head_row = &head[row];
                match ancestry {
                    Ancestry::Paired { base: id, stage } => {
                        pairs += 1;
                        prop_assert!(used.insert(*id), "{:?} paired twice", id);
                        let base_row = &classified.files[id.file].0[id.row];
                        if *stage == Stage::Name {
                            prop_assert_eq!(&base_row.name, &head_row.name);
                        } else {
                            prop_assert_eq!(base_row.fingerprint, head_row.fingerprint);
                        }
                        if *stage != Stage::Moved {
                            prop_assert_eq!(id.file, file);
                        }
                    },
                    Ancestry::Ambiguous { candidates } => {
                        prop_assert!(!candidates.is_empty());
                        for id in candidates {
                            prop_assert_eq!(id.file, file);
                            prop_assert!(matching.removed.contains(id));
                        }
                    },
                    Ancestry::New => {},
                }
            }
        }
        prop_assert!(pairs <= heads);
        prop_assert_eq!(pairs + matching.removed.len(), bases);
        for id in &matching.removed {
            prop_assert!(!used.contains(id));
        }
        prop_assert_eq!(&classified.classify(&order), &matching);
        let reversed: Vec<usize> = order.iter().rev().copied().collect();
        let back = classified.classify(&reversed);
        let flip = |id: RowId| RowId { file: order.len() - 1 - id.file, row: id.row };
        for (file, ancestries) in matching.heads.iter().enumerate() {
            for (row, ancestry) in ancestries.iter().enumerate() {
                let expected = match ancestry {
                    Ancestry::Paired { base, stage } => Ancestry::Paired { base: flip(*base), stage: *stage },
                    Ancestry::New => Ancestry::New,
                    Ancestry::Ambiguous { candidates } => Ancestry::Ambiguous {
                        candidates: candidates.iter().copied().map(flip).collect(),
                    },
                };
                prop_assert_eq!(&back.heads[order.len() - 1 - file][row], &expected);
            }
        }
        let mut removed: Vec<RowId> = back.removed.iter().copied().map(flip).collect();
        removed.sort();
        let mut original = matching.removed.clone();
        original.sort();
        prop_assert_eq!(removed, original);
    }
}
