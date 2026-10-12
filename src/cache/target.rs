//! Where the caches live: one resolver for every cache under
//! `<target>/cargo-crap/` (spec 10).

use std::path::{Path, PathBuf};

/// The two variables cargo reads for its target directory, in its order.
const TARGET_DIR_VARS: [&str; 2] = ["CARGO_TARGET_DIR", "CARGO_BUILD_TARGET_DIR"];

/// The target directory the caches live under, or `None` when there is none
/// to use.
///
/// `metadata` is the `target_directory` `cargo metadata` reported, when the
/// run asked it (`--workspace`, `-p`): cargo has applied every variable and
/// `.cargo/config.toml` already, so it wins outright. Otherwise, without
/// running cargo: `CARGO_TARGET_DIR`, then `CARGO_BUILD_TARGET_DIR` (read
/// through `lookup`, an empty value counting as unset, a relative one taken
/// against `cwd`); else `target/` in the workspace root found by walking up
/// from `analysed`; else `target/` beside the configuration in `config_dir`.
#[must_use]
pub fn target_dir(
    metadata: Option<&Path>,
    analysed: &Path,
    config_dir: Option<&Path>,
    cwd: &Path,
    lookup: impl Fn(&str) -> Option<String>,
) -> Option<PathBuf> {
    if let Some(dir) = metadata {
        return Some(dir.to_path_buf());
    }
    let from_env = TARGET_DIR_VARS
        .iter()
        .find_map(|name| lookup(name).filter(|value| !value.is_empty()));
    if let Some(dir) = from_env {
        return Some(cwd.join(dir));
    }
    let start = cwd.join(analysed);
    let start = std::fs::canonicalize(&start).unwrap_or(start);
    workspace_root(&start)
        .or(config_dir)
        .map(|root| root.join("target"))
}

/// The root cargo would build `start` from: the nearest ancestor whose
/// `Cargo.toml` has a `[workspace]` table, else the nearest ancestor with a
/// `Cargo.toml` at all (one that does not parse still counts).
fn workspace_root(start: &Path) -> Option<&Path> {
    let mut nearest = None;
    for dir in start.ancestors() {
        let Ok(manifest) = std::fs::read_to_string(dir.join("Cargo.toml")) else {
            continue;
        };
        if declares_workspace(&manifest) {
            return Some(dir);
        }
        nearest = nearest.or(Some(dir));
    }
    nearest
}

/// Whether `manifest` has a `[workspace]` table.
fn declares_workspace(manifest: &str) -> bool {
    manifest
        .parse::<toml::Table>()
        .is_ok_and(|table| table.contains_key("workspace"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    /// A lookup over `vars`, as the environment would answer.
    fn env(vars: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
        let vars: Vec<(String, String)> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        move |name| vars.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())
    }

    fn write(
        path: &Path,
        body: &str,
    ) {
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("mkdir");
        std::fs::write(path, body).expect("write");
    }

    const PACKAGE: &str = "[package]\nname = \"a\"\nversion = \"0.1.0\"\n";
    const WORKSPACE: &str = "[workspace]\nmembers = [\"crates/*\"]\n";

    #[test]
    fn the_metadata_target_directory_wins_over_everything() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = &dir.path().canonicalize().expect("canonical");
        write(&root.join("Cargo.toml"), WORKSPACE);
        let lookup = env(&[("CARGO_TARGET_DIR", "/elsewhere")]);
        let found = target_dir(Some(Path::new("/meta/out")), root, Some(root), root, lookup);
        assert_eq!(found, Some(PathBuf::from("/meta/out")));
    }

    #[test]
    fn cargo_target_dir_beats_cargo_build_target_dir_and_the_walk() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = &dir.path().canonicalize().expect("canonical");
        write(&root.join("Cargo.toml"), PACKAGE);
        // Absolute on every platform: `/a` has no drive on Windows, so it
        // would be taken against the working directory's drive.
        let (a, b) = (root.join("a"), root.join("b"));
        let (a_str, b_str) = (a.to_str().expect("utf-8"), b.to_str().expect("utf-8"));
        let both = env(&[
            ("CARGO_TARGET_DIR", a_str),
            ("CARGO_BUILD_TARGET_DIR", b_str),
        ]);
        assert_eq!(target_dir(None, root, None, root, both), Some(a.clone()));
        let build = env(&[("CARGO_BUILD_TARGET_DIR", b_str)]);
        assert_eq!(target_dir(None, root, None, root, build), Some(b.clone()));
    }

    #[test]
    fn a_relative_target_dir_resolves_against_the_working_directory() {
        let lookup = env(&[("CARGO_TARGET_DIR", "out")]);
        let found = target_dir(
            None,
            Path::new("/proj/src"),
            None,
            Path::new("/cwd"),
            lookup,
        );
        assert_eq!(found, Some(PathBuf::from("/cwd/out")));
    }

    #[test]
    fn an_empty_variable_counts_as_unset() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = &dir.path().canonicalize().expect("canonical");
        write(&root.join("Cargo.toml"), PACKAGE);
        let lookup = env(&[("CARGO_TARGET_DIR", ""), ("CARGO_BUILD_TARGET_DIR", "")]);
        assert_eq!(
            target_dir(None, root, None, root, lookup),
            Some(root.join("target"))
        );
    }

    #[test]
    fn a_workspace_ancestor_beats_a_nearer_package() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = &dir.path().canonicalize().expect("canonical");
        write(&root.join("Cargo.toml"), WORKSPACE);
        write(&root.join("crates/a/Cargo.toml"), PACKAGE);
        let member = root.join("crates/a");
        std::fs::create_dir_all(member.join("src")).expect("mkdir");
        let found = target_dir(None, &member.join("src"), None, &member, env(&[]));
        assert_eq!(found, Some(root.join("target")));
    }

    #[test]
    fn without_a_workspace_the_nearest_package_is_used() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = &dir.path().canonicalize().expect("canonical");
        write(&root.join("crates/a/Cargo.toml"), PACKAGE);
        let member = root.join("crates/a");
        let found = target_dir(None, &member, Some(root), root, env(&[]));
        assert_eq!(found, Some(member.join("target")));
    }

    #[test]
    fn a_relative_analysed_path_is_walked_from_the_working_directory() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = &dir.path().canonicalize().expect("canonical");
        write(&root.join("Cargo.toml"), PACKAGE);
        std::fs::create_dir_all(root.join("src")).expect("mkdir");
        let found = target_dir(None, Path::new(".."), None, &root.join("src"), env(&[]));
        assert_eq!(found, Some(root.join("target")));
    }

    #[test]
    fn a_cargo_toml_that_does_not_parse_still_marks_a_package() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = &dir.path().canonicalize().expect("canonical");
        write(&root.join("Cargo.toml"), "this is [not toml");
        assert_eq!(
            target_dir(None, root, None, root, env(&[])),
            Some(root.join("target"))
        );
    }

    #[test]
    fn with_no_cargo_toml_the_config_directory_is_used_else_none() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = &dir.path().canonicalize().expect("canonical");
        let config = Path::new("/config/dir");
        assert_eq!(
            target_dir(None, root, Some(config), root, env(&[])),
            Some(config.join("target"))
        );
        assert_eq!(target_dir(None, root, None, root, env(&[])), None);
    }

    /// What each directory level of a generated layout holds.
    #[derive(Debug, Clone, Copy, PartialEq)]
    enum Level {
        Nothing,
        Package,
        Workspace,
    }

    fn level() -> impl Strategy<Value = Level> {
        prop_oneof![
            Just(Level::Nothing),
            Just(Level::Package),
            Just(Level::Workspace)
        ]
    }

    /// Unset, empty, or `name`: relative, so it resolves against the working
    /// directory the same way on every platform.
    fn var(name: &'static str) -> impl Strategy<Value = Option<&'static str>> {
        prop_oneof![Just(None), Just(Some("")), Just(Some(name))]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        #[test]
        fn the_resolver_follows_its_precedence(
            levels in proptest::collection::vec(level(), 1..5),
            metadata in proptest::option::of(Just("/meta")),
            target in var("target-var"),
            build in var("build-var"),
            config in proptest::option::of(Just("/config")),
        ) {
            // A chain of nested directories l0/l1/…, each holding what
            // `levels` says; the run analyses the deepest one.
            let dir = tempfile::tempdir().expect("temp dir");
            let mut path = dir.path().canonicalize().expect("canonical");
            let mut chain = Vec::new();
            for (i, level) in levels.iter().enumerate() {
                path = path.join(format!("l{i}"));
                std::fs::create_dir_all(&path).expect("mkdir");
                match level {
                    Level::Nothing => {},
                    Level::Package => write(&path.join("Cargo.toml"), PACKAGE),
                    Level::Workspace => write(&path.join("Cargo.toml"), WORKSPACE),
                }
                chain.push(path.clone());
            }
            let mut vars = Vec::new();
            if let Some(v) = target { vars.push(("CARGO_TARGET_DIR", v)); }
            if let Some(v) = build { vars.push(("CARGO_BUILD_TARGET_DIR", v)); }

            let found = target_dir(
                metadata.map(Path::new),
                &path,
                config.map(Path::new),
                dir.path(),
                env(&vars),
            );

            let set = |v: Option<&str>| v.filter(|v| !v.is_empty()).map(|v| dir.path().join(v));
            let workspace = chain.iter().zip(&levels).rev()
                .find(|(_, l)| **l == Level::Workspace).map(|(p, _)| p.join("target"));
            let package = chain.iter().zip(&levels).rev()
                .find(|(_, l)| **l != Level::Nothing).map(|(p, _)| p.join("target"));
            let expected = metadata.map(PathBuf::from)
                .or_else(|| set(target))
                .or_else(|| set(build))
                .or(workspace)
                .or(package)
                .or_else(|| config.map(|c| Path::new(c).join("target")));
            prop_assert_eq!(found, expected);
        }
    }
}
