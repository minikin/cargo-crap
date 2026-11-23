use crate::pages::{self, Page};
use serde_json::Value;
use std::collections::BTreeSet;

pub const INSTALL: &str = "docs/crapts/install.md";

pub const COMMANDS: [&str; 4] = [
    "npx @minikin/crapts",
    "npm install -D @minikin/crapts",
    "cargo binstall crapts",
    "cargo install --locked crapts",
];

pub fn package_json() -> Value {
    serde_json::from_str(&pages::read("npm/crapts/package.json")).unwrap()
}

pub fn rust_version() -> String {
    let manifest: toml::Table = pages::read("crates/crapts/Cargo.toml").parse().unwrap();
    manifest["package"]["rust-version"]
        .as_str()
        .unwrap()
        .to_owned()
}

pub fn check_install(
    page: &Page,
    package: &Value,
    rust_version: &str,
) -> Result<(), String> {
    let lines = pages::command_lines(page);
    for command in COMMANDS {
        if !lines.iter().any(|(_, l)| l == command) {
            return Err(format!("{}: no command line `{command}`", page.path));
        }
    }
    let name = package["name"].as_str().unwrap_or_default();
    for (number, line) in lines
        .iter()
        .filter(|(_, l)| l.starts_with("npm ") || l.starts_with("npx "))
    {
        let Some(named) = line.split_whitespace().find(|w| w.starts_with('@')) else {
            continue;
        };
        let named = named
            .rsplit_once('@')
            .filter(|(p, _)| !p.is_empty())
            .map_or(named, |(p, _)| p);
        if named != name {
            return Err(format!(
                "{}: `{line}` names {named}, package.json names {name}",
                page.at(*number)
            ));
        }
    }
    check_platforms(page, package)?;
    check_rust_version(page, rust_version)
}

fn check_platforms(
    page: &Page,
    package: &Value,
) -> Result<(), String> {
    let prefix = format!("{}-", package["name"].as_str().unwrap_or_default());
    let listed: BTreeSet<String> = pages::prose_code(page)
        .into_iter()
        .map(|(_, code)| code)
        .filter(|code| code.starts_with(&prefix))
        .collect();
    let published: BTreeSet<String> = package["optionalDependencies"]
        .as_object()
        .map_or_default(|deps| deps.keys().cloned().collect());
    let missing: Vec<_> = published.difference(&listed).collect();
    let extra: Vec<_> = listed.difference(&published).collect();
    if missing.is_empty() && extra.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{}: the platform packages lack {missing:?} and list {extra:?}",
            page.path
        ))
    }
}

fn check_rust_version(
    page: &Page,
    rust_version: &str,
) -> Result<(), String> {
    let stated = page
        .lines()
        .into_iter()
        .filter(|l| !l.fenced && l.text.contains("minimum Rust version"))
        .find_map(|l| {
            pages::inline_code(l.text)
                .first()
                .map(|c| (l.number, (*c).to_owned()))
        })
        .ok_or_else(|| format!("{}: no minimum Rust version", page.path))?;
    if stated.1 == rust_version {
        Ok(())
    } else {
        Err(format!(
            "{}: the minimum Rust version reads {}, Cargo.toml says {rust_version}",
            page.at(stated.0),
            stated.1
        ))
    }
}
