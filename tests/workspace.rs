//! The Cargo workspace: its members, its lock, the standalone packages
//! beside it and what cargo-crap's published package holds. Every check is
//! a cargo call on the filesystem and reads no git metadata, because
//! cargo-mutants runs the tests in a scratch copy without `.git`.

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
fn the_workspace_has_cargo_crap_and_an_empty_cargo_crap_core() {
    // Given the repository at the root
    // When cargo metadata runs there
    let metadata = metadata(&root(), &["--locked"]);

    // Then its members are exactly cargo-crap and cargo-crap-core
    assert_eq!(member_names(&metadata), ["cargo-crap", "cargo-crap-core"]);

    // And the core is 0.1.0, edition 2024, rust-version 1.88, not published
    let core = package(&metadata, "cargo-crap-core");
    assert_eq!(core["version"], "0.1.0");
    assert_eq!(core["edition"], "2024");
    assert_eq!(core["rust_version"], "1.88");
    assert_eq!(core["publish"], serde_json::json!([]), "publish = false");

    // And it has no dependencies
    assert_eq!(core["dependencies"], serde_json::json!([]));

    // And cargo-crap's version differs from the core's. The goldens record
    // the report version as a placeholder only when it equals cargo-crap's,
    // so they catch a report stamped by the core only while the two differ.
    let tool = package(&metadata, "cargo-crap");
    assert_ne!(tool["version"], core["version"]);

    // And the core owns its version rather than inheriting one
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

    // And every feature choice stays on the package that makes it
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
fn the_lock_holds_the_core_with_no_dependencies() {
    // Given the committed Cargo.lock
    let lock: toml::Table = std::fs::read_to_string(root().join("Cargo.lock"))
        .expect("reading Cargo.lock")
        .parse()
        .expect("Cargo.lock is TOML");

    // When its packages are read
    let packages = lock["package"].as_array().expect("package array");
    let named = |name: &str| -> Vec<&toml::Table> {
        packages
            .iter()
            .filter_map(toml::Value::as_table)
            .filter(|p| p["name"].as_str() == Some(name))
            .collect()
    };

    // Then it has one cargo-crap-core entry, 0.1.0, with no dependencies
    let core = named("cargo-crap-core");
    assert_eq!(core.len(), 1, "cargo-crap-core entries in Cargo.lock");
    assert_eq!(core[0]["version"].as_str(), Some("0.1.0"));
    assert!(
        core[0].get("dependencies").is_none(),
        "cargo-crap-core has dependencies in Cargo.lock: {:?}",
        core[0].get("dependencies")
    );

    // And one cargo-crap entry, at the root package's version
    let tool = named("cargo-crap");
    assert_eq!(tool.len(), 1, "cargo-crap entries in Cargo.lock");
    assert_eq!(tool[0]["version"].as_str(), Some(env!("CARGO_PKG_VERSION")));
}

#[test]
fn the_example_packages_resolve_on_their_own() {
    for example in ["examples/hero-shop", "examples/triage-demo"] {
        // Given an example package without a [workspace] table
        let dir = root().join(example);
        assert!(
            !manifest(&dir).contains_key("workspace"),
            "{example} declares a workspace"
        );

        // When cargo metadata runs in its directory
        // Then it exits 0 and its workspace root is that directory
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
    // Given the repository
    // When cargo package lists cargo-crap's files
    let listing = cargo(
        &root(),
        &["package", "--list", "-p", "cargo-crap", "--allow-dirty"],
    );
    let paths: Vec<String> = listing.lines().map(|l| l.replace('\\', "/")).collect();

    // Then it lists the manifest and both crate roots
    for expected in ["Cargo.toml", "src/main.rs", "src/lib.rs"] {
        assert!(
            paths.iter().any(|p| p == expected),
            "{expected} not packaged"
        );
    }

    // And nothing from the core or the gate scripts
    let leaked: Vec<&String> = paths
        .iter()
        .filter(|p| p.starts_with("crates/") || p.starts_with("scripts/gates/"))
        .collect();
    assert!(leaked.is_empty(), "packaged: {leaked:?}");
}
