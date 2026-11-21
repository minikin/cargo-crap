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

    fn violations(
        source: &str,
        module: &[&str],
    ) -> Vec<String> {
        let mut found: Vec<String> = ["CARGO_PKG_", "cargo_crap::tool"]
            .into_iter()
            .filter(|token| source.contains(token))
            .map(|token| format!("names `{token}`"))
            .collect();
        if doctest_imports_the_tool_module(source) {
            found.push("imports `cargo_crap::tool` in a doctest".to_string());
        }
        let file = syn::parse_file(source).expect("parses");
        let mut paths = PathsToTool {
            module: module.iter().map(|m| (*m).to_string()).collect(),
            found: Vec::new(),
        };
        syn::visit::Visit::visit_file(&mut paths, &file);
        found.extend(paths.found);
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
        prefix: &mut Vec<String>,
        out: &mut Vec<Vec<String>>,
    ) {
        match tree {
            syn::UseTree::Path(p) => {
                prefix.push(name(&p.ident));
                use_paths(&p.tree, prefix, out);
                prefix.pop();
            },
            syn::UseTree::Name(n) => out.push([prefix.as_slice(), &[name(&n.ident)]].concat()),
            syn::UseTree::Rename(r) => out.push([prefix.as_slice(), &[name(&r.ident)]].concat()),
            syn::UseTree::Glob(_) => out.push(prefix.clone()),
            syn::UseTree::Group(g) => {
                for item in &g.items {
                    use_paths(item, prefix, out);
                }
            },
        }
    }

    /// `text` without `/* */` and `//` comments, every run of whitespace
    /// one space, so neither can split or hide a `use` statement.
    fn without_comments(text: &str) -> String {
        let mut out = String::new();
        let mut rest = text;
        while let Some(c) = rest.chars().next() {
            if let Some(tail) = rest.strip_prefix("/*") {
                rest = tail.find("*/").map_or("", |end| &tail[end + 2..]);
                out.push(' ');
            } else if let Some(tail) = rest.strip_prefix("//") {
                rest = tail.find('\n').map_or("", |end| &tail[end..]);
            } else {
                out.push(c);
                rest = &rest[c.len_utf8()..];
            }
        }
        out.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    /// Doctests are not parsed as code, so their `use cargo_crap::...;`
    /// lines are taken from the doc comments, cleared of comments and extra
    /// whitespace, and parsed as use trees, nested groups and renames
    /// included.
    fn doctest_imports_the_tool_module(source: &str) -> bool {
        let docs: Vec<&str> = source
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
            .collect();
        let docs = without_comments(&docs.join("\n"));
        docs.match_indices("use cargo_crap").any(|(at, _)| {
            let statement = &docs[at..];
            let Some(end) = statement.find(';') else {
                return false;
            };
            let Ok(item) = syn::parse_str::<syn::ItemUse>(&statement[..=end]) else {
                return false;
            };
            let mut paths = Vec::new();
            use_paths(&item.tree, &mut Vec::new(), &mut paths);
            paths.iter().any(|p| p.get(1).is_some_and(|s| s == "tool"))
        })
    }

    /// Every `a::b::c` path in a macro's tokens, which syn leaves unparsed.
    fn token_paths(
        tokens: proc_macro2::TokenStream,
        out: &mut Vec<Vec<String>>,
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
                    let mut path = vec![name(first)];
                    while colon(trees.get(i + 1)) && colon(trees.get(i + 2)) {
                        let Some(TokenTree::Ident(next)) = trees.get(i + 3) else {
                            break;
                        };
                        path.push(name(next));
                        i += 3;
                    }
                    out.push(path);
                },
                _ => {},
            }
            i += 1;
        }
    }

    /// Collects every `use` tree and path that resolves to the root `tool`
    /// module, resolving `crate`, `self` and `super` from the module the
    /// item sits in.
    ///
    /// The guard is interim and has known limits: it does not follow glob
    /// imports or renamed modules, and it reads a macro's tokens as loose
    /// paths, so a macro that emits `use crate::{tool::CARGO_CRAP};` passes.
    struct PathsToTool {
        module: Vec<String>,
        found: Vec<String>,
    }

    impl PathsToTool {
        fn check(
            &mut self,
            segments: &[String],
        ) {
            let mut resolved = self.module.clone();
            let mut rest = segments;
            match segments.first().map(String::as_str) {
                Some("crate") => {
                    resolved.clear();
                    rest = &segments[1..];
                },
                Some("self" | "super") => {
                    while let Some(head) = rest.first() {
                        match head.as_str() {
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
            resolved.extend(rest.iter().cloned());
            if resolved.first().map(String::as_str) == Some("tool") {
                self.found
                    .push(format!("resolves to `{}`", resolved.join("::")));
            }
        }
    }

    impl<'ast> syn::visit::Visit<'ast> for PathsToTool {
        fn visit_item_mod(
            &mut self,
            item: &'ast syn::ItemMod,
        ) {
            self.module.push(name(&item.ident));
            syn::visit::visit_item_mod(self, item);
            self.module.pop();
        }

        fn visit_item_use(
            &mut self,
            item: &'ast syn::ItemUse,
        ) {
            let mut paths = Vec::new();
            use_paths(&item.tree, &mut Vec::new(), &mut paths);
            for path in paths {
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
                self.check(&path);
            }
            syn::visit::visit_macro(self, mac);
        }

        fn visit_path(
            &mut self,
            path: &'ast syn::Path,
        ) {
            let segments: Vec<String> = path.segments.iter().map(|s| name(&s.ident)).collect();
            self.check(&segments);
            syn::visit::visit_path(self, path);
        }
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

    /// Every `.rs` file under `dir`, with the module path it compiles as:
    /// the checker needs it to resolve `super` from each file.
    fn rust_files(
        dir: &Path,
        module: &[String],
        out: &mut Vec<(std::path::PathBuf, Vec<String>)>,
    ) {
        for entry in std::fs::read_dir(dir).expect("reads the directory") {
            let path = entry.expect("entry").path();
            let stem = path
                .file_stem()
                .expect("stem")
                .to_string_lossy()
                .into_owned();
            let mut inner = module.to_vec();
            if stem != "mod" {
                inner.push(stem);
            }
            if path.is_dir() {
                rust_files(&path, &inner, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push((path, inner));
            }
        }
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
                self.legacy |= paths.iter().flatten().any(|segment| legacy_name(segment));
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
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files = vec![
            (src.join("report.rs"), vec!["report".to_string()]),
            (src.join("delta.rs"), vec!["delta".to_string()]),
            (src.join("gate.rs"), vec!["gate".to_string()]),
            (src.join("filter.rs"), vec!["filter".to_string()]),
            (src.join("run.rs"), vec!["run".to_string()]),
            (src.join("policy.rs"), vec!["policy".to_string()]),
        ];
        rust_files(&src.join("report"), &["report".to_string()], &mut files);
        assert!(files.len() > 10, "{files:?}");
        for (path, module) in files {
            let source = std::fs::read_to_string(&path).expect("reads");
            let module: Vec<&str> = module.iter().map(String::as_str).collect();
            let found = violations(&source, &module);
            assert!(found.is_empty(), "{}: {found:?}", path.display());
        }
    }
}
