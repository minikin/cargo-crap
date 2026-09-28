# The path-matching problem

This is where silent failures happen. Complexity analysis produces
absolute paths (whatever was passed to the walker). LCOV files contain
whatever the coverage tool decided to write:

1. Absolute paths: `/home/alice/project/src/foo.rs`
2. Workspace-relative paths: `src/foo.rs`
3. Crate-relative paths in a workspace: `crates/core/src/foo.rs`
4. Paths with `./` or `../` components

A naïve `HashMap<PathBuf, _>` lookup silently returns `None` for 100% of
files when the two don't agree, and every function reports as 0% covered.
`cargo-crap` handles this with a two-level index:

- Absolute coverage paths → direct canonical-path hash lookup.
- Relative coverage paths → suffix match on path components, not bytes:
  `/foo/bar.rs` must not match `oofoo/bar.rs`.

Ambiguous inputs resolve deterministically (spec 26): when several
relative keys suffix-match one file (`src/lib.rs` vs
`vendor/dep/src/lib.rs`), the longest and most specific suffix wins, and
different spellings of the same file (symlinked roots, `lcov -a`-merged
legs, `./`-prefixed variants) merge their line data instead of racing on
map order.

Relative paths are **never** canonicalized against the process's CWD, which
would otherwise silently bind them to whatever file happened to exist
under the tool's working directory. The regression test
`relative_coverage_paths_are_not_resolved_against_cwd` in `src/merge.rs`
pins this.
