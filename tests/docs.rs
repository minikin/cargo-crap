//! Documentation claims that can drift from the code: the flags table against
//! `--help`, the configuration reference against `Config`, and sample outputs
//! against a real run of the fixture project.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;
use cargo_crap::config::{Config, DEFAULT_DUP_THRESHOLD};
use cargo_crap::delta::DEFAULT_EPSILON;
use cargo_crap::merge::SortOrder;
use cargo_crap::score::DEFAULT_THRESHOLD;

const MARKER_OPEN: &str = "<!-- output:";
const MARKER_CLOSE: &str = "-->";

fn repo_path(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(relative)
}

fn read(relative: &str) -> String {
    fs::read_to_string(repo_path(relative)).unwrap_or_else(|e| panic!("reading {relative}: {e}"))
}

/// The binary, with `COLUMNS` cleared so the caller's shell cannot narrow
/// the sample output under test.
fn cmd() -> Command {
    let mut cmd = Command::cargo_bin("cargo-crap").expect("binary must be built");
    cmd.env_remove("COLUMNS");
    cmd
}

/// One option as `--help` prints it.
struct HelpOption {
    long: String,
    takes_value: bool,
    default: Option<String>,
}

/// Every option in `--help`, except clap's own `--help` and `--version`.
fn help_options() -> Vec<HelpOption> {
    let out = cmd().arg("--help").output().unwrap();
    assert!(out.status.success());
    let help = String::from_utf8(out.stdout).unwrap();
    let mut options: Vec<HelpOption> = Vec::new();
    for line in help.lines() {
        // Option lines are indented by two or six spaces, descriptions by ten.
        let indent = line.len() - line.trim_start().len();
        if indent <= 6 && line.trim_start().starts_with('-') {
            let (_, after) = line.split_once("--").unwrap();
            let long = after.split_whitespace().next().unwrap().to_owned();
            let takes_value = after.contains('<');
            options.push(HelpOption {
                long,
                takes_value,
                default: None,
            });
        } else if let (Some(option), Some(rest)) =
            (options.last_mut(), line.trim().strip_prefix("[default: "))
        {
            option.default = Some(rest.trim_end_matches(']').to_owned());
        }
    }
    options.retain(|o| o.long != "help" && o.long != "version");
    options
}

/// The default the flags table must state for an option. clap prints a
/// default only where it applies one; the rest fall back to the config file
/// and then to these built-in values.
fn expected_default(option: &HelpOption) -> String {
    if let Some(default) = &option.default {
        return default.clone();
    }
    match option.long.as_str() {
        "threshold" => DEFAULT_THRESHOLD.to_string(),
        "epsilon" => DEFAULT_EPSILON.to_string(),
        "dup-threshold" => DEFAULT_DUP_THRESHOLD.to_string(),
        "sort" => format!("{:?}", SortOrder::default()).to_lowercase(),
        "missing" => "pessimistic".to_owned(),
        "jobs" => "host CPUs".to_owned(),
        _ if option.takes_value => "none".to_owned(),
        _ => "off".to_owned(),
    }
}

/// The first fenced block in `markdown` that follows a line equal to `opener`.
fn fenced_block_after(
    markdown: &str,
    opener: &str,
) -> String {
    let mut lines = markdown.lines().skip_while(|l| *l != opener).skip(1);
    lines.by_ref().find(|l| l.starts_with("```")).unwrap();
    let body: Vec<&str> = lines.take_while(|l| !l.starts_with("```")).collect();
    format!("{}\n", body.join("\n"))
}

/// `(long flag, default cell)` for each row of the flags table.
fn flags_table_rows() -> BTreeMap<String, String> {
    let page = read("docs/reference/cli.md");
    let mut rows = BTreeMap::new();
    for line in page.lines().filter(|l| l.starts_with("| `-")) {
        let cells: Vec<&str> = line.split('|').map(str::trim).collect();
        let (_, after) = cells[1].split_once("--").unwrap();
        let long: String = after
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        let default = cells[2].trim_matches('`').to_owned();
        assert!(
            rows.insert(long.clone(), default).is_none(),
            "--{long} has two rows"
        );
    }
    rows
}

#[test]
fn every_flag_in_help_has_a_row_in_the_flags_table() {
    let in_help: BTreeSet<String> = help_options().into_iter().map(|o| o.long).collect();
    let in_table: BTreeSet<String> = flags_table_rows().into_keys().collect();
    assert_eq!(
        in_table, in_help,
        "docs/reference/cli.md flags table vs --help"
    );
}

#[test]
fn the_flags_table_states_the_default_the_code_uses() {
    let rows = flags_table_rows();
    for option in help_options() {
        let stated = rows.get(&option.long).map(String::as_str);
        let expected = expected_default(&option);
        assert_eq!(
            stated,
            Some(expected.as_str()),
            "default of --{}",
            option.long
        );
    }
}

/// Every key `.cargo-crap.toml` accepts, as dotted paths in kebab-case,
/// read from the field lists of `Config` and the tables nested in it.
fn config_keys() -> BTreeSet<String> {
    let file = syn::parse_file(&read("src/config.rs")).unwrap();
    let structs: BTreeMap<String, Vec<(String, String)>> = file
        .items
        .iter()
        .filter_map(|item| match item {
            syn::Item::Struct(s) => Some((s.ident.to_string(), struct_fields(s))),
            _ => None,
        })
        .collect();
    let mut keys = BTreeSet::new();
    collect_keys("", "Config", &structs, &mut keys);
    keys
}

/// `(field name, last segment of its type)` for each named field.
fn struct_fields(item: &syn::ItemStruct) -> Vec<(String, String)> {
    item.fields
        .iter()
        .filter_map(|field| {
            let syn::Type::Path(ty) = &field.ty else {
                return None;
            };
            let name = field.ident.as_ref()?.to_string();
            Some((name, ty.path.segments.last()?.ident.to_string()))
        })
        .collect()
}

fn collect_keys(
    prefix: &str,
    name: &str,
    structs: &BTreeMap<String, Vec<(String, String)>>,
    keys: &mut BTreeSet<String>,
) {
    for (field, ty) in &structs[name] {
        let key = format!("{prefix}{}", field.replace('_', "-"));
        if structs.contains_key(ty) {
            collect_keys(&format!("{key}."), ty, structs, keys);
        } else {
            keys.insert(key);
        }
    }
}

fn config_example() -> String {
    fenced_block_after(&read("docs/reference/config.md"), "# Configuration file")
}

/// Dotted key paths of a TOML table, `snake_case` aliases spelled in kebab-case.
fn toml_keys(
    table: &toml::Table,
    prefix: &str,
    keys: &mut BTreeSet<String>,
) {
    for (key, value) in table {
        let key = format!("{prefix}{}", key.replace('_', "-"));
        match value {
            toml::Value::Table(nested) => toml_keys(nested, &format!("{key}."), keys),
            _ => {
                keys.insert(key);
            },
        }
    }
}

#[test]
fn the_configuration_example_sets_every_key() {
    let example: toml::Table = config_example().parse().unwrap();
    let mut documented = BTreeSet::new();
    toml_keys(&example, "", &mut documented);
    assert_eq!(
        documented,
        config_keys(),
        "docs/reference/config.md example vs Config"
    );
}

#[test]
fn the_configuration_example_is_a_valid_config() {
    let parsed: Result<Config, _> = toml::from_str(&config_example());
    assert!(parsed.is_ok(), "{:?}", parsed.err());
}

#[test]
fn the_flag_to_key_table_names_real_flags_and_keys() {
    let page = read("docs/reference/config.md");
    let flags: BTreeSet<String> = help_options().into_iter().map(|o| o.long).collect();
    let keys = config_keys();
    let rows = page
        .lines()
        .filter(|l| l.starts_with("| `--") || l.starts_with("| *(no "));
    for row in rows {
        let cells: Vec<&str> = row.split('|').map(str::trim).collect();
        let quoted = cells[2].split('`').skip(1).step_by(2);
        if cells[1] == "*(no key)*" {
            for flag in quoted.filter_map(|q| q.strip_prefix("--")) {
                assert!(flags.contains(flag), "no flag --{flag}");
            }
            continue;
        }
        if let Some(flag) = cells[1].strip_prefix("`--") {
            assert!(
                flags.contains(flag.trim_end_matches('`')),
                "no flag {}",
                cells[1]
            );
        }
        let key = quoted.into_iter().next().unwrap();
        let known = match key.strip_suffix(".*") {
            Some(table) => keys.iter().any(|k| k.starts_with(&format!("{table}."))),
            None => keys.contains(key),
        };
        assert!(known, "no config key {key}");
    }
}

/// Markdown files a reader sees: the README and every page under docs/.
fn documentation_files() -> Vec<PathBuf> {
    let mut files = vec![repo_path("README.md")];
    let mut dirs = vec![repo_path("docs")];
    while let Some(dir) = dirs.pop() {
        for entry in fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "md") {
                files.push(path);
            }
        }
    }
    files
}

/// Runs the binary in the fixture project, as a reader would in theirs.
/// The docs show Unix paths; on Windows the same run prints `.\src\lib.rs`,
/// which has the same width, so only the separator is normalized.
fn fixture_run(args: &str) -> String {
    let out = cmd()
        .current_dir(repo_path("tests/fixtures/sample_project"))
        .args(args.split_whitespace())
        .env("NO_COLOR", "1")
        .env_remove("FORCE_COLOR")
        .output()
        .unwrap();
    let stdout = String::from_utf8(out.stdout).unwrap();
    if cfg!(windows) {
        stdout.replace('\\', "/")
    } else {
        stdout
    }
}

#[test]
fn every_marked_sample_output_matches_a_real_run() {
    let mut checked = 0;
    for file in documentation_files() {
        let markdown = fs::read_to_string(&file).unwrap();
        for line in markdown.lines() {
            let Some(args) = line
                .strip_prefix(MARKER_OPEN)
                .and_then(|r| r.strip_suffix(MARKER_CLOSE))
            else {
                continue;
            };
            let shown = fenced_block_after(&markdown, line);
            assert_eq!(
                shown,
                fixture_run(args),
                "{} `{}`",
                file.display(),
                args.trim()
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 2,
        "expected the README and getting-started samples"
    );
}
