//! cargo-crap's own [`Tool`] value, the identity its reports carry.
//!
//! It lives outside [`report`](crate::report) on purpose: the report code
//! is shared with other front ends, and `env!("CARGO_PKG_VERSION")` must
//! expand in cargo-crap's package to stamp cargo-crap's version.

use crate::policy::{EnvelopeKind, PairPolicy, Policy, Population, TieOrder};
use crate::report::{Format, Schemas, Tool};

pub const CARGO_CRAP_POLICY: Policy = Policy {
    envelope: EnvelopeKind::CargoCrapV1,
    pairing: PairPolicy::FileFunctionLine,
    fail_above_counts: Population::Shown,
    summary_formats: &[
        Format::Human,
        Format::Markdown,
        Format::PrComment,
        Format::Sarif,
        Format::Shields,
    ],
    tie_order: TieOrder::Stable,
};

/// The published HTTPS URL of a schema file in this repository. `concat!`
/// takes only literals, so the base URL is repeated by expansion.
macro_rules! schema_url {
    ($file:literal) => {
        concat!(
            "https://raw.githubusercontent.com/minikin/cargo-crap/main/schemas/",
            $file
        )
    };
}

pub const CARGO_CRAP: Tool = Tool {
    policy: CARGO_CRAP_POLICY,
    name: "cargo-crap",
    version: env!("CARGO_PKG_VERSION"),
    info_uri: "https://github.com/minikin/cargo-crap",
    schemas: Schemas {
        report: schema_url!("report-v1.json"),
        // v2 added the `moved` status and the optional `previous_file`
        // field, strictly additive for v1 readers.
        delta: schema_url!("delta-v2.json"),
    },
    comment_marker: "<!-- cargo-crap-report -->",
    baseline_hint: "cargo crap --format json",
};

#[cfg(test)]
mod tests {
    use super::CARGO_CRAP;
    use crate::report::Tool;
    use crate::report::test_support::CARGO_CRAP_FIXTURE;
    use std::path::Path;

    #[test]
    fn cargo_crap_counts_shown_rows_and_summarizes_five_formats() {
        use crate::policy::{EnvelopeKind, PairPolicy, Population, TieOrder};
        use crate::report::Format;
        let policy = CARGO_CRAP.policy;
        assert_eq!(policy.envelope, EnvelopeKind::CargoCrapV1);
        assert_eq!(policy.pairing, PairPolicy::FileFunctionLine);
        assert_eq!(policy.tie_order, TieOrder::Stable);
        assert_eq!(policy.fail_above_counts, Population::Shown);
        assert_eq!(
            policy.summary_formats,
            &[
                Format::Human,
                Format::Markdown,
                Format::PrComment,
                Format::Sarif,
                Format::Shields
            ]
        );
    }

    fn path_words(line: &str) -> impl Iterator<Item = &str> {
        line.split(|c: char| !(c.is_alphanumeric() || c == '_' || c == ':'))
    }

    fn violations(
        source: &str,
        module: &[&str],
    ) -> Vec<String> {
        let path = Path::new("src").join(module.join("/")).with_extension("rs");
        violations_in(&path, source, module)
    }

    fn violations_in(
        path: &Path,
        source: &str,
        module: &[&str],
    ) -> Vec<String> {
        let filename = crate::paths::path_key(path);
        let mut found: Vec<String> = doctest_tool_lines(source)
            .into_iter()
            .map(|line| format!("{filename}:{line}: crate::tool"))
            .collect();
        for (idx, line) in source.lines().enumerate() {
            for (marker, root) in [
                ("syn::", "syn"),
                ("quote::", "quote"),
                ("proc_macro2::", "proc_macro2"),
            ] {
                if line.contains(marker) {
                    found.push(format!("{filename}:{}: {root}", idx + 1));
                }
            }
            if line.contains("CARGO_PKG_") {
                found.push(format!("{filename}:{}: CARGO_PKG_", idx + 1));
            }
            if path_words(line).any(|word| {
                let mut parts = word.trim_start_matches("::").split("::");
                parts.next() == Some("cargo_crap") && parts.next() == Some("tool")
            }) {
                found.push(format!("{filename}:{}: crate::tool", idx + 1));
            }
        }
        let file = syn::parse_file(source).expect("parses");
        let mut paths = CorePaths {
            file: &filename,
            module: module.iter().map(|m| (*m).to_string()).collect(),
            found: Vec::new(),
        };
        syn::visit::Visit::visit_file(&mut paths, &file);
        found.extend(paths.found);
        found.sort();
        found.dedup();
        found.sort();
        found.dedup();
        found
    }

    /// An identifier as the compiler resolves it, so `r#tool` is `tool`.
    fn name(ident: &proc_macro2::Ident) -> String {
        use syn::ext::IdentExt;
        ident.unraw().to_string()
    }

    /// Every path a `use` tree imports, each as its segments. A glob imports
    /// the path it hangs off, and a rename imports the path it renames.
    fn use_paths(
        tree: &syn::UseTree,
        prefix: &mut Vec<proc_macro2::Ident>,
        out: &mut Vec<Vec<proc_macro2::Ident>>,
    ) {
        match tree {
            syn::UseTree::Path(p) => {
                prefix.push(p.ident.clone());
                use_paths(&p.tree, prefix, out);
                prefix.pop();
            },
            syn::UseTree::Name(n) => {
                out.push([prefix.as_slice(), std::slice::from_ref(&n.ident)].concat());
            },
            syn::UseTree::Rename(r) => {
                out.push([prefix.as_slice(), std::slice::from_ref(&r.ident)].concat());
            },
            syn::UseTree::Glob(_) => out.push(prefix.clone()),
            syn::UseTree::Group(g) => {
                for item in &g.items {
                    use_paths(item, prefix, out);
                }
            },
        }
    }

    fn without_comments(text: &str) -> String {
        let mut out = String::new();
        let mut rest = text;
        while let Some(c) = rest.chars().next() {
            if let Some(tail) = rest.strip_prefix("/*") {
                let end = tail.find("*/").map_or(tail.len(), |at| at + 2);
                out.extend(tail[..end].chars().filter(|&c| c == '\n'));
                rest = &tail[end..];
                out.push(' ');
            } else if let Some(tail) = rest.strip_prefix("//") {
                rest = tail.find('\n').map_or("", |end| &tail[end..]);
            } else {
                out.push(c);
                rest = &rest[c.len_utf8()..];
            }
        }
        out
    }

    fn doctest_tool_lines(source: &str) -> Vec<usize> {
        let docs = source
            .lines()
            .map(|line| {
                let line = line.trim_start();
                let line = line
                    .strip_prefix("//!")
                    .or_else(|| line.strip_prefix("///"))
                    .unwrap_or(line)
                    .trim_start();
                line.strip_prefix("# ").unwrap_or(line)
            })
            .collect::<Vec<_>>()
            .join("\n");
        let docs = without_comments(&docs);
        let mut lines = Vec::new();
        for (at, _) in docs.match_indices("use") {
            let statement = &docs[at..];
            if !statement["use".len()..]
                .trim_start()
                .trim_start_matches("::")
                .starts_with("cargo_crap")
            {
                continue;
            }
            let Some(end) = statement.find(';') else {
                continue;
            };
            let Ok(item) = syn::parse_str::<syn::ItemUse>(&statement[..=end]) else {
                continue;
            };
            let mut paths = Vec::new();
            use_paths(&item.tree, &mut Vec::new(), &mut paths);
            let offset = docs[..at].bytes().filter(|&c| c == b'\n').count();
            for path in paths {
                if let [root, module, ..] = path.as_slice()
                    && name(root) == "cargo_crap"
                    && name(module) == "tool"
                {
                    lines.push(offset + module.span().start().line);
                }
            }
        }
        lines
    }

    /// Every `a::b::c` path in a macro's tokens, which syn leaves unparsed.
    fn token_paths(
        tokens: proc_macro2::TokenStream,
        out: &mut Vec<Vec<proc_macro2::Ident>>,
    ) {
        use proc_macro2::TokenTree;
        let trees: Vec<TokenTree> = tokens.into_iter().collect();
        let colon =
            |t: Option<&TokenTree>| matches!(t, Some(TokenTree::Punct(p)) if p.as_char() == ':');
        let mut i = 0;
        while i < trees.len() {
            match &trees[i] {
                TokenTree::Group(g) => token_paths(g.stream(), out),
                TokenTree::Ident(first) => {
                    let mut path = vec![first.clone()];
                    while colon(trees.get(i + 1)) && colon(trees.get(i + 2)) {
                        let Some(TokenTree::Ident(next)) = trees.get(i + 3) else {
                            break;
                        };
                        path.push(next.clone());
                        i += 3;
                    }
                    out.push(path);
                },
                _ => {},
            }
            i += 1;
        }
    }

    const FRONT_END_MODULES: &[&[&str]] = &[
        &["tool"],
        &["merge"],
        &["complexity"],
        &["cache"],
        &["config"],
        &["signals", "compute"],
        &["duplicates", "extract"],
        &["duplicates", "normalize"],
        &["duplicates", "fingerprint"],
        &["duplicates", "scan"],
        &["duplicates", "triage"],
    ];

    const EXTERNAL_ROOTS: &[&str] = &[
        "syn",
        "quote",
        "proc_macro2",
        "ignore",
        "indicatif",
        "toml",
        "ureq",
        "rayon",
        "clap",
    ];

    struct CorePaths<'a> {
        file: &'a str,
        module: Vec<String>,
        found: Vec<String>,
    }

    impl CorePaths<'_> {
        fn report(
            &mut self,
            line: usize,
            path: &str,
        ) {
            self.found.push(format!("{}:{line}: {path}", self.file));
        }

        fn check_external(
            &mut self,
            segments: &[proc_macro2::Ident],
        ) {
            if let Some(root) = segments.first() {
                let spelling = name(root);
                if EXTERNAL_ROOTS.contains(&spelling.as_str()) {
                    self.report(root.span().start().line, &spelling);
                }
            }
        }

        fn check_doc(
            &mut self,
            doc: &syn::LitStr,
        ) {
            for (offset, line) in doc.value().lines().enumerate() {
                for word in path_words(line) {
                    let mut parts = word.trim_start_matches("::").split("::");
                    if !matches!(parts.next(), Some("crate" | "cargo_crap")) {
                        continue;
                    }
                    let parts: Vec<_> = parts.collect();
                    if let Some(prefix) = FRONT_END_MODULES.iter().find(|p| parts.starts_with(p)) {
                        self.report(
                            doc.span().start().line + offset,
                            &format!("crate::{}", prefix.join("::")),
                        );
                    }
                }
            }
        }

        fn check(
            &mut self,
            segments: &[proc_macro2::Ident],
        ) {
            let Some(first) = segments.first() else {
                return;
            };
            let mut rest = segments;
            let mut resolved: Vec<_> = self
                .module
                .iter()
                .map(|s| (s.clone(), first.span().start().line))
                .collect();
            match name(first).as_str() {
                "crate" => {
                    resolved.clear();
                    rest = &rest[1..];
                },
                "self" | "super" => {
                    while let Some(head) = rest.first() {
                        match name(head).as_str() {
                            "self" => {},
                            "super" => {
                                resolved.pop();
                            },
                            _ => break,
                        }
                        rest = &rest[1..];
                    }
                },
                _ => return,
            }
            resolved.extend(
                rest.iter()
                    .map(|ident| (name(ident), ident.span().start().line)),
            );
            let names: Vec<_> = resolved.iter().map(|(s, _)| s.as_str()).collect();
            if let Some(prefix) = FRONT_END_MODULES
                .iter()
                .find(|prefix| names.starts_with(prefix))
            {
                self.report(
                    resolved[prefix.len() - 1].1,
                    &format!("crate::{}", prefix.join("::")),
                );
            }
        }
    }

    impl<'ast> syn::visit::Visit<'ast> for CorePaths<'_> {
        fn visit_attribute(
            &mut self,
            attr: &'ast syn::Attribute,
        ) {
            if attr.path().is_ident("doc")
                && let syn::Meta::NameValue(meta) = &attr.meta
                && let syn::Expr::Lit(expr) = &meta.value
                && let syn::Lit::Str(doc) = &expr.lit
            {
                self.check_doc(doc);
            }
            syn::visit::visit_attribute(self, attr);
        }

        fn visit_item_mod(
            &mut self,
            item: &'ast syn::ItemMod,
        ) {
            self.module.push(name(&item.ident));
            syn::visit::visit_item_mod(self, item);
            self.module.pop();
        }

        fn visit_item_extern_crate(
            &mut self,
            item: &'ast syn::ItemExternCrate,
        ) {
            self.check_external(std::slice::from_ref(&item.ident));
            syn::visit::visit_item_extern_crate(self, item);
        }

        fn visit_item_use(
            &mut self,
            item: &'ast syn::ItemUse,
        ) {
            for attr in &item.attrs {
                self.visit_attribute(attr);
            }
            let mut paths = Vec::new();
            use_paths(&item.tree, &mut Vec::new(), &mut paths);
            for path in paths {
                self.check_external(&path);
                self.check(&path);
            }
        }

        fn visit_macro(
            &mut self,
            mac: &'ast syn::Macro,
        ) {
            let mut paths = Vec::new();
            token_paths(mac.tokens.clone(), &mut paths);
            for path in paths {
                if path.len() > 1 {
                    self.check_external(&path);
                }
                self.check(&path);
            }
            syn::visit::visit_macro(self, mac);
        }

        fn visit_path(
            &mut self,
            path: &'ast syn::Path,
        ) {
            let segments: Vec<_> = path.segments.iter().map(|s| s.ident.clone()).collect();
            if path.leading_colon.is_some() || segments.len() > 1 {
                self.check_external(&segments);
            }
            self.check(&segments);
            syn::visit::visit_path(self, path);
        }
    }

    #[test]
    fn a_front_end_module_import_fails_the_source_guard() {
        assert_eq!(
            violations("\n\nuse crate::merge::CrapEntry;", &["delta"]),
            ["src/delta.rs:3: crate::merge"]
        );
    }

    #[test]
    fn a_grouped_front_end_import_fails_the_source_guard() {
        assert_eq!(
            violations(
                "use crate::{\n complexity::FunctionComplexity};",
                &["delta"]
            ),
            ["src/delta.rs:2: crate::complexity"]
        );
    }

    #[test]
    fn the_root_tool_module_fails_the_source_guard_in_a_new_core_bound_file() {
        assert_eq!(
            violations("use crate::tool::CARGO_CRAP;", &["entry"]),
            ["src/entry.rs:1: crate::tool"]
        );
    }

    #[test]
    fn a_sibling_front_end_module_import_fails_the_source_guard() {
        assert_eq!(
            violations("use super::extract::Location;", &["duplicates", "compare"]),
            ["src/duplicates/compare.rs:1: crate::duplicates::extract"]
        );
    }

    #[test]
    fn grouped_doctest_imports_name_each_forbidden_segment_line() {
        let source = "//! # use cargo_crap::{\n//! report::Format, /* ;\n//! */ tool::CARGO_CRAP};\n//! use cargo_crap::{tool::CARGO_CRAP};";
        assert_eq!(
            violations(source, &["report"]),
            [
                "src/report.rs:3: crate::tool",
                "src/report.rs:4: crate::tool"
            ]
        );
    }

    #[test]
    fn a_parser_crate_in_a_test_module_fails_the_source_guard() {
        assert!(
            violations(
                "#[cfg(test)] mod tests { fn example(){ syn :: parse_str(); } }",
                &["entry"]
            )
            .iter()
            .any(|m| m == "src/entry.rs:1: syn")
        );
    }

    #[test]
    fn a_parser_crate_inside_a_macro_fails_the_source_guard() {
        for marker in ["syn", "quote", "proc_macro2"] {
            let source = format!(
                "fn f() {{ assert!({marker}::parse_str::<{marker}::File>(\"\").is_ok()); }}"
            );
            assert!(
                violations(&source, &["entry"])
                    .iter()
                    .any(|m| m.ends_with(&format!(": {marker}")))
            );
        }
    }

    #[test]
    fn external_roots_fail_in_paths_and_grouped_imports() {
        for external in [
            "syn",
            "quote",
            "proc_macro2",
            "ignore",
            "indicatif",
            "toml",
            "ureq",
            "rayon",
            "clap",
        ] {
            for source in [
                format!("use ::{external}::{{a,b}};"),
                format!("fn f() {{ {external} :: example(); }}"),
                format!("fn f() {{ assert!({external} :: example()); }}"),
            ] {
                assert!(
                    violations(&source, &["entry"])
                        .iter()
                        .any(|m| m == &format!("src/entry.rs:1: {external}")),
                    "{source}"
                );
            }
        }
    }

    #[test]
    fn a_doc_link_to_a_front_end_module_fails_the_source_guard() {
        for (doc, module) in [
            ("/// See [`x`](crate::complexity).", "crate::complexity"),
            (
                "/// See [`cargo_crap::duplicates::extract::Location`].",
                "crate::duplicates::extract",
            ),
        ] {
            assert!(
                violations(&format!("\n{doc}\nfn f() {{}}"), &["entry"])
                    .iter()
                    .any(|m| m == &format!("src/entry.rs:2: {module}"))
            );
        }
    }

    #[test]
    fn a_crate_name_in_prose_is_not_a_violation() {
        let src = "/// Files .cargo-crap.toml and the ignore list. See crate::configuration, cargo_crap::toolbox or super::config.\nfn f() { let _ = outer::ignore::word(); }";
        assert_eq!(violations(src, &["signals", "model"]), Vec::<String>::new());
    }

    #[test]
    fn a_doc_link_on_a_reexport_is_checked() {
        assert_eq!(
            violations(
                "/// See crate::merge::CrapEntry.\npub use crate::entry::CrapEntry;",
                &["report"]
            ),
            ["src/report.rs:1: crate::merge"]
        );
    }

    #[test]
    fn a_local_identifier_and_the_builtin_ignore_attribute_are_allowed() {
        let source = "#[test] #[ignore] fn pending() { let ignore = true; assert!(ignore); let _ = ignore; }";
        assert_eq!(violations(source, &["entry"]), Vec::<String>::new());
        assert_eq!(
            violations("use ignore;", &["entry"]),
            ["src/entry.rs:1: ignore"]
        );
        assert_eq!(
            violations("fn f() { let _ = ::clap; }", &["entry"]),
            ["src/entry.rs:1: clap"]
        );
    }

    #[test]
    fn an_explicit_external_crate_alias_is_a_dependency() {
        assert_eq!(
            violations("extern crate syn as parser;", &["entry"]),
            ["src/entry.rs:1: syn"]
        );
    }

    #[test]
    fn a_missing_core_bound_file_fails_the_source_guard() {
        let dir = tempfile::tempdir().unwrap();
        for file in CORE_BOUND_FILES
            .iter()
            .copied()
            .filter(|file| *file != "src/entry.rs")
        {
            let path = dir.path().join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, "").unwrap();
        }
        std::fs::create_dir_all(dir.path().join("src/report")).unwrap();
        let err = check_core(dir.path()).expect_err("a missing listed file must fail");
        assert!(format!("{err:#}").contains("src/entry.rs"), "{err:#}");
    }

    #[test]
    fn every_listed_file_and_nested_report_is_checked_for_branding_and_dependencies() {
        let dir = tempfile::tempdir().unwrap();
        let required = [
            (
                "src",
                "score entry paths delta report filter gate run policy",
            ),
            ("src/coverage", "mod lcov"),
            ("src/signals", "model config"),
            ("src/duplicates", "model compare group novelty verdict"),
            ("src/report/subdirectory", "more"),
        ]
        .into_iter()
        .flat_map(|(dir, stems)| {
            stems
                .split_whitespace()
                .map(move |stem| format!("{dir}/{stem}.rs"))
        })
        .collect::<Vec<_>>();
        let mut expected = Vec::new();
        for file in &required {
            let path = dir.path().join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(
                path,
                "const V: &str = env!(\"CARGO_PKG_VERSION\");\nuse crate::merge::CrapEntry;",
            )
            .unwrap();
            expected.push(format!("{file}:1: CARGO_PKG_"));
            expected.push(format!("{file}:2: crate::merge"));
        }
        expected.sort();
        let mut found = check_core(dir.path()).unwrap();
        found.sort();
        assert_eq!(found, expected);
    }

    #[test]
    fn cargo_crap_fixture_matches_cargo_crap_but_its_version() {
        let fixture = Tool {
            version: CARGO_CRAP.version,
            ..CARGO_CRAP_FIXTURE
        };
        assert_eq!(fixture, CARGO_CRAP);
    }

    #[test]
    fn a_grouped_import_of_the_tool_module_is_a_violation() {
        let found = violations("use crate::{tool::CARGO_CRAP};", &["report", "json"]);
        assert_eq!(found.len(), 1, "{found:?}");
    }

    #[test]
    fn a_super_path_to_the_tool_module_is_a_violation() {
        let found = violations("use super::super::tool;", &["report", "json"]);
        assert_eq!(found.len(), 1, "{found:?}");
    }

    #[test]
    fn a_super_path_from_an_inline_module_is_a_violation() {
        let found = violations("mod tests { use super::super::tool; }", &["delta"]);
        assert_eq!(found.len(), 1, "{found:?}");
    }

    #[test]
    fn an_expression_path_to_the_tool_module_is_a_violation() {
        let found = violations(
            "fn f() { let _ = crate::tool::CARGO_CRAP.name; }",
            &["report"],
        );
        assert_eq!(found.len(), 1, "{found:?}");
    }

    #[test]
    fn a_package_variable_is_a_violation() {
        let found = violations(r#"const N: &str = env!("CARGO_PKG_NAME");"#, &["report"]);
        assert_eq!(found.len(), 1, "{found:?}");
    }

    #[test]
    fn the_doctest_path_to_the_tool_module_is_a_violation() {
        let found = violations(
            "//! RenderOptions::new(&cargo_crap::tool::CARGO_CRAP)",
            &["report"],
        );
        assert_eq!(found.len(), 1, "{found:?}");
    }

    #[test]
    fn a_path_inside_a_macro_is_a_violation() {
        let found = violations(
            "fn f() { assert_eq!(1, crate::tool::CARGO_CRAP.name.len()); }",
            &["report", "json"],
        );
        assert_eq!(found.len(), 1, "{found:?}");
    }

    #[test]
    fn a_grouped_doctest_import_of_the_tool_module_is_a_violation() {
        let found = violations(
            "//! use cargo_crap::{report::RenderOptions, tool::CARGO_CRAP};",
            &["report"],
        );
        assert_eq!(found.len(), 1, "{found:?}");
    }

    #[test]
    fn a_self_path_from_the_crate_root_is_a_violation() {
        let found = violations("use self::tool;", &[]);
        assert_eq!(found.len(), 1, "{found:?}");
    }

    #[test]
    fn a_raw_identifier_for_the_tool_module_is_a_violation() {
        let found = violations("use crate::{r#tool::CARGO_CRAP};", &["report", "json"]);
        assert_eq!(found.len(), 1, "{found:?}");
    }

    #[test]
    fn a_nested_group_doctest_import_of_the_tool_module_is_a_violation() {
        let found = violations(
            "//! use cargo_crap::{report::{Tool, Schemas}, tool::CARGO_CRAP};",
            &["report"],
        );
        assert_eq!(found.len(), 1, "{found:?}");
    }

    #[test]
    fn a_doctest_import_with_extra_spaces_is_a_violation() {
        let found = violations("//! use  cargo_crap::{tool::CARGO_CRAP};", &["report"]);
        assert_eq!(found.len(), 1, "{found:?}");
    }

    #[test]
    fn a_doctest_import_with_a_comment_is_a_violation() {
        let found = violations(
            "//! use cargo_crap::{/* ; */ tool::CARGO_CRAP};",
            &["report"],
        );
        assert_eq!(found.len(), 1, "{found:?}");
    }

    #[test]
    fn a_doctest_alias_named_tool_is_not_a_violation() {
        assert_eq!(
            violations("//! use cargo_crap::{report::Tool as tool};", &["report"]),
            Vec::<String>::new()
        );
    }

    #[test]
    fn the_report_tool_type_is_not_a_violation() {
        assert_eq!(
            violations("use crate::report::Tool;", &["delta"]),
            Vec::<String>::new()
        );
        assert_eq!(
            violations("use super::tool::Tool;", &["report", "json"]),
            Vec::<String>::new()
        );
        assert_eq!(
            violations("fn f() { crate::tools::x(); }", &["report"]),
            Vec::<String>::new()
        );
        assert_eq!(
            violations("use self::tool::Tool;", &["report"]),
            Vec::<String>::new()
        );
        assert_eq!(
            violations("//! use cargo_crap::{report::Tool, tools};", &["report"]),
            Vec::<String>::new()
        );
    }

    const CORE_BOUND_FILES: &[&str] = &[
        "src/score.rs",
        "src/entry.rs",
        "src/paths.rs",
        "src/coverage/mod.rs",
        "src/coverage/lcov.rs",
        "src/signals/model.rs",
        "src/signals/config.rs",
        "src/delta.rs",
        "src/report.rs",
        "src/duplicates/model.rs",
        "src/duplicates/compare.rs",
        "src/duplicates/group.rs",
        "src/duplicates/novelty.rs",
        "src/duplicates/verdict.rs",
        "src/filter.rs",
        "src/gate.rs",
        "src/run.rs",
        "src/policy.rs",
    ];

    fn rust_files(
        root: &Path,
        dir: &Path,
        out: &mut Vec<std::path::PathBuf>,
    ) -> anyhow::Result<()> {
        use anyhow::Context as _;
        let entries = std::fs::read_dir(root.join(dir))
            .with_context(|| format!("reading {}", dir.display()))?;
        for entry in entries {
            let entry = entry?;
            let path = dir.join(entry.file_name());
            if entry.file_type()?.is_dir() {
                rust_files(root, &path, out)?;
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                out.push(path);
            }
        }
        Ok(())
    }

    fn check_core(root: &Path) -> anyhow::Result<Vec<String>> {
        use anyhow::Context as _;
        let mut files: Vec<_> = CORE_BOUND_FILES
            .iter()
            .map(std::path::PathBuf::from)
            .collect();
        rust_files(root, Path::new("src/report"), &mut files)?;
        files.sort();
        let mut found = Vec::new();
        for path in files {
            let source = std::fs::read_to_string(root.join(&path))
                .with_context(|| format!("reading {}", crate::paths::path_key(&path)))?;
            let module_path = path.strip_prefix("src")?.with_extension("");
            let module: Vec<_> = module_path
                .iter()
                .filter_map(|part| part.to_str())
                .filter(|part| *part != "mod")
                .collect();
            found.extend(violations_in(&path, &source, &module));
        }
        Ok(found)
    }

    /// The merge fixtures canonicalize temporary files, so test-only code must be excluded from this check.
    fn merge_delegates(source: &str) -> bool {
        use crate::complexity::{has_attr, is_cfg_test};
        use syn::visit::{self, Visit};
        #[derive(Clone, Copy, Default)]
        enum Scope {
            #[default]
            Root,
            Merge,
            Other,
        }
        #[derive(Default)]
        struct MergeCalls {
            scope: Scope,
            assigned: bool,
            legacy: bool,
        }
        fn legacy_name(name: &str) -> bool {
            matches!(name, "PathIndex" | "path_has_suffix" | "canonicalize")
        }
        impl<'ast> Visit<'ast> for MergeCalls {
            fn visit_item(
                &mut self,
                item: &'ast syn::Item,
            ) {
                let attrs: &[syn::Attribute] = match item {
                    syn::Item::Mod(m) => &m.attrs,
                    syn::Item::Fn(f) => &f.attrs,
                    _ => &[],
                };
                if has_attr(attrs, "test") || is_cfg_test(attrs) {
                    return;
                }
                let parent = self.scope;
                self.scope = if matches!(parent, Scope::Root)
                    && matches!(item, syn::Item::Fn(f) if f.sig.ident == "merge")
                {
                    Scope::Merge
                } else {
                    Scope::Other
                };
                visit::visit_item(self, item);
                self.scope = parent;
            }
            fn visit_expr_block(
                &mut self,
                block: &'ast syn::ExprBlock,
            ) {
                if !is_cfg_test(&block.attrs) {
                    visit::visit_expr_block(self, block);
                }
            }
            fn visit_expr_call(
                &mut self,
                call: &'ast syn::ExprCall,
            ) {
                if is_cfg_test(&call.attrs) {
                    return;
                }
                let function = expr_path_segments(&call.func);
                let function = function.iter().map(String::as_str).collect::<Vec<_>>();
                if matches!(self.scope, Scope::Merge)
                    && matches!(
                        function.as_slice(),
                        ["paths", "assign"] | ["crate", "paths", "assign"]
                    )
                    && let Some(arg) = call.args.last()
                {
                    let policy = expr_path_segments(arg);
                    let policy = policy.iter().map(String::as_str).collect::<Vec<_>>();
                    self.assigned |= matches!(
                        policy.as_slice(),
                        ["TiePolicy", "PerSource"]
                            | ["paths", "TiePolicy", "PerSource"]
                            | ["crate", "paths", "TiePolicy", "PerSource"]
                    );
                }
                visit::visit_expr_call(self, call);
            }
            fn visit_macro(
                &mut self,
                mac: &'ast syn::Macro,
            ) {
                let mut paths = Vec::new();
                token_paths(mac.tokens.clone(), &mut paths);
                self.legacy |= paths
                    .iter()
                    .flatten()
                    .any(|segment| legacy_name(&name(segment)));
                visit::visit_macro(self, mac);
            }
            fn visit_ident(
                &mut self,
                ident: &'ast syn::Ident,
            ) {
                self.legacy |= legacy_name(&name(ident));
            }
        }
        let mut calls = MergeCalls::default();
        calls.visit_file(&syn::parse_file(source).expect("valid merge source"));
        calls.assigned && !calls.legacy
    }

    fn expr_path_segments(expr: &syn::Expr) -> Vec<String> {
        let syn::Expr::Path(path) = expr else {
            return Vec::new();
        };
        path.path.segments.iter().map(|s| name(&s.ident)).collect()
    }

    #[test]
    fn merge_matches_paths_only_through_paths_assign() {
        assert!(merge_delegates(include_str!("merge.rs")));
    }

    #[test]
    fn merge_delegation_guard_checks_the_production_call_and_policy() {
        let valid = "fn merge() { paths::assign(&s, &k, TiePolicy::PerSource); }";
        assert!(merge_delegates(&format!(
            "{valid} #[cfg(test)] mod tests {{ fn probe() {{ path.canonicalize(); }} }}"
        )));
        for wrong in [
            "fn merge() {} mod helpers { fn merge() { paths::assign(&s,&k,TiePolicy::PerSource); } }",
            "fn merge() { fn merge() { paths::assign(&s,&k,TiePolicy::PerSource); } }",
            "fn merge() { #[cfg(test)] { paths::assign(&s,&k,TiePolicy::PerSource); } }",
            "fn merge() {} #[cfg(test)] mod tests { fn merge() { paths::assign(&s,&k,TiePolicy::PerSource); } }",
            "fn merge() { paths::assign(&s,&k,TiePolicy::Unmatched); }",
            "fn merge() { other::assign(&s,&k,TiePolicy::PerSource); }",
            "fn merge() { wrong::paths::assign(&s,&k,TiePolicy::PerSource); }",
            "fn merge() { #[cfg(test)] paths::assign(&s,&k,TiePolicy::PerSource); }",
            "#[cfg(test)] fn merge() { paths::assign(&s,&k,TiePolicy::PerSource); }",
            "fn elsewhere() { paths::assign(&s,&k,TiePolicy::PerSource); }",
        ] {
            assert!(!merge_delegates(wrong), "{wrong}");
        }
        for legacy in [
            "fn f(){std::fs::canonicalize(p);}",
            "fn f(){files.map(Path::canonicalize);}",
            "fn f(){file.and_then(std::fs::canonicalize);}",
            "fn f(){observe!(file.canonicalize());}",
            "fn f(){file.canonicalize();}",
            "struct PathIndex;",
            "fn path_has_suffix(){}",
        ] {
            assert!(!merge_delegates(&format!("{valid} {legacy}")), "{legacy}");
        }
    }

    #[test]
    fn core_bound_files_read_no_package_version() {
        let found =
            check_core(Path::new(env!("CARGO_MANIFEST_DIR"))).expect("read the core-bound files");
        assert!(found.is_empty(), "{found:#?}");
    }
}
