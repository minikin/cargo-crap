use crate::markers;
use crate::pages::{self, Page};
use std::collections::BTreeSet;
use std::path::{Component, Path};

pub const INDEX: &str = "docs/crapts/README.md";
pub const CI: &str = ".github/workflows/ci.yml";

fn normalized(path: &Path) -> String {
    let mut parts: Vec<String> = Vec::new();
    for component in path.components() {
        match component {
            Component::ParentDir => {
                parts.pop();
            },
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            _ => {},
        }
    }
    parts.join("/")
}

pub fn linked_files(page: &Page) -> BTreeSet<String> {
    pages::clickable_links(page)
        .into_iter()
        .filter_map(|(_, target)| {
            let file = target.split('#').next()?;
            (!file.is_empty()).then(|| normalized(&page.dir().join(file)))
        })
        .collect()
}

pub fn check_index(
    index: &Page,
    pages: &[Page],
) -> Result<(), String> {
    let linked = linked_files(index);
    let unlinked: Vec<&str> = pages
        .iter()
        .map(|p| p.path.as_str())
        .filter(|p| *p != index.path && !linked.contains(*p))
        .collect();
    if unlinked.is_empty() {
        Ok(())
    } else {
        Err(format!("{}: no link to {unlinked:?}", index.path))
    }
}

pub fn check_readme_section(readme: &Page) -> Result<(), String> {
    let section = pages::section(readme, pages::README_SECTION)?;
    let linked = linked_files(&section);
    for target in [INDEX, "docs/crapts/install.md"] {
        if !linked.contains(target) {
            return Err(format!(
                "{}: the crapts section does not link {target}",
                readme.path
            ));
        }
    }
    for line in section.lines() {
        let text = line.text.trim_start();
        if text.starts_with(markers::PREFIX) || text.starts_with(pages::ROOT_MARKER) {
            return Err(format!(
                "{}: the crapts section holds a marker",
                section.at(line.number)
            ));
        }
    }
    let installs: Vec<(usize, String)> = pages::command_lines(&section)
        .into_iter()
        .filter(|(_, l)| ["npx ", "npm ", "cargo "].iter().any(|p| l.starts_with(p)))
        .collect();
    if !installs.iter().any(|(_, l)| l == "npx @minikin/crapts") {
        return Err(format!(
            "{}: the crapts section has no install line `npx @minikin/crapts`",
            readme.path
        ));
    }
    for (line, text) in installs {
        let npx = text == "npx @minikin/crapts" || text.starts_with("npx @minikin/crapts ");
        if !npx {
            return Err(format!(
                "{}: the install line `{text}` is not npx @minikin/crapts",
                section.at(line)
            ));
        }
    }
    Ok(())
}

pub fn root_markers(pages: &[Page]) -> Vec<String> {
    pages
        .iter()
        .flat_map(|page| {
            page.lines()
                .into_iter()
                .filter(|l| l.text.starts_with(pages::ROOT_MARKER))
                .map(|l| page.at(l.number))
                .collect::<Vec<_>>()
        })
        .collect()
}

pub fn filter_globs(
    ci: &str,
    name: &str,
) -> Vec<String> {
    let header = format!("            {name}:");
    ci.lines()
        .skip_while(|l| *l != header)
        .skip(1)
        .take_while(|l| l.starts_with("              "))
        .filter_map(|l| l.trim().strip_prefix("- '")?.strip_suffix('\''))
        .map(str::to_owned)
        .collect()
}

/// picomatch keeps `*` inside one path component, so the globs are built
/// with a literal separator.
pub fn matches(
    globs: &[String],
    path: &str,
) -> bool {
    globs.iter().any(|g| {
        globset::GlobBuilder::new(g)
            .literal_separator(true)
            .build()
            .is_ok_and(|glob| glob.compile_matcher().is_match(path))
    })
}

pub fn job_runs(
    ci: &str,
    job: &str,
) -> Vec<String> {
    let header = format!("  {job}:");
    ci.lines()
        .skip_while(|l| *l != header)
        .skip(1)
        .take_while(|l| l.is_empty() || l.starts_with("    "))
        .filter_map(|l| l.trim().strip_prefix("run: ").map(str::to_owned))
        .collect()
}

fn cargo_test(command: &str) -> Option<Vec<&str>> {
    let words: Vec<&str> = command.split_whitespace().collect();
    let runs =
        words.first() == Some(&"cargo") && matches!(words.get(1), Some(&("test" | "nextest")));
    runs.then_some(words)
}

pub fn runs_crapts_docs(command: &str) -> bool {
    let Some(words) = cargo_test(command) else {
        return false;
    };
    let docs = words.windows(2).any(|w| w == ["--test", "docs"]);
    let all_targets = words.contains(&"--all-targets");
    let crapts = words.contains(&"--workspace") || words.windows(2).any(|w| w == ["-p", "crapts"]);
    crapts && (docs || all_targets)
}

pub fn runs_root_docs(command: &str) -> bool {
    let Some(words) = cargo_test(command) else {
        return false;
    };
    let docs =
        words.windows(2).any(|w| w == ["--test", "docs"]) || words.contains(&"--all-targets");
    let root = !words.contains(&"-p") || words.windows(2).any(|w| w == ["-p", "cargo-crap"]);
    docs && root
}

pub fn job_condition(
    ci: &str,
    job: &str,
) -> String {
    let header = format!("  {job}:");
    let mut lines = ci
        .lines()
        .skip_while(|l| *l != header)
        .skip(1)
        .take_while(|l| l.is_empty() || l.starts_with("    "));
    let Some(first) = lines.find_map(|l| l.trim().strip_prefix("if:")) else {
        return String::new();
    };
    let mut condition = first.trim().trim_start_matches(">-").trim().to_owned();
    for more in lines.take_while(|l| l.starts_with("      ")) {
        condition.push(' ');
        condition.push_str(more.trim());
    }
    condition.trim().to_owned()
}

/// Evaluates the `needs.changes.outputs.X == / != 'true'` comparisons
/// joined by `&&`, `||` and parentheses that the CI conditions use.
pub fn evaluate(
    condition: &str,
    set: &BTreeSet<&str>,
) -> Option<bool> {
    let spaced = condition.replace('(', " ( ").replace(')', " ) ");
    let tokens: Vec<&str> = spaced.split_whitespace().collect();
    let mut at = 0;
    let value = disjunction(&tokens, &mut at, set)?;
    (at == tokens.len()).then_some(value)
}

fn disjunction(
    tokens: &[&str],
    at: &mut usize,
    set: &BTreeSet<&str>,
) -> Option<bool> {
    let mut value = conjunction(tokens, at, set)?;
    while tokens.get(*at) == Some(&"||") {
        *at += 1;
        value |= conjunction(tokens, at, set)?;
    }
    Some(value)
}

fn conjunction(
    tokens: &[&str],
    at: &mut usize,
    set: &BTreeSet<&str>,
) -> Option<bool> {
    let mut value = comparison(tokens, at, set)?;
    while tokens.get(*at) == Some(&"&&") {
        *at += 1;
        value &= comparison(tokens, at, set)?;
    }
    Some(value)
}

fn comparison(
    tokens: &[&str],
    at: &mut usize,
    set: &BTreeSet<&str>,
) -> Option<bool> {
    if tokens.get(*at) == Some(&"(") {
        *at += 1;
        let value = disjunction(tokens, at, set)?;
        (tokens.get(*at) == Some(&")")).then_some(())?;
        *at += 1;
        return Some(value);
    }
    let output = tokens.get(*at)?.strip_prefix("needs.changes.outputs.")?;
    let equal = match *tokens.get(*at + 1)? {
        "==" => true,
        "!=" => false,
        _ => return None,
    };
    (tokens.get(*at + 2) == Some(&"'true'")).then_some(())?;
    *at += 3;
    Some(set.contains(output) == equal)
}
