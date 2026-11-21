//! The Cargo workspace: its members, its lock, the standalone packages
//! beside it and what cargo-crap's published package holds. Every check is
//! a cargo call on the filesystem and reads no git metadata, because
//! cargo-mutants runs the tests in a scratch copy without `.git`.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn cargo(
    dir: &Path,
    args: &[&str],
) -> String {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let output = Command::new(&cargo)
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("running {cargo} {args:?}: {e}"));
    assert!(
        output.status.success(),
        "{cargo} {args:?} in {} failed: {}",
        dir.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).expect("utf-8 cargo output")
}

fn metadata(
    dir: &Path,
    extra: &[&str],
) -> Value {
    let mut args = vec!["metadata", "--no-deps", "--format-version", "1"];
    args.extend_from_slice(extra);
    serde_json::from_str(&cargo(dir, &args)).expect("cargo metadata prints JSON")
}

fn manifest(dir: &Path) -> toml::Table {
    let path = dir.join("Cargo.toml");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
        .parse()
        .unwrap_or_else(|e| panic!("{} is not TOML: {e}", path.display()))
}

fn package<'a>(
    metadata: &'a Value,
    name: &str,
) -> &'a Value {
    metadata["packages"]
        .as_array()
        .expect("packages array")
        .iter()
        .find(|p| p["name"] == name)
        .unwrap_or_else(|| panic!("no package {name} in cargo metadata"))
}

fn member_names(metadata: &Value) -> Vec<String> {
    let members = metadata["workspace_members"]
        .as_array()
        .expect("workspace_members array");
    let mut names: Vec<String> = metadata["packages"]
        .as_array()
        .expect("packages array")
        .iter()
        .filter(|p| members.contains(&p["id"]))
        .map(|p| p["name"].as_str().expect("package name").to_owned())
        .collect();
    names.sort();
    names
}

fn same_dir(
    a: &Path,
    b: &Path,
) -> bool {
    a.canonicalize().expect("canonical path") == b.canonicalize().expect("canonical path")
}

#[test]
fn the_workspace_has_cargo_crap_and_cargo_crap_core() {
    let metadata = metadata(&root(), &["--locked"]);

    assert_eq!(
        member_names(&metadata),
        ["cargo-crap", "cargo-crap-core", "crapts"]
    );

    let core = package(&metadata, "cargo-crap-core");
    assert_eq!(core["version"], "0.1.0");
    assert_eq!(core["edition"], "2024");
    assert_eq!(core["rust_version"], "1.88");
    assert_eq!(core["publish"], serde_json::json!([]), "publish = false");

    // And cargo-crap's version differs from the core's. The goldens record
    // the report version as a placeholder only when it equals cargo-crap's,
    // so they catch a report stamped by the core only while the two differ.
    let tool = package(&metadata, "cargo-crap");
    assert_ne!(tool["version"], core["version"]);

    let core_manifest = manifest(&root().join("crates/cargo-crap-core"));
    assert_eq!(
        core_manifest["package"]["version"].as_str(),
        Some("0.1.0"),
        "the core's [package].version is not the literal 0.1.0"
    );
    let root_manifest = manifest(&root());
    let shared = root_manifest["workspace"]["package"]
        .as_table()
        .expect("[workspace.package]");
    assert!(
        !shared.contains_key("version"),
        "[workspace.package] declares a version"
    );

    let featured: Vec<&String> = root_manifest["workspace"]["dependencies"]
        .as_table()
        .expect("[workspace.dependencies]")
        .iter()
        .filter(|(_, spec)| spec.get("features").is_some())
        .map(|(name, _)| name)
        .collect();
    assert!(
        featured.is_empty(),
        "[workspace.dependencies] entries with features: {featured:?}"
    );
}

#[test]
fn add_the_analysis_library_and_its_pinned_dependencies() {
    let metadata = metadata(&root(), &["--locked"]);
    let analysis = package(&metadata, "crapts");
    assert_eq!(analysis["publish"], serde_json::json!([]));
    assert_eq!(analysis["rust_version"], "1.98");
    let targets = analysis["targets"].as_array().expect("library targets");
    assert!(
        targets
            .iter()
            .any(|t| t["kind"] == serde_json::json!(["lib"]))
    );
    assert!(
        targets
            .iter()
            .all(|t| t["kind"] != serde_json::json!(["bin"]))
    );

    let shared = manifest(&root());
    let member = manifest(&root().join("crates/crapts"));
    let pins = [
        "oxc_allocator",
        "oxc_ast",
        "oxc_ast_visit",
        "oxc_parser",
        "oxc_span",
        "oxc_str",
        "oxc_syntax",
    ];
    let dependencies = analysis["dependencies"]
        .as_array()
        .expect("Cargo dependencies");
    let declared: BTreeSet<_> = dependencies
        .iter()
        .filter(|dep| dep["kind"] != "dev")
        .map(|dep| {
            dep["rename"]
                .as_str()
                .or_else(|| dep["name"].as_str())
                .expect("dependency name")
        })
        .collect();
    assert_eq!(
        declared,
        BTreeSet::from(pins),
        "direct dependencies of crapts"
    );
    for name in pins {
        assert_eq!(
            shared["workspace"]["dependencies"][name].as_str(),
            Some("=0.159.0"),
            "{name}"
        );
        assert_eq!(
            member["dependencies"][name]["workspace"].as_bool(),
            Some(true),
            "{name}"
        );
    }
}

#[derive(serde::Deserialize)]
struct LockedPackage {
    name: String,
    version: String,
    source: Option<String>,
    #[serde(default)]
    dependencies: Vec<String>,
}

impl LockedPackage {
    fn id(&self) -> String {
        let name_version = format!("{} {}", self.name, self.version);
        match &self.source {
            Some(source) => format!("{name_version} ({source})"),
            None => name_version,
        }
    }
}

fn resolve_locked<'a>(
    packages: &'a [LockedPackage],
    reference: &str,
) -> &'a LockedPackage {
    // Cargo adds a version, then a parenthesized source, only as needed to identify a dependency.
    let mut parts = reference.splitn(3, ' ');
    let name = parts.next().expect("package name");
    let version = parts.next();
    let source = parts.next().map(|s| {
        s.strip_prefix('(')
            .and_then(|s| s.strip_suffix(')'))
            .expect("parenthesized package source")
    });
    let mut matches = packages.iter().filter(|p| {
        p.name == name
            && version.is_none_or(|v| p.version == v)
            && source.is_none_or(|s| p.source.as_deref() == Some(s))
    });
    let package = matches
        .next()
        .unwrap_or_else(|| panic!("unresolved lockfile dependency {reference}"));
    assert!(
        matches.next().is_none(),
        "ambiguous lockfile dependency {reference}"
    );
    package
}

#[test]
fn the_lock_retains_the_spec_45_ids_and_adds_only_the_crapts_closure() {
    #[derive(serde::Deserialize)]
    struct Lockfile {
        package: Vec<LockedPackage>,
    }

    let base: BTreeSet<_> = include_str!("fixtures/spec45-package-ids.txt")
        .lines()
        .filter(|line| !line.starts_with('#'))
        .collect();
    let lock: Lockfile = toml::from_str(
        &std::fs::read_to_string(root().join("Cargo.lock")).expect("read Cargo.lock"),
    )
    .expect("parse Cargo.lock");
    let current: BTreeSet<_> = lock.package.iter().map(LockedPackage::id).collect();

    let mut closure = BTreeSet::new();
    let mut pending = vec!["crapts"];
    while let Some(edge) = pending.pop() {
        let package = resolve_locked(&lock.package, edge);
        if closure.insert(package.id()) {
            pending.extend(package.dependencies.iter().map(String::as_str));
        }
    }

    for id in &base {
        assert!(current.contains(*id), "missing Spec 45 package ID: {id}");
    }
    for id in &current {
        assert!(
            base.contains(id.as_str()) || closure.contains(id),
            "added package outside the crapts closure: {id}"
        );
    }
}

#[test]
fn the_root_resolves_exactly_the_local_core() {
    let declared = metadata(&root(), &["--locked"]);
    let core_edges: Vec<&Value> = package(&declared, "cargo-crap")["dependencies"]
        .as_array()
        .expect("dependencies array")
        .iter()
        .filter(|dep| dep["name"] == "cargo-crap-core")
        .collect();

    assert_eq!(core_edges.len(), 1, "{core_edges:#?}");
    let edge = core_edges[0];
    assert_eq!(edge["req"], "=0.1.0");
    assert_eq!(edge["kind"], Value::Null, "a normal dependency");
    assert_eq!(edge["optional"], false);

    let resolved: Value = serde_json::from_str(&cargo(
        &root(),
        &["metadata", "--format-version", "1", "--locked"],
    ))
    .expect("cargo metadata prints JSON");
    let core = package(&resolved, "cargo-crap-core");
    assert_eq!(core["version"], "0.1.0");
    assert_eq!(core["source"], Value::Null, "the core comes from a path");
    assert!(same_dir(
        Path::new(core["manifest_path"].as_str().expect("manifest path"))
            .parent()
            .expect("manifest directory"),
        &root().join("crates/cargo-crap-core")
    ));
    let tool_id = &package(&resolved, "cargo-crap")["id"];
    let tool_node = resolved["resolve"]["nodes"]
        .as_array()
        .expect("resolve nodes")
        .iter()
        .find(|node| node["id"] == *tool_id)
        .expect("cargo-crap's resolve node");
    let core_deps: Vec<&Value> = tool_node["deps"]
        .as_array()
        .expect("resolved deps")
        .iter()
        .filter(|dep| dep["pkg"] == core["id"])
        .collect();
    assert_eq!(core_deps.len(), 1, "{core_deps:#?}");
    assert_eq!(
        core_deps[0]["dep_kinds"],
        serde_json::json!([{ "kind": null, "target": null }])
    );

    let lock: toml::Table = std::fs::read_to_string(root().join("Cargo.lock"))
        .expect("reading Cargo.lock")
        .parse()
        .expect("Cargo.lock is TOML");
    let packages = lock["package"].as_array().expect("package array");
    let named = |name: &str| -> Vec<&toml::Table> {
        packages
            .iter()
            .filter_map(toml::Value::as_table)
            .filter(|p| p["name"].as_str() == Some(name))
            .collect()
    };
    let locked_core = named("cargo-crap-core");
    assert_eq!(
        locked_core.len(),
        1,
        "cargo-crap-core entries in Cargo.lock"
    );
    assert_eq!(locked_core[0]["version"].as_str(), Some("0.1.0"));
    assert!(
        locked_core[0].get("source").is_none(),
        "{:?}",
        locked_core[0]
    );
    let core_dependencies = locked_core[0]
        .get("dependencies")
        .and_then(toml::Value::as_array)
        .expect("cargo-crap-core lists its dependencies in Cargo.lock");
    assert!(
        core_dependencies.iter().any(|d| d.as_str() == Some("lcov")),
        "{core_dependencies:?}"
    );

    let tool = named("cargo-crap");
    assert_eq!(tool.len(), 1, "cargo-crap entries in Cargo.lock");
    assert_eq!(tool[0]["version"].as_str(), Some(env!("CARGO_PKG_VERSION")));
    assert!(
        tool[0]["dependencies"]
            .as_array()
            .expect("cargo-crap's locked dependencies")
            .iter()
            .any(|d| d.as_str() == Some("cargo-crap-core")),
        "cargo-crap does not list cargo-crap-core in Cargo.lock"
    );
}

/// Every file below `dir`, as its path relative to `dir` with `/` separators.
fn files_under(dir: &Path) -> Vec<String> {
    fn walk(
        base: &Path,
        dir: &Path,
        found: &mut Vec<String>,
    ) {
        let entries =
            std::fs::read_dir(dir).unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()));
        for entry in entries {
            let path = entry.expect("directory entry").path();
            if path.is_dir() {
                walk(base, &path, found);
            } else {
                let relative = path.strip_prefix(base).expect("under the base");
                found.push(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut found = Vec::new();
    walk(dir, dir, &mut found);
    found.sort();
    found
}

fn package_identity_reads(src: &Path) -> Vec<String> {
    files_under(src)
        .into_iter()
        .filter(|file| {
            std::fs::read_to_string(src.join(file))
                .unwrap_or_else(|e| panic!("reading {file}: {e}"))
                .contains("CARGO_PKG_")
        })
        .map(|file| format!("{file}: CARGO_PKG_"))
        .collect()
}

#[test]
fn core_source_cannot_read_cargo_s_package_identity() {
    let dir = tempfile::tempdir().expect("tempdir");
    let src = dir.path().join("src");
    std::fs::create_dir_all(src.join("report")).expect("fixture directories");
    std::fs::write(src.join("lib.rs"), "pub mod report;\n").expect("fixture lib.rs");
    std::fs::write(
        src.join("report/json.rs"),
        "pub const VERSION: &str = env!(\"CARGO_PKG_VERSION\");\n",
    )
    .expect("fixture json.rs");

    assert_eq!(package_identity_reads(&src), ["report/json.rs: CARGO_PKG_"]);

    assert_eq!(
        package_identity_reads(&root().join("crates/cargo-crap-core/src")),
        Vec::<String>::new()
    );
}

#[test]
fn the_example_packages_resolve_on_their_own() {
    for example in ["examples/hero-shop", "examples/triage-demo"] {
        let dir = root().join(example);
        assert!(
            !manifest(&dir).contains_key("workspace"),
            "{example} declares a workspace"
        );

        let root_dir = metadata(&dir, &[])["workspace_root"]
            .as_str()
            .expect("workspace_root")
            .to_owned();
        assert!(
            same_dir(Path::new(&root_dir), &dir),
            "{example} resolves into the workspace at {root_dir}"
        );
    }
}

#[test]
fn cargo_crap_s_published_package_holds_no_core_and_no_gate_script() {
    let listing = cargo(
        &root(),
        &["package", "--list", "-p", "cargo-crap", "--allow-dirty"],
    );
    let paths: Vec<String> = listing.lines().map(|l| l.replace('\\', "/")).collect();

    for expected in ["Cargo.toml", "src/main.rs", "src/lib.rs"] {
        assert!(
            paths.iter().any(|p| p == expected),
            "{expected} not packaged"
        );
    }

    let leaked: Vec<&String> = paths
        .iter()
        .filter(|p| p.starts_with("crates/") || p.starts_with("scripts/gates/"))
        .collect();
    assert!(leaked.is_empty(), "packaged: {leaked:?}");
}

#[test]
fn mutants_of_the_core_run_the_whole_workspace_s_tests() {
    let path = root().join(".cargo/mutants.toml");
    let config: toml::Table = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
        .parse()
        .expect(".cargo/mutants.toml is TOML");
    assert_eq!(
        config.get("test_workspace"),
        Some(&toml::Value::Boolean(true))
    );
}

#[test]
fn the_core_dependency_graph_excludes_language_parsers() {
    let tree = cargo(
        &root(),
        &[
            "tree",
            "-p",
            "cargo-crap-core",
            "--all-features",
            "-e",
            "no-proc-macro",
            "--prefix",
            "none",
            "--locked",
        ],
    );
    let lines: Vec<&str> = tree.lines().collect();

    assert!(
        lines
            .first()
            .is_some_and(|l| l.starts_with("cargo-crap-core ")),
        "{tree}"
    );
    for dependency in ["lcov ", "comfy-table "] {
        assert!(
            lines.iter().any(|l| l.starts_with(dependency)),
            "no {dependency:?} line in:\n{tree}"
        );
    }
    let parsers: Vec<&&str> = lines
        .iter()
        .filter(|l| l.starts_with("syn ") || l.starts_with("oxc_"))
        .collect();
    assert!(parsers.is_empty(), "{parsers:?}");
}

/// Every recorded seed file, by the package directory that replays it and
/// its path under that package's `proptest-regressions/`. A new seed file
/// joins this list in the change that records it.
const SEEDS: &[(&str, &[&str])] = &[
    (
        "",
        &[
            "complexity.txt",
            "duplicates/fingerprint.txt",
            "duplicates/normalize.txt",
            "duplicates/triage/budget.txt",
            "duplicates/triage/cache.txt",
            "signals/compute.txt",
        ],
    ),
    (
        "crates/cargo-crap-core",
        &[
            "duplicates/compare.txt",
            "duplicates/group.txt",
            "duplicates/novelty.txt",
            "report/duplicates.txt",
            "report/human.txt",
            "report/pr_comment.txt",
            "report/types.txt",
        ],
    ),
];

/// The module whose properties replay `seed` in the package rooted at
/// `package`, when that module defines any.
fn seed_owner(
    package: &Path,
    seed: &str,
) -> Option<PathBuf> {
    let module = package.join("src").join(seed).with_extension("rs");
    std::fs::read_to_string(&module)
        .is_ok_and(|source| source.contains("proptest!"))
        .then_some(module)
}

#[test]
fn recorded_property_test_seeds_follow_the_tests_that_replay_them() {
    for (dir, seeds) in SEEDS {
        let package = root().join(dir);
        assert_eq!(
            files_under(&package.join("proptest-regressions")),
            *seeds,
            "the seed files of {}",
            package.display()
        );
        for seed in *seeds {
            assert!(
                seed_owner(&package, seed).is_some(),
                "no module in {} replays {seed}",
                package.display()
            );
        }
    }
}
