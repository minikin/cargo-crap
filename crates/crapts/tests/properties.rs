mod support;

use std::collections::HashSet;
use std::fmt::Write as _;
use std::path::Path;

use crapts::{Mode, Side, analyze};
use globset::GlobBuilder;
use proptest::prelude::*;

fn names(
    path: &str,
    source: &str,
) -> Vec<String> {
    analyze(Path::new(path), source.as_bytes(), Mode::Strict(Side::Head))
        .unwrap()
        .rows
        .into_iter()
        .map(|row| row.name)
        .collect()
}

struct Nest<'a> {
    role: &'a str,
    depth: usize,
    siblings: usize,
    expression: bool,
}

impl Nest<'_> {
    fn source(
        &self,
        name: &str,
        gap: &str,
        body: &str,
    ) -> String {
        let mut callbacks = vec!["() => 2".to_owned(); self.siblings];
        callbacks[0] = format!("() => {body}");
        let args = callbacks.join(&format!(",{gap}"));
        let role = self.role;
        let mut members = format!("const box{gap}={gap}{{run(){gap}{{{role}({gap}{args});}}}};");
        for level in 0..self.depth {
            members = format!("function level{level}(){{{gap}{members}{gap}}}");
        }
        if self.expression {
            format!("const {name}{gap}={gap}function inner(){{{gap}{members}{gap}}};")
        } else {
            format!("function {name}(){{{gap}{members}{gap}}}")
        }
    }
}

fn code_units() -> impl Strategy<Value = Vec<u16>> {
    prop::collection::vec(
        prop_oneof![
            any::<u16>(),
            prop::sample::select(vec![
                0, 0x22, 0x25, 0x2a, 0x2f, 0x3a, 0x3f, 0x5b, 0x5c, 0x5d, 0x7b, 0x7d, 0x200d,
                0xd800, 0xdfff
            ])
        ],
        0..72,
    )
}

fn property(units: &[u16]) -> String {
    let mut text = String::from("'");
    for unit in units {
        write!(text, "\\u{unit:04X}").unwrap();
    }
    text.push_str("':()=>0");
    text
}

proptest! {
    #![proptest_config(support::proptest_config())]

    #[test]
    fn name_stability_under_edits_that_preserve_naming_sites(
        root in "root[a-zA-Z0-9]{0,12}", role in "call[a-zA-Z0-9]{0,12}",
        gap in prop::sample::select(vec!["/*gap*/", "\n", "\r\n", "  "]),
        body in any::<i16>(), expression in any::<bool>(),
        depth in 0usize..5, siblings in 1usize..7,
    ) {
        let shape = Nest { role: &role, depth, siblings, expression };
        let value = body.to_string();
        let original = shape.source(&root, " ", &value);
        let expected = names("src/a.ts", &original);
        prop_assert_eq!(expected.len(), 2 + depth + siblings);
        prop_assert_eq!(&expected[0], &root);
        let moved = names("moved/b.ts", &original);
        prop_assert_eq!(&moved, &expected);
        let trivia = shape.source(&root, gap, &value);
        prop_assert_eq!(names("src/a.ts", &trivia), &expected[..]);
        let block = format!("{{ const changed = {value}; return changed; }}");
        let edited = shape.source(&root, " ", &block);
        prop_assert_eq!(names("src/a.ts", &edited), &expected[..]);
        let with_sibling = format!("function addedSibling() {{}}\n{original}");
        let mut with_sibling = names("src/a.ts", &with_sibling);
        prop_assert_eq!(with_sibling.remove(0), "addedSibling");
        prop_assert_eq!(&with_sibling, &expected);
        let new_root = format!("renamed_{root}");
        let renamed = shape.source(&new_root, gap, &value);
        let actual = names("src/a.ts", &renamed);
        prop_assert_eq!(actual.len(), expected.len());
        for (before, after) in expected.iter().zip(actual) {
            let suffix = before.strip_prefix(&root).unwrap();
            prop_assert_eq!(after, format!("{new_root}{suffix}"));
        }
    }

    #[test]
    fn generated_names_can_be_used_literally(
        keys in prop::collection::vec(code_units(), 1..6),
        choices in prop::collection::vec(0usize..6, 0..12),
    ) {
        let mut keys = keys;
        let generated = keys.len();
        let mut wildcard: Vec<_> = "key-::".encode_utf16().collect();
        wildcard.extend(keys[0].iter().copied().take(10));
        let mut neighbor = wildcard.clone();
        wildcard.push(u16::from(b'*'));
        neighbor.push(u16::from(b'x'));
        keys.extend([wildcard, neighbor, vec![]]);
        keys.extend(["bare", "Bare", "12"].map(|key| key.encode_utf16().collect()));
        let members: Vec<_> = keys.iter().map(|key| property(key)).collect();
        let stems: Vec<_> = members.iter().map(|member| {
            let source = format!("const api = {{{member}}}");
            names("single.ts", &source).pop().unwrap()
        }).collect();
        let order: Vec<_> = std::iter::once(0)
            .chain(choices.into_iter().map(|i| i % generated))
            .chain(generated..keys.len())
            .chain([0])
            .collect();
        let all = order.iter().map(|&i| members[i].as_str()).collect::<Vec<_>>().join(",\n");
        let actual = names("multiple.ts", &format!("const api = {{{all}}}"));
        prop_assert_eq!(actual.len(), order.len());
        prop_assert_eq!(actual.iter().collect::<HashSet<_>>().len(), actual.len());
        for (index, (name, &key)) in actual.iter().zip(&order).enumerate() {
            let stem = &stems[key];
            let seen = order[..index].iter().filter(|&&previous| stems[previous] == *stem);
            let ordinal = 1 + seen.count();
            let expected = if ordinal == 1 { stem.clone() } else { format!("{stem}#{ordinal}") };
            prop_assert_eq!(name, &expected);
            let parts: Vec<_> = name.split("::").collect();
            prop_assert_eq!(parts.len(), 2);
            prop_assert_eq!(parts[0], "api");
            if key == generated {
                prop_assert!(parts[1].starts_with("\"key-%3A%3A"));
            }
            let matcher = GlobBuilder::new(name)
                .case_insensitive(false)
                .backslash_escape(true)
                .build().unwrap().compile_matcher();
            let matches: Vec<_> = actual.iter().enumerate()
                .filter(|(_, n)| matcher.is_match(n))
                .map(|(i, _)| i).collect();
            prop_assert_eq!(matches, [index]);
            prop_assert!(!matcher.is_match(format!("prefix{name}")), "prefix matched {:?}", name);
            prop_assert!(!matcher.is_match(format!("{name}suffix")), "suffix matched {:?}", name);
            prop_assert!(!matcher.is_match(name.to_uppercase()));
        }
    }
}
