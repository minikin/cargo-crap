//! cargo-crap's own [`Tool`] value, the identity its reports carry.
//!
//! It lives outside [`report`](crate::report) on purpose: the report code
//! is shared with other front ends, and `env!("CARGO_PKG_VERSION")` must
//! expand in cargo-crap's package to stamp cargo-crap's version.

use crate::policy::{EnvelopeKind, NoticeSpacing, PairPolicy, Policy, Population, TieOrder};
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
    signal_detail_hint: Some("run without --summary for the reasons"),
    signal_slice_hint: Some(
        "use --format json with the top and min limits removed for the reasons",
    ),
    tie_order: TieOrder::Stable,
    notice_spacing: NoticeSpacing::Separated,
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

    /// An identifier as the compiler resolves it, so `r#tool` is `tool`.
    fn name(ident: &proc_macro2::Ident) -> String {
        use syn::ext::IdentExt;
        ident.unraw().to_string()
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
}
